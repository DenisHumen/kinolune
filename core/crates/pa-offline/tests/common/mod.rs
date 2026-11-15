// SPDX-License-Identifier: MPL-2.0
//! A "network share" on the local disk: an `smb` source whose connector
//! serves a folder through `LocalFs`, with a switch that takes the share
//! offline, a byte counter, read throttling and a cut after N bytes.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pa_catalog::{CancelToken, Catalog, Connector, Error, Source};
use pa_jobs::{Backoff, JobQueue, QueueConfig};
use pa_offline::{CopyConfig, Offline};
use pa_storage::{FakeFs, StorageConfig, StorageManager};
use pa_vfs::{ByteSource, DirEntry, ErrorKind, LocalFs, Metadata, Vfs, VfsError, VfsPath};

/// Switches of the share, shared by every connection.
#[derive(Debug, Default)]
pub struct Control {
    pub offline: AtomicBool,
    /// Bytes served by reads.
    pub read: AtomicU64,
    /// Go offline once this many bytes were served (0: never).
    pub cut_after: AtomicU64,
    /// Sleep this long per read (ms).
    pub throttle_ms: AtomicU64,
    /// Offsets of the first read of every opened file.
    pub first_reads: Mutex<Vec<u64>>,
}

impl Control {
    fn check(&self) -> pa_vfs::Result<()> {
        if self.offline.load(Ordering::SeqCst) {
            Err(VfsError::new(ErrorKind::Unavailable, "connection reset"))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
pub struct Share {
    fs: LocalFs,
    ctl: Arc<Control>,
}

#[derive(Debug)]
struct ShareFile {
    inner: Box<dyn ByteSource>,
    ctl: Arc<Control>,
    first: AtomicBool,
}

impl ByteSource for ShareFile {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> pa_vfs::Result<usize> {
        self.ctl.check()?;
        if self.first.swap(false, Ordering::SeqCst) {
            self.ctl.first_reads.lock().unwrap().push(offset);
        }
        let ms = self.ctl.throttle_ms.load(Ordering::SeqCst);
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms));
        }
        let n = self.inner.read_at(offset, buf)?;
        let total = self.ctl.read.fetch_add(n as u64, Ordering::SeqCst) + n as u64;
        let cut = self.ctl.cut_after.load(Ordering::SeqCst);
        if cut > 0 && total >= cut {
            self.ctl.cut_after.store(0, Ordering::SeqCst);
            self.ctl.offline.store(true, Ordering::SeqCst);
        }
        Ok(n)
    }
}

impl Vfs for Share {
    fn list(&self, dir: &VfsPath) -> pa_vfs::Result<Vec<DirEntry>> {
        self.ctl.check()?;
        self.fs.list(dir)
    }
    fn stat(&self, path: &VfsPath) -> pa_vfs::Result<Metadata> {
        self.ctl.check()?;
        self.fs.stat(path)
    }
    fn open(&self, path: &VfsPath) -> pa_vfs::Result<Box<dyn ByteSource>> {
        self.ctl.check()?;
        Ok(Box::new(ShareFile {
            inner: self.fs.open(path)?,
            ctl: Arc::clone(&self.ctl),
            first: AtomicBool::new(true),
        }))
    }
}

#[derive(Debug)]
pub struct ShareConnector {
    pub root: PathBuf,
    pub ctl: Arc<Control>,
}

impl Connector for ShareConnector {
    fn connect(&self, source: &Source) -> Result<Arc<dyn Vfs>, Error> {
        if let Some(root) = &source.local_root {
            return Ok(Arc::new(LocalFs::new(root)?));
        }
        if source.kind != "smb" {
            return Err(Error::Invalid(source.kind.clone()));
        }
        if self.ctl.offline.load(Ordering::SeqCst) {
            return Err(Error::Io("source unavailable: connection refused".into()));
        }
        Ok(Arc::new(Share {
            fs: LocalFs::new(&self.root)?,
            ctl: Arc::clone(&self.ctl),
        }))
    }
}

/// Deterministic content of `len` bytes.
pub fn content(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) ^ ((i >> 11) as u8))
        .collect()
}

pub fn put(root: &Path, rel: &str, data: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
}

pub struct Env {
    pub dir: PathBuf,
    pub db: PathBuf,
    pub share: PathBuf,
    pub offline_dir: PathBuf,
    pub ctl: Arc<Control>,
    pub catalog: Arc<Catalog>,
    pub source: i64,
    pub queue: JobQueue,
    pub storage: StorageManager,
    pub offline: Offline,
}

/// Fast settings for tests: small chunks, quick reconnects and retries.
pub fn test_copy_config() -> CopyConfig {
    CopyConfig {
        chunk: 64 * 1024,
        checkpoint_every: Duration::from_millis(50),
        reconnect_window: Duration::from_millis(300),
        reconnect_max_delay: Duration::from_millis(100),
        max_bytes_per_sec: None,
    }
}

/// Opens everything on `dir` (an existing setup is reused: a restart).
pub fn open(dir: &Path, copy: CopyConfig) -> Env {
    let db = dir.join("library.db");
    let share = dir.join("share");
    let offline_dir = dir.join("offline");
    std::fs::create_dir_all(&share).unwrap();
    let catalog = Arc::new(Catalog::open(&db).unwrap());
    let ctl = Arc::new(Control::default());
    catalog.set_connector(Arc::new(ShareConnector {
        root: share.clone(),
        ctl: Arc::clone(&ctl),
    }));
    let source = match catalog.sources().unwrap().first() {
        Some(s) => s.id,
        None => {
            catalog
                .add_source(None, "smb", "NAS", "smb://nas.test/media", Some("u"))
                .unwrap()
                .id
        }
    };
    let fs = Arc::new(FakeFs::new());
    fs.add_volume(&offline_dir, 100 << 30);
    let storage = StorageManager::open(
        &db,
        StorageConfig {
            fs,
            ..StorageConfig::default()
        },
    )
    .unwrap();
    let queue = JobQueue::open(
        &db,
        QueueConfig {
            backoff: Backoff {
                base: Duration::from_millis(50),
                max: Duration::from_millis(200),
            },
            progress_interval: Duration::from_millis(10),
            tick_interval: Duration::from_millis(20),
            ..QueueConfig::default()
        },
    )
    .unwrap();
    storage.attach(&queue);
    let offline =
        Offline::with_config(Arc::clone(&catalog), queue.clone(), storage.clone(), copy).unwrap();
    offline.set_folder(&offline_dir).unwrap();
    offline.register();
    Env {
        dir: dir.to_path_buf(),
        db,
        share,
        offline_dir,
        ctl,
        catalog,
        source,
        queue,
        storage,
        offline,
    }
}

impl Env {
    pub fn scan(&self) {
        let mut errors = Vec::new();
        self.catalog
            .scan(None, &CancelToken::new(), &mut |u| errors.extend(u.error))
            .unwrap();
        assert_eq!(errors, Vec::<String>::new());
        // The scan read the files (fingerprints): count from here.
        self.ctl.read.store(0, Ordering::SeqCst);
        self.ctl.first_reads.lock().unwrap().clear();
    }

    /// Runs the queue until nothing is left to do.
    pub fn run(&self) {
        assert!(self.queue.run_until_idle(Duration::from_secs(60)).unwrap());
    }
}
