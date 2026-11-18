// SPDX-License-Identifier: MPL-2.0
//! Storage targets, usage, retention and trash (`pa-storage`, CORE-012)
//! for Dart: the minimal API for the settings screen and the downloads
//! dashboard. Jobs that write into targets (TWI-017, offline copies) use
//! the Rust API directly.

use std::path::PathBuf;

use flutter_rust_bridge::frb;
use pa_storage::{RetentionRule, StorageConfig, StorageManager, Target, TargetKind};

use crate::api::library::CoreLibrary;

/// Error of a storage call; Dart receives it as an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageError {
    /// Stable code (CORE-009): `not_found` (no such target or item),
    /// `exists`, `no_space` (a job can never fit), `invalid`, `io`,
    /// `database`.
    pub code: String,
    /// Detail for logs (no secrets).
    pub message: String,
}

fn err(e: pa_storage::Error) -> StorageError {
    use pa_storage::Error as E;
    let code = match &e {
        E::NoTarget(_) | E::NotFound(_) => "not_found",
        E::Exists(_) => "exists",
        E::WontFit { .. } => "no_space",
        E::Invalid(_) => "invalid",
        E::Io(_) => "io",
        _ => "database",
    };
    StorageError {
        code: code.into(),
        message: e.to_string(),
    }
}

/// What a target holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreStorageKind {
    Library,
    Downloads,
    Recordings,
    TranscodeCache,
}

impl From<TargetKind> for CoreStorageKind {
    fn from(k: TargetKind) -> Self {
        match k {
            TargetKind::Library => Self::Library,
            TargetKind::Downloads => Self::Downloads,
            TargetKind::Recordings => Self::Recordings,
            TargetKind::TranscodeCache => Self::TranscodeCache,
        }
    }
}

impl From<CoreStorageKind> for TargetKind {
    fn from(k: CoreStorageKind) -> Self {
        match k {
            CoreStorageKind::Library => Self::Library,
            CoreStorageKind::Downloads => Self::Downloads,
            CoreStorageKind::Recordings => Self::Recordings,
            CoreStorageKind::TranscodeCache => Self::TranscodeCache,
        }
    }
}

/// A storage target: a folder with a quota and a free-space reserve.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreStorageTarget {
    /// `[a-z0-9_-]+`.
    pub id: String,
    pub kind: CoreStorageKind,
    /// Absolute folder path.
    pub path: String,
    pub quota_bytes: Option<u64>,
    /// Share of the volume (0–100]; with `quota_bytes` the smaller wins.
    pub quota_percent: Option<f64>,
    pub reserve_bytes: u64,
    /// Share of the volume [0–100); with `reserve_bytes` the larger wins.
    pub reserve_percent: f64,
}

/// Space of a target now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreStorageUsage {
    pub target_id: String,
    /// `None` when the volume is not available.
    pub total: Option<u64>,
    pub available: Option<u64>,
    pub reserve: u64,
    pub quota: Option<u64>,
    pub used: u64,
    pub trash: u64,
    /// Bytes running jobs still write.
    pub pending: u64,
    /// Free space is below the reserve: jobs are paused.
    pub low: bool,
}

/// Retention rule of a target (`group_key` `None`) or of one group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreRetentionRule {
    pub target_id: String,
    pub group_key: Option<String>,
    pub keep_last: Option<u32>,
    pub max_age_days: Option<u32>,
    pub watched_after_days: Option<u32>,
    pub keep_favorites: bool,
}

/// A file kept in a target (or in its trash).
#[derive(Debug, Clone, PartialEq)]
pub struct CoreStorageItem {
    pub id: i64,
    pub target_id: String,
    pub rel_path: String,
    pub group_key: Option<String>,
    pub size_bytes: u64,
    pub favorite: bool,
    pub created_at_ms: i64,
    pub trashed_at_ms: Option<i64>,
    /// For the retention preview: `keep_last`, `max_age`, `watched`.
    pub expiry_reason: Option<String>,
}

fn item(i: pa_storage::Item, reason: Option<&str>) -> CoreStorageItem {
    CoreStorageItem {
        id: i.id,
        target_id: i.target_id,
        rel_path: i.rel_path,
        group_key: i.group_key,
        size_bytes: i.size_bytes,
        favorite: i.favorite,
        created_at_ms: i.created_at_ms,
        trashed_at_ms: i.trashed_at_ms,
        expiry_reason: reason.map(str::to_owned),
    }
}

/// Storage management of one library.
#[frb(opaque)]
pub struct CoreStorage {
    manager: StorageManager,
}

impl CoreStorage {
    /// The storage manager over the database of `library`.
    pub fn open(library: &CoreLibrary) -> Result<CoreStorage, StorageError> {
        let db: PathBuf = library.catalog().path().to_path_buf();
        Ok(Self {
            manager: StorageManager::open(db, StorageConfig::default()).map_err(err)?,
        })
    }

    pub fn targets(&self) -> Vec<CoreStorageTarget> {
        self.manager
            .targets()
            .into_iter()
            .map(|t| CoreStorageTarget {
                id: t.id,
                kind: t.kind.into(),
                path: t.path.to_string_lossy().into_owned(),
                quota_bytes: t.quota_bytes,
                quota_percent: t.quota_percent,
                reserve_bytes: t.reserve_bytes,
                reserve_percent: t.reserve_percent,
            })
            .collect()
    }

    /// Adds or replaces a target.
    pub fn save_target(&self, target: CoreStorageTarget) -> Result<(), StorageError> {
        let t = Target {
            id: target.id,
            kind: target.kind.into(),
            path: PathBuf::from(target.path),
            quota_bytes: target.quota_bytes,
            quota_percent: target.quota_percent,
            reserve_bytes: target.reserve_bytes,
            reserve_percent: target.reserve_percent,
        };
        self.manager.set_target(&t).map_err(err)
    }

    /// Forgets a target, its items and rules; files stay on disk.
    pub fn remove_target(&self, id: String) -> Result<bool, StorageError> {
        self.manager.remove_target(&id).map_err(err)
    }

    pub fn usage(&self) -> Result<Vec<CoreStorageUsage>, StorageError> {
        Ok(self
            .manager
            .usage()
            .map_err(err)?
            .into_iter()
            .map(|u| CoreStorageUsage {
                low: u.below_reserve(),
                target_id: u.target_id,
                total: u.total,
                available: u.available,
                reserve: u.reserve,
                quota: u.quota,
                used: u.used,
                trash: u.trash,
                pending: u.pending,
            })
            .collect())
    }

    pub fn retention_rules(&self) -> Result<Vec<CoreRetentionRule>, StorageError> {
        Ok(self
            .manager
            .retention_rules()
            .map_err(err)?
            .into_iter()
            .map(|r| CoreRetentionRule {
                target_id: r.target_id,
                group_key: r.group_key,
                keep_last: r.keep_last,
                max_age_days: r.max_age_days,
                watched_after_days: r.watched_after_days,
                keep_favorites: r.keep_favorites,
            })
            .collect())
    }

    /// Adds or replaces the rule of (target, group).
    pub fn save_retention(&self, rule: CoreRetentionRule) -> Result<(), StorageError> {
        self.manager
            .set_retention(&RetentionRule {
                target_id: rule.target_id,
                group_key: rule.group_key,
                keep_last: rule.keep_last,
                max_age_days: rule.max_age_days,
                watched_after_days: rule.watched_after_days,
                keep_favorites: rule.keep_favorites,
            })
            .map_err(err)
    }

    pub fn remove_retention(
        &self,
        target_id: String,
        group_key: Option<String>,
    ) -> Result<bool, StorageError> {
        self.manager
            .remove_retention(&target_id, group_key.as_deref())
            .map_err(err)
    }

    /// What the next cleanup moves to the trash.
    pub fn retention_preview(&self) -> Result<Vec<CoreStorageItem>, StorageError> {
        Ok(self
            .manager
            .plan_retention()
            .map_err(err)?
            .into_iter()
            .map(|e| item(e.item, Some(e.reason.as_str())))
            .collect())
    }

    pub fn trash(&self) -> Result<Vec<CoreStorageItem>, StorageError> {
        Ok(self
            .manager
            .trash()
            .map_err(err)?
            .into_iter()
            .map(|i| item(i, None))
            .collect())
    }

    /// Moves an item to the trash (deleted for good after 7 days).
    pub fn delete_item(&self, id: i64) -> Result<(), StorageError> {
        self.manager.trash_item(id).map(|_| ()).map_err(err)
    }

    pub fn restore(&self, id: i64) -> Result<(), StorageError> {
        self.manager.restore(id).map(|_| ()).map_err(err)
    }

    /// Deletes everything in the trash now; returns freed bytes.
    pub fn empty_trash(&self) -> Result<u64, StorageError> {
        Ok(self.manager.purge_trash(None).map_err(err)?.purged_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip() {
        for k in [
            TargetKind::Library,
            TargetKind::Downloads,
            TargetKind::Recordings,
            TargetKind::TranscodeCache,
        ] {
            assert_eq!(TargetKind::from(CoreStorageKind::from(k)), k);
        }
    }
}
