// SPDX-License-Identifier: MPL-2.0
//! Streaming speed of one file on a real SMB server (NET-021): reads it
//! sequentially in 256 KiB reads, as a player does, and prints the speed
//! every second with the tuned number of parallel blocks and the window.
//! For the owner's measurement on real Wi-Fi (`docs/qa/test-catalog.md`,
//! SRC-121); in Docker the same is `tests/servers/bench.sh`.
//!
//! ```text
//! PA_SMB2_LIB=…/libsmb2.so.1 PA_SMB_PASS=… \
//!   cargo run --release -p pa-vfs-smb --example smb_speed -- \
//!   <server> <share> </path/in/share/file.mkv> [user] [seconds] [fixed]
//! ```
//!
//! `seconds` (default 60) stops early on a long file; `fixed` (a number)
//! turns the tuning off and fetches that many blocks in parallel — `4` is
//! the behaviour before NET-021, for a before/after comparison.

use std::time::{Duration, Instant};

use pa_vfs::{ByteSource, Vfs, VfsPath};
use pa_vfs_smb::{Credentials, SmbConfig, SmbFs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [server, share, path, rest @ ..] = args.as_slice() else {
        return Err("usage: smb_speed <server> <share> <path> [user] [seconds] [fixed]".into());
    };
    let mut config = SmbConfig::new(server, share).with_timeout(Duration::from_secs(30));
    if let Some(user) = rest.first().filter(|u| !u.is_empty()) {
        let pass = std::env::var("PA_SMB_PASS").unwrap_or_default();
        config = config.with_credentials(Credentials::new(user, pass));
    }
    let limit = Duration::from_secs(rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(60));
    if let Some(n) = rest.get(2).and_then(|s| s.parse::<usize>().ok()) {
        config = config
            .with_prefetch_parallel(n)
            .with_max_prefetch_parallel(n)
            .with_max_connections(n + 2);
    }
    let fs = SmbFs::new(config)?;
    let src = fs.open(&VfsPath::parse(path)?)?;
    let mut buf = vec![0u8; 256 * 1024];
    let (mut off, mut tick_bytes) = (0u64, 0u64);
    let started = Instant::now();
    let mut tick = started;
    while off < src.size() && started.elapsed() < limit {
        let n = src.read_at(off, &mut buf)?;
        if n == 0 {
            break;
        }
        off += n as u64;
        tick_bytes += n as u64;
        if tick.elapsed() >= Duration::from_secs(1) {
            println!(
                "{:>5.1} s  {:>7.1} Mbit/s  {} parallel, window {} MiB",
                started.elapsed().as_secs_f64(),
                mbit(tick_bytes, tick.elapsed()),
                fs.prefetch_parallel(),
                fs.read_ahead_window() >> 20
            );
            tick = Instant::now();
            tick_bytes = 0;
        }
    }
    println!(
        "total: {} MiB in {:.1} s = {:.1} Mbit/s",
        off >> 20,
        started.elapsed().as_secs_f64(),
        mbit(off, started.elapsed())
    );
    Ok(())
}

#[allow(clippy::cast_precision_loss)]
fn mbit(bytes: u64, took: Duration) -> f64 {
    bytes as f64 * 8.0 / took.as_secs_f64() / 1e6
}
