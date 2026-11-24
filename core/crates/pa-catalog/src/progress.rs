// SPDX-License-Identifier: MPL-2.0
//! Progress of items on the active profile, through `pa_library::progress`
//! (LIB-011): position reports, "Continue watching" with Up Next.
//!
//! Progress is stored under the stable media key of an item: `tmdb:movie:` /
//! `tmdb:episode:` when the work has a TMDB id, otherwise `file:` with the
//! fingerprint of the item's file ([`crate::fingerprint`]).

use pa_library::progress::{
    self, ContinueQuery, ContinueReason, LocalOnly, PositionReport, Thresholds,
};
use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_model::{MediaKey, MediaKind};

use crate::items::{self, Item, ItemRef};
use crate::{Catalog, Error, LOCAL_DEVICE_ID};
use pa_sync::library::AliasedSink;

/// The active profile of the connection (LIB-012,
/// [`pa_library::profiles::activate`]); the default profile before any.
pub(crate) fn profile(conn: &Connection) -> Result<i64, Error> {
    Ok(pa_library::profiles::active(conn)?.unwrap_or(pa_library::profiles::DEFAULT_PROFILE))
}

/// Playback state of an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemProgress {
    pub item: ItemRef,
    pub position_ms: i64,
    /// 0 if unknown.
    pub duration_ms: i64,
    pub updated_at_ms: i64,
    pub watched: bool,
}

/// One entry of "Continue watching".
#[derive(Debug, Clone, PartialEq)]
pub struct ContinueItem {
    pub item: Item,
    pub progress: ItemProgress,
    /// The next episode after a finished one (Up Next), not a started item.
    pub up_next: bool,
}

/// The watch state an item had before a bulk mark (UI-049): what
/// «Отменить» puts back with [`Catalog::restore_watch_marks`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchMark {
    pub item: ItemRef,
    pub position_ms: i64,
    /// 0 if unknown.
    pub duration_ms: i64,
    pub watched: bool,
}

/// Runs `f` in one transaction of `conn` (UI-049): the progress writers of
/// `pa_library` open savepoints inside it, so a whole season is marked or
/// put back at once, or not at all.
fn in_transaction<T>(
    conn: &mut Connection,
    f: impl FnOnce(&mut Connection) -> Result<T, Error>,
) -> Result<T, Error> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let out = f(conn).and_then(|v| {
        conn.execute_batch("COMMIT")?;
        Ok(v)
    });
    if out.is_err() && !conn.is_autocommit() {
        let _ = conn.execute_batch("ROLLBACK");
    }
    out
}

/// `file:` key of the file with this id, if it is fingerprinted.
fn file_key(conn: &Connection, file_id: i64) -> Result<Option<MediaKey>, Error> {
    let fp: Option<String> = conn
        .query_row(
            "SELECT fingerprint FROM files WHERE id = ?1",
            [file_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    Ok(fp.map(|fingerprint| MediaKey::File { fingerprint }))
}

/// The key progress of `item` is stored under, and its kind.
pub(crate) fn key_of(
    conn: &Connection,
    item: ItemRef,
) -> Result<Option<(MediaKey, MediaKind)>, Error> {
    Ok(match item {
        ItemRef::Movie(work) => {
            let tmdb: Option<String> = conn
                .query_row(
                    "SELECT ext_id FROM external_ids
                     WHERE work_id = ?1 AND provider = 'tmdb' AND ext_kind = 'movie'",
                    [work],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = tmdb.and_then(|t| t.parse().ok()) {
                Some((MediaKey::TmdbMovie { id }, MediaKind::Movie))
            } else {
                let file: Option<i64> = conn
                    .query_row(
                        "SELECT f.id FROM matches m JOIN files f ON f.id = m.file_id
                         WHERE m.work_id = ?1 AND m.state <> 'rejected' AND f.fingerprint IS NOT NULL
                         ORDER BY f.missing_since_ms IS NOT NULL, f.id LIMIT 1",
                        [work],
                        |r| r.get(0),
                    )
                    .optional()?;
                match file {
                    Some(f) => file_key(conn, f)?.map(|k| (k, MediaKind::Movie)),
                    None => None,
                }
            }
        }
        ItemRef::Episode(ep) => progress::episode_keys(conn, ep)?
            .into_iter()
            .find(MediaKey::is_v1_wire)
            .map(|k| (k, MediaKind::Episode)),
        ItemRef::Track(track) => {
            let file: Option<i64> = conn
                .query_row(
                    "SELECT file_id FROM music_tracks WHERE id = ?1",
                    [track],
                    |r| r.get(0),
                )
                .optional()?;
            match file {
                Some(f) => file_key(conn, f)?.map(|k| (k, MediaKind::Track)),
                None => None,
            }
        }
        ItemRef::Show(_) => None,
    })
}

/// The item a stored key belongs to: the key itself, else through the keys
/// other devices linked to it (SYNC-004, [`crate::matching::resolve`]).
pub(crate) fn item_of_key(conn: &Connection, key: &MediaKey) -> Result<Option<ItemRef>, Error> {
    crate::matching::resolve(conn, key, &[])
}

/// Duration of the file `item` plays here (its default version), if known.
pub(crate) fn local_duration(conn: &Connection, item: ItemRef) -> Result<Option<u64>, Error> {
    Ok(items::item(conn, item)?
        .and_then(|i| i.duration_ms)
        .and_then(|d| u64::try_from(d).ok())
        .filter(|d| *d > 0))
}

/// A stored row as the progress of `item` here (SYNC-004): a position of
/// another version carries over exactly for the same cut, else by share
/// ([`pa_sync::matching::carry_position`]).
fn item_progress(
    conn: &Connection,
    item: ItemRef,
    p: &pa_model::PlaybackItem,
) -> Result<ItemProgress, Error> {
    let (position, duration) = pa_sync::matching::carry_position(
        p.position_ms,
        p.duration_ms,
        local_duration(conn, item)?,
    );
    Ok(ItemProgress {
        item,
        position_ms: i64::try_from(position).unwrap_or(i64::MAX),
        duration_ms: duration.map_or(0, |d| i64::try_from(d).unwrap_or(i64::MAX)),
        updated_at_ms: p.updated_at_ms,
        watched: p.watched,
    })
}

impl Catalog {
    /// Stored progress of an item: the newest change under any of its keys
    /// or the keys other devices linked to them (SYNC-004), with a position
    /// from another version carried onto this file.
    pub fn progress(&self, item: ItemRef) -> Result<Option<ItemProgress>, Error> {
        let conn = self.lock();
        crate::profiles::require_visible(&conn, item)?;
        match crate::matching::merged(&conn, profile(&conn)?, item)? {
            Some(p) => Ok(Some(item_progress(&conn, item, &p)?)),
            None => Ok(None),
        }
    }

    /// Records a position report of the player (and "watched" when asked or
    /// past 95 %). Returns `false` if the item has no key yet (its file is
    /// not fingerprinted): nothing is stored. The write is synced to disk
    /// before this returns, so it survives a power loss (SYNC-014,
    /// [`pa_library::durably`]).
    pub fn save_progress(
        &self,
        item: ItemRef,
        position_ms: i64,
        duration_ms: i64,
        updated_at_ms: i64,
        watched: bool,
    ) -> Result<bool, Error> {
        let mut guard = self.lock();
        crate::profiles::require_visible(&guard, item)?;
        let Some((key, kind)) = key_of(&guard, item)? else {
            return Ok(false);
        };
        pa_library::durably(&mut guard, |conn| {
            self.save_keyed(
                conn,
                item,
                key,
                kind,
                position_ms,
                duration_ms,
                updated_at_ms,
                watched,
            )
        })?;
        Ok(true)
    }

    /// The writes of [`Catalog::save_progress`] for an item with a key.
    #[allow(clippy::too_many_arguments)]
    fn save_keyed(
        &self,
        conn: &mut Connection,
        item: ItemRef,
        key: MediaKey,
        kind: MediaKind,
        position_ms: i64,
        duration_ms: i64,
        updated_at_ms: i64,
        watched: bool,
    ) -> Result<(), Error> {
        // The other keys go with the op, so a device that keys the item
        // differently finds it (SYNC-004).
        let aliases = crate::matching::item_keys(conn, item)?
            .map(|k| k.alias_texts())
            .unwrap_or_default();
        self.report(
            conn,
            key,
            kind,
            aliases,
            position_ms,
            duration_ms,
            updated_at_ms,
            watched,
        )?;
        if duration_ms > 0 {
            // The duration the player saw fills the file until probing (LIB-006).
            let files: &str = match item {
                ItemRef::Movie(_) => {
                    "UPDATE files SET duration_ms = ?2 WHERE duration_ms IS NULL AND id IN
                       (SELECT m.file_id FROM matches m WHERE m.work_id = ?1 AND m.state <> 'rejected')"
                }
                ItemRef::Episode(_) => {
                    "UPDATE files SET duration_ms = ?2 WHERE duration_ms IS NULL AND id IN
                       (SELECT m.file_id FROM match_episodes me JOIN matches m ON m.id = me.match_id
                         WHERE me.episode_id = ?1 AND m.state <> 'rejected')"
                }
                ItemRef::Track(_) | ItemRef::Show(_) => "",
            };
            let id = match item {
                ItemRef::Movie(i) | ItemRef::Episode(i) | ItemRef::Track(i) | ItemRef::Show(i) => i,
            };
            if !files.is_empty() {
                conn.execute(files, params![id, duration_ms])?;
            }
        }
        Ok(())
    }

    /// Records a position report of a title that is not a catalog item, by
    /// its media key: a Twitch VOD played in the official embed (TWI-005)
    /// is stored under `twitch:vod:<id>`, like the native player's, so it
    /// shows in the Twitch "Continue watching" and syncs between devices.
    /// Other keys are rejected.
    pub fn save_key_progress(
        &self,
        key: &MediaKey,
        position_ms: i64,
        duration_ms: i64,
        updated_at_ms: i64,
        watched: bool,
    ) -> Result<(), Error> {
        let kind = match key {
            MediaKey::TwitchVod { .. } if key.is_valid() => MediaKind::TwitchVod,
            _ => return Err(Error::Invalid(format!("progress key {key}"))),
        };
        let mut conn = self.lock();
        pa_library::durably(&mut conn, |conn| {
            self.report(
                conn,
                key.clone(),
                kind,
                Vec::new(),
                position_ms,
                duration_ms,
                updated_at_ms,
                watched,
            )
        })
    }

    /// A position report through the sync replica (or locally when sync is
    /// off), and "watched" when asked. `aliases` go with the ops.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn report(
        &self,
        conn: &mut Connection,
        key: MediaKey,
        kind: MediaKind,
        aliases: Vec<String>,
        position_ms: i64,
        duration_ms: i64,
        updated_at_ms: i64,
        watched: bool,
    ) -> Result<(), Error> {
        let profile_id = profile(conn)?;
        let mut local = LocalOnly::new(LOCAL_DEVICE_ID);
        let mut replica = self.sync_lock();
        let mut aliased;
        let sink: &mut dyn progress::ProgressSink = match replica.as_mut() {
            Some(r) => {
                aliased = AliasedSink {
                    replica: r,
                    aliases,
                };
                &mut aliased
            }
            None => &mut local,
        };
        let report = PositionReport {
            profile_id,
            key: key.clone(),
            kind,
            position_ms: u64::try_from(position_ms.max(0)).unwrap_or(0),
            duration_ms: u64::try_from(duration_ms).ok().filter(|d| *d > 0),
        };
        let th = Thresholds::default();
        let state = progress::report_position(conn, sink, &th, &report, updated_at_ms)?;
        if watched && !state.watched {
            progress::mark_watched(conn, sink, profile_id, &key, updated_at_ms)?;
        }
        Ok(())
    }

    /// "Mark unwatched" (QA-015): clears "watched" and the position of
    /// `item` under each of its keys and the keys other devices linked to
    /// them (SYNC-004), so [`Catalog::progress`] no longer finds it
    /// watched. Sends `unwatch` ops through sync. Returns `false` if there
    /// was nothing to clear (a show has no progress of its own: callers go
    /// over its episodes).
    pub fn mark_unwatched(&self, item: ItemRef, updated_at_ms: i64) -> Result<bool, Error> {
        let mut conn = self.lock();
        crate::profiles::require_visible(&conn, item)?;
        self.unwatch_in(&mut conn, item, updated_at_ms)
    }

    /// The writes of [`Catalog::mark_unwatched`] on a locked connection.
    fn unwatch_in(
        &self,
        conn: &mut Connection,
        item: ItemRef,
        updated_at_ms: i64,
    ) -> Result<bool, Error> {
        let own = crate::matching::own_keys(conn, item)?;
        if own.is_empty() {
            return Ok(false);
        }
        let keys = pa_library::key_links::equivalent(conn, &own)?;
        let aliases = crate::matching::item_keys(conn, item)?
            .map(|k| k.alias_texts())
            .unwrap_or_default();
        let profile_id = profile(conn)?;
        let mut local = LocalOnly::new(LOCAL_DEVICE_ID);
        let mut replica = self.sync_lock();
        let mut aliased;
        let sink: &mut dyn progress::ProgressSink = match replica.as_mut() {
            Some(r) => {
                aliased = AliasedSink {
                    replica: r,
                    aliases,
                };
                &mut aliased
            }
            None => &mut local,
        };
        let mut cleared = false;
        for key in &keys {
            cleared |= progress::mark_unwatched(conn, sink, profile_id, key, updated_at_ms)?;
        }
        Ok(cleared)
    }

    /// The state of `item` as a [`WatchMark`]: what it was before a bulk
    /// mark.
    fn watch_mark(&self, conn: &Connection, item: ItemRef) -> Result<WatchMark, Error> {
        let p = match crate::matching::merged(conn, profile(conn)?, item)? {
            Some(p) => Some(item_progress(conn, item, &p)?),
            None => None,
        };
        Ok(WatchMark {
            item,
            position_ms: p.as_ref().map_or(0, |p| p.position_ms),
            duration_ms: p.as_ref().map_or(0, |p| p.duration_ms),
            watched: p.is_some_and(|p| p.watched),
        })
    }

    /// "Mark watched" / "Mark unwatched" of many items at once (UI-049: an
    /// episode with the ones before it, a season, a whole show), in one
    /// transaction. Watched: the position goes to the end, as after
    /// [`Catalog::save_progress`] with `watched`; unwatched: as
    /// [`Catalog::mark_unwatched`]. Each change goes to sync like the
    /// single-item path. Items without a key (a show, a file not
    /// fingerprinted yet) and items already in that state are skipped.
    /// Returns the previous state of each changed item, for
    /// [`Catalog::restore_watch_marks`].
    pub fn set_watched_many(
        &self,
        items: &[ItemRef],
        watched: bool,
        updated_at_ms: i64,
    ) -> Result<Vec<WatchMark>, Error> {
        let mut guard = self.lock();
        for item in items {
            crate::profiles::require_visible(&guard, *item)?;
        }
        pa_library::durably(&mut guard, |conn| {
            in_transaction(conn, |conn| {
                let mut changed = Vec::new();
                let mut seen = std::collections::HashSet::new();
                for &item in items {
                    if !seen.insert(item) {
                        continue;
                    }
                    let before = self.watch_mark(conn, item)?;
                    if watched {
                        if before.watched {
                            continue;
                        }
                        let Some((key, kind)) = key_of(conn, item)? else {
                            continue;
                        };
                        let end = match local_duration(conn, item)? {
                            Some(d) => i64::try_from(d).unwrap_or(i64::MAX),
                            None => before.duration_ms,
                        };
                        self.save_keyed(conn, item, key, kind, end, end, updated_at_ms, true)?;
                        changed.push(before);
                    } else if self.unwatch_in(conn, item, updated_at_ms)? {
                        changed.push(before);
                    }
                }
                Ok(changed)
            })
        })
    }

    /// «Отменить» of [`Catalog::set_watched_many`] (UI-049): puts back the
    /// watched mark and the position each item had, in one transaction,
    /// through sync like any other change.
    pub fn restore_watch_marks(
        &self,
        marks: &[WatchMark],
        updated_at_ms: i64,
    ) -> Result<(), Error> {
        let mut guard = self.lock();
        for m in marks {
            crate::profiles::require_visible(&guard, m.item)?;
        }
        pa_library::durably(&mut guard, |conn| {
            in_transaction(conn, |conn| {
                for m in marks {
                    let Some((key, kind)) = key_of(conn, m.item)? else {
                        continue;
                    };
                    let now = self.watch_mark(conn, m.item)?;
                    let (mut watched, mut position) = (now.watched, now.position_ms);
                    if !m.watched && (watched || (position > 0 && position != m.position_ms)) {
                        self.unwatch_in(conn, m.item, updated_at_ms)?;
                        (watched, position) = (false, 0);
                    }
                    if (m.watched && !watched) || (m.position_ms > 0 && position != m.position_ms) {
                        // The position, and "watched" again when it was.
                        self.save_keyed(
                            conn,
                            m.item,
                            key,
                            kind,
                            m.position_ms.max(0),
                            m.duration_ms,
                            updated_at_ms,
                            m.watched,
                        )?;
                    }
                }
                Ok(())
            })
        })
    }

    /// "Continue watching": started items between 2 % and 95 % and Up Next
    /// episodes, latest activity first.
    pub fn continue_watching(&self, limit: usize) -> Result<Vec<ContinueItem>, Error> {
        let conn = self.lock();
        let query = ContinueQuery {
            kinds: vec![MediaKind::Movie, MediaKind::Episode, MediaKind::Video],
            up_next: true,
            limit,
        };
        let profile_id = profile(&conn)?;
        let entries =
            progress::continue_watching(&conn, &Thresholds::default(), profile_id, &query)?;
        let mut out = Vec::with_capacity(entries.len());
        let mut seen = std::collections::HashSet::new();
        for e in entries {
            let id = match &e.episode {
                Some(ep) => Some(ItemRef::Episode(ep.episode_id)),
                None => item_of_key(&conn, &e.item.media_key)?,
            };
            let Some(id) = id else { continue };
            // One item may have rows under several keys (SYNC-004): it is
            // in the row once, and only if its newest row is this one.
            if !seen.insert(id) {
                continue;
            }
            let up_next = e.reason == ContinueReason::UpNext;
            if !up_next
                && crate::matching::merged(&conn, profile_id, id)?
                    .is_some_and(|m| m.media_key != e.item.media_key)
            {
                continue;
            }
            let Some(item) = items::item(&conn, id)? else {
                continue;
            };
            out.push(ContinueItem {
                progress: item_progress(&conn, id, &e.item)?,
                item,
                up_next,
            });
        }
        Ok(out)
    }
}
