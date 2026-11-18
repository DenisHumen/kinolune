// SPDX-License-Identifier: MPL-2.0
//! Playlists of music servers in the library (MUS-012, ADR-0130).
//!
//! A Subsonic (Navidrome), Jellyfin or Emby source reads as a tree of
//! files; its playlists come from the connector as paths of that tree
//! ([`Connector::server_playlists`](crate::Connector::server_playlists)).
//! The scan of the source mirrors each one into an ordinary audio
//! playlist of every profile that may see the source, marked in
//! `server_playlists`: the music UI, the queue and "download playlist"
//! (NET-019) take it as it is. The app does not change a mirrored
//! playlist (the next scan would undo it): it is edited on the server.

use std::collections::{HashMap, HashSet};

use pa_library::lists::{self, PlaylistKind};
use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_model::MediaKey;
use pa_sources::ServerPlaylist;

use crate::{Error, Source};

/// What one sync of a source changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlaylistSync {
    /// Server playlists the source has.
    pub playlists: usize,
    /// Mirrored playlists made (one per playlist and profile).
    pub created: usize,
    /// Mirrored playlists whose name or tracks changed.
    pub updated: usize,
    /// Mirrored playlists removed (gone from the server or the profile).
    pub removed: usize,
}

/// Profiles that see `source_id`: those without a folder white list, and
/// those whose white list names a folder of it.
fn profiles_seeing(conn: &Connection, source_id: i64) -> Result<Vec<i64>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT p.id FROM profiles p
          WHERE p.restrict_folders = 0
             OR EXISTS (SELECT 1 FROM profile_folders pf
                         WHERE pf.profile_id = p.id AND pf.source_id = ?1)
          ORDER BY p.id",
    )?;
    let rows = stmt.query_map([source_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The media key of the present, hashed file at `path` of `source_id`.
fn key_at(conn: &Connection, source_id: i64, path: &str) -> Result<Option<String>, Error> {
    let fp: Option<Option<String>> = conn
        .prepare_cached(
            "SELECT fingerprint FROM files
              WHERE source_id = ?1 AND path = ?2 AND missing_since_ms IS NULL",
        )?
        .query_row(params![source_id, path], |r| r.get(0))
        .optional()?;
    Ok(fp
        .flatten()
        .map(|fingerprint| MediaKey::File { fingerprint }.to_string()))
}

/// Mirrors `lists`, the playlists of the server of `source`, into the
/// library: new ones are made, renamed and changed ones updated, and
/// mirrored ones the server no longer has removed. Songs the scan has not
/// hashed yet (or never saw) are left out until a later scan.
pub(crate) fn sync(
    conn: &mut Connection,
    source: &Source,
    server: &[ServerPlaylist],
    now_ms: i64,
) -> Result<PlaylistSync, Error> {
    let tx = conn.transaction()?;
    let profiles = profiles_seeing(&tx, source.id)?;
    // (remote id, profile) → (playlist id, name).
    let mut existing: HashMap<(String, i64), (i64, String)> = HashMap::new();
    {
        let mut stmt = tx.prepare_cached(
            "SELECT sp.remote_id, p.profile_id, p.id, p.name
               FROM server_playlists sp JOIN playlists p ON p.id = sp.playlist_id
              WHERE sp.source_id = ?1",
        )?;
        let rows = stmt.query_map([source.id], |r| {
            Ok(((r.get(0)?, r.get(1)?), (r.get(2)?, r.get(3)?)))
        })?;
        for row in rows {
            let (k, v) = row?;
            existing.insert(k, v);
        }
    }
    let mut report = PlaylistSync {
        playlists: server.len(),
        ..PlaylistSync::default()
    };
    let mut kept: HashSet<i64> = HashSet::new();
    for list in server {
        // The tracks in the server's order, each once (a library playlist
        // holds an item once).
        let mut keys: Vec<String> = Vec::with_capacity(list.paths.len());
        for path in &list.paths {
            if let Some(k) = key_at(&tx, source.id, path)?
                && !keys.contains(&k)
            {
                keys.push(k);
            }
        }
        for &profile in &profiles {
            let id = match existing.get(&(list.remote_id.clone(), profile)) {
                Some((id, name)) => {
                    let mut changed = false;
                    if *name != list.name {
                        lists::rename_playlist(&tx, *id, &list.name, now_ms)?;
                        changed = true;
                    }
                    let current: Vec<String> = lists::playlist_items(&tx, *id)?
                        .into_iter()
                        .map(|i| i.media_key)
                        .collect();
                    if current != keys {
                        replace_items(&tx, *id, &keys, now_ms)?;
                        changed = true;
                    }
                    if changed {
                        report.updated += 1;
                    }
                    *id
                }
                None => {
                    let p = lists::create_playlist(
                        &tx,
                        profile,
                        &list.name,
                        PlaylistKind::Audio,
                        now_ms,
                    )?;
                    tx.prepare_cached(
                        "INSERT INTO server_playlists (playlist_id, source_id, remote_id,
                                                       synced_at_ms)
                         VALUES (?1, ?2, ?3, ?4)",
                    )?
                    .execute(params![
                        p.id,
                        source.id,
                        list.remote_id,
                        now_ms
                    ])?;
                    replace_items(&tx, p.id, &keys, now_ms)?;
                    report.created += 1;
                    p.id
                }
            };
            kept.insert(id);
        }
    }
    for (id, _) in existing.values() {
        if !kept.contains(id) {
            // The trigger `server_playlists_gone` would also do it.
            lists::delete_playlist(&tx, *id)?;
            report.removed += 1;
        }
    }
    tx.prepare_cached("UPDATE server_playlists SET synced_at_ms = ?2 WHERE source_id = ?1")?
        .execute(params![source.id, now_ms])?;
    tx.commit()?;
    Ok(report)
}

/// Sets the items of playlist `id` to `keys` in order.
fn replace_items(conn: &Connection, id: i64, keys: &[String], now_ms: i64) -> Result<(), Error> {
    conn.prepare_cached("DELETE FROM playlist_items WHERE playlist_id = ?1")?
        .execute([id])?;
    for k in keys {
        lists::add_to_playlist(conn, id, k, now_ms)?;
    }
    Ok(())
}

/// The source name of each mirrored playlist: playlist id → server name.
pub(crate) fn servers(conn: &Connection) -> Result<HashMap<i64, String>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT sp.playlist_id, s.name FROM server_playlists sp
           JOIN sources s ON s.id = sp.source_id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// [`Error::Invalid`] for a mirrored playlist: the app does not edit it.
pub(crate) fn require_own(conn: &Connection, id: i64) -> Result<(), Error> {
    let server: bool = conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM server_playlists WHERE playlist_id = ?1)")?
        .query_row([id], |r| r.get(0))?;
    if server {
        return Err(Error::Invalid(format!(
            "playlist {id} comes from a music server: change it there"
        )));
    }
    Ok(())
}
