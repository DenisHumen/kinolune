// SPDX-License-Identifier: MPL-2.0
//! Offline copies of library files (NET-019): where the device keeps a
//! downloaded copy of a source file.
//!
//! `pa-offline` copies files of network sources into the `offline` storage
//! target and records them in `offline_copies` / `offline_files`. The
//! catalog only reads that mapping: the source file stays the canonical
//! library file, so items, progress and versions do not change, and
//! [`SourceOpener`](crate::SourceOpener) serves the local copy instead of
//! the network file. Playback, sidecar subtitles, the parts of a split
//! movie and local art then work with the server off.

use std::path::{Path, PathBuf};

use pa_library::rusqlite::{Connection, params};
use pa_vfs::{ByteSource, LocalFs, Vfs, VfsPath};

use crate::{Catalog, Error};

/// Folders of the live copies (not in the trash) that hold `path` of
/// `source_id`, newest first. The file is the folder joined with `path`.
pub(crate) const COPY_ROOTS_SQL: &str = "
SELECT t.path, si.rel_path
FROM offline_files o
JOIN offline_copies c ON c.id = o.copy_id
JOIN storage_items si ON si.id = c.storage_item_id AND si.trashed_at_ms IS NULL
JOIN storage_targets t ON t.id = si.target_id
WHERE o.source_id = ?1 AND o.path = ?2
ORDER BY c.created_at_ms DESC, c.id DESC";

fn vfs_path(path: &str) -> String {
    format!("/{}", path.trim_start_matches('/'))
}

pub(crate) fn copy_roots(
    conn: &Connection,
    source_id: i64,
    path: &str,
) -> Result<Vec<PathBuf>, Error> {
    let mut stmt = conn.prepare_cached(COPY_ROOTS_SQL)?;
    let roots = stmt
        .query_map(params![source_id, vfs_path(path)], |r| {
            let target: String = r.get(0)?;
            let rel: String = r.get(1)?;
            Ok(pa_vfs::host::join_relative(Path::new(&target), &rel))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(roots)
}

impl Catalog {
    /// The local file of the newest offline copy of `path` (relative to
    /// the source root or a `files.path`) of source `source_id`, if a live
    /// copy holds it and the file is on disk.
    pub fn offline_file(&self, source_id: i64, path: &str) -> Result<Option<PathBuf>, Error> {
        let roots = copy_roots(&self.lock(), source_id, path)?;
        let rel = path.trim_start_matches('/');
        Ok(roots
            .into_iter()
            .map(|root| pa_vfs::host::join_relative(&root, rel))
            .find(|p| p.is_file()))
    }

    /// Opens the offline copy of `path` of source `source_id`, if there is
    /// one (see [`offline_file`](Self::offline_file)).
    pub fn open_offline(
        &self,
        source_id: i64,
        path: &VfsPath,
    ) -> Result<Option<Box<dyn ByteSource>>, Error> {
        let roots = copy_roots(&self.lock(), source_id, path.as_str())?;
        for root in roots {
            let Ok(fs) = LocalFs::new(&root) else {
                continue;
            };
            if let Ok(file) = fs.open(path) {
                return Ok(Some(file));
            }
        }
        Ok(None)
    }

    /// Whether `path` of source `source_id` has a live offline copy.
    pub fn is_offline(&self, source_id: i64, path: &str) -> Result<bool, Error> {
        Ok(self.offline_file(source_id, path)?.is_some())
    }
}
