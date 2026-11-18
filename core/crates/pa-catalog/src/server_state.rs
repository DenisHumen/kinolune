// SPDX-License-Identifier: MPL-2.0
//! The user's state at a media server brought into the library (NET-014
//! Jellyfin and Emby, NET-013 Plex; ADR-0075, ADR-0134).
//!
//! A server source reads as a tree of files; after its scan the app asks
//! the server where the user is in each file (resume position, watched,
//! favorite, when last watched) and hands it here by the file's path. A
//! position or "watched" newer than the library's wins; a favorite is
//! only ever added. Files the scan has not matched yet are skipped (the
//! next scan brings them).

use std::collections::HashMap;

use crate::{Catalog, Error, Item, ItemRef};

/// The user's state of one file at the server.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerItemState {
    /// Path of the file relative to the source root, without a leading
    /// `/` (as [`Item::rel_path`]).
    pub rel_path: String,
    /// Resume position, ms (0: none).
    pub position_ms: i64,
    /// Duration, ms (0: unknown).
    pub duration_ms: i64,
    /// Watched.
    pub played: bool,
    /// A favorite of the user (Jellyfin, Emby; Plex has none).
    pub favorite: bool,
    /// When the user last played it, Unix ms (0: unknown — then the
    /// server's position only fills an item the library has no progress
    /// of).
    pub at_ms: i64,
}

/// What [`Catalog::import_server_state`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServerImport {
    /// Resume positions taken.
    pub progress: u32,
    /// Items marked watched.
    pub watched: u32,
    /// Items made favorites.
    pub favorites: u32,
}

impl Catalog {
    /// Brings the server's state of the files of source `source_id` into
    /// the library (see the module docs).
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn import_server_state(
        &self,
        source_id: i64,
        states: &[ServerItemState],
    ) -> Result<ServerImport, Error> {
        let mut by_path: HashMap<String, ItemRef> = HashMap::new();
        let mut add = |items: Vec<Item>| {
            for i in items.into_iter().filter(|i| i.source_id == source_id) {
                by_path.entry(i.rel_path).or_insert(i.id);
            }
        };
        add(self.movies()?);
        add(self.tracks()?);
        for show in self.shows()? {
            add(self.episodes(show.id)?);
        }
        let mut report = ServerImport::default();
        for s in states {
            let Some(item) = by_path.get(s.rel_path.trim_start_matches('/')).copied() else {
                continue;
            };
            let local = self.progress(item)?;
            let newer = local.as_ref().is_none_or(|p| p.updated_at_ms < s.at_ms);
            let duration = s.duration_ms.max(0);
            if s.played && newer && !local.as_ref().is_some_and(|p| p.watched) {
                if self.save_progress(item, duration, duration, s.at_ms.max(1), true)? {
                    report.watched += 1;
                }
            } else if !s.played
                && s.position_ms > 0
                && newer
                && self.save_progress(item, s.position_ms, duration, s.at_ms.max(1), false)?
            {
                report.progress += 1;
            }
            if s.favorite
                && !matches!(item, ItemRef::Track(_))
                && !self.is_favorite(item).unwrap_or(true)
                && self.set_favorite(item, true).unwrap_or(false)
            {
                report.favorites += 1;
            }
        }
        Ok(report)
    }
}
