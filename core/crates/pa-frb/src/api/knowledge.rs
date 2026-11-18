// SPDX-License-Identifier: MPL-2.0
//! Knowledge out to the files (LIB-014) for Dart: «Записать NFO» and
//! «Добавить ID в имя папки» on the «Исправить распознавание» page. The
//! calls are methods of [`CoreLibrary`]; see `pa_catalog::knowledge` and
//! ADR-0122.

use crate::api::library::{CoreLibrary, LibraryError, item_ref};

type Result<T> = std::result::Result<T, LibraryError>;

/// An NFO file the export writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreNfoTarget {
    pub source_id: i64,
    /// Path in the source (`/Movies/Dune (2021)/movie.nfo`).
    pub path: String,
    /// A file is there already: ask before replacing it.
    pub exists: bool,
}

/// A folder that gets the id at the end of its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreRenameTarget {
    pub source_id: i64,
    /// The folder now.
    pub path: String,
    /// Its new name (`Dune (2021) [tmdbid-438631]`).
    pub new_name: String,
}

/// What the export would do for a work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreKnowledgePlan {
    /// `tmdb 438631`, `imdb tt1160419`: the ids an NFO gets; empty — the
    /// work is not recognised yet, nothing to write.
    pub ids: Vec<String>,
    pub nfo: Vec<CoreNfoTarget>,
    pub rename: Vec<CoreRenameTarget>,
    /// Some files are on a source that cannot be written now.
    pub read_only: bool,
}

/// What «Записать NFO» did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreNfoReport {
    pub written: u32,
    /// Old files moved to the source's trash.
    pub replaced: u32,
    /// Existing files left alone.
    pub skipped: u32,
}

fn n(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

impl CoreLibrary {
    /// What «Записать NFO» and «Добавить ID в имя папки» would do for an
    /// item's work (an episode: its show). Probes the sources for writes.
    pub fn knowledge_plan(&self, item_id: String) -> Result<CoreKnowledgePlan> {
        let p = self.catalog().knowledge_plan(item_ref(&item_id)?)?;
        Ok(CoreKnowledgePlan {
            ids: p.ids.into_iter().map(|(k, v)| format!("{k} {v}")).collect(),
            nfo: p
                .nfo
                .into_iter()
                .map(|t| CoreNfoTarget {
                    source_id: t.source_id,
                    path: t.path,
                    exists: t.exists,
                })
                .collect(),
            rename: p
                .rename
                .into_iter()
                .map(|t| CoreRenameTarget {
                    source_id: t.source_id,
                    path: t.path,
                    new_name: t.new_name,
                })
                .collect(),
            read_only: p.read_only,
        })
    }

    /// «Записать NFO»: Kodi/Jellyfin NFO files next to the work's files.
    /// An existing file stays unless `replace`; with `replace` it goes to
    /// the source's trash first.
    pub fn export_nfo(&self, item_id: String, replace: bool) -> Result<CoreNfoReport> {
        let r = self.catalog().export_nfo(item_ref(&item_id)?, replace)?;
        Ok(CoreNfoReport {
            written: n(r.written),
            replaced: n(r.replaced),
            skipped: n(r.skipped),
        })
    }

    /// «Добавить ID в имя папки»: the work's own folders get
    /// ` [tmdbid-…]` at the end of their names. Returns how many.
    pub fn rename_with_ids(&self, item_id: String) -> Result<u32> {
        Ok(n(self.catalog().rename_with_ids(item_ref(&item_id)?)?))
    }
}
