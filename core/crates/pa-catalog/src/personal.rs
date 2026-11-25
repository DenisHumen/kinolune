// SPDX-License-Identifier: MPL-2.0
//! Personal media (UI-025, "Личные медиа"): the user's own videos in
//! personal libraries of the active profile, with locks, outside the main
//! library.
//!
//! - A source added with [`ContentType::Personal`] is never recognized and
//!   never reaches a metadata provider; its files and those of any folder
//!   given to a library ([`Catalog::add_personal_folder`]) belong to that
//!   library. An item of the main library can be moved into a library and
//!   back ([`Catalog::move_to_personal`], [`Catalog::move_to_main`]): it
//!   keeps its match, so it comes back as it was.
//! - No query of the main library returns a personal file: `file_visible!`
//!   leaves them out (pa_library::personal). Lists, search, "Continue
//!   watching", recently added (the app's notices, Watch Next and Top Shelf
//!   read these), details, playback and the file browser all go through it.
//! - A library is read only through the calls here, one call at a time in
//!   its scope, and only while it is unlocked: a library without a lock
//!   always is, one with a PIN or password after
//!   [`Catalog::unlock_personal_library`] until
//!   [`Catalog::lock_personal_library`], a profile switch or the end of the
//!   process. A locked library answers [`Error::Denied`]; one of another
//!   profile [`Error::NotFound`]. PIN and password checks share the
//!   wrong-PIN backoff of profiles.
//! - Categories, tags and ratings (LIB-022) of a library live in its own
//!   label space ([`Catalog::personal_labels`]).
//! - Playback positions (UI-033, [`Catalog::save_personal_progress`]) live
//!   in the library too (`personal_progress`), under the item key of a
//!   moved video or the `file:` key of any other: the library's own
//!   "Continue watching" ([`Catalog::personal_continue`]) reads them, the
//!   main one, the sync outbox and other profiles never do.

use std::collections::HashSet;

use pa_library::labels::{self as lab, Scope};
use pa_library::lists::Entry;
use pa_library::personal::{self as lib, PersonalLibrary};
use pa_library::progress::Thresholds;
use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_model::MediaKey;
use pa_scanner::MediaKind;

use crate::items::{self, ItemRef, relative};
use crate::labels::{LabelSpace, Labels};
use crate::profiles::PinVerdict;
use crate::progress::profile;
use crate::{Catalog, Error, PlayPart, now_ms};

pub use pa_library::personal::{ContentType, Lock, LockKind, PersonalFolder, PersonalPosition};

/// Which personal libraries are unlocked in this process. Never stored.
#[derive(Debug, Default)]
pub(crate) struct PersonalSession {
    unlocked: HashSet<i64>,
}

/// A personal library as the app lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalLibraryInfo {
    pub id: i64,
    pub name: String,
    pub lock: LockKind,
    /// Its contents cannot be read until it is unlocked.
    pub locked: bool,
    /// Videos in it; `None` while locked.
    pub item_count: Option<usize>,
    /// Its folders; empty while locked.
    pub folders: Vec<PersonalFolder>,
}

/// A video of a personal library.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonalItem {
    /// `files.id`.
    pub file_id: i64,
    /// Its `file:` key, `None` until the file is fingerprinted (it cannot
    /// be labelled before).
    pub key: Option<MediaKey>,
    pub source_id: i64,
    /// Root URI of the source.
    pub source_uri: String,
    /// Path relative to the source root.
    pub rel_path: String,
    /// The recognized title of an item moved from the main library, else
    /// the file name without its extension.
    pub title: String,
    pub size_bytes: i64,
    pub duration_ms: Option<i64>,
    pub added_at_ms: i64,
    /// Moved from the main library ([`Catalog::move_to_main`] puts it
    /// back).
    pub moved: bool,
    /// The main-library item of a moved, recognized file.
    pub item: Option<ItemRef>,
    /// 1–5 stars of the active profile.
    pub stars: Option<u8>,
    /// Tags in this library, by name.
    pub tags: Vec<String>,
    /// Categories of this library that hold it.
    pub categories: Vec<i64>,
    /// Where it was left in this library (UI-033); never synced.
    pub progress: Option<PersonalPosition>,
}

/// Which videos of a library to list. Within a field any value matches,
/// between fields all must (as LIB-009).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersonalFilter {
    /// Category ids.
    pub categories: Vec<i64>,
    /// Tag names (any case).
    pub tags: Vec<String>,
    /// Stars; `0` = not rated.
    pub ratings: Vec<u8>,
    /// Words of the title (case-insensitive).
    pub text: Option<String>,
}

/// Order of a library's videos.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PersonalSort {
    /// Newest first.
    #[default]
    Added,
    Title,
    /// Best first; unrated last.
    Rating,
    /// Longest first.
    Duration,
    /// Largest first.
    Size,
}

fn denied_locked(id: i64) -> Error {
    Error::Denied(format!("personal library {id} is locked"))
}

fn not_found(id: i64) -> Error {
    Error::NotFound(format!("personal library {id}"))
}

fn invalid(e: pa_library::Error) -> Error {
    match e {
        pa_library::Error::Invalid(what) => Error::Invalid(what.to_owned()),
        other => Error::Library(other),
    }
}

/// The label space key of a personal library.
pub(crate) fn space_key(id: i64) -> String {
    format!("personal:{id}")
}

/// A catalog connection that reads one personal library until dropped.
pub(crate) struct Scoped<'a> {
    conn: std::sync::MutexGuard<'a, Connection>,
    scoped: bool,
}

impl std::ops::Deref for Scoped<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.conn
    }
}

impl std::ops::DerefMut for Scoped<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

impl Drop for Scoped<'_> {
    fn drop(&mut self) {
        if self.scoped && lib::set_scope(&self.conn, None).is_err() {
            // The scope must never outlive the call: without it cleared the
            // next query of the main library would read this library.
            let _ = self
                .conn
                .execute_batch("DROP TABLE IF EXISTS temp.personal_scope");
        }
    }
}

/// SQL: the main-library item (`ItemRef` text) of the file row `f`, if it
/// was recognized (a moved item, or a file recognized before its folder
/// became personal); NULL otherwise.
macro_rules! item_of_file {
    () => {
        "(SELECT CASE WHEN w.kind = 'movie' THEN 'm' || w.id
                      ELSE ifnull((SELECT 'e' || min(me.episode_id) FROM match_episodes me
                                    WHERE me.match_id = m.id), 's' || w.id) END
            FROM matches m JOIN works w ON w.id = m.work_id
           WHERE m.file_id = f.id AND m.state <> 'rejected' ORDER BY m.id LIMIT 1)"
    };
}

/// Files of the library in scope the active profile may open, with what
/// the list needs.
const ITEMS_SQL: &str = concat!(
    "SELECT f.id, f.fingerprint, f.source_id, s.uri, f.path, f.size_bytes, f.duration_ms,
            f.added_at_ms,
            EXISTS (SELECT 1 FROM personal_library_files x WHERE x.file_id = f.id), ",
    item_of_file!(),
    " FROM files f
     JOIN sources s ON s.id = f.source_id
     WHERE f.missing_since_ms IS NULL
       AND NOT EXISTS (SELECT 1 FROM group_assets ga WHERE ga.file_id = f.id AND ga.role = 'ignore')
       AND ",
    pa_library::file_allowed!("f"),
    " ORDER BY f.id"
);

fn file_stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_owned(),
        _ => name.to_owned(),
    }
}

impl Catalog {
    fn personal_session(&self) -> std::sync::MutexGuard<'_, PersonalSession> {
        self.personal.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A library of the active profile, else [`Error::NotFound`].
    fn own_library(conn: &Connection, id: i64) -> Result<PersonalLibrary, Error> {
        lib::get(conn, id)?
            .filter(|l| profile(conn).is_ok_and(|p| p == l.profile_id))
            .ok_or_else(|| not_found(id))
    }

    /// `true` if library `id` (of any profile) can be read now.
    fn is_unlocked(&self, l: &PersonalLibrary) -> bool {
        l.lock == LockKind::None || self.personal_session().unlocked.contains(&l.id)
    }

    /// A library of the active profile that is unlocked, else
    /// [`Error::NotFound`] or [`Error::Denied`].
    fn require_unlocked(&self, conn: &Connection, id: i64) -> Result<PersonalLibrary, Error> {
        let l = Self::own_library(conn, id)?;
        if self.is_unlocked(&l) {
            Ok(l)
        } else {
            Err(denied_locked(id))
        }
    }

    /// The catalog connection reading library `id` (`Some`, which must be
    /// unlocked) or the main library (`None`).
    pub(crate) fn scoped(&self, id: Option<i64>) -> Result<Scoped<'_>, Error> {
        let conn = self.lock();
        let Some(id) = id else {
            return Ok(Scoped {
                conn,
                scoped: false,
            });
        };
        self.require_unlocked(&conn, id)?;
        lib::set_scope(&conn, Some(id))?;
        Ok(Scoped { conn, scoped: true })
    }

    /// `true` if the active profile may open `path` of `source_id`, a file
    /// of an unlocked personal library of its own included.
    pub(crate) fn may_open_personal(
        &self,
        conn: &Connection,
        source_id: i64,
        path: &str,
    ) -> Result<Option<bool>, Error> {
        let Some(library) = lib::library_of_path(conn, source_id, path)? else {
            return Ok(None);
        };
        let Some(id) = library else {
            return Ok(Some(false));
        };
        let Ok(l) = Self::own_library(conn, id) else {
            return Ok(Some(false));
        };
        if !self.is_unlocked(&l) {
            return Ok(Some(false));
        }
        lib::set_scope(conn, Some(id))?;
        let allowed = crate::profiles::may_open(conn, source_id, path);
        lib::set_scope(conn, None)?;
        allowed.map(Some)
    }

    fn library_info(
        &self,
        conn: &Connection,
        l: PersonalLibrary,
    ) -> Result<PersonalLibraryInfo, Error> {
        let locked = !self.is_unlocked(&l);
        let (item_count, folders) = if locked {
            (None, Vec::new())
        } else {
            lib::set_scope(conn, Some(l.id))?;
            let n = personal_rows(conn);
            lib::set_scope(conn, None)?;
            (Some(n?.len()), lib::folders(conn, l.id)?)
        };
        Ok(PersonalLibraryInfo {
            id: l.id,
            name: l.name,
            lock: l.lock,
            locked,
            item_count,
            folders,
        })
    }

    /// The personal libraries of the active profile in the user's order.
    /// Names are shown while locked; contents are not.
    pub fn personal_libraries(&self) -> Result<Vec<PersonalLibraryInfo>, Error> {
        let conn = self.lock();
        lib::list(&conn, profile(&conn)?)?
            .into_iter()
            .map(|l| self.library_info(&conn, l))
            .collect()
    }

    /// One library of the active profile.
    pub fn personal_library(&self, id: i64) -> Result<PersonalLibraryInfo, Error> {
        let conn = self.lock();
        let l = Self::own_library(&conn, id)?;
        self.library_info(&conn, l)
    }

    /// Creates a library of the active profile; it starts unlocked.
    pub fn create_personal_library(
        &self,
        name: &str,
        lock: &Lock<'_>,
    ) -> Result<PersonalLibraryInfo, Error> {
        let conn = self.lock();
        let l = lib::create(&conn, profile(&conn)?, name, lock, now_ms()).map_err(invalid)?;
        self.personal_session().unlocked.insert(l.id);
        self.library_info(&conn, l)
    }

    /// Renames an unlocked library.
    pub fn rename_personal_library(&self, id: i64, name: &str) -> Result<(), Error> {
        let conn = self.lock();
        self.require_unlocked(&conn, id)?;
        lib::rename(&conn, id, name, now_ms()).map_err(invalid)
    }

    /// Replaces the lock of an unlocked library; it stays unlocked.
    pub fn set_personal_lock(&self, id: i64, lock: &Lock<'_>) -> Result<(), Error> {
        let conn = self.lock();
        self.require_unlocked(&conn, id)?;
        lib::set_lock(&conn, id, lock, now_ms()).map_err(invalid)?;
        self.personal_session().unlocked.insert(id);
        Ok(())
    }

    /// Deletes an unlocked library with its folders, labels space, preview
    /// frames (UI-032) and moved-file marks: moved items go back to the main library. Files of
    /// `personal` sources stay out of the main library.
    pub fn delete_personal_library(&self, id: i64) -> Result<(), Error> {
        let mut conn = self.lock();
        self.require_unlocked(&conn, id)?;
        let tx = conn.transaction()?;
        let space = space_key(id);
        tx.execute("DELETE FROM user_categories WHERE space = ?1", [&space])?;
        tx.execute("DELETE FROM user_tags WHERE space = ?1", [&space])?;
        lib::delete(&tx, id)?;
        tx.commit()?;
        self.personal_session().unlocked.remove(&id);
        // UI-032: its preview frames go with it.
        self.prune_personal_frames(&conn);
        pa_library::search::sync(&conn)?;
        Ok(())
    }

    /// Unlocks a library with its PIN or password until it is locked again.
    /// Counted by the wrong-PIN backoff; a library without a lock opens
    /// without a check.
    pub fn unlock_personal_library(&self, id: i64, secret: &str) -> Result<PinVerdict, Error> {
        let conn = self.lock();
        let l = Self::own_library(&conn, id)?;
        if l.lock == LockKind::None {
            return Ok(PinVerdict {
                ok: true,
                token: None,
                locked: false,
                attempts: pa_library::profiles::pin_attempts(&conn, now_ms())?,
            });
        }
        let v = self.check_pin(&conn, |c| Ok((lib::verify_secret(c, id, secret)?, false)))?;
        if v.ok {
            self.personal_session().unlocked.insert(id);
        }
        Ok(v)
    }

    /// Locks a library again (no-op for one without a lock).
    pub fn lock_personal_library(&self, id: i64) {
        self.personal_session().unlocked.remove(&id);
    }

    /// Locks every library (the app went to the background, the profile
    /// changed).
    pub fn lock_personal_libraries(&self) {
        self.personal_session().unlocked.clear();
    }

    /// What a source holds.
    pub fn source_content(&self, source_id: i64) -> Result<ContentType, Error> {
        lib::content_type(&self.lock(), source_id)?
            .ok_or_else(|| Error::NotFound(format!("source {source_id}")))
    }

    /// Sets what a source holds. [`ContentType::Personal`] needs `library`
    /// (unlocked): the whole source joins it, and its files are never
    /// recognized again (works recognized before are dropped with their
    /// matches). Leaving `Personal` needs the source's library unlocked and
    /// takes the source out of it; the next scan recognizes its files.
    pub fn set_source_content(
        &self,
        source_id: i64,
        content: ContentType,
        library: Option<i64>,
    ) -> Result<(), Error> {
        let mut conn = self.lock();
        let current = lib::content_type(&conn, source_id)?
            .ok_or_else(|| Error::NotFound(format!("source {source_id}")))?;
        let holder: Option<i64> = conn
            .query_row(
                "SELECT library_id FROM personal_library_folders WHERE source_id = ?1 AND path = '/'",
                [source_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(h) = holder
            && (content != ContentType::Personal || library != Some(h))
        {
            self.require_unlocked(&conn, h)?;
        }
        if content == ContentType::Personal {
            let Some(id) = library else {
                return Err(Error::Invalid("a personal source needs a library".into()));
            };
            self.require_unlocked(&conn, id)?;
        }
        let tx = conn.transaction()?;
        lib::set_content_type(&tx, source_id, content).map_err(invalid)?;
        if let Some(h) = holder {
            lib::remove_folder(&tx, h, source_id, "/")?;
        }
        if content == ContentType::Personal {
            let id = library.unwrap_or_default();
            lib::add_folder(&tx, id, source_id, "/", now_ms()).map_err(invalid)?;
            if current != ContentType::Personal {
                forget_recognition(&tx, source_id, None)?;
            }
        }
        tx.commit()?;
        pa_library::search::sync(&conn)?;
        drop(conn);
        // UI-034: a source other devices know leaves them now that it is
        // personal (and comes back when it stops being personal).
        if (current == ContentType::Personal) != (content == ContentType::Personal) {
            self.sync_sources()?;
        }
        Ok(())
    }

    /// Adds a folder (relative to the source root, `""` for all of it) of a
    /// source to an unlocked library. Its files leave the main library;
    /// unless they were recognized, they never will be.
    pub fn add_personal_folder(&self, id: i64, source_id: i64, path: &str) -> Result<(), Error> {
        let mut conn = self.lock();
        self.require_unlocked(&conn, id)?;
        if lib::content_type(&conn, source_id)?.is_none() {
            return Err(Error::NotFound(format!("source {source_id}")));
        }
        let tx = conn.transaction()?;
        lib::add_folder(&tx, id, source_id, path, now_ms()).map_err(invalid)?;
        tx.commit()?;
        pa_library::search::sync(&conn)?;
        Ok(())
    }

    /// Takes a folder out of an unlocked library: its files return to the
    /// main library (those of a `personal` source stay hidden and
    /// unrecognized). `false` if it was not there.
    pub fn remove_personal_folder(
        &self,
        id: i64,
        source_id: i64,
        path: &str,
    ) -> Result<bool, Error> {
        let conn = self.lock();
        self.require_unlocked(&conn, id)?;
        let removed = lib::remove_folder(&conn, id, source_id, path)?;
        pa_library::search::sync(&conn)?;
        Ok(removed)
    }

    /// Moves an item of the main library (a movie with all its versions, a
    /// whole show, or an episode) into an unlocked library. Returns how
    /// many files moved. The item keeps its match: it disappears from every
    /// main-library list and comes back with [`Catalog::move_to_main`].
    pub fn move_to_personal(&self, item: ItemRef, id: i64) -> Result<usize, Error> {
        let mut conn = self.lock();
        self.require_unlocked(&conn, id)?;
        crate::profiles::require_visible(&conn, item)?;
        let files = item_files(&conn, item)?;
        let tx = conn.transaction()?;
        lib::move_files(&tx, id, &files, now_ms())?;
        tx.commit()?;
        pa_library::search::sync(&conn)?;
        Ok(files.len())
    }

    /// Moves a video that came from the main library back there, together
    /// with the other moved files of its movie or show in this library.
    /// Returns how many files moved; a video of a personal folder is
    /// `Invalid` (take the folder out instead).
    pub fn move_to_main(&self, id: i64, file_id: i64) -> Result<usize, Error> {
        let mut conn = self.lock();
        self.require_unlocked(&conn, id)?;
        let moved_here: bool = conn
            .query_row(
                "SELECT 1 FROM personal_library_files WHERE file_id = ?1 AND library_id = ?2",
                params![file_id, id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !moved_here {
            let in_library = conn.query_row(
                concat!(
                    "SELECT ",
                    pa_library::file_personal_library!("f"),
                    " FROM files f WHERE f.id = ?1"
                ),
                [file_id],
                |r| r.get::<_, Option<i64>>(0),
            );
            return match in_library.optional()?.flatten() {
                Some(l) if l == id => Err(Error::Invalid(format!(
                    "file {file_id} is in a folder of the library, not moved"
                ))),
                _ => Err(Error::NotFound(format!("file {file_id}"))),
            };
        }
        let mut stmt = conn.prepare(
            "SELECT DISTINCT x.file_id FROM personal_library_files x
             WHERE x.library_id = ?2
               AND (x.file_id = ?1 OR EXISTS (
                    SELECT 1 FROM matches a JOIN matches b ON b.work_id = a.work_id
                     WHERE a.file_id = ?1 AND a.state <> 'rejected'
                       AND b.file_id = x.file_id AND b.state <> 'rejected'))",
        )?;
        let files: Vec<i64> = stmt
            .query_map(params![file_id, id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        drop(stmt);
        let tx = conn.transaction()?;
        let n = lib::unmove_files(&tx, &files)?;
        tx.commit()?;
        pa_library::search::sync(&conn)?;
        Ok(n)
    }

    /// The videos of an unlocked library that match `filter`, in `sort`
    /// order.
    pub fn personal_items(
        &self,
        id: i64,
        filter: &PersonalFilter,
        sort: PersonalSort,
    ) -> Result<Vec<PersonalItem>, Error> {
        let conn = self.scoped(Some(id))?;
        let mut items = personal_rows(&conn)?;
        label_items(&conn, id, &mut items)?;
        fill_progress(&conn, id, &mut items)?;
        let tag_keys: Vec<String> = filter.tags.iter().map(|t| lab::name_key(t)).collect();
        let words: Vec<String> = filter
            .text
            .as_deref()
            .unwrap_or("")
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        items.retain(|i| {
            (filter.categories.is_empty()
                || i.categories.iter().any(|c| filter.categories.contains(c)))
                && (tag_keys.is_empty()
                    || i.tags.iter().any(|t| tag_keys.contains(&lab::name_key(t))))
                && (filter.ratings.is_empty() || filter.ratings.contains(&i.stars.unwrap_or(0)))
                && {
                    let title = i.title.to_lowercase();
                    words.iter().all(|w| title.contains(w.as_str()))
                }
        });
        let by_title = |a: &PersonalItem, b: &PersonalItem| {
            a.title
                .to_lowercase()
                .cmp(&b.title.to_lowercase())
                .then(a.file_id.cmp(&b.file_id))
        };
        match sort {
            PersonalSort::Added => items.sort_by(|a, b| {
                b.added_at_ms
                    .cmp(&a.added_at_ms)
                    .then_with(|| by_title(a, b))
            }),
            PersonalSort::Title => items.sort_by(by_title),
            PersonalSort::Rating => items.sort_by(|a, b| {
                (b.stars.is_some(), b.stars)
                    .cmp(&(a.stars.is_some(), a.stars))
                    .then_with(|| by_title(a, b))
            }),
            PersonalSort::Duration => items.sort_by(|a, b| {
                b.duration_ms
                    .cmp(&a.duration_ms)
                    .then_with(|| by_title(a, b))
            }),
            PersonalSort::Size => {
                items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then_with(|| by_title(a, b)))
            }
        }
        Ok(items)
    }

    /// The files that play a video of an unlocked library (all parts of a
    /// split file, in order). Its playback through the stream proxy is
    /// allowed while the library stays unlocked.
    pub fn personal_play_parts(&self, id: i64, file_id: i64) -> Result<Vec<PlayPart>, Error> {
        let conn = self.scoped(Some(id))?;
        let row: Option<(i64, String)> = conn
            .query_row(
                concat!(
                    "SELECT f.source_id, f.path FROM files f WHERE f.id = ?1 AND ",
                    pa_library::file_allowed!("f")
                ),
                [file_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((source_id, path)) = row else {
            return Err(Error::NotFound(format!("file {file_id}")));
        };
        crate::play::play_parts(&conn, source_id, &relative(&path))
    }

    /// Records a position report of the player for a video of an unlocked
    /// library (UI-033), and "watched" when asked or past 95 %. The
    /// position stays in this library: not in the main "Continue
    /// watching", not in a sync op, not visible to another profile.
    /// Returns `false` if the video has no key yet (its file is not
    /// fingerprinted): nothing is stored.
    pub fn save_personal_progress(
        &self,
        id: i64,
        file_id: i64,
        position_ms: i64,
        duration_ms: i64,
        updated_at_ms: i64,
        watched: bool,
    ) -> Result<bool, Error> {
        let conn = self.scoped(Some(id))?;
        let Some(key) = file_progress_key(&conn, file_id)? else {
            return Ok(false);
        };
        let position = position_ms.max(0);
        let duration = (duration_ms > 0).then_some(duration_ms);
        let th = Thresholds::default();
        let complete = th.is_complete(
            u64::try_from(position).unwrap_or(0),
            duration.and_then(|d| u64::try_from(d).ok()),
        );
        lib::save_position(
            &conn,
            id,
            &key,
            &PersonalPosition {
                position_ms: position,
                duration_ms: duration,
                watched: watched || complete,
                updated_at_ms,
            },
        )?;
        if let Some(d) = duration {
            // The duration the player saw fills the file until probing;
            // not a part of a split movie, whose timeline is all its parts
            // (BUG-021).
            conn.execute(
                concat!(
                    "UPDATE files SET duration_ms = ?2 WHERE id = ?1 AND duration_ms IS NULL AND ",
                    crate::items::not_a_part!()
                ),
                params![file_id, d],
            )?;
        }
        Ok(true)
    }

    /// Forgets where a video of an unlocked library was left (it leaves
    /// the library's "Continue watching"); `false` if nothing was stored.
    pub fn clear_personal_progress(&self, id: i64, file_id: i64) -> Result<bool, Error> {
        let conn = self.scoped(Some(id))?;
        match file_progress_key(&conn, file_id)? {
            Some(key) => Ok(lib::clear_position(&conn, id, &key)?),
            None => Ok(false),
        }
    }

    /// "Continue watching" of an unlocked library (UI-033): its videos
    /// started and not finished (between 2 % and 95 %), latest first, at
    /// most `limit`. Only this call and [`Catalog::personal_items`] read
    /// these positions.
    pub fn personal_continue(&self, id: i64, limit: usize) -> Result<Vec<PersonalItem>, Error> {
        let th = Thresholds::default();
        let mut items: Vec<PersonalItem> = self
            .personal_items(id, &PersonalFilter::default(), PersonalSort::Added)?
            .into_iter()
            .filter(|i| {
                i.progress.is_some_and(|p| {
                    !p.watched
                        && th.is_in_progress(
                            u64::try_from(p.position_ms).unwrap_or(0),
                            p.duration_ms
                                .or(i.duration_ms)
                                .and_then(|d| u64::try_from(d).ok()),
                        )
                })
            })
            .collect();
        items.sort_by(|a, b| {
            let at = |i: &PersonalItem| i.progress.map_or(0, |p| p.updated_at_ms);
            at(b).cmp(&at(a)).then(a.file_id.cmp(&b.file_id))
        });
        // One entry per item: the versions of a moved movie share a key.
        let mut seen = HashSet::new();
        items.retain(|i| {
            seen.insert(
                i.item
                    .map_or_else(|| i.file_id.to_string(), |r| r.to_string()),
            )
        });
        items.truncate(limit);
        Ok(items)
    }

    /// Categories, tags and ratings of personal library `id`. Every call
    /// of the handle checks that the library is unlocked.
    pub fn personal_labels(&self, id: i64) -> Labels<'_> {
        self.labels_in(LabelSpace::Personal(id))
    }
}

/// Present files of an item of the main library.
fn item_files(conn: &Connection, item: ItemRef) -> Result<Vec<i64>, Error> {
    let (sql, id) = match item {
        ItemRef::Movie(w) | ItemRef::Show(w) => (
            "SELECT DISTINCT m.file_id FROM matches m JOIN files f ON f.id = m.file_id
             WHERE m.work_id = ?1 AND m.state <> 'rejected' AND f.missing_since_ms IS NULL",
            w,
        ),
        ItemRef::Episode(e) => (
            "SELECT DISTINCT m.file_id FROM match_episodes me
               JOIN matches m ON m.id = me.match_id
               JOIN files f ON f.id = m.file_id
             WHERE me.episode_id = ?1 AND m.state <> 'rejected' AND f.missing_since_ms IS NULL",
            e,
        ),
        ItemRef::Track(_) => {
            return Err(Error::Invalid(format!(
                "{item}: only video moves to personal media"
            )));
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let files: Vec<i64> = stmt
        .query_map([id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    if files.is_empty() {
        return Err(Error::NotFound(format!("item {item}")));
    }
    Ok(files)
}

/// Drops what recognition made of the files of `source_id` (under `folder`
/// if given): automatic matches and parse results, so the files stay
/// unrecognized. Works left without files go too.
fn forget_recognition(
    conn: &Connection,
    source_id: i64,
    folder: Option<&str>,
) -> Result<(), Error> {
    let folder = folder.map(lib::folder_path).unwrap_or_else(|| "/".into());
    let in_folder = "f.source_id = ?1
          AND (?2 = '/' OR f.path = ?2 OR substr(f.path, 1, length(?2) + 1) = ?2 || '/')";
    conn.execute(
        &format!(
            "DELETE FROM matches WHERE file_id IN (SELECT f.id FROM files f WHERE {in_folder})
               AND file_id NOT IN (SELECT file_id FROM personal_library_files)"
        ),
        params![source_id, folder],
    )?;
    conn.execute(
        &format!(
            "DELETE FROM parse_results WHERE file_id IN (SELECT f.id FROM files f WHERE {in_folder})
               AND file_id NOT IN (SELECT file_id FROM personal_library_files)"
        ),
        params![source_id, folder],
    )?;
    crate::recognize::remove_orphans(conn)?;
    Ok(())
}

/// The videos of the library in the connection's scope, without labels.
fn personal_rows(conn: &Connection) -> Result<Vec<PersonalItem>, Error> {
    let mut stmt = conn.prepare_cached(ITEMS_SQL)?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, Option<i64>>(6)?,
            r.get::<_, i64>(7)?,
            r.get::<_, bool>(8)?,
            r.get::<_, Option<String>>(9)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (file_id, fp, source_id, uri, path, size, duration, added, moved, item) = row?;
        if pa_scanner::media_kind(&path) != Some(MediaKind::Video) {
            continue;
        }
        let item: Option<ItemRef> = item.and_then(|s| s.parse().ok());
        let title = match item.map(|i| items::item(conn, i)).transpose()?.flatten() {
            Some(it) => match (it.show_id, it.season, it.episode) {
                (Some(show), Some(s), Some(e)) => {
                    let show = items::item(conn, show)?.map_or(it.title.clone(), |sh| sh.title);
                    format!("{show} · S{s:02}E{e:02}")
                }
                _ => it.title,
            },
            None => file_stem(&path),
        };
        out.push(PersonalItem {
            file_id,
            key: fp.map(|fingerprint| MediaKey::File { fingerprint }),
            source_id,
            source_uri: uri,
            rel_path: relative(&path),
            title,
            size_bytes: size,
            duration_ms: duration,
            added_at_ms: added,
            moved,
            item,
            stars: None,
            tags: Vec::new(),
            categories: Vec::new(),
            progress: None,
        });
    }
    Ok(out)
}

/// The key a video's position is stored under in a personal library: the
/// item key of a recognized (moved) video, else its `file:` key. `None`
/// for a file outside the library in scope, or not fingerprinted yet.
fn file_progress_key(conn: &Connection, file_id: i64) -> Result<Option<String>, Error> {
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            concat!(
                "SELECT f.fingerprint, ",
                item_of_file!(),
                " FROM files f WHERE f.id = ?1 AND f.missing_since_ms IS NULL AND ",
                pa_library::file_allowed!("f")
            ),
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((fingerprint, item)) = row else {
        return Err(Error::NotFound(format!("file {file_id}")));
    };
    let item: Option<ItemRef> = item.and_then(|s| s.parse().ok());
    progress_key(
        conn,
        item,
        fingerprint.map(|fingerprint| MediaKey::File { fingerprint }),
    )
}

fn progress_key(
    conn: &Connection,
    item: Option<ItemRef>,
    file_key: Option<MediaKey>,
) -> Result<Option<String>, Error> {
    if let Some(item) = item
        && let Some((key, _)) = crate::progress::key_of(conn, item)?
    {
        return Ok(Some(key.to_string()));
    }
    Ok(file_key.map(|k| k.to_string()))
}

/// Fills the positions of `items` (library `id` in scope).
fn fill_progress(conn: &Connection, id: i64, items: &mut [PersonalItem]) -> Result<(), Error> {
    let positions = lib::positions(conn, id)?;
    if positions.is_empty() {
        return Ok(());
    }
    for i in items.iter_mut() {
        if let Some(key) = progress_key(conn, i.item, i.key.clone())? {
            i.progress = positions.get(&key).copied();
        }
    }
    Ok(())
}

/// Fills the stars, tags and categories of `items` (library `id` in
/// scope).
fn label_items(conn: &Connection, id: i64, items: &mut [PersonalItem]) -> Result<(), Error> {
    let space = space_key(id);
    let scope = Scope {
        profile_id: profile(conn)?,
        space: &space,
    };
    for i in items.iter_mut() {
        let entry = match (i.item, &i.key) {
            (Some(item), _) => crate::lists::entry_of(conn, item)?,
            (None, Some(key)) => Some(Entry::Media(key.to_string())),
            (None, None) => None,
        };
        let Some(entry) = entry else { continue };
        i.stars = lab::rating(conn, scope.profile_id, &entry)?;
        i.tags = lab::tags_of(conn, scope, &entry)?
            .into_iter()
            .map(|t| t.name)
            .collect();
        i.categories = lab::categories_of(conn, scope, &entry)?
            .into_iter()
            .map(|c| c.id)
            .collect();
    }
    Ok(())
}
