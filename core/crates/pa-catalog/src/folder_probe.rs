// SPDX-License-Identifier: MPL-2.0
//! Cheap looks at the sync folder between rounds (SYNC-014, ADR-0137).
//!
//! While a title plays, rounds of folder sync ([`Catalog::run_folder_sync`])
//! run every second or two; between them a reader only lists the journals
//! ([`Catalog::folder_probe`]) and starts a round when one of them changed.
//! Both reach the folder through one kept connection
//! ([`Catalog::folder_connect`]), so a network share is not logged into
//! again every few seconds.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pa_library::rusqlite::OptionalExtension;
use pa_vfs::{Vfs, VfsPath};

use crate::{Catalog, Error};

/// A kept connection is dropped after this long and made again.
const KEEP: Duration = Duration::from_secs(5 * 60);

/// The connection to the folder's source kept between rounds.
#[derive(Default)]
pub(crate) struct FolderVfs(Option<(i64, Instant, Arc<dyn Vfs>)>);

impl std::fmt::Debug for FolderVfs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("FolderVfs")
            .field(&self.0.as_ref().map(|(id, ..)| id))
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

    /// The file system of the folder's source `source_id`: the kept
    /// connection if it is for this source and fresh, else a new one.
    pub(crate) fn folder_connect(&self, source_id: i64) -> Result<Arc<dyn Vfs>, Error> {
        if let Some((id, at, vfs)) = &self.folder_vfs().0
            && *id == source_id
            && at.elapsed() < KEEP
        {
            return Ok(Arc::clone(vfs));
        }
        let vfs = self.connect(source_id)?;
        self.folder_vfs().0 = Some((source_id, Instant::now(), Arc::clone(&vfs)));
        Ok(vfs)
    }

    /// Drops the kept connection (a round failed: the next one connects
    /// again).
    pub fn forget_folder_connection(&self) {
        self.folder_vfs().0 = None;
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
        let Ok(vfs) = self.folder_connect(source) else {
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
