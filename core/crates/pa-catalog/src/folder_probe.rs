// SPDX-License-Identifier: MPL-2.0
//! Cheap looks at the sync folder between rounds (SYNC-014, ADR-0137).
//!
//! While a title plays, rounds of folder sync ([`Catalog::run_folder_sync`])
//! run every second or two; between them a reader only lists the journals
//! ([`Catalog::folder_probe`]) and starts a round when one of them changed.
//! Probes reach the folder through one kept connection
//! ([`Catalog::folder_connect`]); rounds use it only while a title plays
//! ([`Catalog::keep_folder_connection`]), so a network share is not logged
//! into again every second then, while a round at any other time connects
//! anew and notices a source that went away (the switch to another folder
//! after ten minutes away, SYNC-015, counts on that).

use std::sync::Arc;
use std::time::{Duration, Instant};

use pa_library::rusqlite::OptionalExtension;
use pa_vfs::{Vfs, VfsPath};

use crate::{Catalog, Error};

/// A kept connection is dropped after this long and made again.
const KEEP: Duration = Duration::from_secs(5 * 60);

/// The connection to the folder's source kept between rounds.
#[derive(Default)]
pub(crate) struct FolderVfs {
    kept: Option<(i64, Instant, Arc<dyn Vfs>)>,
    /// Rounds may use `kept` (a title plays here).
    rounds_keep: bool,
}

impl std::fmt::Debug for FolderVfs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("FolderVfs")
            .field(&self.kept.as_ref().map(|(id, ..)| id))
            .field(&self.rounds_keep)
            .finish()
    }
}

/// A journal in the folder as listed: its file name, size and change time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderJournal {
    pub name: String,
    pub size: u64,
    pub modified_ms: Option<i64>,
}

impl Catalog {
    fn folder_vfs(&self) -> std::sync::MutexGuard<'_, FolderVfs> {
        self.folder_vfs.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The file system of the folder's source `source_id` for a round: the
    /// kept connection while rounds may keep it and it is for this source
    /// and fresh, else a new one (kept for the next probe).
    pub(crate) fn folder_connect(&self, source_id: i64) -> Result<Arc<dyn Vfs>, Error> {
        let keep = self.folder_vfs().rounds_keep;
        self.folder_vfs_for(source_id, keep)
    }

    fn folder_vfs_for(&self, source_id: i64, reuse: bool) -> Result<Arc<dyn Vfs>, Error> {
        if reuse
            && let Some((id, at, vfs)) = &self.folder_vfs().kept
            && *id == source_id
            && at.elapsed() < KEEP
        {
            return Ok(Arc::clone(vfs));
        }
        let vfs = match self.connect(source_id) {
            Ok(vfs) => vfs,
            Err(e) => {
                self.forget_folder_connection();
                return Err(e);
            }
        };
        self.folder_vfs().kept = Some((source_id, Instant::now(), Arc::clone(&vfs)));
        Ok(vfs)
    }

    /// Whether rounds of folder sync may use the kept connection: on while
    /// a title plays here (a round every second), off otherwise, so a
    /// round connects anew and a source that went away shows as
    /// `unavailable`. A kept connection that fails is dropped after the
    /// round's error ([`Catalog::forget_folder_connection`]).
    pub fn keep_folder_connection(&self, keep: bool) {
        self.folder_vfs().rounds_keep = keep;
    }

    /// Drops the kept connection (a round failed: the next one connects
    /// again).
    pub fn forget_folder_connection(&self) {
        self.folder_vfs().kept = None;
    }

    /// The journals in the sync folder, sorted by name: one listing, no
    /// file is opened. `None` while folder sync is off or the folder
    /// cannot be listed (the next round says why).
    pub fn folder_probe(&self) -> Result<Option<Vec<FolderJournal>>, Error> {
        let source: Option<i64> = self
            .lock()
            .query_row("SELECT source_id FROM sync_folder WHERE id = 1", [], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(source) = source else {
            return Ok(None);
        };
        let Ok(vfs) = self.folder_vfs_for(source, true) else {
            return Ok(None);
        };
        let dir = VfsPath::parse(pa_sync::folder::DIR).map_err(|e| Error::Io(e.to_string()))?;
        let entries = match vfs.list(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == pa_vfs::ErrorKind::NotFound => Vec::new(),
            Err(_) => {
                self.forget_folder_connection();
                return Ok(None);
            }
        };
        let mut out: Vec<FolderJournal> = entries
            .into_iter()
            .filter(|e| e.metadata.is_file())
            .map(|e| FolderJournal {
                size: e.metadata.size,
                modified_ms: e.metadata.modified_ms,
                name: e.name,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Some(out))
    }
}
