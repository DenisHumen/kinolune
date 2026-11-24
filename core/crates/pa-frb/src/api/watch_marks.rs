// SPDX-License-Identifier: MPL-2.0
//! Watch marks for Dart: "Mark unwatched" of one item (QA-015, through
//! `pa_library::progress::mark_unwatched`) and the bulk marks of the show
//! page (UI-049: an episode with the ones before it, a season, a whole
//! show) with «Отменить», each in one transaction and synced like any
//! other progress change. Methods of [`CoreLibrary`].

use pa_catalog as c;

use crate::api::library::{CoreLibrary, LibraryError, item_ref};

type Result<T> = std::result::Result<T, LibraryError>;

/// The watch state an item had before a bulk mark (UI-049): what
/// «Отменить» puts back with [`CoreLibrary::restore_watch_marks`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreWatchMark {
    pub item_id: String,
    pub position_ms: i64,
    /// 0 if unknown.
    pub duration_ms: i64,
    pub watched: bool,
}

impl From<c::WatchMark> for CoreWatchMark {
    fn from(m: c::WatchMark) -> Self {
        Self {
            item_id: m.item.to_string(),
            position_ms: m.position_ms,
            duration_ms: m.duration_ms,
            watched: m.watched,
        }
    }
}

impl CoreLibrary {
    /// Clears "watched" and the position of a movie or an episode (a show:
    /// call it for each episode). `false`: there was nothing to clear.
    pub fn mark_unwatched(&self, item_id: String, updated_at_ms: i64) -> Result<bool> {
        let cleared = self
            .catalog()
            .mark_unwatched(item_ref(&item_id)?, updated_at_ms)?;
        if cleared {
            self.flush_sync();
        }
        Ok(cleared)
    }

    /// Marks movies and episodes watched (or not) in one transaction
    /// (UI-049). Returns the previous state of each item that changed:
    /// pass it to [`CoreLibrary::restore_watch_marks`] for «Отменить».
    pub fn set_watched_many(
        &self,
        item_ids: Vec<String>,
        watched: bool,
        updated_at_ms: i64,
    ) -> Result<Vec<CoreWatchMark>> {
        let items = item_ids
            .iter()
            .map(|id| item_ref(id))
            .collect::<Result<Vec<_>>>()?;
        let changed = self
            .catalog()
            .set_watched_many(&items, watched, updated_at_ms)?;
        if !changed.is_empty() {
            self.flush_sync();
        }
        Ok(changed.into_iter().map(Into::into).collect())
    }

    /// «Отменить» of [`CoreLibrary::set_watched_many`]: puts back the
    /// watched mark and the position of each item, in one transaction.
    pub fn restore_watch_marks(&self, marks: Vec<CoreWatchMark>, updated_at_ms: i64) -> Result<()> {
        let marks = marks
            .into_iter()
            .map(|m| {
                Ok(c::WatchMark {
                    item: item_ref(&m.item_id)?,
                    position_ms: m.position_ms,
                    duration_ms: m.duration_ms,
                    watched: m.watched,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if marks.is_empty() {
            return Ok(());
        }
        self.catalog().restore_watch_marks(&marks, updated_at_ms)?;
        self.flush_sync();
        Ok(())
    }
}
