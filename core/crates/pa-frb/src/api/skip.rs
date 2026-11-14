// SPDX-License-Identifier: MPL-2.0
//! Skip intro and credits (`pa-skip`, PLAY-011) for Dart.
//!
//! [`CoreSkip::segments`] answers from the library at once: the stored
//! segments of a file (the best origin of each kind). [`CoreSkip::analyze`]
//! queues the season (or folder) of a file for the background job, which
//! reads the files without a current analysis through the sources' own
//! connections, matches the episodes and stores the segments; seasons are
//! analysed one at a time on one background thread. Before the analysis
//! the job imports marker sidecars next to the files (`<stem>.edl`,
//! `<stem>.plex.json`, `<stem>.plex.xml`, `<stem>.jellyfin.json`;
//! PLAY-015); [`CoreSkip::import_markers`] stores markers handed over by
//! the app (a media server's answer). Imported markers win over chapters
//! and detection; each segment says where it comes from.

use std::path::PathBuf;

use flutter_rust_bridge::frb;
use pa_library::skip::{self, SkipKind, SkipProvider};

use crate::api::library::CoreLibrary;
use crate::skip_queue::SkipQueue;

/// Error of a skip call; Dart receives it as an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipError {
    /// Human-readable message (no secrets).
    pub message: String,
}

fn err(e: impl std::fmt::Display) -> SkipError {
    SkipError {
        message: e.to_string(),
    }
}

/// What a segment is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreSkipKind {
    Intro,
    Recap,
    Credits,
    Preview,
}

/// A segment of a file playback may skip.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreSkipSegment {
    pub kind: CoreSkipKind,
    pub start_ms: u64,
    pub end_ms: u64,
    /// Where it comes from: `user`, `import`, `chapter`, `audio`, `video`
    /// (best first: playback gets the best one of each kind).
    pub origin: String,
    pub confidence: f64,
    /// For `import`: whose markers — `plex`, `jellyfin` or `edl`.
    pub provider: Option<String>,
    /// For `import`: the sidecar file next to the video they were read
    /// from (its name); `None` when the app handed them over.
    pub provider_file: Option<String>,
}

/// Skip segments of one library.
#[frb(opaque)]
pub struct CoreSkip {
    db: PathBuf,
    queue: SkipQueue,
}

impl CoreSkip {
    /// The service over `library` (its database and sources). The
    /// background thread starts with the first [`analyze`](Self::analyze).
    pub fn open(library: &CoreLibrary) -> Result<CoreSkip, SkipError> {
        let db = library.catalog().path().to_path_buf();
        Ok(Self {
            queue: SkipQueue::new(db.clone(), library.opener()),
            db,
        })
    }

    fn file_id(&self, source_id: i64, rel_path: &str) -> Result<Option<i64>, SkipError> {
        let conn = pa_library::open(&self.db).map_err(err)?;
        // The library stores paths from the source root with a leading
        // slash.
        let path = format!("/{}", rel_path.trim_start_matches('/'));
        skip::file_id(&conn, source_id, &path).map_err(err)
    }

    /// The segments of the file `rel_path` of source `source_id` (as in
    /// `CoreItem`), in time order: the best origin of each kind. Empty
    /// until the season was analysed (or the file has no marked chapters).
    pub fn segments(
        &self,
        source_id: i64,
        rel_path: String,
    ) -> Result<Vec<CoreSkipSegment>, SkipError> {
        let Some(file_id) = self.file_id(source_id, &rel_path)? else {
            return Ok(Vec::new());
        };
        let conn = pa_library::open(&self.db).map_err(err)?;
        Ok(skip::effective_segments(&conn, file_id)
            .map_err(err)?
            .into_iter()
            .map(|s| CoreSkipSegment {
                kind: match s.kind {
                    SkipKind::Intro => CoreSkipKind::Intro,
                    SkipKind::Recap => CoreSkipKind::Recap,
                    SkipKind::Credits => CoreSkipKind::Credits,
                    SkipKind::Preview => CoreSkipKind::Preview,
                },
                start_ms: s.start_ms,
                end_ms: s.end_ms,
                origin: s.origin.as_str().to_owned(),
                confidence: s.confidence,
                provider: s.provider.map(|p| p.as_str().to_owned()),
                provider_file: s.provider_ref,
            })
            .collect())
    }

    /// Stores markers of another program for the file: `provider` is
    /// `plex` (an answer of `/library/metadata/<id>?includeMarkers=1`,
    /// JSON or XML), `jellyfin` (an answer of `/MediaSegments/<id>` or of
    /// the intro-skipper plugin) or `edl` (an EDL file's text); `payload`
    /// is that text. Replaces markers handed over before; a sidecar next
    /// to the file still wins. Returns how many segments were stored;
    /// `0` if the library does not know the file.
    pub fn import_markers(
        &self,
        source_id: i64,
        rel_path: String,
        provider: String,
        payload: String,
    ) -> Result<u32, SkipError> {
        let provider = SkipProvider::parse(&provider).ok_or_else(|| SkipError {
            message: format!("unknown marker provider {provider:?}"),
        })?;
        let Some(file_id) = self.file_id(source_id, &rel_path)? else {
            return Ok(0);
        };
        let conn = pa_library::open(&self.db).map_err(err)?;
        let n = pa_skip::import_markers(&conn, file_id, provider, &payload).map_err(err)?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// Queues the season (or folder) of the file for analysis in the
    /// background and returns at once; `false` if the library does not
    /// know the file. Files analysed before are not read again.
    pub fn analyze(&self, source_id: i64, rel_path: String) -> Result<bool, SkipError> {
        let Some(file_id) = self.file_id(source_id, &rel_path)? else {
            return Ok(false);
        };
        self.queue.push(file_id);
        Ok(true)
    }
}
