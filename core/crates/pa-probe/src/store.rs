// SPDX-License-Identifier: MPL-2.0
//! Optional hook for the library pipeline: probe the files a scan left
//! unprobed and write the results into `pa-library`'s `files` table
//! (`container`, `duration_ms`, `video_json`, `audio_langs`, `sub_langs`,
//! `embedded_tags_json`, `probed_at_ms`).
//!
//! `pa-scanner` resets these columns when a file's size or mtime changes,
//! so calling [`probe_pending`] after every scan probes new and changed
//! files only. The recognition scorer reads `duration_ms` from there.

use std::sync::Arc;

use pa_library::rusqlite::{self, Connection, params};
use pa_vfs::{Vfs, VfsPath};

use crate::error::ProbeError;
use crate::library::Ffmpeg;
use crate::model::MediaInfo;
use crate::probe::ProbeOptions;

/// What [`probe_pending`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingReport {
    /// Files probed and written.
    pub probed: usize,
    /// Files libavformat could not read; marked probed (with no data) so
    /// they are not retried until they change.
    pub invalid: usize,
    /// Files left for a later run: source errors that may be transient.
    pub retry_later: usize,
}

/// Up to `limit` files of `source_id` that are present and not probed yet,
/// oldest first: `(file id, path)`.
///
/// # Errors
///
/// Database errors.
pub fn pending_files(
    conn: &Connection,
    source_id: i64,
    limit: usize,
) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, path FROM files
         WHERE source_id = ?1 AND probed_at_ms IS NULL AND missing_since_ms IS NULL
         ORDER BY id LIMIT ?2",
    )?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    stmt.query_map(params![source_id, limit], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

/// Writes `info` to file `file_id` and marks it probed at `now_ms`.
///
/// `video_json` is the main video stream ([`MediaInfo::main_video`]) as
/// JSON; `audio_langs` and `sub_langs` are JSON arrays of language codes;
/// `audio_formats` (LIB-009) a JSON array of [`MediaInfo::audio_formats`].
///
/// # Errors
///
/// Database errors.
pub fn store(
    conn: &Connection,
    file_id: i64,
    info: &MediaInfo,
    now_ms: i64,
) -> rusqlite::Result<()> {
    let video = info.main_video().map(to_json);
    let duration = info.duration_ms.and_then(|d| i64::try_from(d).ok());
    conn.prepare_cached(
        "UPDATE files SET container = ?2, duration_ms = ?3, video_json = ?4,
             audio_langs = ?5, sub_langs = ?6, probed_at_ms = ?7, audio_formats = ?8,
             embedded_tags_json = ?9
         WHERE id = ?1",
    )?
    .execute(params![
        file_id,
        info.container,
        duration,
        video,
        to_json(&info.audio_languages()),
        to_json(&info.subtitle_languages()),
        now_ms,
        to_json(&info.audio_formats()),
        embedded_tags(info),
    ])?;
    Ok(())
}

/// Container tags a phone or camera writes. Location tags are kept as
/// `"1"`: whether a file has them is a signal, where it was shot is not
/// stored.
const CAMERA_TAGS: &[&str] = &[
    "make",
    "model",
    "creation_time",
    "firmware",
    "com.apple.quicktime.make",
    "com.apple.quicktime.model",
    "com.apple.quicktime.software",
    "com.apple.quicktime.creationdate",
    "com.android.version",
    "com.android.manufacturer",
    "com.android.model",
    "com.android.capture.fps",
];
const LOCATION_TAGS: &[&str] = &[
    "location",
    "location-eng",
    "com.apple.quicktime.location.iso6709",
];

/// Audiobook tags (MUS-018) as they come from ID3 `TXXX`, Vorbis comments
/// and MP4 atoms, and the key they are stored under in `"book"`. The first
/// tag of a key wins.
const BOOK_TAGS: &[(&str, &str)] = &[
    ("narrator", "narrator"),
    ("narratedby", "narrator"),
    ("narrated_by", "narrator"),
    ("reader", "narrator"),
    ("read_by", "narrator"),
    ("series", "series"),
    ("movementname", "series"),
    ("mvnm", "series"),
    ("series-part", "series_part"),
    ("series_part", "series_part"),
    ("seriespart", "series_part"),
    ("series_index", "series_part"),
    ("movement", "series_part"),
    ("mvin", "series_part"),
];

/// Most chapters kept of one file: an audiobook of 1 000 chapters is
/// already unusual, and the column stays small.
const MAX_CHAPTERS: usize = 1000;

/// `embedded_tags_json` (LIB-025): the camera and phone tags of the
/// container and the number of chapters, what the content kind classifier
/// (`pa_recognition::content`) reads: `{"tags": {…}, "chapters": N}`.
///
/// MUS-018: a file without a picture (audio) also keeps its chapters,
/// `"chapter_list": [[start_ms, end_ms, "title"], …]`, and its audiobook
/// tags, `"book": {"narrator", "series", "series_part"}`, for the
/// audiobook library.
#[must_use]
pub fn embedded_tags(info: &MediaInfo) -> String {
    let mut tags = serde_json::Map::new();
    let mut book = serde_json::Map::new();
    for (k, v) in &info.tags {
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        if CAMERA_TAGS.contains(&k.as_str()) {
            let v: String = v.chars().take(100).collect();
            tags.insert(k.clone(), v.into());
        } else if LOCATION_TAGS.contains(&k.as_str()) {
            tags.insert(k.clone(), "1".into());
        }
    }
    let audio = info.main_video().is_none() && !info.audio.is_empty();
    let mut json = serde_json::json!({ "tags": tags, "chapters": info.chapters.len() });
    if audio {
        for (tag, key) in BOOK_TAGS {
            if book.contains_key(*key) {
                continue;
            }
            if let Some(v) = info
                .tags
                .get(*tag)
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
            {
                let v: String = v.chars().take(200).collect();
                book.insert((*key).to_owned(), v.into());
            }
        }
        if !book.is_empty() {
            json["book"] = book.into();
        }
        if info.chapters.len() > 1 {
            let list: Vec<serde_json::Value> = info
                .chapters
                .iter()
                .take(MAX_CHAPTERS)
                .map(|c| {
                    let title = c
                        .title
                        .as_deref()
                        .map(str::trim)
                        .filter(|t| !t.is_empty())
                        .map(|t| t.chars().take(200).collect::<String>());
                    serde_json::json!([c.start_ms, c.end_ms, title])
                })
                .collect();
            json["chapter_list"] = list.into();
        }
    }
    json.to_string()
}

/// Marks file `file_id` probed at `now_ms` without data, so it is not
/// probed again until a scan sees it change.
///
/// # Errors
///
/// Database errors.
pub fn mark_unreadable(conn: &Connection, file_id: i64, now_ms: i64) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "UPDATE files SET container = NULL, duration_ms = NULL, video_json = NULL,
             audio_langs = NULL, sub_langs = NULL, audio_formats = NULL,
             embedded_tags_json = NULL, probed_at_ms = ?2
         WHERE id = ?1",
    )?
    .execute(params![file_id, now_ms])?;
    Ok(())
}

/// Probes up to `limit` [pending](pending_files) files of `source_id`
/// whose path passes `filter` (e.g. video extensions only) and writes the
/// results, stamped with the system clock. Files rejected by `filter` are
/// left untouched.
///
/// Stops early with the error when FFmpeg is unavailable or the probe is
/// cancelled ([`ProbeError::is_retryable`] kinds other than source errors).
///
/// # Errors
///
/// [`PendingError::Db`] on database errors, [`PendingError::Probe`] when
/// probing cannot continue.
pub fn probe_pending(
    conn: &Connection,
    source_id: i64,
    vfs: &dyn Vfs,
    ffmpeg: &Ffmpeg,
    opts: &ProbeOptions,
    filter: &dyn Fn(&str) -> bool,
    limit: usize,
) -> Result<PendingReport, PendingError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    let mut report = PendingReport::default();
    for (id, path) in pending_files(conn, source_id, limit)? {
        if !filter(&path) {
            continue;
        }
        let result = VfsPath::parse(&path)
            .map_err(|e| ProbeError::from_vfs(&e))
            .and_then(|p| vfs.open(&p).map_err(|e| ProbeError::from_vfs(&e)))
            .and_then(|src| ffmpeg.probe(Arc::from(src), opts));
        match result {
            Ok(info) => {
                store(conn, id, &info, now_ms)?;
                report.probed += 1;
            }
            Err(e) if e.kind() == crate::ProbeErrorKind::Source => {
                if e.is_retryable() {
                    report.retry_later += 1;
                } else {
                    // Permission denied, not found, bad path: retried after
                    // the next scan sees the file change.
                    mark_unreadable(conn, id, now_ms)?;
                    report.invalid += 1;
                }
            }
            Err(e) if e.is_retryable() => return Err(PendingError::Probe(e)),
            Err(_) => {
                mark_unreadable(conn, id, now_ms)?;
                report.invalid += 1;
            }
        }
    }
    Ok(report)
}

/// Why [`probe_pending`] stopped.
#[derive(Debug)]
pub enum PendingError {
    /// The database failed.
    Db(rusqlite::Error),
    /// FFmpeg is unavailable or the probe was cancelled.
    Probe(ProbeError),
}

impl From<rusqlite::Error> for PendingError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e)
    }
}

impl std::fmt::Display for PendingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Db(e) => write!(f, "library database: {e}"),
            Self::Probe(e) => write!(f, "probe: {e}"),
        }
    }
}

impl std::error::Error for PendingError {}

/// Compact JSON; serialising plain data cannot fail.
fn to_json<T: serde::Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AudioStream, StreamCommon, VideoStream};

    fn db() -> Connection {
        let conn = pa_library::open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO sources (id, kind, name, uri, created_at_ms) VALUES (1, 'local', 'Movies', 'file:///movies', 0)",
            [],
        )
        .unwrap();
        for (id, path, missing) in [
            (1, "/a.mkv", None),
            (2, "/b.mp4", None),
            (3, "/gone.mkv", Some(5)),
        ] {
            conn.execute(
                "INSERT INTO files (id, source_id, path, size_bytes, added_at_ms, missing_since_ms)
                 VALUES (?1, 1, ?2, 100, 0, ?3)",
                params![id, path, missing],
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn stores_and_marks() {
        let conn = db();
        assert_eq!(
            pending_files(&conn, 1, 10).unwrap(),
            [(1, "/a.mkv".to_owned()), (2, "/b.mp4".to_owned())]
        );
        let lang = |l: &str| AudioStream {
            common: StreamCommon {
                codec: "aac".into(),
                language: Some(l.into()),
                ..StreamCommon::default()
            },
            ..AudioStream::default()
        };
        let info = MediaInfo {
            container: "matroska,webm".into(),
            duration_ms: Some(5_400_000),
            video: vec![VideoStream {
                width: 1920,
                height: 1080,
                ..VideoStream::default()
            }],
            audio: vec![lang("rus"), lang("eng")],
            ..MediaInfo::default()
        };
        store(&conn, 1, &info, 42).unwrap();
        mark_unreadable(&conn, 2, 43).unwrap();
        assert!(pending_files(&conn, 1, 10).unwrap().is_empty());

        let (container, duration, video, audio, subs, at): (
            String,
            i64,
            String,
            String,
            String,
            i64,
        ) = conn
            .query_row(
                "SELECT container, duration_ms, video_json, audio_langs, sub_langs, probed_at_ms
                 FROM files WHERE id = 1",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(container, "matroska,webm");
        assert_eq!(duration, 5_400_000);
        let video: serde_json::Value = serde_json::from_str(&video).unwrap();
        assert_eq!(video["width"], 1920);
        assert_eq!(audio, r#"["rus","eng"]"#);
        assert_eq!(subs, "[]");
        assert_eq!(at, 42);
        let formats: String = conn
            .query_row("SELECT audio_formats FROM files WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(formats, r#"["aac"]"#);
        // A scan that sees the file change resets the probe columns; the
        // LIB-009 trigger clears the audio formats with them.
        conn.execute("UPDATE files SET probed_at_ms = NULL WHERE id = 1", [])
            .unwrap();
        let formats: Option<String> = conn
            .query_row("SELECT audio_formats FROM files WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(formats, None);
        store(&conn, 1, &info, 42).unwrap();

        let (container, at): (Option<String>, i64) = conn
            .query_row(
                "SELECT container, probed_at_ms FROM files WHERE id = 2",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((container, at), (None, 43));
    }

    #[test]
    fn camera_tags_without_the_place() {
        let mut info = MediaInfo::default();
        for (k, v) in [
            ("com.apple.quicktime.make", "Apple"),
            ("com.apple.quicktime.location.iso6709", "+55.7558+037.6173/"),
            ("encoder", "Lavf58"),
            ("title", "Holiday"),
        ] {
            info.tags.insert(k.to_owned(), v.to_owned());
        }
        info.chapters = vec![crate::model::Chapter::default(); 3];
        let json: serde_json::Value = serde_json::from_str(&embedded_tags(&info)).unwrap();
        assert_eq!(json["tags"]["com.apple.quicktime.make"], "Apple");
        assert_eq!(json["tags"]["com.apple.quicktime.location.iso6709"], "1");
        assert!(json["tags"].get("encoder").is_none());
        assert!(json["tags"].get("title").is_none());
        assert_eq!(json["chapters"], 3);
        // A video keeps no chapter list and no book tags.
        assert!(json.get("chapter_list").is_none());
        assert!(json.get("book").is_none());
    }

    #[test]
    fn an_audiobook_keeps_its_chapters_and_book_tags() {
        let mut info = MediaInfo::default();
        info.audio.push(crate::model::AudioStream::default());
        for (k, v) in [
            ("narrator", "Кирилл Радциг"),
            ("reader", "someone else"),
            ("series", " Мир Полудня "),
            ("series-part", "3"),
            ("title", "Обитаемый остров"),
        ] {
            info.tags.insert(k.to_owned(), v.to_owned());
        }
        info.chapters = vec![
            crate::model::Chapter {
                start_ms: 0,
                end_ms: 60_000,
                title: Some("Пролог".into()),
            },
            crate::model::Chapter {
                start_ms: 60_000,
                end_ms: 120_000,
                title: Some("  ".into()),
            },
        ];
        let json: serde_json::Value = serde_json::from_str(&embedded_tags(&info)).unwrap();
        assert_eq!(json["chapters"], 2);
        assert_eq!(
            json["chapter_list"],
            serde_json::json!([[0, 60_000, "Пролог"], [60_000, 120_000, null]])
        );
        assert_eq!(json["book"]["narrator"], "Кирилл Радциг");
        assert_eq!(json["book"]["series"], "Мир Полудня");
        assert_eq!(json["book"]["series_part"], "3");
        assert!(json["tags"].get("title").is_none());
    }
}
