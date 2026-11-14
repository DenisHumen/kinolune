// SPDX-License-Identifier: MPL-2.0
//! The background job over a library: analyse the files of a season that
//! have no current analysis, match them, store the segments.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use pa_library::rusqlite::Connection;
use pa_library::skip::{self, SeasonFile, SkipAnalysis, SkipOrigin, SkipSegment};
use pa_probe::{Ffmpeg, ProbeErrorKind};
use pa_vfs::ByteSource;

use crate::analyze::{AnalyzeOptions, FileAnalysis, analyze_file, season_segments};
use crate::chapters::ChapterMark;
use crate::fingerprint::Fingerprint;
use crate::Error;

/// `pa-jobs` kind of a season analysis (payload: the file id), for the
/// media server's queue.
pub const JOB_KIND: &str = "skip_segments";

/// Version of the analysis stored in `skip_analysis`; raising it makes
/// every file be analysed again.
pub const ANALYSIS_VERSION: i64 = 1;

/// A failed analysis is retried after this long (a network source that
/// was away).
const RETRY_AFTER_MS: i64 = 60 * 60 * 1000;

/// Most files of a season handled at once (around the requested one).
const MAX_FILES: usize = 40;

/// Opens files of the library's sources.
pub trait Opener: Send + Sync {
    /// The file at `path` (relative to the source root) of `source_id`.
    ///
    /// # Errors
    ///
    /// Any reason the file cannot be read, as text.
    fn open(&self, source_id: i64, path: &str) -> Result<Arc<dyn ByteSource>, String>;
}

/// What [`analyze_season`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeasonReport {
    /// Files of the season.
    pub files: usize,
    /// Files read now (the others had a current analysis).
    pub analysed: usize,
    /// Files that could not be read.
    pub failed: usize,
    /// Files with an intro / credits segment now.
    pub intros: usize,
    pub credits: usize,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Whether a stored analysis still describes `file`.
fn is_current(a: &SkipAnalysis, file: &SeasonFile, now: i64) -> bool {
    a.version == ANALYSIS_VERSION
        && a.size_bytes == file.size_bytes
        && a.mtime_ms == file.mtime_ms
        && (a.error.is_none() || now - a.analyzed_at_ms < RETRY_AFTER_MS)
}

/// The analysis as stored (chapters are stored as segments right away).
fn stored(file: &SeasonFile, f: &FileAnalysis, now: i64) -> SkipAnalysis {
    SkipAnalysis {
        file_id: file.file_id,
        version: ANALYSIS_VERSION,
        size_bytes: file.size_bytes,
        mtime_ms: file.mtime_ms,
        duration_ms: f.duration_ms,
        intro_fp: f.intro.as_ref().map(Fingerprint::to_bytes),
        intro_fp_start_ms: f.intro.as_ref().map(|p| p.start_ms),
        outro_fp: f.outro.as_ref().map(Fingerprint::to_bytes),
        outro_fp_start_ms: f.outro.as_ref().map(|p| p.start_ms),
        video_credits: f.video_credits,
        error: None,
        analyzed_at_ms: now,
    }
}

/// The analysis back from storage. Chapters are not stored there: the
/// chapter segments already are.
fn restored(a: &SkipAnalysis) -> FileAnalysis {
    let fp = |bytes: &Option<Vec<u8>>, start: Option<u64>| {
        bytes
            .as_deref()
            .and_then(|b| Fingerprint::from_bytes(start.unwrap_or(0), b))
    };
    FileAnalysis {
        duration_ms: a.duration_ms,
        chapters: Vec::new(),
        intro: fp(&a.intro_fp, a.intro_fp_start_ms),
        outro: fp(&a.outro_fp, a.outro_fp_start_ms),
        video_credits: a.video_credits,
    }
}

/// Analyses the season (or folder) of `file_id`: reads the files without
/// a current analysis, matches all of them and replaces their `audio`
/// and `video` segments (and the `chapter` segments of the files read).
///
/// # Errors
///
/// Database errors and [`Error::Cancelled`]; a file that cannot be read is
/// counted in [`SeasonReport::failed`] and remembered for an hour.
pub fn analyze_season(
    conn: &Connection,
    ffmpeg: &Ffmpeg,
    opener: &dyn Opener,
    file_id: i64,
    opts: &AnalyzeOptions,
) -> Result<SeasonReport, Error> {
    let mut files = skip::season_files(conn, file_id)?;
    if files.len() > MAX_FILES {
        let at = files
            .iter()
            .position(|f| f.file_id == file_id)
            .unwrap_or(0);
        let lo = at.saturating_sub(MAX_FILES / 2).min(files.len() - MAX_FILES);
        files = files[lo..lo + MAX_FILES].to_vec();
    }
    let mut report = SeasonReport {
        files: files.len(),
        ..SeasonReport::default()
    };
    let mut analyses: Vec<FileAnalysis> = Vec::with_capacity(files.len());
    for file in &files {
        let now = now_ms();
        if let Some(a) = skip::analysis(conn, file.file_id)?
            && is_current(&a, file, now)
        {
            if a.error.is_some() {
                report.failed += 1;
            }
            analyses.push(restored(&a));
            continue;
        }
        let result = opener
            .open(file.source_id, &file.path)
            .map_err(|e| (ProbeErrorKind::Source, e))
            .and_then(|source| {
                analyze_file(ffmpeg, &source, opts).map_err(|e| (e.kind(), e.to_string()))
            });
        match result {
            Ok(f) => {
                report.analysed += 1;
                skip::save_analysis(conn, &stored(file, &f, now))?;
                let chapter_segments: Vec<SkipSegment> = crate::chapters::segments(&f.chapters)
                    .into_iter()
                    .map(|(kind, start_ms, end_ms)| SkipSegment {
                        kind,
                        origin: SkipOrigin::Chapter,
                        start_ms,
                        end_ms,
                        confidence: 1.0,
                    })
                    .collect();
                skip::replace_segments(
                    conn,
                    file.file_id,
                    &[SkipOrigin::Chapter],
                    &chapter_segments,
                    now,
                )?;
                analyses.push(FileAnalysis {
                    chapters: Vec::<ChapterMark>::new(),
                    ..f
                });
            }
            Err((ProbeErrorKind::Cancelled, _)) => return Err(Error::Cancelled),
            Err((_, message)) => {
                report.failed += 1;
                skip::save_analysis(
                    conn,
                    &SkipAnalysis {
                        file_id: file.file_id,
                        version: ANALYSIS_VERSION,
                        size_bytes: file.size_bytes,
                        mtime_ms: file.mtime_ms,
                        error: Some(message),
                        analyzed_at_ms: now,
                        ..SkipAnalysis::default()
                    },
                )?;
                analyses.push(FileAnalysis::default());
            }
        }
    }
    let refs: Vec<&FileAnalysis> = analyses.iter().collect();
    let found = season_segments(&refs);
    let now = now_ms();
    for (file, segs) in files.iter().zip(found) {
        // Chapters are stored when read; here only what the season gives.
        let segs: Vec<SkipSegment> = segs
            .into_iter()
            .filter(|s| s.origin != SkipOrigin::Chapter)
            .map(|s| SkipSegment {
                kind: s.kind,
                origin: s.origin,
                start_ms: s.start_ms,
                end_ms: s.end_ms,
                confidence: s.confidence,
            })
            .collect();
        report.intros += usize::from(segs.iter().any(|s| s.kind == skip::SkipKind::Intro));
        report.credits += usize::from(segs.iter().any(|s| s.kind == skip::SkipKind::Credits));
        skip::replace_segments(
            conn,
            file.file_id,
            &[SkipOrigin::Audio, SkipOrigin::Video],
            &segs,
            now,
        )?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_analysis_round_trips() {
        let file = SeasonFile {
            file_id: 3,
            source_id: 1,
            path: "a.mkv".into(),
            size_bytes: 10,
            mtime_ms: Some(4),
            duration_ms: None,
        };
        let f = FileAnalysis {
            duration_ms: Some(1_000_000),
            chapters: Vec::new(),
            intro: Some(Fingerprint {
                start_ms: 0,
                hashes: vec![1, 2, 3],
            }),
            outro: Some(Fingerprint {
                start_ms: 700_000,
                hashes: vec![4, 5],
            }),
            video_credits: Some((900_000, 1_000_000)),
        };
        let a = stored(&file, &f, 50);
        assert!(is_current(&a, &file, 60));
        assert_eq!(restored(&a), f);
        let changed = SeasonFile {
            size_bytes: 11,
            ..file.clone()
        };
        assert!(!is_current(&a, &changed, 60));
        let failed = SkipAnalysis {
            error: Some("offline".into()),
            ..a
        };
        assert!(is_current(&failed, &file, 60));
        assert!(!is_current(&failed, &file, 50 + RETRY_AFTER_MS));
    }
}
