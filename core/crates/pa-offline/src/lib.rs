// SPDX-License-Identifier: MPL-2.0
//! Offline copies of library items on the device (NET-019, ADR-NNNN).
//!
//! "Download for offline" copies the files of a movie, an episode or a
//! track from a network source (SMB, WebDAV through `pa-sources`) into the
//! `offline` storage target (`pa-storage`, CORE-012) with a job of the
//! `offline_copy` type in the persistent queue (`pa-jobs`, CORE-011). A
//! season, an album or a playlist is one job per item.
//!
//! - **What is copied** ([`plan`]): the version the user picked or the
//!   default one (LIB-010), every part of a split movie or DVD title set
//!   (LIB-020), the sidecar subtitles of each part (PLAY-007) and the local
//!   poster next to the files (LIB-007).
//! - **Resume**: files are read with positional (range) reads and appended
//!   to `<file>.part` in the job's tmp folder; the length of that file is
//!   the offset to continue from after a pause, a dropped connection or a
//!   killed process. A dropped connection is reconnected in the same run
//!   with a growing delay ([`CopyConfig::reconnect_window`]); after that
//!   the job is retried by the queue.
//! - **Linked, not a new item**: the finished folder is committed with
//!   [`StorageManager::commit_file`] and recorded in `offline_copies` /
//!   `offline_files` by source path. The source file stays the canonical
//!   library file: items, progress and versions do not change, and the
//!   stream opener of the catalog serves the copy instead of the network
//!   file ([`pa_catalog::Catalog::offline_file`]).
//! - **Retention**: copies are storage items (group `movies`,
//!   `show:<id>`, `album:<id>` or `music`); "delete watched" is a
//!   `pa-storage` retention rule of the target, fed by
//!   [`Offline::sync_watched`] from the library progress of the active
//!   profile.
//! - **Wi-Fi only**: the job's network policy (`unmetered` or `any`);
//!   the platform reports the network to the queue.

mod copy;
mod manager;
mod plan;

pub use copy::{CopyConfig, CopyHandler};
pub use manager::{
    Download, DownloadOptions, DownloadState, Offline, OfflineCopy, OfflineSettings, Selection,
};
pub use plan::{CopyFile, CopyPlan, FileRole, plan};

/// Id of the storage target of offline copies.
pub const TARGET: &str = "offline";

/// Job type of an offline copy.
pub const KIND: &str = pa_jobs::kinds::OFFLINE_COPY;

/// Error of offline copies.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("catalog: {0}")]
    Catalog(#[from] pa_catalog::Error),
    #[error("database: {0}")]
    Db(#[from] pa_library::rusqlite::Error),
    #[error("library: {0}")]
    Library(#[from] pa_library::Error),
    #[error("jobs: {0}")]
    Jobs(#[from] pa_jobs::Error),
    #[error("storage: {0}")]
    Storage(#[from] pa_storage::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0} not found")]
    NotFound(String),
    /// The item is on a local folder already: nothing to download.
    #[error("{0} is not on a network source")]
    Local(String),
    /// The item cannot be played (a disc image), so a copy is useless.
    #[error("{0} cannot be played")]
    Unplayable(String),
    /// The storage folder cannot change while copies or downloads use it.
    #[error("the offline folder is in use: {0}")]
    Busy(&'static str),
    #[error("invalid {0}")]
    Invalid(String),
}
