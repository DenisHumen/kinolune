// SPDX-License-Identifier: MPL-2.0
//! Skippable segments of files — intro, recap, credits, preview — and the
//! per-file analysis behind them (PLAY-011).
//!
//! A file may have one segment of each kind per [`SkipOrigin`]: a chapter
//! named "Opening", an audio fingerprint shared with the other episodes of
//! the season, credits seen in the picture, markers imported from Plex,
//! Jellyfin or an EDL file (PLAY-015), a user correction. Playback asks
//! [`effective_segments`], which keeps the best origin of each kind:
//! user > import > chapter > audio > video.
//!
//! [`season_files`] lists the files the background job (`pa-skip`)
//! compares: the files matched to the same season of the same work, or,
//! for files without a match, the video files of the same folder.

use rusqlite::{Connection, OptionalExtension, params};

use crate::Error;

/// What a segment is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SkipKind {
    /// Opening titles / theme.
    Intro,
    /// "Previously on…".
    Recap,
    /// End credits (an ending theme included).
    Credits,
    /// A preview of the next episode after the credits.
    Preview,
}

impl SkipKind {
    pub const ALL: [Self; 4] = [Self::Intro, Self::Recap, Self::Credits, Self::Preview];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intro => "intro",
            Self::Recap => "recap",
            Self::Credits => "credits",
            Self::Preview => "preview",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// Where a segment comes from, best first (the declaration order is the
/// precedence of [`effective_segments`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SkipOrigin {
    /// Set or corrected by the user.
    User,
    /// Markers of another program (Plex, Jellyfin, EDL; PLAY-015): someone
    /// placed them for this very file, so they win over guesses.
    Import,
    /// A chapter of the file named like the segment.
    Chapter,
    /// Audio fingerprint shared across episodes of a season.
    Audio,
    /// Picture analysis (black frames, text on black).
    Video,
}

impl SkipOrigin {
    pub const ALL: [Self; 5] = [
        Self::User,
        Self::Import,
        Self::Chapter,
        Self::Audio,
        Self::Video,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Chapter => "chapter",
            Self::Import => "import",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// The program whose markers an [`SkipOrigin::Import`] segment carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SkipProvider {
    /// Plex Media Server markers (`intro`, `credits`).
    Plex,
    /// Jellyfin Media Segments or the intro-skipper plugin.
    Jellyfin,
    /// A Kodi/MPlayer edit decision list (`movie.edl`).
    Edl,
}

impl SkipProvider {
    pub const ALL: [Self; 3] = [Self::Plex, Self::Jellyfin, Self::Edl];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plex => "plex",
            Self::Jellyfin => "jellyfin",
            Self::Edl => "edl",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// One segment of a file.
#[derive(Debug, Clone, PartialEq)]
pub struct SkipSegment {
    pub kind: SkipKind,
    pub origin: SkipOrigin,
    pub start_ms: u64,
    pub end_ms: u64,
    /// 0–1.
    pub confidence: f64,
    /// For [`SkipOrigin::Import`]: whose markers these are.
    pub provider: Option<SkipProvider>,
    /// For [`SkipOrigin::Import`]: the sidecar file (name, next to the
    /// video) they were read from; `None` when a program handed them over.
    pub provider_ref: Option<String>,
}

impl SkipSegment {
    /// A segment without provider.
    #[must_use]
    pub fn new(
        kind: SkipKind,
        origin: SkipOrigin,
        start_ms: u64,
        end_ms: u64,
        confidence: f64,
    ) -> Self {
        Self {
            kind,
            origin,
            start_ms,
            end_ms,
            confidence,
            provider: None,
            provider_ref: None,
        }
    }
}

/// What the analysis of one file found (`skip_analysis`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SkipAnalysis {
    pub file_id: i64,
    /// Version of the analysis; a newer `pa-skip` redoes older rows.
    pub version: i64,
    /// Size and mtime of the file when analysed.
    pub size_bytes: u64,
    pub mtime_ms: Option<i64>,
    pub duration_ms: Option<u64>,
    /// Audio fingerprint of the intro window and where it starts.
    pub intro_fp: Option<Vec<u8>>,
    pub intro_fp_start_ms: Option<u64>,
    /// Audio fingerprint of the outro window and where it starts.
    pub outro_fp: Option<Vec<u8>>,
    pub outro_fp_start_ms: Option<u64>,
    /// Credits seen in the picture.
    pub video_credits: Option<(u64, u64)>,
    /// Why the file could not be analysed.
    pub error: Option<String>,
    pub analyzed_at_ms: i64,
}

/// A file of a season (or folder), in episode order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeasonFile {
    pub file_id: i64,
    pub source_id: i64,
    /// Path relative to the source root.
    pub path: String,
    pub size_bytes: u64,
    pub mtime_ms: Option<i64>,
    pub duration_ms: Option<u64>,
}

fn to_u64(v: Option<i64>) -> Option<u64> {
    v.and_then(|v| u64::try_from(v).ok())
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// All segments of a file, in kind and origin order.
pub fn segments(conn: &Connection, file_id: i64) -> Result<Vec<SkipSegment>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT kind, origin, start_ms, end_ms, confidence, provider, provider_ref
         FROM skip_segments WHERE file_id = ?1",
    )?;
    let rows = stmt.query_map([file_id], |r| {
        Ok((
            (
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, f64>(4)?,
            ),
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let ((kind, origin, start, end, confidence), provider, provider_ref) = row?;
        let (Some(kind), Some(origin)) = (SkipKind::parse(&kind), SkipOrigin::parse(&origin))
        else {
            continue;
        };
        out.push(SkipSegment {
            kind,
            origin,
            start_ms: u64::try_from(start).unwrap_or(0),
            end_ms: u64::try_from(end).unwrap_or(0),
            confidence,
            provider: provider.as_deref().and_then(SkipProvider::parse),
            provider_ref,
        });
    }
    out.sort_by_key(|s| (s.kind, s.origin));
    Ok(out)
}

/// The segments playback uses: the best origin of each kind, in time
/// order.
pub fn effective_segments(conn: &Connection, file_id: i64) -> Result<Vec<SkipSegment>, Error> {
    let mut best: Vec<SkipSegment> = Vec::new();
    for s in segments(conn, file_id)? {
        // `segments` is sorted by kind, then origin (best first).
        if best.last().is_none_or(|b| b.kind != s.kind) {
            best.push(s);
        }
    }
    best.sort_by_key(|s| s.start_ms);
    Ok(best)
}

/// A provider only on imported segments, and imported segments always
/// with one.
fn provider_fits(s: &SkipSegment) -> bool {
    (s.origin == SkipOrigin::Import) == s.provider.is_some()
        && (s.provider_ref.is_none() || s.origin == SkipOrigin::Import)
}

fn insert_segment(
    conn: &Connection,
    file_id: i64,
    s: &SkipSegment,
    replace: bool,
    now_ms: i64,
) -> Result<(), Error> {
    let sql = if replace {
        "INSERT OR REPLACE INTO skip_segments
             (file_id, kind, origin, start_ms, end_ms, confidence, updated_at_ms,
              provider, provider_ref)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
    } else {
        "INSERT OR IGNORE INTO skip_segments
             (file_id, kind, origin, start_ms, end_ms, confidence, updated_at_ms,
              provider, provider_ref)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
    };
    conn.execute(
        sql,
        params![
            file_id,
            s.kind.as_str(),
            s.origin.as_str(),
            to_i64(s.start_ms),
            to_i64(s.end_ms),
            s.confidence.clamp(0.0, 1.0),
            now_ms,
            s.provider.map(SkipProvider::as_str),
            s.provider_ref
        ],
    )?;
    Ok(())
}

/// Replaces the segments of `origins` of a file with `segments` (whose
/// origins must be among `origins`). Rejects empty or inverted segments,
/// and a provider on anything but [`SkipOrigin::Import`] (or an import
/// without one).
pub fn replace_segments(
    conn: &Connection,
    file_id: i64,
    origins: &[SkipOrigin],
    segments: &[SkipSegment],
    now_ms: i64,
) -> Result<(), Error> {
    if segments
        .iter()
        .any(|s| s.end_ms <= s.start_ms || !origins.contains(&s.origin) || !provider_fits(s))
    {
        return Err(Error::Invalid("skip segment out of range or origin"));
    }
    let tx = conn.unchecked_transaction()?;
    for o in origins {
        tx.execute(
            "DELETE FROM skip_segments WHERE file_id = ?1 AND origin = ?2",
            params![file_id, o.as_str()],
        )?;
    }
    for s in segments {
        insert_segment(&tx, file_id, s, true, now_ms)?;
    }
    tx.commit()?;
    Ok(())
}

/// Stores imported markers of a file (PLAY-015). There is one imported
/// segment per kind; sidecar files next to the video win over markers a
/// program hands over (the user put the file there):
///
/// - `from_sidecars` — the result of a scan of the file's sidecars
///   (every segment has a [`SkipSegment::provider_ref`]): replaces what
///   earlier scans stored and overrides handed-over markers of the same
///   kind. An empty list forgets sidecars that are gone.
/// - otherwise — markers handed over by a program (`provider_ref` is
///   `None`): replace earlier handed-over ones, but not a sidecar's.
///
/// Every segment must be [`SkipOrigin::Import`] with a provider.
pub fn replace_imported(
    conn: &Connection,
    file_id: i64,
    from_sidecars: bool,
    segments: &[SkipSegment],
    now_ms: i64,
) -> Result<(), Error> {
    if segments.iter().any(|s| {
        s.end_ms <= s.start_ms
            || s.origin != SkipOrigin::Import
            || s.provider.is_none()
            || s.provider_ref.is_some() != from_sidecars
    }) {
        return Err(Error::Invalid("imported skip segment"));
    }
    let tx = conn.unchecked_transaction()?;
    let delete = if from_sidecars {
        "DELETE FROM skip_segments
         WHERE file_id = ?1 AND origin = 'import' AND provider_ref IS NOT NULL"
    } else {
        "DELETE FROM skip_segments
         WHERE file_id = ?1 AND origin = 'import' AND provider_ref IS NULL"
    };
    tx.execute(delete, [file_id])?;
    for s in segments {
        insert_segment(&tx, file_id, s, from_sidecars, now_ms)?;
    }
    tx.commit()?;
    Ok(())
}

/// The stored analysis of a file.
pub fn analysis(conn: &Connection, file_id: i64) -> Result<Option<SkipAnalysis>, Error> {
    Ok(conn
        .query_row(
            "SELECT version, size_bytes, mtime_ms, duration_ms, intro_fp, intro_fp_start_ms,
                    outro_fp, outro_fp_start_ms, video_credits_start_ms, video_credits_end_ms,
                    error, analyzed_at_ms
             FROM skip_analysis WHERE file_id = ?1",
            [file_id],
            |r| {
                let vs: Option<i64> = r.get(8)?;
                let ve: Option<i64> = r.get(9)?;
                Ok(SkipAnalysis {
                    file_id,
                    version: r.get(0)?,
                    size_bytes: to_u64(r.get(1)?).unwrap_or(0),
                    mtime_ms: r.get(2)?,
                    duration_ms: to_u64(r.get(3)?),
                    intro_fp: r.get(4)?,
                    intro_fp_start_ms: to_u64(r.get(5)?),
                    outro_fp: r.get(6)?,
                    outro_fp_start_ms: to_u64(r.get(7)?),
                    video_credits: to_u64(vs).zip(to_u64(ve)),
                    error: r.get(10)?,
                    analyzed_at_ms: r.get(11)?,
                })
            },
        )
        .optional()?)
}

/// Stores (replaces) the analysis of a file.
pub fn save_analysis(conn: &Connection, a: &SkipAnalysis) -> Result<(), Error> {
    if a.version <= 0 {
        return Err(Error::Invalid("skip analysis version"));
    }
    conn.execute(
        "INSERT OR REPLACE INTO skip_analysis
             (file_id, version, size_bytes, mtime_ms, duration_ms, intro_fp, intro_fp_start_ms,
              outro_fp, outro_fp_start_ms, video_credits_start_ms, video_credits_end_ms,
              error, analyzed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            a.file_id,
            a.version,
            to_i64(a.size_bytes),
            a.mtime_ms,
            a.duration_ms.map(to_i64),
            a.intro_fp,
            a.intro_fp_start_ms.map(to_i64),
            a.outro_fp,
            a.outro_fp_start_ms.map(to_i64),
            a.video_credits.map(|v| to_i64(v.0)),
            a.video_credits.map(|v| to_i64(v.1)),
            a.error,
            a.analyzed_at_ms
        ],
    )?;
    Ok(())
}

/// The id of the file at `path` of a source.
pub fn file_id(conn: &Connection, source_id: i64, path: &str) -> Result<Option<i64>, Error> {
    Ok(conn
        .query_row(
            "SELECT id FROM files WHERE source_id = ?1 AND path = ?2",
            params![source_id, path],
            |r| r.get(0),
        )
        .optional()?)
}

/// Extensions of video files, for folders without matches.
const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "ts", "m2ts", "mts", "webm", "wmv", "mpg", "mpeg", "ogv",
    "flv", "3gp", "vob",
];

fn is_video(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| VIDEO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// The files of the season of `file_id`, in episode order, the file
/// itself included: the regular episodes matched to the same work and
/// season number (season 1 if unknown). A file without a match gets the
/// video files of its folder, in name order. Missing files are left out.
pub fn season_files(conn: &Connection, file_id: i64) -> Result<Vec<SeasonFile>, Error> {
    let season: Option<(i64, i64)> = conn
        .query_row(
            "SELECT e.work_id, ifnull(e.season_number, 1) FROM matches m
             JOIN match_episodes me ON me.match_id = m.id
             JOIN episodes e ON e.id = me.episode_id
             WHERE m.file_id = ?1 AND m.state <> 'rejected'
             ORDER BY me.position LIMIT 1",
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let row = |r: &rusqlite::Row<'_>| {
        Ok(SeasonFile {
            file_id: r.get(0)?,
            source_id: r.get(1)?,
            path: r.get(2)?,
            size_bytes: to_u64(r.get(3)?).unwrap_or(0),
            mtime_ms: r.get(4)?,
            duration_ms: to_u64(r.get(5)?),
        })
    };
    if let Some((work_id, season)) = season {
        let mut stmt = conn.prepare_cached(
            "SELECT f.id, f.source_id, f.path, f.size_bytes, f.mtime_ms, f.duration_ms,
                    min(ifnull(e.episode_number, e.absolute_number)) AS n
             FROM episodes e
             JOIN match_episodes me ON me.episode_id = e.id
             JOIN matches m ON m.id = me.match_id AND m.state <> 'rejected'
             JOIN files f ON f.id = m.file_id
             WHERE e.work_id = ?1 AND ifnull(e.season_number, 1) = ?2
               AND e.episode_type = 'regular' AND f.missing_since_ms IS NULL
             GROUP BY f.id
             ORDER BY n, f.path",
        )?;
        let files = stmt
            .query_map(params![work_id, season], row)?
            .collect::<Result<Vec<_>, _>>()?;
        if files.iter().any(|f| f.file_id == file_id) {
            return Ok(files);
        }
    }
    let Some((source_id, path)): Option<(i64, String)> = conn
        .query_row(
            "SELECT source_id, path FROM files WHERE id = ?1",
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
    else {
        return Ok(Vec::new());
    };
    let dir = parent(&path).to_owned();
    let mut stmt = conn.prepare_cached(
        "SELECT id, source_id, path, size_bytes, mtime_ms, duration_ms FROM files
         WHERE source_id = ?1 AND missing_since_ms IS NULL
           AND (?2 = '' OR substr(path, 1, length(?2) + 1) = ?2 || '/')
         ORDER BY path",
    )?;
    let files = stmt
        .query_map(params![source_id, dir], row)?
        .filter_map(Result::ok)
        .filter(|f| parent(&f.path) == dir && (f.file_id == file_id || is_video(&f.path)))
        .collect();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> Connection {
        let conn = crate::open_in_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO sources (id, kind, name, uri, created_at_ms) VALUES (1, 'local', 'L', 'file:///m', 0);",
        )
        .unwrap();
        conn
    }

    fn add_file(conn: &Connection, id: i64, path: &str) {
        conn.execute(
            "INSERT INTO files (id, source_id, path, size_bytes, added_at_ms) VALUES (?1, 1, ?2, 10, 0)",
            params![id, path],
        )
        .unwrap();
    }

    fn seg(kind: SkipKind, origin: SkipOrigin, start_ms: u64, end_ms: u64) -> SkipSegment {
        SkipSegment::new(kind, origin, start_ms, end_ms, 0.5)
    }

    fn imported(
        kind: SkipKind,
        provider: SkipProvider,
        sidecar: Option<&str>,
        start_ms: u64,
        end_ms: u64,
    ) -> SkipSegment {
        SkipSegment {
            provider: Some(provider),
            provider_ref: sidecar.map(str::to_owned),
            ..SkipSegment::new(kind, SkipOrigin::Import, start_ms, end_ms, 1.0)
        }
    }

    #[test]
    fn import_wins_over_chapters_and_detection() {
        let conn = library();
        add_file(&conn, 1, "S01/e1.mkv");
        replace_segments(
            &conn,
            1,
            &[SkipOrigin::Audio, SkipOrigin::Chapter],
            &[
                seg(SkipKind::Intro, SkipOrigin::Audio, 60_000, 90_000),
                seg(SkipKind::Intro, SkipOrigin::Chapter, 61_000, 91_000),
                seg(SkipKind::Credits, SkipOrigin::Audio, 1_290_000, 1_380_000),
            ],
            1,
        )
        .unwrap();
        replace_imported(
            &conn,
            1,
            false,
            &[imported(
                SkipKind::Intro,
                SkipProvider::Jellyfin,
                None,
                62_500,
                92_000,
            )],
            2,
        )
        .unwrap();
        let eff = effective_segments(&conn, 1).unwrap();
        assert_eq!(eff.len(), 2);
        assert_eq!(eff[0].origin, SkipOrigin::Import);
        assert_eq!(eff[0].provider, Some(SkipProvider::Jellyfin));
        assert_eq!((eff[0].start_ms, eff[0].end_ms), (62_500, 92_000));
        assert_eq!(eff[1].origin, SkipOrigin::Audio);
        // A user correction still wins.
        replace_segments(
            &conn,
            1,
            &[SkipOrigin::User],
            &[seg(SkipKind::Intro, SkipOrigin::User, 63_000, 93_000)],
            3,
        )
        .unwrap();
        assert_eq!(
            effective_segments(&conn, 1).unwrap()[0].origin,
            SkipOrigin::User
        );
    }

    #[test]
    fn sidecars_win_over_handed_over_markers() {
        let conn = library();
        add_file(&conn, 1, "e1.mkv");
        let plex = |kind, s, e| imported(kind, SkipProvider::Plex, None, s, e);
        let edl = |kind, s, e| imported(kind, SkipProvider::Edl, Some("e1.edl"), s, e);
        replace_imported(
            &conn,
            1,
            false,
            &[
                plex(SkipKind::Intro, 1_000, 31_000),
                plex(SkipKind::Credits, 1_300_000, 1_400_000),
            ],
            1,
        )
        .unwrap();
        // A sidecar intro overrides Plex's intro; Plex's credits stay.
        replace_imported(&conn, 1, true, &[edl(SkipKind::Intro, 2_000, 32_000)], 2).unwrap();
        let all = segments(&conn, 1).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].provider, Some(SkipProvider::Edl));
        assert_eq!(all[0].provider_ref.as_deref(), Some("e1.edl"));
        assert_eq!(all[1].provider, Some(SkipProvider::Plex));
        // Plex again: the sidecar's intro is kept.
        replace_imported(
            &conn,
            1,
            false,
            &[
                plex(SkipKind::Intro, 1_000, 31_000),
                plex(SkipKind::Credits, 1_310_000, 1_400_000),
            ],
            3,
        )
        .unwrap();
        let all = segments(&conn, 1).unwrap();
        assert_eq!(all[0].provider, Some(SkipProvider::Edl));
        assert_eq!(all[1].start_ms, 1_310_000);
        // The sidecar is gone: its rows go, Plex's credits stay.
        replace_imported(&conn, 1, true, &[], 4).unwrap();
        let all = segments(&conn, 1).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].kind, SkipKind::Credits);
        // Bad input: a sidecar segment without file name, a provider on
        // a detected segment, an import without provider.
        assert!(replace_imported(&conn, 1, true, &[plex(SkipKind::Intro, 1, 2)], 5).is_err());
        let mut audio = seg(SkipKind::Intro, SkipOrigin::Audio, 1, 2);
        audio.provider = Some(SkipProvider::Edl);
        assert!(replace_segments(&conn, 1, &[SkipOrigin::Audio], &[audio], 5).is_err());
        assert!(
            replace_segments(
                &conn,
                1,
                &[SkipOrigin::Import],
                &[seg(SkipKind::Intro, SkipOrigin::Import, 1, 2)],
                5
            )
            .is_err()
        );
    }

    #[test]
    fn best_origin_per_kind() {
        let conn = library();
        add_file(&conn, 1, "S01/e1.mkv");
        replace_segments(
            &conn,
            1,
            &[SkipOrigin::Audio, SkipOrigin::Video],
            &[
                seg(SkipKind::Intro, SkipOrigin::Audio, 60_000, 90_000),
                seg(SkipKind::Credits, SkipOrigin::Video, 1_300_000, 1_380_000),
                seg(SkipKind::Credits, SkipOrigin::Audio, 1_290_000, 1_380_000),
            ],
            1,
        )
        .unwrap();
        replace_segments(
            &conn,
            1,
            &[SkipOrigin::Chapter],
            &[seg(SkipKind::Intro, SkipOrigin::Chapter, 61_000, 91_000)],
            2,
        )
        .unwrap();
        let eff = effective_segments(&conn, 1).unwrap();
        assert_eq!(eff.len(), 2);
        assert_eq!(eff[0].origin, SkipOrigin::Chapter);
        assert_eq!(eff[0].start_ms, 61_000);
        assert_eq!(eff[1].kind, SkipKind::Credits);
        assert_eq!(eff[1].origin, SkipOrigin::Audio);
        assert_eq!(segments(&conn, 1).unwrap().len(), 4);
        // Replacing one origin leaves the others.
        replace_segments(&conn, 1, &[SkipOrigin::Audio], &[], 3).unwrap();
        let eff = effective_segments(&conn, 1).unwrap();
        assert_eq!(eff[1].origin, SkipOrigin::Video);
        // Bad input changes nothing.
        assert!(
            replace_segments(
                &conn,
                1,
                &[SkipOrigin::Video],
                &[seg(SkipKind::Intro, SkipOrigin::Video, 5, 5)],
                4
            )
            .is_err()
        );
        assert!(
            replace_segments(
                &conn,
                1,
                &[SkipOrigin::Video],
                &[seg(SkipKind::Intro, SkipOrigin::Audio, 5, 6)],
                4
            )
            .is_err()
        );
        assert_eq!(segments(&conn, 1).unwrap().len(), 2);
    }

    #[test]
    fn analysis_round_trip_and_cascade() {
        let conn = library();
        add_file(&conn, 7, "a.mkv");
        assert_eq!(analysis(&conn, 7).unwrap(), None);
        let a = SkipAnalysis {
            file_id: 7,
            version: 1,
            size_bytes: 10,
            mtime_ms: Some(5),
            duration_ms: Some(1_400_000),
            intro_fp: Some(vec![1, 2, 3, 4]),
            intro_fp_start_ms: Some(0),
            outro_fp: None,
            outro_fp_start_ms: None,
            video_credits: Some((1_300_000, 1_400_000)),
            error: None,
            analyzed_at_ms: 9,
        };
        save_analysis(&conn, &a).unwrap();
        assert_eq!(analysis(&conn, 7).unwrap(), Some(a.clone()));
        assert!(save_analysis(&conn, &SkipAnalysis { version: 0, ..a }).is_err());
        conn.execute("DELETE FROM files WHERE id = 7", []).unwrap();
        assert_eq!(analysis(&conn, 7).unwrap(), None);
    }

    #[test]
    fn folder_season_without_matches() {
        let conn = library();
        add_file(&conn, 1, "Show/S01/e02.mkv");
        add_file(&conn, 2, "Show/S01/e01.mkv");
        add_file(&conn, 3, "Show/S01/e01.srt");
        add_file(&conn, 4, "Show/S01/extras/bts.mkv");
        add_file(&conn, 5, "Show/S02/e01.mkv");
        add_file(&conn, 6, "Show/S01x/e01.mkv");
        let files = season_files(&conn, 1).unwrap();
        let ids: Vec<i64> = files.iter().map(|f| f.file_id).collect();
        assert_eq!(ids, [2, 1]);
        add_file(&conn, 8, "root.mkv");
        let ids: Vec<i64> = season_files(&conn, 8)
            .unwrap()
            .iter()
            .map(|f| f.file_id)
            .collect();
        assert_eq!(ids, [8]);
        assert!(season_files(&conn, 99).unwrap().is_empty());
    }

    #[test]
    fn matched_season_in_episode_order() {
        let conn = library();
        conn.execute_batch(
            "INSERT INTO works (id, kind, title, created_at_ms, updated_at_ms) VALUES (1, 'show', 'S', 0, 0);
             INSERT INTO episodes (id, work_id, season_number, episode_number) VALUES
                 (11, 1, 1, 2), (12, 1, 1, 1), (13, 1, 2, 1);",
        )
        .unwrap();
        add_file(&conn, 1, "a/x.mkv");
        add_file(&conn, 2, "b/y.mkv");
        add_file(&conn, 3, "c/z.mkv");
        for (m, file, ep) in [(1, 1, 11), (2, 2, 12), (3, 3, 13)] {
            conn.execute(
                "INSERT INTO matches (id, file_id, work_id, confidence, state, decided_by, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, 1, 0.9, 'auto', 'scorer', 0, 0)",
                params![m, file],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO match_episodes (match_id, position, episode_id) VALUES (?1, 0, ?2)",
                params![m, ep],
            )
            .unwrap();
        }
        let ids: Vec<i64> = season_files(&conn, 1)
            .unwrap()
            .iter()
            .map(|f| f.file_id)
            .collect();
        assert_eq!(ids, [2, 1]);
        assert_eq!(file_id(&conn, 1, "c/z.mkv").unwrap(), Some(3));
    }
}
