// SPDX-License-Identifier: MPL-2.0
//! File fingerprints (LIB-004): ed2k bookkeeping and moved files.
//!
//! The hashes themselves are computed in `pa-catalog` (`fingerprint`):
//! the OSHash of every file during the scan (`files.oshash`, also the
//! `file:` media key in `files.fingerprint`, ADR-0032) and the ed2k hash
//! (`files.ed2k`) in a background job. This module stores the job's results
//! and finds moved files.
//!
//! **Moves.** A file that is renamed or moved inside a source shows up in
//! the scan as a missing row (the old path, `missing_since_ms` set) and a
//! new row (the new path). Once the new row has its OSHash,
//! [`follow_moves`] pairs it with a missing row of the same source with the
//! same OSHash and size and keeps the **old** row: it takes the new path,
//! the folder-structure rows of the new path, and is live again; the new
//! row is deleted. Everything that hangs on the file id — matches, offline
//! copies, versions, skip segments, Twitch files — therefore follows the
//! file, and progress follows it anyway (its `file:` key is the OSHash).

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};

use crate::Error;

/// A new row pairs with a missing one only if it was added after the
/// missing one went missing, or at most this long before (the change
/// watcher may see the new path before the old one's folder is rescanned).
pub const MOVE_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

/// A file whose ed2k attempt failed is retried after this long.
pub const ED2K_RETRY_MS: i64 = 24 * 60 * 60 * 1000;

/// When the ed2k job reads the files of a source (`sources.ed2k_policy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ed2kPolicy {
    /// Local folders in every background run; network sources only in runs
    /// the app starts while the device is idle on Wi-Fi, or on demand.
    #[default]
    Auto,
    /// Every background run, network sources too.
    Always,
    /// Only on demand.
    Never,
}

impl Ed2kPolicy {
    /// Value of `sources.ed2k_policy`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Ed2kPolicy::Auto => "auto",
            Ed2kPolicy::Always => "always",
            Ed2kPolicy::Never => "never",
        }
    }

    /// Parses `sources.ed2k_policy`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Ed2kPolicy::Auto),
            "always" => Some(Ed2kPolicy::Always),
            "never" => Some(Ed2kPolicy::Never),
            _ => None,
        }
    }
}

/// The ed2k policy of source `source_id`, `None` if there is no such source.
pub fn ed2k_policy(conn: &Connection, source_id: i64) -> Result<Option<Ed2kPolicy>, Error> {
    let value: Option<String> = conn
        .query_row(
            "SELECT ed2k_policy FROM sources WHERE id = ?1",
            [source_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(value.and_then(|v| Ed2kPolicy::parse(&v)))
}

/// Sets the ed2k policy of source `source_id`. Returns `false` if there is
/// no such source.
pub fn set_ed2k_policy(
    conn: &Connection,
    source_id: i64,
    policy: Ed2kPolicy,
) -> Result<bool, Error> {
    Ok(conn.execute(
        "UPDATE sources SET ed2k_policy = ?2 WHERE id = ?1",
        params![source_id, policy.as_str()],
    )? > 0)
}

/// A file the ed2k job still has to hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kCandidate {
    pub file_id: i64,
    /// Path inside the source.
    pub path: String,
    pub size_bytes: u64,
    pub mtime_ms: Option<i64>,
}

/// Live video files of `source_id` without an ed2k hash, smallest first,
/// at most `limit`. Music tracks are left out (ed2k serves anime
/// recognition, 03 §4.2), and so are files whose last attempt failed less
/// than [`ED2K_RETRY_MS`] before `now_ms`.
pub fn ed2k_candidates(
    conn: &Connection,
    source_id: i64,
    now_ms: i64,
    limit: usize,
) -> Result<Vec<Ed2kCandidate>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT f.id, f.path, f.size_bytes, f.mtime_ms FROM files f
         WHERE f.source_id = ?1 AND f.ed2k IS NULL AND f.missing_since_ms IS NULL
           AND (f.ed2k_failed_at_ms IS NULL OR f.ed2k_failed_at_ms <= ?2)
           AND NOT EXISTS (SELECT 1 FROM music_tracks t WHERE t.file_id = f.id)
         ORDER BY f.size_bytes, f.id
         LIMIT ?3",
    )?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = stmt.query_map(
        params![source_id, now_ms.saturating_sub(ED2K_RETRY_MS), limit],
        |r| {
            Ok(Ed2kCandidate {
                file_id: r.get(0)?,
                path: r.get(1)?,
                size_bytes: u64::try_from(r.get::<_, i64>(2)?).unwrap_or(0),
                mtime_ms: r.get(3)?,
            })
        },
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Stores the ed2k hash (32 lowercase hex digits) of `file`, unless the
/// file changed while it was read (size or mtime differ from `file`, or the
/// scan reset its hashes). Returns `true` if it was stored.
pub fn set_ed2k(conn: &Connection, file: &Ed2kCandidate, ed2k: &str) -> Result<bool, Error> {
    Ok(conn.execute(
        "UPDATE files SET ed2k = ?2, ed2k_failed_at_ms = NULL
         WHERE id = ?1 AND size_bytes = ?3 AND mtime_ms IS ?4 AND ed2k IS NULL",
        params![
            file.file_id,
            ed2k,
            i64::try_from(file.size_bytes).unwrap_or(i64::MAX),
            file.mtime_ms
        ],
    )? > 0)
}

/// Records a failed ed2k attempt of `file_id` at `now_ms`.
pub fn mark_ed2k_failed(conn: &Connection, file_id: i64, now_ms: i64) -> Result<(), Error> {
    conn.execute(
        "UPDATE files SET ed2k_failed_at_ms = ?2 WHERE id = ?1",
        params![file_id, now_ms],
    )?;
    Ok(())
}

/// A file found at a new path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    /// The kept row (`files.id` before the move).
    pub file_id: i64,
    pub from_path: String,
    pub to_path: String,
}

/// Pairs new rows of `source_id` with missing rows of the same OSHash and
/// size and merges each pair into the old row (see the module docs).
///
/// A new row with a match the user set or confirmed is left alone: the user
/// already decided about it. Each row takes part in at most one pair; among
/// several missing candidates the one that went missing last wins.
pub fn follow_moves(conn: &mut Connection, source_id: i64) -> Result<Vec<Moved>, Error> {
    let tx = conn.transaction()?;
    let pairs: Vec<(i64, i64)> = {
        let mut stmt = tx.prepare(
            "SELECT n.id, o.id FROM files n
             JOIN files o ON o.source_id = n.source_id AND o.oshash = n.oshash
                         AND o.size_bytes = n.size_bytes AND o.id <> n.id
             WHERE n.source_id = ?1 AND n.oshash IS NOT NULL AND n.missing_since_ms IS NULL
               AND o.missing_since_ms IS NOT NULL
               AND n.id > o.id
               AND n.added_at_ms >= o.missing_since_ms - ?2
               AND NOT EXISTS (SELECT 1 FROM matches m
                               WHERE m.file_id = n.id AND substr(m.state, 1, 5) = 'user_')
             ORDER BY n.id, o.missing_since_ms DESC, o.id DESC",
        )?;
        stmt.query_map(params![source_id, MOVE_WINDOW_MS], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<Result<_, _>>()?
    };
    let mut used = HashSet::new();
    let mut moved = Vec::new();
    for (new, old) in pairs {
        if used.contains(&new) || used.contains(&old) {
            continue;
        }
        used.insert(new);
        used.insert(old);
        moved.push(merge(&tx, new, old)?);
    }
    tx.commit()?;
    Ok(moved)
}

/// Moves row `old` to the path of row `new` and deletes `new`.
fn merge(tx: &Connection, new: i64, old: i64) -> Result<Moved, Error> {
    let from_path: String =
        tx.query_row("SELECT path FROM files WHERE id = ?1", [old], |r| r.get(0))?;
    // The folder structure belongs to the path: the old path's rows go,
    // the new path's rows move to the kept row.
    tx.execute("DELETE FROM file_group_members WHERE file_id = ?1", [old])?;
    tx.execute(
        "UPDATE group_assets SET file_id = NULL WHERE file_id = ?1",
        [old],
    )?;
    tx.execute(
        "UPDATE file_group_members SET file_id = ?2 WHERE file_id = ?1",
        params![new, old],
    )?;
    tx.execute(
        "UPDATE group_assets SET file_id = ?2 WHERE file_id = ?1",
        params![new, old],
    )?;
    // Free the new path (it is unique), move the old row there, delete
    // the new row. Probe results and hashes of the old row are kept (same
    // content); the new row fills what the old one lacks.
    let to_path: String =
        tx.query_row("SELECT path FROM files WHERE id = ?1", [new], |r| r.get(0))?;
    tx.execute(
        "UPDATE files SET path = '/lib004-moved/' || id WHERE id = ?1",
        [new],
    )?;
    tx.execute(
        "UPDATE files SET
             path = ?3, mtime_ms = n.mtime_ms, inode = n.inode,
             scanned_at_ms = n.scanned_at_ms, missing_since_ms = NULL,
             container = ifnull(files.container, n.container),
             duration_ms = ifnull(files.duration_ms, n.duration_ms),
             video_json = ifnull(files.video_json, n.video_json),
             audio_langs = ifnull(files.audio_langs, n.audio_langs),
             sub_langs = ifnull(files.sub_langs, n.sub_langs),
             audio_formats = ifnull(files.audio_formats, n.audio_formats),
             embedded_tags_json = ifnull(files.embedded_tags_json, n.embedded_tags_json),
             probed_at_ms = ifnull(files.probed_at_ms, n.probed_at_ms),
             ed2k = ifnull(files.ed2k, n.ed2k),
             crc32 = ifnull(files.crc32, n.crc32),
             crc32_in_name = n.crc32_in_name
         FROM files n
         WHERE files.id = ?1 AND n.id = ?2",
        params![old, new, to_path],
    )?;
    tx.execute("DELETE FROM files WHERE id = ?1", [new])?;
    Ok(Moved {
        file_id: old,
        from_path,
        to_path,
    })
}
