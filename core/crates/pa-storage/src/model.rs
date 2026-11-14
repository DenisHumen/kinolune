// SPDX-License-Identifier: MPL-2.0
//! Storage targets, items, retention rules and reports.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

/// What a storage target holds (04 §1.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetKind {
    Library,
    Downloads,
    Recordings,
    TranscodeCache,
}

impl TargetKind {
    /// Name stored in `storage_targets.kind`.
    pub const fn as_str(self) -> &'static str {
        match self {
            TargetKind::Library => "library",
            TargetKind::Downloads => "downloads",
            TargetKind::Recordings => "recordings",
            TargetKind::TranscodeCache => "transcode_cache",
        }
    }
}

impl FromStr for TargetKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "library" => Ok(TargetKind::Library),
            "downloads" => Ok(TargetKind::Downloads),
            "recordings" => Ok(TargetKind::Recordings),
            "transcode_cache" => Ok(TargetKind::TranscodeCache),
            _ => Err(format!("unknown storage kind {s:?}")),
        }
    }
}

impl fmt::Display for TargetKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A place jobs write to: a folder or a mounted share. The core keeps
/// unfinished files in `<path>/.tmp/` and the trash in `<path>/.trash/`,
/// so a target is expected to be one volume.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// `[a-z0-9_-]+`; jobs refer to it in `target_storage`.
    pub id: String,
    pub kind: TargetKind,
    pub path: PathBuf,
    /// At most this many bytes of kept items.
    pub quota_bytes: Option<u64>,
    /// At most this share of the volume (0–100]. With both limits the
    /// smaller wins.
    pub quota_percent: Option<f64>,
    /// Keep at least this many bytes of the volume free.
    pub reserve_bytes: u64,
    /// Keep at least this share of the volume free [0–100). With both the
    /// larger wins.
    pub reserve_percent: f64,
}

impl Target {
    /// A target without quota and reserve.
    pub fn new(id: impl Into<String>, kind: TargetKind, path: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            kind,
            path: path.into(),
            quota_bytes: None,
            quota_percent: None,
            reserve_bytes: 0,
            reserve_percent: 0.0,
        }
    }

    pub fn quota_bytes(mut self, bytes: u64) -> Self {
        self.quota_bytes = Some(bytes);
        self
    }

    pub fn quota_percent(mut self, percent: f64) -> Self {
        self.quota_percent = Some(percent);
        self
    }

    pub fn reserve_bytes(mut self, bytes: u64) -> Self {
        self.reserve_bytes = bytes;
        self
    }

    pub fn reserve_percent(mut self, percent: f64) -> Self {
        self.reserve_percent = percent;
        self
    }

    /// The quota on a volume of `total` bytes, if any.
    pub fn quota(&self, total: u64) -> Option<u64> {
        let percent = self.quota_percent.map(|p| share(total, p));
        match (self.quota_bytes, percent) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The free-space reserve on a volume of `total` bytes.
    pub fn reserve(&self, total: u64) -> u64 {
        self.reserve_bytes.max(share(total, self.reserve_percent))
    }
}

fn share(total: u64, percent: f64) -> u64 {
    // f64 is exact enough for a reserve; saturates on overflow.
    (total as f64 * percent.clamp(0.0, 100.0) / 100.0) as u64
}

/// A file the core keeps in a target.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: i64,
    pub target_id: String,
    /// Path relative to the target, `/`-separated.
    pub rel_path: String,
    /// Retention group: a channel, a recording rule.
    pub group_key: Option<String>,
    pub size_bytes: u64,
    pub profile_id: Option<i64>,
    pub favorite: bool,
    pub watched_at_ms: Option<i64>,
    pub created_at_ms: i64,
    /// In the trash since.
    pub trashed_at_ms: Option<i64>,
}

/// Where and how to keep a new file ([`StorageManager::commit_file`](crate::StorageManager::commit_file)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewItem {
    pub target_id: String,
    pub rel_path: String,
    pub group_key: Option<String>,
    pub profile_id: Option<i64>,
    pub favorite: bool,
}

impl NewItem {
    pub fn new(target_id: impl Into<String>, rel_path: impl Into<String>) -> Self {
        Self {
            target_id: target_id.into(),
            rel_path: rel_path.into(),
            group_key: None,
            profile_id: None,
            favorite: false,
        }
    }

    pub fn group(mut self, key: impl Into<String>) -> Self {
        self.group_key = Some(key.into());
        self
    }

    pub fn profile(mut self, id: i64) -> Self {
        self.profile_id = Some(id);
        self
    }
}

/// Retention rule of a target, or of one group in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionRule {
    pub target_id: String,
    /// `None`: the default for every group of the target without its own
    /// rule. Each group is counted separately.
    pub group_key: Option<String>,
    /// Keep the newest N items of the group.
    pub keep_last: Option<u32>,
    /// Delete items older than X days.
    pub max_age_days: Option<u32>,
    /// Delete watched items Y days after they were watched.
    pub watched_after_days: Option<u32>,
    /// Never delete favourites (they do not count for `keep_last`).
    pub keep_favorites: bool,
}

impl RetentionRule {
    /// A rule that keeps everything; set the limits with the fields.
    pub fn new(target_id: impl Into<String>, group_key: Option<String>) -> Self {
        Self {
            target_id: target_id.into(),
            group_key,
            keep_last: None,
            max_age_days: None,
            watched_after_days: None,
            keep_favorites: true,
        }
    }
}

/// Why retention removes an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpiryReason {
    KeepLast,
    MaxAge,
    Watched,
}

impl ExpiryReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            ExpiryReason::KeepLast => "keep_last",
            ExpiryReason::MaxAge => "max_age",
            ExpiryReason::Watched => "watched",
        }
    }
}

/// An item the next cleanup moves to the trash.
#[derive(Debug, Clone, PartialEq)]
pub struct Expiry {
    pub item: Item,
    pub reason: ExpiryReason,
}

/// What a cleanup did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetentionReport {
    /// Items moved to the trash.
    pub trashed: Vec<i64>,
    /// Items deleted for good (in the trash longer than the trash period).
    pub purged: Vec<i64>,
    /// Bytes freed by the purge.
    pub purged_bytes: u64,
}

/// Space of a target now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    pub target_id: String,
    /// `None` if the volume is not available (unmounted share).
    pub total: Option<u64>,
    pub available: Option<u64>,
    pub reserve: u64,
    pub quota: Option<u64>,
    /// Bytes of kept items.
    pub used: u64,
    /// Bytes in the trash.
    pub trash: u64,
    /// Bytes running jobs still expect to write.
    pub pending: u64,
}

impl Usage {
    /// True if free space is below the reserve or the volume is gone.
    pub fn below_reserve(&self) -> bool {
        self.available.is_none_or(|a| a < self.reserve)
    }
}

/// Reasons a job waits, written to `jobs.wait_reason`.
pub mod wait {
    /// Free space minus the reserve is smaller than the job.
    pub const DISK_RESERVE: &str = "disk_reserve";
    /// The quota of the target has no room for the job.
    pub const QUOTA: &str = "quota";
    /// The job is larger than the quota or the volume: it will never fit.
    pub const TOO_LARGE: &str = "too_large";
    /// The target path is not available (unmounted share, removed disk).
    pub const DISK_UNAVAILABLE: &str = "disk_unavailable";
    /// No storage target with the job's id.
    pub const NO_TARGET: &str = "no_storage_target";
}

/// Does a write of some size fit into a target?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fit {
    Fits,
    /// Not now; may fit when space is freed. Reason from [`wait`].
    NotNow(&'static str),
    /// Never fits (larger than the quota or the volume, no such target).
    Never(&'static str),
}
