// SPDX-License-Identifier: MPL-2.0
//! Personal media (UI-025, "Личные медиа") for Dart: personal libraries
//! with a PIN or password lock, the content type of sources, moving items
//! between the main library and a personal one, the categories, tags
//! and ratings of a personal library and its own playback positions
//! (UI-033). The calls are methods of
//! [`CoreLibrary`]; a locked library answers with an error that starts
//! with `denied:`.

use pa_catalog as c;

use crate::api::library::{CoreLibrary, LibraryError, item_ref, label_target};
use crate::api::library_labels::{CoreCategory, CoreTag};
use crate::api::profiles::CorePinVerdict;
use crate::api::subtitles::CorePlayPart;

type Result<T> = std::result::Result<T, LibraryError>;

fn invalid(message: impl Into<String>) -> LibraryError {
    LibraryError::new("invalid", format!("invalid: {}", message.into()))
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A folder of a personal library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePersonalFolder {
    pub source_id: i64,
    /// Path inside the source, `/` for all of it.
    pub path: String,
}

/// A personal library of the active profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePersonalLibrary {
    pub id: i64,
    pub name: String,
    /// `none`, `pin` or `password`.
    pub lock: String,
    /// Its contents cannot be read until it is unlocked.
    pub locked: bool,
    /// Videos in it; `None` while locked.
    pub item_count: Option<u32>,
    /// Empty while locked.
    pub folders: Vec<CorePersonalFolder>,
}

impl From<c::PersonalLibraryInfo> for CorePersonalLibrary {
    fn from(l: c::PersonalLibraryInfo) -> Self {
        Self {
            id: l.id,
            name: l.name,
            lock: l.lock.as_str().into(),
            locked: l.locked,
            item_count: l.item_count.map(count),
            folders: l
                .folders
                .into_iter()
                .map(|f| CorePersonalFolder {
                    source_id: f.source_id,
                    path: f.path,
                })
                .collect(),
        }
    }
}

/// A video of a personal library.
#[derive(Debug, Clone, PartialEq)]
pub struct CorePersonalItem {
    pub file_id: i64,
    /// What the label calls take for it: the main-library item id of a
    /// moved, recognized video, else its `file:` key; `None` before the
    /// file is fingerprinted.
    pub label_id: Option<String>,
    pub source_id: i64,
    pub source_uri: String,
    /// Path relative to the source root.
    pub rel_path: String,
    pub title: String,
    pub size_bytes: i64,
    pub duration_ms: Option<i64>,
    pub added_at_ms: i64,
    /// Moved from the main library (it can go back).
    pub moved: bool,
    /// 1–5.
    pub stars: Option<u8>,
    pub tags: Vec<String>,
    pub categories: Vec<i64>,
    /// Where it was left in this library (UI-033); never synced.
    pub progress: Option<CorePersonalProgress>,
}

/// Where a video of a personal library was left (UI-033).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorePersonalProgress {
    pub position_ms: i64,
    /// `None` if unknown.
    pub duration_ms: Option<i64>,
    pub watched: bool,
    pub updated_at_ms: i64,
}

impl From<c::PersonalPosition> for CorePersonalProgress {
    fn from(p: c::PersonalPosition) -> Self {
        Self {
            position_ms: p.position_ms,
            duration_ms: p.duration_ms,
            watched: p.watched,
            updated_at_ms: p.updated_at_ms,
        }
    }
}

impl From<c::PersonalItem> for CorePersonalItem {
    fn from(i: c::PersonalItem) -> Self {
        let label_id = match (i.item, &i.key) {
            (Some(item), _) => Some(item.to_string()),
            (None, Some(key)) => Some(key.to_string()),
            (None, None) => None,
        };
        Self {
            file_id: i.file_id,
            label_id,
            source_id: i.source_id,
            source_uri: i.source_uri,
            rel_path: i.rel_path,
            title: i.title,
            size_bytes: i.size_bytes,
            duration_ms: i.duration_ms,
            added_at_ms: i.added_at_ms,
            moved: i.moved,
            stars: i.stars,
            tags: i.tags,
            categories: i.categories,
            progress: i.progress.map(Into::into),
        }
    }
}

/// Which videos of a personal library to list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CorePersonalFilter {
    pub categories: Vec<i64>,
    pub tags: Vec<String>,
    /// Stars; 0 = not rated.
    pub ratings: Vec<u8>,
    pub text: Option<String>,
}

fn lock_of<'a>(kind: &str, secret: Option<&'a str>) -> Result<c::Lock<'a>> {
    match (kind, secret) {
        ("none", _) => Ok(c::Lock::None),
        ("pin", Some(s)) => Ok(c::Lock::Pin(s)),
        ("password", Some(s)) => Ok(c::Lock::Password(s)),
        _ => Err(invalid(format!("lock {kind:?}"))),
    }
}

fn content_of(s: &str) -> Result<c::ContentType> {
    match s {
        "video" => Ok(c::ContentType::Video),
        "music" => Ok(c::ContentType::Music),
        "personal" => Ok(c::ContentType::Personal),
        _ => Err(invalid(format!("content type {s:?}"))),
    }
}

fn sort_of(s: &str) -> c::PersonalSort {
    match s {
        "title" => c::PersonalSort::Title,
        "rating" => c::PersonalSort::Rating,
        "duration" => c::PersonalSort::Duration,
        "size" => c::PersonalSort::Size,
        _ => c::PersonalSort::Added,
    }
}

impl CoreLibrary {
    /// Personal libraries of the active profile, names shown while locked.
    pub fn personal_libraries(&self) -> Result<Vec<CorePersonalLibrary>> {
        Ok(self
            .catalog()
            .personal_libraries()?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Creates a personal library; `lock` is `none`, `pin` (4–8 digits in
    /// `secret`) or `password` (6–128 characters). It starts unlocked.
    pub fn create_personal_library(
        &self,
        name: String,
        lock: String,
        secret: Option<String>,
    ) -> Result<CorePersonalLibrary> {
        Ok(self
            .catalog()
            .create_personal_library(&name, &lock_of(&lock, secret.as_deref())?)?
            .into())
    }

    pub fn rename_personal_library(&self, id: i64, name: String) -> Result<()> {
        Ok(self.catalog().rename_personal_library(id, &name)?)
    }

    /// Replaces the lock of an unlocked library.
    pub fn set_personal_lock(&self, id: i64, lock: String, secret: Option<String>) -> Result<()> {
        Ok(self
            .catalog()
            .set_personal_lock(id, &lock_of(&lock, secret.as_deref())?)?)
    }

    /// Deletes an unlocked library; moved items return to the main library.
    pub fn delete_personal_library(&self, id: i64) -> Result<()> {
        Ok(self.catalog().delete_personal_library(id)?)
    }

    /// Unlocks a library with its PIN or password (shared wrong-PIN
    /// backoff).
    pub fn unlock_personal_library(&self, id: i64, secret: String) -> Result<CorePinVerdict> {
        Ok(self.catalog().unlock_personal_library(id, &secret)?.into())
    }

    pub fn lock_personal_library(&self, id: i64) {
        self.catalog().lock_personal_library(id);
    }

    /// Locks every personal library (the app went to the background).
    pub fn lock_personal_libraries(&self) {
        self.catalog().lock_personal_libraries();
    }

    /// What a source holds: `video`, `music` or `personal`.
    pub fn source_content(&self, source_id: i64) -> Result<String> {
        Ok(self
            .catalog()
            .source_content(source_id)?
            .as_str()
            .to_owned())
    }

    /// Sets what a source holds; `personal` needs an unlocked library.
    /// Call it before the first scan of a new source.
    pub fn set_source_content(
        &self,
        source_id: i64,
        content: String,
        library_id: Option<i64>,
    ) -> Result<()> {
        Ok(self
            .catalog()
            .set_source_content(source_id, content_of(&content)?, library_id)?)
    }

    /// Adds a folder of a source (path inside it, `""` for all of it) to an
    /// unlocked library.
    pub fn add_personal_folder(&self, id: i64, source_id: i64, path: String) -> Result<()> {
        Ok(self.catalog().add_personal_folder(id, source_id, &path)?)
    }

    pub fn remove_personal_folder(&self, id: i64, source_id: i64, path: String) -> Result<bool> {
        Ok(self
            .catalog()
            .remove_personal_folder(id, source_id, &path)?)
    }

    /// Moves a movie, show or episode of the main library into an unlocked
    /// personal library; how many files moved.
    pub fn move_to_personal(&self, item_id: String, library_id: i64) -> Result<u32> {
        Ok(count(
            self.catalog()
                .move_to_personal(item_ref(&item_id)?, library_id)?,
        ))
    }

    /// Moves a video that came from the main library back; how many files
    /// moved.
    pub fn move_to_main(&self, library_id: i64, file_id: i64) -> Result<u32> {
        Ok(count(self.catalog().move_to_main(library_id, file_id)?))
    }

    /// Videos of an unlocked library. `sort`: `added` (newest first),
    /// `title`, `rating`, `duration`, `size`.
    pub fn personal_items(
        &self,
        id: i64,
        filter: CorePersonalFilter,
        sort: String,
    ) -> Result<Vec<CorePersonalItem>> {
        let filter = c::PersonalFilter {
            categories: filter.categories,
            tags: filter.tags,
            ratings: filter.ratings,
            text: filter.text,
        };
        Ok(self
            .catalog()
            .personal_items(id, &filter, sort_of(&sort))?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Records where a video of an unlocked library was left (UI-033;
    /// "watched" when asked or past 95 %). Stays in the library: never in
    /// the shared "Continue watching" or a sync op. `false` if the file is
    /// not fingerprinted yet (nothing stored).
    pub fn save_personal_progress(
        &self,
        id: i64,
        file_id: i64,
        position_ms: i64,
        duration_ms: i64,
        updated_at_ms: i64,
        watched: bool,
    ) -> Result<bool> {
        Ok(self.catalog().save_personal_progress(
            id,
            file_id,
            position_ms,
            duration_ms,
            updated_at_ms,
            watched,
        )?)
    }

    /// Forgets where a video of an unlocked library was left.
    pub fn clear_personal_progress(&self, id: i64, file_id: i64) -> Result<bool> {
        Ok(self.catalog().clear_personal_progress(id, file_id)?)
    }

    /// "Continue watching" of an unlocked library (UI-033): started and
    /// not finished videos, latest first.
    pub fn personal_continue(&self, id: i64, limit: u32) -> Result<Vec<CorePersonalItem>> {
        Ok(self
            .catalog()
            .personal_continue(id, limit as usize)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// The files that play a video of an unlocked library.
    pub fn personal_play_parts(&self, id: i64, file_id: i64) -> Result<Vec<CorePlayPart>> {
        Ok(self
            .catalog()
            .personal_play_parts(id, file_id)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// The preview frame (JPEG) of a video of an unlocked library (UI-032),
    /// made from the file on first use and cached in the library's own
    /// folder; `None` when the file has no frame worth showing. A locked
    /// library answers `denied:` and nothing is read.
    pub fn personal_frame(&self, id: i64, file_id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self.catalog().personal_frame(
            id,
            file_id,
            &c::frame_search::FfmpegFrames {
                width: c::video_frames::FRAME_WIDTH,
            },
        )?)
    }

    /// Categories of a personal library.
    pub fn personal_categories(&self, id: i64) -> Result<Vec<CoreCategory>> {
        Ok(self
            .catalog()
            .personal_labels(id)
            .categories()?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub fn personal_create_category(&self, id: i64, name: String) -> Result<CoreCategory> {
        Ok(self
            .catalog()
            .personal_labels(id)
            .create_category(&name)?
            .into())
    }

    pub fn personal_rename_category(&self, id: i64, category: i64, name: String) -> Result<()> {
        Ok(self
            .catalog()
            .personal_labels(id)
            .rename_category(category, &name)?)
    }

    pub fn personal_delete_category(&self, id: i64, category: i64) -> Result<()> {
        Ok(self
            .catalog()
            .personal_labels(id)
            .delete_category(category)?)
    }

    /// Puts a video (its `label_id`) into a category or takes it out.
    pub fn personal_set_in_category(
        &self,
        id: i64,
        category: i64,
        label_id: String,
        on: bool,
    ) -> Result<bool> {
        Ok(self.catalog().personal_labels(id).set_in_category(
            category,
            label_target(&label_id)?,
            on,
        )?)
    }

    /// Tags of a personal library, by name.
    pub fn personal_tags(&self, id: i64) -> Result<Vec<CoreTag>> {
        Ok(self
            .catalog()
            .personal_labels(id)
            .tags()?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Autocomplete of the library's tags.
    pub fn personal_suggest_tags(
        &self,
        id: i64,
        prefix: String,
        limit: u32,
    ) -> Result<Vec<CoreTag>> {
        Ok(self
            .catalog()
            .personal_labels(id)
            .suggest_tags(&prefix, limit as usize)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Makes `tags` the tags of a video; whether anything changed.
    pub fn personal_set_tags(&self, id: i64, label_id: String, tags: Vec<String>) -> Result<bool> {
        Ok(self
            .catalog()
            .personal_labels(id)
            .set_tags(label_target(&label_id)?, &tags)?)
    }

    /// Rates a video 1–5 stars or clears it (`None`).
    pub fn personal_set_rating(&self, id: i64, label_id: String, stars: Option<u8>) -> Result<()> {
        self.catalog()
            .personal_labels(id)
            .set_rating(label_target(&label_id)?, stars)?;
        Ok(())
    }
}
