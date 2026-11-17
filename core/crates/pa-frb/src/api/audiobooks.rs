// SPDX-License-Identifier: MPL-2.0
//! The audiobook library (MUS-018) for Dart: books made of the tracks the
//! content kind classifier marked (LIB-025), their parts and chapters, and
//! where the listener stopped in each, synced between devices under the
//! book key. The calls are methods of [`CoreLibrary`].

use pa_catalog as c;

use crate::api::library::{CoreItem, CoreLibrary, LibraryError};

type Result<T> = std::result::Result<T, LibraryError>;

/// A chapter of a book.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreBookChapter {
    /// `None`: the file has no title for it (the app numbers it).
    pub title: Option<String>,
    /// Index into [`CoreAudiobook::parts`].
    pub part_index: u32,
    /// Start within the part, milliseconds.
    pub start_ms: i64,
    /// Start within the book, milliseconds.
    pub book_start_ms: i64,
    /// 0 if unknown.
    pub duration_ms: i64,
}

/// Where the listener stopped in a book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreBookProgress {
    /// Milliseconds from the start of the book.
    pub position_ms: i64,
    /// 0 if unknown.
    pub duration_ms: i64,
    pub updated_at_ms: i64,
    /// Listened to the end.
    pub finished: bool,
}

/// An audiobook.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreAudiobook {
    /// The key the place is saved and synced under; `None` until the first
    /// part is fingerprinted.
    pub key: Option<String>,
    pub title: String,
    pub author: Option<String>,
    pub narrator: Option<String>,
    pub series: Option<String>,
    pub series_number: Option<f64>,
    pub year: Option<i32>,
    pub source_id: i64,
    /// The book's folder (or file), relative to the source root.
    pub rel_path: String,
    /// Folder cover relative to the source root.
    pub local_poster: Option<String>,
    /// Cover Art Archive cover.
    pub cover_url: Option<String>,
    /// 0 if unknown.
    pub duration_ms: i64,
    pub added_at_ms: Option<i64>,
    /// Tracks in play order.
    pub parts: Vec<CoreItem>,
    pub chapters: Vec<CoreBookChapter>,
    pub progress: Option<CoreBookProgress>,
}

impl From<c::BookProgress> for CoreBookProgress {
    fn from(p: c::BookProgress) -> Self {
        Self {
            position_ms: p.position_ms,
            duration_ms: p.duration_ms,
            updated_at_ms: p.updated_at_ms,
            finished: p.finished,
        }
    }
}

impl From<c::Audiobook> for CoreAudiobook {
    fn from(b: c::Audiobook) -> Self {
        Self {
            key: b.key,
            title: b.title,
            author: b.author,
            narrator: b.narrator,
            series: b.series,
            series_number: b.series_number,
            year: b.year.and_then(|y| i32::try_from(y).ok()),
            source_id: b.source_id,
            rel_path: b.rel_path,
            local_poster: b.local_poster,
            cover_url: b.cover_url,
            duration_ms: b.duration_ms,
            added_at_ms: b.added_at_ms,
            parts: b.parts.into_iter().map(Into::into).collect(),
            chapters: b
                .chapters
                .into_iter()
                .map(|c| CoreBookChapter {
                    title: c.title,
                    part_index: c.part,
                    start_ms: c.start_ms,
                    book_start_ms: c.book_start_ms,
                    duration_ms: c.duration_ms,
                })
                .collect(),
            progress: b.progress.map(Into::into),
        }
    }
}

impl CoreLibrary {
    /// The audiobooks the active profile sees, by author, series and title,
    /// with their parts, chapters and place.
    pub fn audiobooks(&self) -> Result<Vec<CoreAudiobook>> {
        Ok(self
            .catalog()
            .audiobooks()?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Where the listener stopped in the book with key `key`.
    pub fn book_progress(&self, key: String) -> Result<Option<CoreBookProgress>> {
        Ok(self.catalog().book_progress(&key)?.map(Into::into))
    }

    /// Saves the place in a book (milliseconds from its start) and sends it
    /// to paired devices.
    pub fn save_book_progress(
        &self,
        key: String,
        position_ms: i64,
        duration_ms: i64,
        updated_at_ms: i64,
        finished: bool,
    ) -> Result<()> {
        self.catalog().save_book_progress(
            &key,
            position_ms,
            duration_ms,
            updated_at_ms,
            finished,
        )?;
        self.flush_sync();
        Ok(())
    }
}
