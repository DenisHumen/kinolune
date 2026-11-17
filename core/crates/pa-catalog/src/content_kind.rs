// SPDX-License-Identifier: MPL-2.0
//! The content kind of every media file (LIB-025, ADR-0099): the user never
//! picks it. After the container probe, each scan decides the files of a
//! source folder by folder with `pa_recognition::content` and stores the
//! verdicts in `file_content_kinds`:
//!
//! - `video`: a movie, show or anime; recognized as before;
//! - `own_video`: footage from a phone or camera. «Мои видео»
//!   ([`Catalog::own_videos`]): never recognized, never sent to a
//!   provider, neither hidden nor locked (moving it to «Личное» is the
//!   user's choice, ADR-0099 §3). A file that becomes one loses its
//!   automatic match;
//! - `music` and `audiobook`: tracks of the music library; audiobooks are
//!   their own section of «Музыка» until MUS-018.
//!
//! «Это не то» ([`Catalog::not_this`]) is one action on a folder: its files
//! of the folder's main medium become the other kind of that medium, and
//! the folder keeps it (`folder_content_overrides`). The same action again
//! brings back what was detected. A user's own match (or an NFO's) keeps a
//! video a video.

use std::collections::{BTreeMap, HashMap};

use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_recognition::content::{
    AudioFile, CLASSIFIER_VERSION, ContentKind, Verdict, VideoFile, classify_audio,
    classify_videos, folder_of,
};
use pa_scanner::MediaKind;

use crate::items::{local_path, relative};
use crate::{Catalog, Error};

/// One file as read for the classifier.
struct Row {
    id: i64,
    path: String,
    audio: bool,
    duration_ms: Option<i64>,
    tags: Vec<(String, String)>,
    chapters: u32,
    genres: Vec<String>,
    texts: Vec<String>,
    /// A live match the user (or an NFO) made: stays a video.
    decided: bool,
}

fn split(v: Option<String>) -> Vec<String> {
    v.map(|s| {
        s.split('\u{1f}')
            .filter(|x| !x.is_empty())
            .map(str::to_owned)
            .collect()
    })
    .unwrap_or_default()
}

/// `embedded_tags_json` as `pa_probe::store` writes it.
fn embedded(json: Option<&str>) -> (Vec<(String, String)>, u32) {
    let Some(v) = json.and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok()) else {
        return (Vec::new(), 0);
    };
    let tags = v["tags"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let chapters = v["chapters"]
        .as_u64()
        .and_then(|c| u32::try_from(c).ok())
        .unwrap_or(0);
    (tags, chapters)
}

const ROWS_SQL: &str = "
SELECT f.id, f.path, coalesce(mt.duration_ms, f.duration_ms), f.embedded_tags_json,
       mt.title, ma.title, mt.album_artist_tag,
       (SELECT group_concat(a.name, char(31)) FROM music_track_artists x
          JOIN music_artists a ON a.id = x.artist_id WHERE x.track_id = mt.id),
       (SELECT group_concat(g.name, char(31)) FROM music_track_genres x
          JOIN music_genres g ON g.id = x.genre_id WHERE x.track_id = mt.id),
       EXISTS (SELECT 1 FROM matches m WHERE m.file_id = f.id AND m.state <> 'rejected'
                 AND (m.decided_by IN ('user', 'nfo')
                      OR m.state IN ('user_confirmed', 'user_set')))
FROM files f
LEFT JOIN music_tracks mt ON mt.file_id = f.id
LEFT JOIN music_albums ma ON ma.id = mt.album_id
WHERE f.source_id = ?1 AND f.missing_since_ms IS NULL";

fn rows(conn: &Connection, source_id: i64) -> Result<Vec<Row>, Error> {
    let mut stmt = conn.prepare_cached(ROWS_SQL)?;
    let mut out = Vec::new();
    let mut q = stmt.query([source_id])?;
    while let Some(r) = q.next()? {
        let path: String = r.get(1)?;
        let audio = match pa_scanner::media_kind(&path) {
            Some(MediaKind::Audio) => true,
            Some(MediaKind::Video) => false,
            None => continue,
        };
        let (tags, chapters) = embedded(r.get::<_, Option<String>>(3)?.as_deref());
        let mut texts: Vec<String> = [r.get::<_, Option<String>>(4)?, r.get(5)?, r.get(6)?]
            .into_iter()
            .flatten()
            .collect();
        texts.extend(split(r.get(7)?));
        out.push(Row {
            id: r.get(0)?,
            path,
            audio,
            duration_ms: r.get(2)?,
            tags,
            chapters,
            genres: split(r.get(8)?),
            texts,
            decided: r.get(9)?,
        });
    }
    Ok(out)
}

/// The classifier's verdict for every row, folder by folder.
fn detect(rows: &[Row]) -> Vec<Verdict> {
    let mut out: Vec<Option<Verdict>> = vec![None; rows.len()];
    let mut groups: BTreeMap<(&str, bool), Vec<usize>> = BTreeMap::new();
    for (i, r) in rows.iter().enumerate() {
        groups
            .entry((folder_of(&r.path), r.audio))
            .or_default()
            .push(i);
    }
    let ms = |d: Option<i64>| d.and_then(|d| u64::try_from(d).ok());
    for ((_, audio), idx) in groups {
        let verdicts = if audio {
            let files: Vec<AudioFile<'_>> = idx
                .iter()
                .map(|&i| AudioFile {
                    path: &rows[i].path,
                    duration_ms: ms(rows[i].duration_ms),
                    chapters: rows[i].chapters,
                    genres: rows[i].genres.iter().map(String::as_str).collect(),
                    texts: rows[i].texts.iter().map(String::as_str).collect(),
                })
                .collect();
            classify_audio(&files)
        } else {
            let files: Vec<VideoFile<'_>> = idx
                .iter()
                .map(|&i| VideoFile {
                    path: &rows[i].path,
                    duration_ms: ms(rows[i].duration_ms),
                    tags: rows[i]
                        .tags
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect(),
                })
                .collect();
            classify_videos(&files)
        };
        for (v, &i) in verdicts.into_iter().zip(&idx) {
            out[i] = Some(v);
        }
    }
    out.into_iter()
        .map(|v| {
            v.unwrap_or(Verdict {
                kind: ContentKind::Video,
                score: 0.0,
                reasons: Vec::new(),
            })
        })
        .collect()
}

/// `true` if `folder` (a VfsPath) is `path` or one of its parents.
fn covers(folder: &str, path: &str) -> bool {
    folder == "/"
        || path == folder
        || (path.starts_with(folder) && path.as_bytes().get(folder.len()) == Some(&b'/'))
}

/// «Это не то» of `source_id`: folder → kind, longest first.
fn overrides(conn: &Connection, source_id: i64) -> Result<Vec<(String, ContentKind)>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT path, kind FROM folder_content_overrides WHERE source_id = ?1
         ORDER BY length(path) DESC",
    )?;
    let rows = stmt
        .query_map([source_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(p, k)| ContentKind::parse(&k).map(|k| (p, k)))
        .collect())
}

/// The override of `path` for its medium, if any.
fn override_of(
    overrides: &[(String, ContentKind)],
    path: &str,
    audio: bool,
) -> Option<ContentKind> {
    overrides
        .iter()
        .find(|(f, k)| k.is_audio() == audio && covers(f, path))
        .map(|(_, k)| *k)
}

/// What [`classify`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ClassifyReport {
    /// Files whose kind was written (new or changed).
    pub written: usize,
    /// Automatic matches dropped from files that are own videos now.
    pub unmatched: usize,
}

/// Decides the kind of every present media file of `source_id` and stores
/// what changed. Files of a `personal` source are not decided (they are
/// never recognized anyway). Own videos lose their automatic matches; works
/// left without files go with `recognize::remove_orphans`.
pub(crate) fn classify(conn: &mut Connection, source_id: i64) -> Result<ClassifyReport, Error> {
    let mut report = ClassifyReport::default();
    if pa_library::personal::content_type(conn, source_id)?
        == Some(pa_library::personal::ContentType::Personal)
    {
        return Ok(report);
    }
    let rows = rows(conn, source_id)?;
    let verdicts = detect(&rows);
    let overrides = overrides(conn, source_id)?;
    let existing: HashMap<i64, (String, String, i64)> = {
        let mut stmt = conn.prepare_cached(
            "SELECT k.file_id, k.kind, k.detected, k.version FROM file_content_kinds k
             JOIN files f ON f.id = k.file_id WHERE f.source_id = ?1",
        )?;
        stmt.query_map([source_id], |r| {
            Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?)))
        })?
        .collect::<Result<_, _>>()?
    };
    let now = crate::now_ms();
    let version = i64::from(CLASSIFIER_VERSION);
    let tx = conn.transaction()?;
    {
        let mut upsert = tx.prepare_cached(
            "INSERT INTO file_content_kinds
                 (file_id, kind, detected, version, score, reasons_json, classified_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (file_id) DO UPDATE SET kind = excluded.kind,
                 detected = excluded.detected, version = excluded.version,
                 score = excluded.score, reasons_json = excluded.reasons_json,
                 classified_at_ms = excluded.classified_at_ms",
        )?;
        let mut unmatch = tx.prepare_cached(
            "DELETE FROM matches WHERE file_id = ?1 AND state IN ('auto', 'auto_review')
                 AND decided_by NOT IN ('user', 'nfo')",
        )?;
        for (row, v) in rows.iter().zip(&verdicts) {
            let mut kind = override_of(&overrides, &row.path, row.audio).unwrap_or(v.kind);
            let mut reasons = v.reasons.clone();
            if kind != v.kind {
                reasons.push("not_this");
            }
            if kind == ContentKind::OwnVideo && row.decided {
                kind = ContentKind::Video;
                reasons.push("user_match");
            }
            let same = existing.get(&row.id).is_some_and(|(k, d, ver)| {
                k == kind.as_str() && d == v.kind.as_str() && *ver == version
            });
            if !same {
                let json = serde_json::to_string(&reasons).unwrap_or_else(|_| "[]".to_owned());
                upsert.execute(params![
                    row.id,
                    kind.as_str(),
                    v.kind.as_str(),
                    version,
                    f64::from(v.score),
                    json,
                    now
                ])?;
                report.written += 1;
            }
            if kind == ContentKind::OwnVideo {
                report.unmatched += unmatch.execute([row.id])?;
            }
        }
    }
    tx.commit()?;
    Ok(report)
}

/// A video of «Мои видео».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnVideo {
    /// `files.id`.
    pub file_id: i64,
    pub source_id: i64,
    /// Root URI of the source.
    pub source_uri: String,
    /// Path relative to the source root.
    pub rel_path: String,
    /// Absolute path for a local source.
    pub local_path: Option<String>,
    /// The file name without its extension.
    pub title: String,
    pub size_bytes: i64,
    pub duration_ms: Option<i64>,
    /// When the file was modified (shot, for a camera), else added.
    pub taken_at_ms: i64,
}

/// What a folder holds, for the file browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderKind {
    /// Relative to the source root, no leading slash.
    pub path: String,
    /// The kind most of its media files have.
    pub kind: ContentKind,
    /// The user said «Это не то» on it or a folder above it.
    pub overridden: bool,
}

const OWN_SQL: &str = concat!(
    "
SELECT f.id, f.source_id, s.uri, f.path, f.size_bytes, f.duration_ms,
       coalesce(f.mtime_ms, f.added_at_ms)
FROM file_content_kinds k
JOIN files f ON f.id = k.file_id AND f.missing_since_ms IS NULL
JOIN sources s ON s.id = f.source_id
WHERE k.kind = 'own_video'
  AND NOT EXISTS (SELECT 1 FROM matches m WHERE m.file_id = f.id AND m.state <> 'rejected')
  AND NOT EXISTS (SELECT 1 FROM group_assets ga WHERE ga.file_id = f.id AND ga.role = 'ignore')
  AND ",
    pa_library::file_visible!("f"),
    "
ORDER BY coalesce(f.mtime_ms, f.added_at_ms) DESC, f.path"
);

fn file_stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_owned(),
        _ => name.to_owned(),
    }
}

/// Live media files under `dir` (a VfsPath) of `source_id` with their kind.
fn kinds_under(
    conn: &Connection,
    source_id: i64,
    dir: &str,
) -> Result<Vec<(String, ContentKind)>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT f.path, k.kind FROM files f JOIN file_content_kinds k ON k.file_id = f.id
         WHERE f.source_id = ?1 AND f.missing_since_ms IS NULL
           AND (?2 = '/' OR f.path = ?2 OR substr(f.path, 1, length(?2) + 1) = ?2 || '/')",
    )?;
    let rows = stmt
        .query_map(params![source_id, dir], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(p, k)| ContentKind::parse(&k).map(|k| (p, k)))
        .collect())
}

/// The kind most files have; ties go to the titled kinds (video, music).
fn dominant(kinds: impl Iterator<Item = ContentKind>) -> Option<ContentKind> {
    let mut counts: BTreeMap<ContentKind, usize> = BTreeMap::new();
    for k in kinds {
        *counts.entry(k).or_default() += 1;
    }
    // `ContentKind` orders Video < OwnVideo < Music < Audiobook: the first
    // of equal counts wins.
    counts
        .into_iter()
        .fold(
            None,
            |best: Option<(ContentKind, usize)>, (k, n)| match best {
                Some((_, m)) if m >= n => best,
                _ => Some((k, n)),
            },
        )
        .map(|(k, _)| k)
}

fn vfs_path(rel: &str) -> String {
    format!("/{}", rel.trim_matches('/'))
}

impl Catalog {
    /// «Мои видео»: footage from phones and cameras in the main library
    /// (not personal, visible to the active profile), newest first. Never
    /// recognized; a file the user matched by hand is an item instead.
    pub fn own_videos(&self) -> Result<Vec<OwnVideo>, Error> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(OWN_SQL)?;
        let rows = stmt
            .query_map([], |r| {
                let uri: String = r.get(2)?;
                let path: String = r.get(3)?;
                let rel = relative(&path);
                Ok(OwnVideo {
                    file_id: r.get(0)?,
                    source_id: r.get(1)?,
                    local_path: local_path(&uri, &rel),
                    source_uri: uri,
                    title: file_stem(&path),
                    rel_path: rel,
                    size_bytes: r.get(4)?,
                    duration_ms: r.get(5)?,
                    taken_at_ms: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// What the subfolders of `dir` (relative to the source root, `""` for
    /// the root) of `source_id` hold, by name. Folders without decided
    /// media files are left out.
    pub fn folder_kinds(&self, source_id: i64, dir: &str) -> Result<Vec<FolderKind>, Error> {
        let conn = self.lock();
        let base = vfs_path(dir);
        let overrides = overrides(&conn, source_id)?;
        let prefix = if base == "/" {
            "/".to_owned()
        } else {
            format!("{base}/")
        };
        let mut children: BTreeMap<String, Vec<ContentKind>> = BTreeMap::new();
        for (path, kind) in kinds_under(&conn, source_id, &base)? {
            let Some(rest) = path.strip_prefix(&prefix) else {
                continue;
            };
            if let Some((child, _)) = rest.split_once('/') {
                children.entry(child.to_owned()).or_default().push(kind);
            }
        }
        Ok(children
            .into_iter()
            .filter_map(|(child, kinds)| {
                let kind = dominant(kinds.into_iter())?;
                let folder = format!("{prefix}{child}");
                let overridden = overrides
                    .iter()
                    .any(|(f, k)| k.is_audio() == kind.is_audio() && covers(f, &folder));
                Some(FolderKind {
                    path: relative(&folder),
                    kind,
                    overridden,
                })
            })
            .collect())
    }

    /// «Это не то» on folder `path` (relative to the source root, `""` for
    /// the whole source) of `source_id`: its files of the folder's main
    /// medium become the other kind of that medium (a movie folder becomes
    /// own videos and the other way round, music becomes an audiobook and
    /// back), and the folder keeps it. The same action again brings back
    /// what was detected. Own videos lose their automatic matches; videos
    /// are recognized right away. Returns the folder's new kind, `None` if
    /// it holds no decided media file.
    pub fn not_this(&self, source_id: i64, path: &str) -> Result<Option<ContentKind>, Error> {
        let _serial = self.scan_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut conn = pa_library::open(&self.path)?;
        let folder = vfs_path(path);
        let files = kinds_under(&conn, source_id, &folder)?;
        let Some(current) = dominant(files.iter().map(|(_, k)| *k)) else {
            return Ok(None);
        };
        let new = current.flipped();
        let audio = new.is_audio();
        // What the classifier says for the medium, without any override.
        let detected: Option<ContentKind> = {
            let mut stmt = conn.prepare_cached(
                "SELECT k.detected FROM files f JOIN file_content_kinds k ON k.file_id = f.id
                 WHERE f.source_id = ?1 AND f.missing_since_ms IS NULL
                   AND (?2 = '/' OR f.path = ?2 OR substr(f.path, 1, length(?2) + 1) = ?2 || '/')",
            )?;
            let kinds = stmt
                .query_map(params![source_id, folder], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            dominant(
                kinds
                    .iter()
                    .filter_map(|k| ContentKind::parse(k))
                    .filter(|k| k.is_audio() == audio),
            )
        };
        let parents = overrides(&conn, source_id)?
            .into_iter()
            .any(|(f, k)| f != folder && k.is_audio() == audio && covers(&f, &folder));
        let now = crate::now_ms();
        if detected == Some(new) && !parents {
            conn.execute(
                "DELETE FROM folder_content_overrides WHERE source_id = ?1 AND path = ?2",
                params![source_id, folder],
            )?;
        } else {
            // A folder holds one override; one of the other medium goes.
            conn.execute(
                "INSERT INTO folder_content_overrides (source_id, path, kind, set_at_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (source_id, path) DO UPDATE SET kind = excluded.kind,
                     set_at_ms = excluded.set_at_ms",
                params![source_id, folder, new.as_str(), now],
            )?;
        }
        classify(&mut conn, source_id)?;
        crate::recognize::run(&mut conn, source_id, &mut |_, _| {})?;
        {
            let tx = conn.transaction()?;
            crate::recognize::remove_orphans(&tx)?;
            tx.commit()?;
        }
        pa_library::search::sync(&conn)?;
        Ok(Some(new))
    }

    /// The kind of file `rel_path` of `source_id`, if decided.
    pub fn content_kind(
        &self,
        source_id: i64,
        rel_path: &str,
    ) -> Result<Option<ContentKind>, Error> {
        let conn = self.lock();
        let kind: Option<String> = conn
            .query_row(
                "SELECT k.kind FROM files f JOIN file_content_kinds k ON k.file_id = f.id
                 WHERE f.source_id = ?1 AND f.path = ?2",
                params![source_id, vfs_path(rel_path)],
                |r| r.get(0),
            )
            .optional()?;
        Ok(kind.and_then(|k| ContentKind::parse(&k)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_cover_their_files() {
        assert!(covers("/", "/a/b.mp4"));
        assert!(covers("/a", "/a/b.mp4"));
        assert!(covers("/a", "/a"));
        assert!(!covers("/a", "/ab/c.mp4"));
    }

    #[test]
    fn ties_go_to_titled_kinds() {
        use ContentKind::*;
        assert_eq!(dominant([OwnVideo, Video].into_iter()), Some(Video));
        assert_eq!(
            dominant([OwnVideo, OwnVideo, Video].into_iter()),
            Some(OwnVideo)
        );
        assert_eq!(dominant([Audiobook, Music].into_iter()), Some(Music));
        assert_eq!(dominant(std::iter::empty()), None);
    }

    #[test]
    fn embedded_tags_are_read() {
        let (tags, ch) = embedded(Some(r#"{"tags":{"make":"Apple"},"chapters":4}"#));
        assert_eq!(tags, [("make".to_owned(), "Apple".to_owned())]);
        assert_eq!(ch, 4);
        assert_eq!(embedded(None), (Vec::new(), 0));
        assert_eq!(embedded(Some("not json")), (Vec::new(), 0));
    }
}
