// SPDX-License-Identifier: MPL-2.0
//! Saved Twitch files in the library (TWI-019): downloads, recordings and
//! imported archives matched into the TWI-007 model.
//!
//! - Each folder that holds saved files (a storage target, an imported
//!   archive) is a row of `sources` of kind [`SOURCE_KIND`]: not a media
//!   library source (pa-catalog and pa-sources skip it), only the root the
//!   `files` rows are relative to.
//! - A saved file is a `files` row plus a `twitch_assets` row (kind `vod`,
//!   `recording` or `clip`, its key `twitch:vod:<id>`…), its chat a
//!   `twitch_chat_logs` row. A VOD also gets its `twitch_videos` row and its
//!   chapters (`twitch_category_segments`, category names in
//!   `twitch_categories`) when the sidecar carries them — TwitchDownloader
//!   saves the category ids.
//! - **Deduplication** (06 §4.3): a file whose key (video id, stream id,
//!   clip slug) is already saved with about the same length (2 %) is a
//!   duplicate and is not added again; a part of a VOD (another length) is
//!   not. A path already registered is skipped.
//!
//! Metadata written here comes from the user's own files, not from Twitch,
//! but goes into the same cache tables: `purge_expired` keeps rows a saved
//! file refers to.

use std::path::{Path, PathBuf};

use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_library::twitch::{
    self as lib, Asset, AssetKind, Category, CategorySegment, ChatLog, SegmentOwner,
};
use pa_model::{BroadcasterType, Streamer, Video, VideoAvailability, VideoType};

use super::import::{Found, FoundKind, Skipped, scan};

/// `sources.kind` of folders with saved Twitch files.
pub const SOURCE_KIND: &str = "twitch_archive";

/// Error of the library side.
#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("library: {0}")]
    Library(#[from] pa_library::Error),
    #[error("database: {0}")]
    Sql(#[from] pa_library::rusqlite::Error),
    #[error("{0}")]
    Invalid(String),
}

/// Finds a channel's id by its login (the library's streamers, then Helix
/// in the app). yt-dlp and streamlink save the login only.
pub trait UserLookup {
    /// `(user_id, display_name)`.
    fn user_by_login(&self, login: &str) -> Option<(String, String)>;
}

/// `file://` URI of an absolute folder (the same encoding as pa-catalog's
/// local sources).
pub fn path_to_file_uri(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !s.starts_with('/') {
        out.push('/');
    }
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let s = String::from_utf8(out).ok()?;
    // `/C:/x` → `C:/x`.
    let b = s.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        return Some(PathBuf::from(&s[1..]));
    }
    Some(PathBuf::from(s))
}

/// The `sources` row of a folder with saved Twitch files, created if
/// needed (not scanned into the media library).
pub fn ensure_source(conn: &Connection, root: &Path, name: &str, now_ms: i64) -> Result<i64, LibraryError> {
    let uri = path_to_file_uri(root);
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM sources WHERE uri = ?1 AND kind = ?2",
            params![uri, SOURCE_KIND],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO sources (kind, name, uri, position, enabled, created_at_ms,
                              scan_into_library, auto_scan)
         VALUES (?1, ?2, ?3, 0, 1, ?4, 0, 0)",
        params![SOURCE_KIND, name, uri, now_ms],
    )?;
    Ok(conn.last_insert_rowid())
}

/// What happened to one found file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Added: `twitch_assets.id`.
    Added(i64),
    /// The same item is saved already (`twitch_assets.id` of it).
    Duplicate(i64),
    /// This path is registered already.
    Known(i64),
    /// No channel id (yt-dlp, streamlink without a known login).
    NoUser,
}

/// Result of [`import_dir`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub added: Vec<(PathBuf, i64)>,
    pub duplicates: Vec<PathBuf>,
    pub known: usize,
    /// Files whose channel is unknown.
    pub no_user: Vec<PathBuf>,
    /// Video files without a Twitch id.
    pub skipped: Vec<Skipped>,
    /// Chapters stored for the added VODs.
    pub chapters: usize,
}

fn rel_path(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn asset_kind(k: FoundKind) -> AssetKind {
    match k {
        FoundKind::Vod => AssetKind::Vod,
        FoundKind::Recording => AssetKind::Recording,
        FoundKind::Clip => AssetKind::Clip,
    }
}

fn is_id(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && !s.starts_with('0')
}

/// Saved files of the same key, `(asset id, duration)`.
fn same_key(conn: &Connection, f: &Found) -> Result<Vec<(i64, Option<u64>)>, LibraryError> {
    let (col, kind) = match f.kind {
        FoundKind::Vod => ("video_id", "vod"),
        FoundKind::Recording => ("stream_id", "recording"),
        FoundKind::Clip => ("clip_slug", "clip"),
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT a.id, coalesce(a.duration_ms, f.duration_ms) FROM twitch_assets a
         JOIN files f ON f.id = a.file_id
         WHERE a.kind = ?1 AND a.{col} = ?2 AND f.missing_since_ms IS NULL"
    ))?;
    let rows = stmt.query_map(params![kind, f.key()], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (id, d) = r?;
        out.push((id, d.and_then(|d| u64::try_from(d).ok())));
    }
    Ok(out)
}

fn close(a: Option<u64>, b: Option<u64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.abs_diff(b) as f64 <= a.max(b) as f64 * 0.02 + 1000.0,
        _ => true,
    }
}

/// Whether the item `kind`/`key` is saved already (for the "already
/// downloaded" check before queueing; a part download passes its length).
pub fn saved_asset(
    conn: &Connection,
    kind: FoundKind,
    key: &str,
    duration_ms: Option<u64>,
) -> Result<Option<i64>, LibraryError> {
    let probe = Found {
        video: PathBuf::new(),
        kind,
        video_id: (kind == FoundKind::Vod).then(|| key.to_owned()),
        stream_id: (kind == FoundKind::Recording).then(|| key.to_owned()),
        clip_slug: (kind == FoundKind::Clip).then(|| key.to_owned()),
        user_id: None,
        login: None,
        display_name: None,
        title: String::new(),
        at_ms: None,
        duration_ms,
        chapters: Vec::new(),
        chat: None,
        info: None,
        origin: super::import::Origin::FileName,
        quality: None,
        has_ads_marked: false,
        part: None,
    };
    Ok(same_key(conn, &probe)?
        .into_iter()
        .find(|(_, d)| close(*d, duration_ms))
        .map(|(id, _)| id))
}

fn category_by_name(conn: &Connection, name: &str) -> Result<Option<String>, LibraryError> {
    Ok(conn
        .query_row(
            "SELECT game_id FROM twitch_categories WHERE name = ?1 COLLATE NOCASE LIMIT 1",
            [name],
            |r| r.get(0),
        )
        .optional()?)
}

/// Registers one found file under the source `source_id` (root `root`).
pub fn register(
    conn: &mut Connection,
    source_id: i64,
    root: &Path,
    f: &Found,
    origin: &str,
    lookup: Option<&dyn UserLookup>,
    now_ms: i64,
) -> Result<(Outcome, usize), LibraryError> {
    let Some(rel) = rel_path(root, &f.video) else {
        return Err(LibraryError::Invalid(format!(
            "{} is not under {}",
            f.video.display(),
            root.display()
        )));
    };
    if let Some(id) = conn
        .query_row(
            "SELECT a.id FROM files f JOIN twitch_assets a ON a.file_id = f.id
             WHERE f.source_id = ?1 AND f.path = ?2",
            params![source_id, rel],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok((Outcome::Known(id), 0));
    }
    if let Some((id, _)) = same_key(conn, f)?
        .into_iter()
        .find(|(_, d)| close(*d, f.duration_ms))
    {
        return Ok((Outcome::Duplicate(id), 0));
    }
    // The channel.
    let mut user_id = f.user_id.clone().filter(|u| is_id(u));
    let mut display = f.display_name.clone();
    if user_id.is_none()
        && let Some(login) = f.login.as_deref()
    {
        user_id = conn
            .query_row(
                "SELECT user_id FROM twitch_streamers WHERE login = ?1 COLLATE NOCASE
                 ORDER BY updated_at_ms DESC LIMIT 1",
                [login],
                |r| r.get(0),
            )
            .optional()?;
        if user_id.is_none()
            && let Some((id, name)) = lookup.and_then(|l| l.user_by_login(login))
        {
            user_id = Some(id);
            display.get_or_insert(name);
        }
    }
    let Some(user_id) = user_id else {
        return Ok((Outcome::NoUser, 0));
    };

    let meta = std::fs::metadata(&f.video).ok();
    let size = meta.as_ref().map_or(0, std::fs::Metadata::len);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    let container = f
        .video
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);

    let tx = conn.transaction()?;
    // Streamer: only when not known (a cached Helix row is better).
    if let Some(login) = f.login.as_deref().filter(|l| !l.is_empty())
        && lib::streamer(&tx, &user_id)?.is_none()
    {
        lib::upsert_streamer(
            &tx,
            &Streamer {
                user_id: user_id.clone(),
                login: login.to_owned(),
                display_name: display.clone().unwrap_or_else(|| login.to_owned()),
                broadcaster_type: BroadcasterType::Normal,
                description: None,
                avatar_url: None,
                offline_banner_url: None,
                last_seen_live_at_ms: None,
                updated_at_ms: now_ms,
            },
            now_ms,
        )?;
    }
    tx.execute(
        "INSERT INTO files (source_id, path, size_bytes, mtime_ms, container, duration_ms, added_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            source_id,
            rel,
            i64::try_from(size).unwrap_or(i64::MAX),
            mtime,
            container,
            f.duration_ms.and_then(|d| i64::try_from(d).ok()),
            now_ms
        ],
    )?;
    let file_id = tx.last_insert_rowid();
    let title = if f.part.is_some() {
        format!("{} — {}", f.title, f.part.as_deref().unwrap_or(""))
    } else {
        f.title.clone()
    };
    let asset = Asset {
        id: 0,
        file_id,
        kind: asset_kind(f.kind),
        user_id: user_id.clone(),
        video_id: f.video_id.clone().filter(|_| f.kind == FoundKind::Vod),
        stream_id: f.stream_id.clone().filter(|s| is_id(s)),
        clip_slug: f.clip_slug.clone(),
        title,
        recorded_at_ms: f.at_ms,
        duration_ms: f.duration_ms,
        quality: f.quality.clone(),
        has_ads_marked: f.has_ads_marked,
        offset_rec_to_vod_ms: None,
        origin: origin.to_owned(),
        created_at_ms: now_ms,
    };
    let asset_id = lib::add_asset(&tx, &asset, now_ms)?;
    if let Some(chat) = &f.chat
        && let Some(path) = rel_path(root, &chat.path)
    {
        lib::set_chat_log(
            &tx,
            asset_id,
            &ChatLog {
                path,
                format: "tdl_json_v1".to_owned(),
                message_count: chat.message_count,
                first_offset_ms: chat.first_offset_ms,
                last_offset_ms: chat.last_offset_ms,
                emote_snapshot: chat.emote_snapshot,
            },
            now_ms,
        )?;
    }
    // The VOD and its chapters.
    let mut chapters = 0;
    if f.kind == FoundKind::Vod
        && let Some(video_id) = f.video_id.as_deref()
    {
        let known = lib::video(&tx, video_id)?;
        if known.is_none() {
            lib::upsert_video(
                &tx,
                &Video {
                    video_id: video_id.to_owned(),
                    user_id: user_id.clone(),
                    stream_id: f.stream_id.clone().filter(|s| is_id(s)),
                    video_type: VideoType::Archive,
                    title: f.title.clone(),
                    published_at_ms: f.at_ms.unwrap_or(0),
                    duration_ms: if f.part.is_some() { 0 } else { f.duration_ms.unwrap_or(0) },
                    view_count: 0,
                    thumbnail_url: None,
                    is_sub_only: None,
                    availability: VideoAvailability::Unknown,
                },
                now_ms,
            )?;
        }
        let stored = lib::segments(&tx, &SegmentOwner::Video(video_id.to_owned()))?;
        // A part's chapters are shifted to the part: keep the VOD's own.
        if stored.is_empty() && f.part.is_none() {
            let mut segs = Vec::new();
            for c in &f.chapters {
                let game_id = match &c.game_id {
                    Some(id) => {
                        if !c.name.is_empty() {
                            lib::upsert_category(
                                &tx,
                                &Category {
                                    game_id: id.clone(),
                                    name: c.name.clone(),
                                    box_art_url: None,
                                    igdb_id: None,
                                },
                                now_ms,
                            )?;
                        }
                        Some(id.clone())
                    }
                    None if !c.name.is_empty() => category_by_name(&tx, &c.name)?,
                    None => None,
                };
                if let Some(game_id) = game_id {
                    segs.push(CategorySegment {
                        game_id,
                        title_at_time: None,
                        start_ms: c.start_ms,
                        end_ms: c.end_ms,
                        source: "import".to_owned(),
                    });
                }
            }
            if !segs.is_empty() {
                chapters = segs.len();
                // `replace_segments` opens its own transaction.
                insert_segments(&tx, video_id, &segs)?;
            }
        }
    }
    tx.commit()?;
    Ok((Outcome::Added(asset_id), chapters))
}

fn insert_segments(
    conn: &Connection,
    video_id: &str,
    segs: &[CategorySegment],
) -> Result<(), LibraryError> {
    let mut stmt = conn.prepare(
        "INSERT INTO twitch_category_segments (video_id, stream_id, game_id, title_at_time,
                                               start_ms, end_ms, source)
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for s in segs {
        stmt.execute(params![
            video_id,
            s.game_id,
            s.title_at_time,
            i64::try_from(s.start_ms).unwrap_or(i64::MAX),
            s.end_ms.and_then(|e| i64::try_from(e).ok()),
            s.source,
        ])?;
    }
    Ok(())
}

/// Imports every saved Twitch file under `root` (06 §4.3): finds them
/// ([`scan`]), creates the source of `root` and registers each file.
/// `origin` goes into `twitch_assets.origin` (`import`, `desktop`…).
pub fn import_dir(
    conn: &mut Connection,
    root: &Path,
    name: &str,
    origin: &str,
    lookup: Option<&dyn UserLookup>,
    now_ms: i64,
) -> Result<ImportReport, LibraryError> {
    if !root.is_dir() {
        return Err(LibraryError::Invalid(format!(
            "{} is not a folder",
            root.display()
        )));
    }
    let (found, skipped) = scan(root);
    let source = ensure_source(conn, root, name, now_ms)?;
    let mut report = ImportReport {
        skipped,
        ..ImportReport::default()
    };
    for f in &found {
        match register(conn, source, root, f, origin, lookup, now_ms)? {
            (Outcome::Added(id), ch) => {
                report.added.push((f.video.clone(), id));
                report.chapters += ch;
            }
            (Outcome::Duplicate(_), _) => report.duplicates.push(f.video.clone()),
            (Outcome::Known(_), _) => report.known += 1,
            (Outcome::NoUser, _) => report.no_user.push(f.video.clone()),
        }
    }
    Ok(report)
}

/// A saved file for the library views.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub asset_id: i64,
    pub kind: AssetKind,
    pub user_id: String,
    pub login: Option<String>,
    pub display_name: Option<String>,
    pub video_id: Option<String>,
    pub stream_id: Option<String>,
    pub clip_slug: Option<String>,
    pub title: String,
    pub recorded_at_ms: Option<i64>,
    pub duration_ms: Option<u64>,
    pub size_bytes: u64,
    /// Absolute path of the video.
    pub path: PathBuf,
    pub origin: String,
    pub has_chat: bool,
    pub chapters: usize,
    pub has_ads_marked: bool,
    pub created_at_ms: i64,
}

/// Saved files, newest first; `kind` filters, files gone from disk
/// (retention moved them to the trash, the user deleted them) are left
/// out.
pub fn entries(conn: &Connection, kind: Option<AssetKind>) -> Result<Vec<Entry>, LibraryError> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.kind, a.user_id, st.login, st.display_name, a.video_id, a.stream_id,
                a.clip_slug, a.title, a.recorded_at_ms, coalesce(a.duration_ms, f.duration_ms),
                f.size_bytes, s.uri, f.path, a.origin,
                EXISTS (SELECT 1 FROM twitch_chat_logs c WHERE c.asset_id = a.id),
                (SELECT count(*) FROM twitch_category_segments g
                  WHERE a.video_id IS NOT NULL AND g.video_id = a.video_id),
                a.has_ads_marked, a.created_at_ms
         FROM twitch_assets a
         JOIN files f ON f.id = a.file_id
         JOIN sources s ON s.id = f.source_id
         LEFT JOIN twitch_streamers st ON st.user_id = a.user_id
         WHERE (?1 IS NULL OR a.kind = ?1) AND f.missing_since_ms IS NULL
         ORDER BY coalesce(a.recorded_at_ms, a.created_at_ms) DESC, a.id DESC",
    )?;
    let rows = stmt.query_map([kind.map(AssetKind::as_str)], |r| {
        let kind: String = r.get(1)?;
        let uri: String = r.get(12)?;
        let rel: String = r.get(13)?;
        Ok((
            kind,
            uri,
            rel,
            Entry {
                asset_id: r.get(0)?,
                kind: AssetKind::Vod,
                user_id: r.get(2)?,
                login: r.get(3)?,
                display_name: r.get(4)?,
                video_id: r.get(5)?,
                stream_id: r.get(6)?,
                clip_slug: r.get(7)?,
                title: r.get(8)?,
                recorded_at_ms: r.get(9)?,
                duration_ms: r.get::<_, Option<i64>>(10)?.and_then(|d| u64::try_from(d).ok()),
                size_bytes: u64::try_from(r.get::<_, i64>(11)?).unwrap_or(0),
                path: PathBuf::new(),
                origin: r.get(14)?,
                has_chat: r.get(15)?,
                chapters: usize::try_from(r.get::<_, i64>(16)?).unwrap_or(0),
                has_ads_marked: r.get(17)?,
                created_at_ms: r.get(18)?,
            },
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (kind, uri, rel, mut e) = row?;
        e.kind = match kind.as_str() {
            "recording" => AssetKind::Recording,
            "clip" => AssetKind::Clip,
            _ => AssetKind::Vod,
        };
        let Some(root) = uri_to_path(&uri) else {
            continue;
        };
        e.path = rel.split('/').fold(root, |p, c| p.join(c));
        if e.path.is_file() {
            out.push(e);
        }
    }
    Ok(out)
}

/// Removes a saved file from the library (not from disk).
pub fn forget(conn: &Connection, asset_id: i64) -> Result<bool, LibraryError> {
    Ok(conn.execute(
        "DELETE FROM files WHERE id = (SELECT file_id FROM twitch_assets WHERE id = ?1)",
        [asset_id],
    )? > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_round_trip() {
        for p in ["/home/x/Фильмы/Twitch [1]", "C:/Media/Twitch"] {
            let uri = path_to_file_uri(Path::new(p));
            assert_eq!(uri_to_path(&uri).unwrap(), PathBuf::from(p), "{uri}");
        }
    }
}
