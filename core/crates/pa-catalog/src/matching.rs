// SPDX-License-Identifier: MPL-2.0
//! The same media across devices (SYNC-004, ADR-NNNN): every key of an
//! item, the item of a key another device sent, and the progress of an
//! item over all its keys.
//!
//! - **Keys of an item** ([`own_keys`]): provider ids (`tmdb:movie:`,
//!   `tmdb:episode:`, `anidb:episode:`), then the `file:` key (OSHash) of
//!   each of its versions (present files first), then their `ed2k:` keys.
//!   [`pa_sync::matching::ItemKeys`] makes the first v1 key the one progress
//!   is written under (unchanged since ADR-0032: the TMDB key, else `file:`
//!   of the first file) and sends the others as aliases.
//! - **Item of a key** ([`resolve`]): the key itself, then its aliases by
//!   priority, then the keys other devices linked to them
//!   (`pa_library::key_links`). A `file:` key whose sender also sent an
//!   `ed2k:` key is not matched to a local file of another size or ed2k
//!   (an OSHash collision: equal size, head and tail, other middle).
//! - **Progress of an item** ([`merged`]): the newest row over its own keys
//!   and the keys linked to them. Rows stay under the key they were written
//!   with, so `file:` rows from before an item was recognised, journals and
//!   peers without SYNC-004 keep working.

use pa_library::key_links;
use pa_library::progress as lib_progress;
use pa_library::rusqlite::{Connection, OptionalExtension, ToSql, params};
use pa_model::{MediaKey, PlaybackItem};
use pa_sync::matching::{self, ItemKeys};

use crate::Error;
use crate::items::ItemRef;
use crate::progress::key_of;

/// `file:` and `ed2k:` keys of the files of a movie, present files first.
fn movie_content_keys(conn: &Connection, work: i64) -> Result<Vec<MediaKey>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT f.fingerprint, f.ed2k, f.size_bytes FROM matches m JOIN files f ON f.id = m.file_id
         WHERE m.work_id = ?1 AND m.state <> 'rejected' AND f.fingerprint IS NOT NULL
         ORDER BY f.missing_since_ms IS NOT NULL, f.id",
    )?;
    let rows = stmt.query_map([work], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<i64>>(2)?,
        ))
    })?;
    let mut files = Vec::new();
    let mut hashes = Vec::new();
    for row in rows {
        let (fingerprint, ed2k, size) = row?;
        files.push(MediaKey::File { fingerprint });
        if let (Some(hash), Some(size)) = (ed2k, size.and_then(|s| u64::try_from(s).ok())) {
            hashes.push(MediaKey::Ed2k { hash, size });
        }
    }
    files.extend(hashes);
    Ok(files)
}

/// Every key of `item` on this device, the key progress is written under
/// ([`key_of`]) first, then by priority. Empty while it has none.
pub(crate) fn own_keys(conn: &Connection, item: ItemRef) -> Result<Vec<MediaKey>, Error> {
    let Some((primary, _)) = key_of(conn, item)? else {
        return Ok(Vec::new());
    };
    let mut rest = match item {
        ItemRef::Movie(work) => movie_content_keys(conn, work)?,
        ItemRef::Episode(ep) => lib_progress::episode_keys(conn, ep)?,
        ItemRef::Track(_) | ItemRef::Show(_) => Vec::new(),
    };
    rest.sort_by_key(matching::priority);
    let mut keys = vec![primary];
    for k in rest {
        if k.is_valid() && !keys.contains(&k) {
            keys.push(k);
        }
    }
    Ok(keys)
}

/// The key and aliases of `item` for ops and `pb.*` messages.
pub(crate) fn item_keys(conn: &Connection, item: ItemRef) -> Result<Option<ItemKeys>, Error> {
    let keys = own_keys(conn, item)?;
    let Some(primary) = keys.first().cloned() else {
        return Ok(None);
    };
    let mut aliases: Vec<MediaKey> = keys.into_iter().skip(1).collect();
    aliases.truncate(matching::MAX_ALIASES);
    Ok(Some(ItemKeys {
        key: primary,
        aliases,
    }))
}

/// The newest progress row of `item` on a profile over its own keys and the
/// keys other devices linked to them.
pub(crate) fn merged(
    conn: &Connection,
    profile_id: i64,
    item: ItemRef,
) -> Result<Option<PlaybackItem>, Error> {
    let own = own_keys(conn, item)?;
    if own.is_empty() {
        return Ok(None);
    }
    let keys = key_links::equivalent(conn, &own)?;
    Ok(lib_progress::newest(conn, profile_id, &keys)?)
}

/// The item of a file selected by `filter` (on `f`), with `args`.
fn item_of_file(
    conn: &Connection,
    filter: &str,
    args: &[&dyn ToSql],
) -> Result<Option<ItemRef>, Error> {
    let row: Option<(i64, Option<i64>, Option<String>)> = conn
        .query_row(
            &format!(
                "SELECT f.id, m.work_id, w.kind FROM files f
                 LEFT JOIN matches m ON m.file_id = f.id AND m.state <> 'rejected'
                 LEFT JOIN works w ON w.id = m.work_id
                 WHERE {filter}
                 ORDER BY f.missing_since_ms IS NOT NULL, f.id LIMIT 1"
            ),
            args,
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(match row {
        Some((_, Some(work), Some(kind))) if kind == "movie" => Some(ItemRef::Movie(work)),
        Some((file, _, _)) => conn
            .query_row(
                "SELECT id FROM music_tracks WHERE file_id = ?1",
                [file],
                |r| r.get(0),
            )
            .optional()?
            .map(ItemRef::Track),
        None => None,
    })
}

/// The `ed2k:` key a sender put among its aliases: hash and size of its copy.
fn sender_ed2k(aliases: &[MediaKey]) -> Option<(&str, u64)> {
    aliases.iter().find_map(|a| match a {
        MediaKey::Ed2k { hash, size } => Some((hash.as_str(), *size)),
        _ => None,
    })
}

/// Whether no local file with OSHash `fingerprint` can be the sender's copy
/// of `size` bytes and ed2k `hash`: every one has another size, or an ed2k
/// that differs. `false` when there is no such file.
fn collides(
    conn: &Connection,
    fingerprint: &str,
    (hash, size): (&str, u64),
) -> Result<bool, Error> {
    let size = i64::try_from(size).unwrap_or(i64::MAX);
    let (any, same): (i64, i64) = conn.query_row(
        "SELECT count(*),
                ifnull(sum(size_bytes = ?2 AND (ed2k IS NULL OR ed2k = ?3)), 0)
         FROM files WHERE fingerprint = ?1",
        params![fingerprint, size, hash],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(any > 0 && same == 0)
}

/// The item a key names on this device, without links.
fn local_item(
    conn: &Connection,
    key: &MediaKey,
    guard: Option<(&str, u64)>,
) -> Result<Option<ItemRef>, Error> {
    if let MediaKey::File { fingerprint } = key
        && let Some(g) = guard
        && collides(conn, fingerprint, g)?
    {
        return Ok(None);
    }
    if let Some(ep) = lib_progress::episode_for_local_key(conn, key)? {
        return Ok(Some(ItemRef::Episode(ep)));
    }
    Ok(match key {
        MediaKey::TmdbMovie { id } => conn
            .query_row(
                "SELECT work_id FROM external_ids
                 WHERE provider = 'tmdb' AND ext_kind = 'movie' AND ext_id = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .map(ItemRef::Movie),
        MediaKey::File { fingerprint } => {
            item_of_file(conn, "f.fingerprint = ?1", &[fingerprint as &dyn ToSql])?
        }
        MediaKey::Ed2k { hash, size } => {
            let size = i64::try_from(*size).unwrap_or(i64::MAX);
            item_of_file(
                conn,
                "f.ed2k = ?1 AND f.size_bytes = ?2",
                &[hash as &dyn ToSql, &size],
            )?
        }
        _ => None,
    })
}

/// The item of this library a message about `key` with `aliases` is about:
/// the key, then the aliases by priority, then keys other devices linked to
/// any of them.
pub(crate) fn resolve(
    conn: &Connection,
    key: &MediaKey,
    aliases: &[MediaKey],
) -> Result<Option<ItemRef>, Error> {
    let guard = sender_ed2k(aliases);
    let mut tried = vec![key.clone()];
    let mut ordered: Vec<&MediaKey> = aliases.iter().collect();
    ordered.sort_by_key(|k| matching::priority(k));
    for a in ordered {
        if !tried.contains(a) {
            tried.push(a.clone());
        }
    }
    for k in &tried {
        if let Some(item) = local_item(conn, k, guard)? {
            return Ok(Some(item));
        }
    }
    for k in key_links::linked(conn, &tried)? {
        if let Some(item) = local_item(conn, &k, None)? {
            return Ok(Some(item));
        }
    }
    Ok(None)
}
