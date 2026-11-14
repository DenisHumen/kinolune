// SPDX-License-Identifier: MPL-2.0
//! Storage management of the core (CORE-012, docs/research/04 §1.10).
//!
//! - **Targets** — [`Target`]: a folder or share of a kind (library,
//!   downloads, recordings, transcode cache) with a quota and a free-space
//!   reserve, in the `storage_targets` table of the library database.
//! - **Admission** — [`StorageManager::gate`] is the
//!   [`ResourceGate`](pa_jobs::ResourceGate) of the job queue: a job with
//!   `target_storage` and an estimated `bytes_total` starts only if it fits
//!   above the reserve and inside the quota, counting what running jobs
//!   still write; otherwise it waits in `waiting_resources` with a reason
//!   from [`wait`]. [`StorageManager::enqueue`] refuses a job that can never
//!   fit.
//! - **Monitoring** — [`StorageManager::check`] (or the thread of
//!   [`StorageManager::start_monitor`]) pauses running jobs of a target whose
//!   free space fell below the reserve or whose quota is exceeded; the queue
//!   moves them to `waiting_resources` and the gate lets them continue once
//!   space is freed.
//! - **Retention** — [`RetentionRule`] per target or group: keep the last
//!   N, delete after X days, delete watched after Y days, keep favourites.
//!   The `cleanup` job ([`StorageManager::register_cleanup`]) moves expired
//!   items to the target's `.trash/` and deletes what has been there for 7
//!   days.
//! - **Files** — jobs write into [`StorageManager::job_tmp_dir`] and finish
//!   with [`StorageManager::commit_file`]: an atomic rename into place (a
//!   synced copy when the source is on another volume, [`move_atomic`]).
//!   [`StorageManager::clean_tmp`] at startup deletes what jobs that no
//!   longer exist left in `.tmp/`.
//!
//! Free space comes from an injectable [`FsStats`]: [`SystemFs`] in the
//! apps, [`FakeFs`] (a simulated small volume over real files) in tests.

mod fs;
mod manager;
mod model;

pub use fs::{FakeFs, FsStats, Space, SystemFs, dir_size, is_cross_device, move_atomic};
pub use manager::{
    MonitorDriver, MonitorReport, StorageConfig, StorageManager, TMP_DIR, TRASH_DIR,
};
pub use model::{
    Expiry, ExpiryReason, Fit, Item, NewItem, RetentionReport, RetentionRule, Target, TargetKind,
    Usage, wait,
};

use pa_library::rusqlite;

/// Error of the storage manager.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("library: {0}")]
    Library(#[from] pa_library::Error),
    #[error("jobs: {0}")]
    Jobs(#[from] pa_jobs::Error),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage target {0:?} not found")]
    NoTarget(String),
    #[error("storage item {0} not found")]
    NotFound(i64),
    #[error("{0} already exists")]
    Exists(String),
    #[error("job does not fit into storage target {target:?}: {reason}")]
    WontFit {
        target: String,
        reason: &'static str,
    },
    #[error("invalid {0}")]
    Invalid(&'static str),
}
