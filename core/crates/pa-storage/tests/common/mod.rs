// SPDX-License-Identifier: MPL-2.0
//! Shared setup: a library database in a temp dir, a simulated volume
//! ([`FakeFs`] over the real files of the temp dir) and a manual clock.
//!
//! Free space is simulated: mounting a size-limited tmpfs or a loop device
//! needs root, so the volume size is injected through `FsStats` while the
//! files themselves are real.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use pa_jobs::{JobQueue, QueueConfig};
use pa_storage::{FakeFs, NewItem, StorageConfig, StorageManager, Target, TargetKind};
use tempfile::TempDir;

pub const T0: i64 = 1_800_000_000_000;
pub const DAY: i64 = 86_400_000;

pub struct Env {
    pub dir: TempDir,
    pub db: PathBuf,
    /// Root of the simulated volume (the target lives under it).
    pub volume: PathBuf,
    pub fs: Arc<FakeFs>,
    pub now: Arc<AtomicI64>,
    pub storage: StorageManager,
    pub queue: JobQueue,
}

impl Env {
    /// A volume of `volume_bytes` with the target `dl` on it.
    pub fn new(volume_bytes: u64, target: impl FnOnce(Target) -> Target) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("library.db");
        let volume = dir.path().join("volume");
        std::fs::create_dir_all(&volume).unwrap();
        let fs = Arc::new(FakeFs::new());
        fs.add_volume(&volume, volume_bytes);
        let now = Arc::new(AtomicI64::new(T0));
        let clock_now = Arc::clone(&now);
        let clock: pa_jobs::Clock = Arc::new(move || clock_now.load(Ordering::SeqCst));
        let storage = StorageManager::open(
            &db,
            StorageConfig {
                fs: fs.clone(),
                clock: clock.clone(),
                ..StorageConfig::default()
            },
        )
        .unwrap();
        let queue = JobQueue::open(
            &db,
            QueueConfig {
                clock,
                ..QueueConfig::default()
            },
        )
        .unwrap();
        storage
            .set_target(&target(Target::new(
                "dl",
                TargetKind::Downloads,
                volume.join("downloads"),
            )))
            .unwrap();
        storage.attach(&queue);
        Env {
            dir,
            db,
            volume,
            fs,
            now,
            storage,
            queue,
        }
    }

    pub fn target_path(&self) -> PathBuf {
        self.volume.join("downloads")
    }

    pub fn set_now(&self, ms: i64) {
        self.now.store(ms, Ordering::SeqCst);
    }

    pub fn advance(&self, ms: i64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }

    /// Writes `size` bytes outside the volume and commits them as an item
    /// (a cross-volume move from the temp dir root is still a rename here).
    pub fn add_item(&self, rel: &str, group: Option<&str>, size: usize) -> i64 {
        let src = self
            .dir
            .path()
            .join(format!("src-{}", rel.replace('/', "_")));
        std::fs::write(&src, vec![1u8; size]).unwrap();
        let mut item = NewItem::new("dl", rel);
        item.group_key = group.map(str::to_owned);
        self.storage.commit_file(&src, &item, None).unwrap().id
    }

    pub fn idle(&self) {
        assert!(self.queue.run_until_idle(Duration::from_secs(10)).unwrap());
    }
}

/// Waits until `cond` holds, up to 10 s.
pub fn wait_until(mut cond: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(std::time::Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}
