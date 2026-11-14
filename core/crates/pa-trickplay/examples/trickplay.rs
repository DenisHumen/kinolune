// SPDX-License-Identifier: MPL-2.0
//! Generates previews for one file and measures what PLAY-009 promises:
//! time until previews cover the timeline, total generation time, and the
//! latency of `frame_at` lookups.
//!
//! ```text
//! PA_FFMPEG_LIB_DIR=… cargo run --release -p pa-trickplay --example trickplay -- \
//!     local /media "/Movies/Film.mkv"
//! PA_SMB2_LIB=… PA_SMB_USER=… PA_SMB_PASS=… cargo run --release -p pa-trickplay \
//!     --example trickplay -- smb 127.0.0.1:18545 share "/Movies/Film.mkv"
//! ```
//!
//! `PA_TRICKPLAY_OUT=<file.bif>` also writes the result; `PA_TRICKPLAY_WORKERS`
//! overrides the number of parallel grabbers.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use pa_trickplay::{GenerateOptions, Media, Status, Trickplay, TrickplayOptions};
use pa_vfs::{Vfs, VfsPath};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (vfs, path): (Arc<dyn Vfs>, VfsPath) = match args.first().map(String::as_str) {
        Some("local") if args.len() == 3 => (
            Arc::new(pa_vfs::LocalFs::new(&args[1])?),
            VfsPath::parse(&args[2])?,
        ),
        Some("smb") if args.len() == 4 => {
            let mut config = pa_vfs_smb::SmbConfig::new(args[1].clone(), args[2].clone());
            if let (Ok(user), Ok(pass)) =
                (std::env::var("PA_SMB_USER"), std::env::var("PA_SMB_PASS"))
            {
                config = config.with_credentials(pa_vfs_smb::Credentials::new(user, pass));
            }
            (
                Arc::new(pa_vfs_smb::SmbFs::new(config)?),
                VfsPath::parse(&args[3])?,
            )
        }
        _ => {
            eprintln!("usage: trickplay local <root> <path> | smb <host[:port]> <share> <path>");
            std::process::exit(2);
        }
    };

    let started = Instant::now();
    let counted = Arc::new(Counted {
        inner: Arc::from(vfs.open(&path)?),
        bytes: AtomicU64::new(0),
        reads: AtomicU64::new(0),
    });
    let source: Arc<dyn pa_vfs::ByteSource> = counted.clone();
    let mut generation = GenerateOptions::default();
    if let Some(n) = std::env::var("PA_TRICKPLAY_WORKERS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        generation.workers = n;
    }
    println!("workers: {}", generation.workers);
    let cache = tempfile::tempdir()?;
    let service = Trickplay::open(
        cache.path(),
        TrickplayOptions {
            generation,
            ..TrickplayOptions::default()
        },
    )?;
    let session = service.prepare(Media {
        key: format!("bench:{}", path.as_str()),
        source,
        sidecar: None,
        duration_ms: None,
    });

    let mut covered = None;
    let status = loop {
        let status = session.status();
        if let Status::Generating { done, total } = status {
            // Every 32nd slot done: previews across the whole timeline.
            if covered.is_none() && done >= total.div_ceil(32) {
                covered = Some(started.elapsed());
                println!(
                    "coverage of the timeline after {:.2?} ({done}/{total})",
                    started.elapsed()
                );
            }
        } else if !matches!(status, Status::Searching) {
            break status;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let total_time = started.elapsed();
    println!("status: {status:?} after {total_time:.2?}");
    println!(
        "read {} MiB in {} reads",
        counted.bytes.load(Ordering::Relaxed) >> 20,
        counted.reads.load(Ordering::Relaxed)
    );
    let interval = session.interval_ms().unwrap_or(10_000);
    println!("interval: {interval} ms");

    // Lookup latency over the whole range.
    let mut worst = Duration::ZERO;
    let mut sum = Duration::ZERO;
    let mut bytes = 0usize;
    let n = 2000u64;
    let span = interval * 800;
    for i in 0..n {
        let at = (i * 7919 % n) * span / n;
        let t = Instant::now();
        let frame = session.frame_at(at);
        let dt = t.elapsed();
        bytes += frame.map_or(0, |f| f.image.len());
        worst = worst.max(dt);
        sum += dt;
    }
    println!(
        "frame_at: {n} lookups, mean {:.1?}, worst {worst:.1?}, {} KiB served",
        sum / u32::try_from(n)?,
        bytes / 1024
    );
    if let Ok(out) = std::env::var("PA_TRICKPLAY_OUT")
        && let Some(bif) = service.cache().load(&format!("bench:{}", path.as_str()))
    {
        std::fs::write(&out, &bif)?;
        println!("wrote {out} ({} KiB)", bif.len() / 1024);
    }
    session.release();
    Ok(())
}

/// Counts what the generator reads from the source.
#[derive(Debug)]
struct Counted {
    inner: Arc<dyn pa_vfs::ByteSource>,
    bytes: AtomicU64,
    reads: AtomicU64,
}

impl pa_vfs::ByteSource for Counted {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> pa_vfs::Result<usize> {
        let n = self.inner.read_at(offset, buf)?;
        self.bytes.fetch_add(n as u64, Ordering::Relaxed);
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(n)
    }

    fn hint_random_access(&self) {
        self.inner.hint_random_access();
    }
}
