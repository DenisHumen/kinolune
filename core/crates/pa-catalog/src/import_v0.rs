// SPDX-License-Identifier: MPL-2.0
//! Import of a `pa_library` v0 database (release 0.1 Dart package), by the
//! mapping of `docs/dev/db.md` §«Переход с pa_library v0» (LIB-015,
//! ADR-NNNN).
//!
//! The v0 file is opened read-only and never changed; renaming it to
//! `*.v0.bak` after success is the caller's job. A second run adds nothing:
//! sources are found by URI, files by `(source, path)`, works by title key or
//! TMDB id, a file that already has a live match keeps it, and progress keeps
//! the newer of the stored and the imported state.
//!
//! What moves:
//!
//! - local-folder sources (`file://` URI) and SMB sources (`smb://` URI
//!   without the user name, which goes to `username`);
//! - movies, episodes and tracks: `files` (path relative to the source
//!   root), works (with the TMDB id as `external_ids`), catalog episodes,
//!   matches (`user_set` for items the user fixed, `auto` otherwise); track
//!   tags are read by the next scan. v0 read SMB shares through their GVfs
//!   mount (`…/gvfs/smb-share:server=nas,share=video/Movies/x.mkv`): the
//!   path inside the share comes from that mount path, so the next scan
//!   over `pa-vfs` finds the same `files` rows;
//! - progress under `tmdb:` keys for recognized items (also when the file
//!   could not be placed), otherwise under the `file:` key of the file,
//!   fingerprinted now. A file that cannot be read now (share not mounted,
//!   disk offline) keeps its progress in `import_v0_progress` until a scan
//!   fingerprints it ([`apply_pending`]). Progress of an item without a key
//!   and without a file is skipped and counted.
//!
//! Not moved: TMDB cache and settings (the app keeps the TMDB key itself),
//! parse results (files are parsed again by the next scan).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pa_library::rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use pa_model::MediaKey;
use pa_sources::SmbAddress;

use crate::recognize::{EpisodeNo, WorkIndex, ensure_episode, insert_match};
use crate::{Catalog, Error, fingerprint, sources, uri};

/// What [`Catalog::import_v0`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// v0 sources (found or added).
    pub sources: usize,
    /// v0 items imported as files.
    pub files: usize,
    /// Files that got a match in this run.
    pub matched: usize,
    /// Progress rows written (new or newer).
    pub progress: usize,
    /// Progress rows left waiting for their file's fingerprint (new or
    /// newer), see [`apply_pending`].
    pub progress_pending: usize,
    /// Progress rows without a key and without a file.
    pub progress_skipped: usize,
    /// Items that could not be placed under a source root (unknown source,
    /// moved, an SMB path that is not a GVfs mount of its share).
    pub items_skipped: usize,
    /// v0 source id → `sources.id`, in v0 order. The app kept SMB
    /// passwords under the v0 id and moves them with this.
    pub v0_sources: Vec<(String, i64)>,
}

struct V0Item {
    id: String,
    kind: String,
    source_id: Option<String>,
    title: String,
    original_title: Option<String>,
    year: Option<i64>,
    overview: Option<String>,
    show_id: Option<String>,
    season: Option<i64>,
    episode: Option<i64>,
    episode_end: Option<i64>,
    file_path: String,
    file_size: Option<i64>,
    file_mtime: Option<i64>,
    duration_ms: Option<i64>,
    match_confidence: Option<f64>,
    tmdb_id: Option<i64>,
    locked: bool,
    added_at: i64,
}

fn load_items(v0: &Connection) -> Result<Vec<V0Item>, Error> {
    let mut stmt = v0.prepare(
        "SELECT id, kind, source_id, title, original_title, year, overview, show_id, season,
                episode, episode_end, file_path, file_size, file_mtime, duration_ms,
                match_confidence, tmdb_id, locked, added_at
         FROM items ORDER BY CASE kind WHEN 'show' THEN 0 ELSE 1 END, file_path",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(V0Item {
            id: r.get(0)?,
            kind: r.get(1)?,
            source_id: r.get(2)?,
            title: r.get(3)?,
            original_title: r.get(4)?,
            year: r.get(5)?,
            overview: r.get(6)?,
            show_id: r.get(7)?,
            season: r.get(8)?,
            episode: r.get(9)?,
            episode_end: r.get(10)?,
            file_path: r.get(11)?,
            file_size: r.get(12)?,
            file_mtime: r.get(13)?,
            duration_ms: r.get(14)?,
            match_confidence: r.get(15)?,
            tmdb_id: r.get(16)?,
            locked: r.get::<_, Option<i64>>(17)?.unwrap_or(0) != 0,
            added_at: r.get(18)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Where the paths of a v0 source's items are relative to.
enum Root {
    /// A local folder.
    Local(PathBuf),
    /// An SMB share (folder): v0 items are under its GVfs mount.
    Smb(SmbAddress),
}

/// Percent-decoding (`%20` → space); `None` for a malformed escape or
/// non-UTF-8 result.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// v0 `smb://[user[:pass]@]host/share[/folder]` → the address without user
/// info and the user name. A password in the address is dropped.
fn split_smb_user(address: &str) -> (String, Option<String>) {
    let Some(rest) = address
        .get(..6)
        .filter(|s| s.eq_ignore_ascii_case("smb://"))
        .map(|_| &address[6..])
    else {
        return (address.to_owned(), None);
    };
    let end = rest.find(['/', '\\']).unwrap_or(rest.len());
    let Some(at) = rest[..end].rfind('@') else {
        return (address.to_owned(), None);
    };
    let info = &rest[..at];
    let user = info.split(':').next().unwrap_or_default();
    let user = percent_decode(user).unwrap_or_else(|| user.to_owned());
    (
        format!("smb://{}", &rest[at + 1..]),
        Some(user).filter(|u| !u.is_empty()),
    )
}

/// Path of a v0 SMB item inside the folder of `addr`, from its GVfs mount
/// path (`/run/user/1000/gvfs/smb-share:server=nas,share=video/Movies/x.mkv`
/// → `/Movies/x.mkv` for `smb://nas/video`). Server and share compare like
/// v0 did (case-insensitive, percent-decoded); `None` if the path is not
/// under that mount and folder.
fn gvfs_relative(addr: &SmbAddress, file_path: &str) -> Option<String> {
    let parts: Vec<&str> = file_path
        .split(['/', '\\'])
        .filter(|p| !p.is_empty())
        .collect();
    let server = addr.server.to_lowercase();
    // `host:port`: GVfs keeps the port in its own attribute.
    let host = server
        .rsplit_once(':')
        .filter(|(h, p)| !h.contains(':') && p.chars().all(|c| c.is_ascii_digit()))
        .map_or(server.as_str(), |(h, _)| h);
    let share = addr.share.to_lowercase();
    let mount = parts.iter().position(|seg| {
        let Some(attrs) = seg.strip_prefix("smb-share:") else {
            return false;
        };
        let (mut srv, mut shr) = (None, None);
        for kv in attrs.split(',') {
            if let Some((k, v)) = kv.split_once('=') {
                let v = percent_decode(v)
                    .unwrap_or_else(|| v.to_owned())
                    .to_lowercase();
                match k {
                    "server" => srv = Some(v),
                    "share" => shr = Some(v),
                    _ => {}
                }
            }
        }
        srv.is_some_and(|s| s == server || s == host) && shr.is_some_and(|s| s == share)
    })?;
    let mut rest = parts[mount + 1..].iter();
    for folder in addr.folder.split('/').filter(|f| !f.is_empty()) {
        if rest.next()?.to_lowercase() != folder.to_lowercase() {
            return None;
        }
    }
    let rel: Vec<&str> = rest.copied().collect();
    (!rel.is_empty()).then(|| format!("/{}", rel.join("/")))
}

/// `files.path` of a v0 item (a `VfsPath`, like the scanner stores it:
/// `/Movies/x.mkv`); `None` if it is not under the root.
fn relative_path(root: &Root, item: &V0Item) -> Option<String> {
    match root {
        Root::Local(root) => {
            let rel = Path::new(&item.file_path).strip_prefix(root).ok()?;
            let rel = format!("/{}", rel.to_string_lossy().replace('\\', "/"));
            (rel != "/").then_some(rel)
        }
        Root::Smb(addr) => gvfs_relative(addr, &item.file_path),
    }
}

/// Work with this TMDB id, or the local one by title key; the TMDB id and
/// v0 metadata are attached.
fn ensure_work(
    tx: &Transaction<'_>,
    index: &mut WorkIndex,
    kind: &str,
    item: &V0Item,
    now: i64,
) -> Result<i64, Error> {
    let ext_kind = if kind == "movie" { "movie" } else { "tv" };
    if let Some(tmdb) = item.tmdb_id {
        let found: Option<i64> = tx
            .query_row(
                "SELECT work_id FROM external_ids WHERE provider = 'tmdb' AND ext_kind = ?1 AND ext_id = ?2",
                params![ext_kind, tmdb.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(w) = found {
            return Ok(w);
        }
    }
    let work = index.ensure(tx, kind, &item.title, item.year, now)?;
    tx.execute(
        "UPDATE works SET original_title = ifnull(original_title, ?2),
                          overview = ifnull(overview, ?3),
                          year = ifnull(year, ?4)
         WHERE id = ?1",
        params![
            work,
            item.original_title,
            item.overview,
            item.year.filter(|y| (1800..=3000).contains(y))
        ],
    )?;
    if let Some(tmdb) = item.tmdb_id {
        tx.execute(
            "INSERT OR IGNORE INTO external_ids (work_id, provider, ext_kind, ext_id)
             VALUES (?1, 'tmdb', ?2, ?3)",
            params![work, ext_kind, tmdb.to_string()],
        )?;
    }
    Ok(work)
}

/// `files` row of a v0 item at `rel` (inserted if new).
fn ensure_file(tx: &Transaction<'_>, source: i64, rel: &str, item: &V0Item) -> Result<i64, Error> {
    tx.execute(
        "INSERT INTO files (source_id, path, size_bytes, mtime_ms, duration_ms, added_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (source_id, path) DO NOTHING",
        params![
            source,
            rel,
            item.file_size.unwrap_or(0).max(0),
            item.file_mtime,
            item.duration_ms.filter(|d| *d >= 0),
            item.added_at
        ],
    )?;
    Ok(tx.query_row(
        "SELECT id FROM files WHERE source_id = ?1 AND path = ?2",
        params![source, rel],
        |r| r.get(0),
    )?)
}

/// Key of an imported v0 item for its progress.
enum KeyPlan {
    Key(MediaKey, &'static str),
    /// Fingerprint this local file and use its `file:` key.
    File(i64, PathBuf, &'static str),
}

/// Season, first and last episode of a v0 episode item.
fn episode_range(item: &V0Item) -> (i64, i64, i64) {
    let season = item.season.unwrap_or(1);
    let first = item.episode.unwrap_or(1);
    let last = item.episode_end.filter(|e| *e >= first).unwrap_or(first);
    (season, first, last)
}

/// The `tmdb:` key of a recognized v0 movie or episode.
fn tmdb_key(item: &V0Item, shows: &HashMap<&str, &V0Item>) -> Option<KeyPlan> {
    match item.kind.as_str() {
        "movie" => item.tmdb_id.map(|id| {
            KeyPlan::Key(
                MediaKey::TmdbMovie {
                    id: id.unsigned_abs(),
                },
                "movie",
            )
        }),
        "episode" => {
            let show = shows.get(item.show_id.as_deref()?)?;
            let (season, _, last) = episode_range(item);
            show.tmdb_id.map(|id| {
                KeyPlan::Key(
                    MediaKey::TmdbEpisode {
                        show_id: id.unsigned_abs(),
                        season: u32::try_from(season).unwrap_or(0),
                        episode: u32::try_from(last).unwrap_or(0),
                    },
                    "episode",
                )
            })
        }
        _ => None,
    }
}

/// Moves imported v0 progress whose file got a fingerprint into
/// `progress` (newer state wins) and drops it from `import_v0_progress`.
/// Run after the scan fingerprints files. Returns the rows written.
pub(crate) fn apply_pending(conn: &Connection) -> Result<usize, Error> {
    let written = conn.execute(
        "INSERT INTO progress (profile_id, media_key, source_type, position_ms, duration_ms,
                               watched, updated_at_ms, device_id)
         SELECT 1, 'file:' || f.fingerprint, p.source_type, p.position_ms, p.duration_ms,
                p.watched, p.updated_at_ms, 'import-v0'
           FROM import_v0_progress p JOIN files f ON f.id = p.file_id
          WHERE f.fingerprint IS NOT NULL
         ON CONFLICT (profile_id, media_key) DO UPDATE SET
             position_ms = excluded.position_ms,
             duration_ms = ifnull(excluded.duration_ms, progress.duration_ms),
             watched = max(progress.watched, excluded.watched),
             updated_at_ms = excluded.updated_at_ms
         WHERE excluded.updated_at_ms > progress.updated_at_ms",
        [],
    )?;
    conn.execute(
        "DELETE FROM import_v0_progress
          WHERE file_id IN (SELECT id FROM files WHERE fingerprint IS NOT NULL)",
        [],
    )?;
    Ok(written)
}

impl Catalog {
    /// Imports the `pa_library` v0 database at `v0_path` (read-only).
    pub fn import_v0(&self, v0_path: &Path) -> Result<ImportReport, Error> {
        let v0 = Connection::open_with_flags(
            v0_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        if !pa_library::is_legacy_v0(&v0)? {
            return Err(Error::Invalid(format!(
                "not a pa_library v0 database: {}",
                v0_path.display()
            )));
        }
        let mut report = ImportReport::default();
        let now = crate::now_ms();
        let mut conn = self.lock();
        let tx = conn.transaction()?;

        // Sources.
        let mut src_map: HashMap<String, (i64, Option<Root>)> = HashMap::new();
        {
            let mut stmt = v0.prepare(
                "SELECT id, kind, name, path, username FROM sources ORDER BY position, name",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?;
            for row in rows {
                let (id, kind, name, path, username) = row?;
                let (kind, uri, root, username) = match kind.as_str() {
                    "smb" => {
                        let (address, user) = split_smb_user(&path);
                        let username = username.filter(|u| !u.is_empty()).or(user);
                        match SmbAddress::parse(&address) {
                            Ok(a) => ("smb", a.to_uri(), Some(Root::Smb(a)), username),
                            // Kept as v0 had it; the Sources screen shows
                            // the error and lets the user fix it.
                            Err(_) => ("smb", address, None, username),
                        }
                    }
                    // Not in v0 databases (Rust core only); never a folder.
                    "webdav" => ("webdav", path.clone(), None, username),
                    _ => {
                        let root = PathBuf::from(&path);
                        let uri = uri::path_to_file_uri(&root);
                        ("local", uri, Some(Root::Local(root)), username)
                    }
                };
                // An earlier import kept the v0 address as is.
                let found = match sources::by_uri(&tx, &uri)? {
                    Some(id) => Some(id),
                    None if uri != path => sources::by_uri(&tx, &path)?,
                    None => None,
                };
                let new_id = match found {
                    Some(existing) => existing,
                    None => sources::insert(&tx, kind, &name, &uri, username.as_deref(), now)?,
                };
                report.v0_sources.push((id.clone(), new_id));
                src_map.insert(id, (new_id, root));
                report.sources += 1;
            }
        }

        // Items.
        let items = load_items(&v0)?;
        let mut index = WorkIndex::load(&tx)?;
        let mut show_works: HashMap<String, i64> = HashMap::new();
        let mut keys: HashMap<String, KeyPlan> = HashMap::new();
        let shows: HashMap<&str, &V0Item> = items
            .iter()
            .filter(|i| i.kind == "show")
            .map(|i| (i.id.as_str(), i))
            .collect();
        for item in &items {
            if item.kind == "show" {
                continue;
            }
            let placed = item
                .source_id
                .as_ref()
                .and_then(|s| src_map.get(s))
                .and_then(|(source, root)| Some((*source, relative_path(root.as_ref()?, item)?)));
            let Some((source, rel)) = placed else {
                report.items_skipped += 1;
                // The progress of a recognized item does not need its file:
                // the next scan finds the work by its TMDB id.
                if let Some(plan) = tmdb_key(item, &shows) {
                    keys.insert(item.id.clone(), plan);
                }
                continue;
            };
            let file = ensure_file(&tx, source, &rel, item)?;
            report.files += 1;
            let (state, decided_by, conf) = if item.locked {
                ("user_set", "user", 1.0)
            } else {
                ("auto", "scorer", item.match_confidence.unwrap_or(0.5))
            };
            let abs = PathBuf::from(&item.file_path);
            match item.kind.as_str() {
                "movie" => {
                    let work = ensure_work(&tx, &mut index, "movie", item, now)?;
                    if insert_match(&tx, file, work, &[], conf, state, decided_by, None, now)? {
                        report.matched += 1;
                    }
                    let plan = tmdb_key(item, &shows).unwrap_or(KeyPlan::File(file, abs, "movie"));
                    keys.insert(item.id.clone(), plan);
                }
                "episode" => {
                    let show = item.show_id.as_deref().and_then(|s| shows.get(s).copied());
                    let work = match (show, item.show_id.as_ref()) {
                        (Some(show), Some(sid)) => match show_works.get(sid) {
                            Some(w) => *w,
                            None => {
                                let w = ensure_work(&tx, &mut index, "show", show, now)?;
                                show_works.insert(sid.clone(), w);
                                w
                            }
                        },
                        _ => {
                            // No show to match it to: the next scan
                            // recognizes the file; its progress waits on it.
                            keys.insert(item.id.clone(), KeyPlan::File(file, abs, "episode"));
                            continue;
                        }
                    };
                    let (season, first, last) = episode_range(item);
                    let mut eps = Vec::new();
                    for number in first..=last.min(first + 50) {
                        let no = EpisodeNo {
                            season: Some(season),
                            episode: Some(number),
                            absolute: None,
                            kind: "regular",
                        };
                        let title = (first == last).then_some(item.title.as_str());
                        eps.push(ensure_episode(&tx, work, no, title)?);
                    }
                    if insert_match(&tx, file, work, &eps, conf, state, decided_by, None, now)? {
                        report.matched += 1;
                    }
                    let plan =
                        tmdb_key(item, &shows).unwrap_or(KeyPlan::File(file, abs, "episode"));
                    keys.insert(item.id.clone(), plan);
                }
                "track" => {
                    keys.insert(item.id.clone(), KeyPlan::File(file, abs, "track"));
                }
                _ => {}
            }
        }

        // Progress.
        let mut stmt = v0.prepare(
            "SELECT item_id, position_ms, duration_ms, updated_at, watched FROM progress",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        for row in rows {
            let (item_id, position, duration, updated, watched) = row?;
            let position = position.max(0);
            let duration = Some(duration).filter(|d| *d > 0);
            let watched = i64::from(watched != 0);
            let (key, kind) = match keys.get(&item_id) {
                Some(KeyPlan::Key(k, kind)) => (k.clone(), *kind),
                Some(KeyPlan::File(file, path, kind)) => {
                    let existing: Option<String> =
                        tx.query_row("SELECT fingerprint FROM files WHERE id = ?1", [file], |r| {
                            r.get(0)
                        })?;
                    let fp = match existing {
                        Some(fp) => fp,
                        None => match fingerprint::oshash_file(path) {
                            Ok(fp) => {
                                tx.execute(
                                    "UPDATE files SET fingerprint = ?2, oshash = ?2, hashed_at_ms = ?3
                                     WHERE id = ?1",
                                    params![file, fp, now],
                                )?;
                                fp
                            }
                            Err(_) => {
                                // Not readable now: waits for the scan
                                // that fingerprints the file.
                                report.progress_pending += tx.execute(
                                    "INSERT INTO import_v0_progress (file_id, source_type,
                                         position_ms, duration_ms, watched, updated_at_ms)
                                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                                     ON CONFLICT (file_id) DO UPDATE SET
                                         source_type = excluded.source_type,
                                         position_ms = excluded.position_ms,
                                         duration_ms = ifnull(excluded.duration_ms,
                                                              import_v0_progress.duration_ms),
                                         watched = max(import_v0_progress.watched, excluded.watched),
                                         updated_at_ms = excluded.updated_at_ms
                                     WHERE excluded.updated_at_ms > import_v0_progress.updated_at_ms",
                                    params![file, kind, position, duration, watched, updated],
                                )?;
                                continue;
                            }
                        },
                    };
                    (MediaKey::File { fingerprint: fp }, *kind)
                }
                None => {
                    report.progress_skipped += 1;
                    continue;
                }
            };
            let written = tx.execute(
                "INSERT INTO progress (profile_id, media_key, source_type, position_ms, duration_ms,
                                       watched, updated_at_ms, device_id)
                 VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, 'import-v0')
                 ON CONFLICT (profile_id, media_key) DO UPDATE SET
                     position_ms = excluded.position_ms,
                     duration_ms = ifnull(excluded.duration_ms, progress.duration_ms),
                     watched = max(progress.watched, excluded.watched),
                     updated_at_ms = excluded.updated_at_ms
                 WHERE excluded.updated_at_ms > progress.updated_at_ms",
                params![key.to_string(), kind, position, duration, watched, updated],
            )?;
            report.progress += written;
        }
        drop(stmt);
        tx.commit()?;
        pa_library::search::sync(&conn)?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(uri: &str) -> SmbAddress {
        SmbAddress::parse(uri).unwrap()
    }

    #[test]
    fn smb_user_moves_out_of_the_address() {
        assert_eq!(
            split_smb_user("smb://me@nas/video/Films"),
            ("smb://nas/video/Films".to_owned(), Some("me".to_owned()))
        );
        assert_eq!(
            split_smb_user("smb://dom%3Bme:secret@nas/video"),
            ("smb://nas/video".to_owned(), Some("dom;me".to_owned()))
        );
        assert_eq!(
            split_smb_user("smb://nas/video"),
            ("smb://nas/video".to_owned(), None)
        );
        assert_eq!(split_smb_user("/media/x"), ("/media/x".to_owned(), None));
    }

    #[test]
    fn gvfs_path_maps_to_the_path_in_the_share() {
        let gvfs = "/run/user/1000/gvfs";
        assert_eq!(
            gvfs_relative(
                &addr("smb://NAS/Video"),
                &format!("{gvfs}/smb-share:server=nas,share=video,user=me/Movies/x.mkv")
            )
            .as_deref(),
            Some("/Movies/x.mkv")
        );
        // The source folder is part of the mount path, not of `files.path`.
        assert_eq!(
            gvfs_relative(
                &addr("smb://nas/video/Films"),
                &format!("{gvfs}/smb-share:server=nas,share=video/Films/Heat (1995)/Heat.mkv")
            )
            .as_deref(),
            Some("/Heat (1995)/Heat.mkv")
        );
        // Percent-encoded share name, server with a port.
        assert_eq!(
            gvfs_relative(
                &addr("smb://nas:4450/my media"),
                &format!("{gvfs}/smb-share:port=4450,server=nas,share=my%20media/a.mkv")
            )
            .as_deref(),
            Some("/a.mkv")
        );
        // Another share, another folder, not a GVfs path, the root itself.
        let a = addr("smb://nas/video/Films");
        for path in [
            format!("{gvfs}/smb-share:server=nas,share=music/Films/a.mkv"),
            format!("{gvfs}/smb-share:server=nas,share=video/Shows/a.mkv"),
            "/mnt/nas/video/Films/a.mkv".to_owned(),
            format!("{gvfs}/smb-share:server=nas,share=video/Films"),
        ] {
            assert_eq!(gvfs_relative(&a, &path), None, "{path}");
        }
    }
}
