// SPDX-License-Identifier: MPL-2.0
//! "Mark unwatched" for Dart (QA-015): the item menu's entry clears the
//! watched mark and the position through the core (`pa_library::progress::
//! mark_unwatched`), synced like any other progress change. A method of
//! [`CoreLibrary`].

use crate::api::library::{CoreLibrary, LibraryError, item_ref};

type Result<T> = std::result::Result<T, LibraryError>;

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
}
