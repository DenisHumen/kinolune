// SPDX-License-Identifier: MPL-2.0
//! Skip intro and credits (`pa-skip`, PLAY-011) for Dart.
//!
//! [`CoreSkip::segments`] answers from the library at once: the stored
//! segments of a file (the best origin of each kind). [`CoreSkip::analyze`]
//! queues the season (or folder) of a file for the background job, which
//! reads the files without a current analysis through the sources' own
//! connections, matches the episodes and stores the segments; seasons are
//! analysed one at a time on one background thread.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use flutter_rust_bridge::frb;
use pa_catalog::SourceOpener;
use pa_library::skip::{self, SkipKind};
use pa_vfs::VfsPath;

use crate::api::library::CoreLibrary;

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
    /// Where it comes from: `chapter`, `audio`, `video`, `import`, `user`.
    pub origin: String,
    pub confidence: f64,
}

/// The queue of the background thread: file ids whose seasons wait.
#[derive(Default)]
struct Queue {
    pending: Mutex<VecDeque<i64>>,
    wake: Condvar,
    /// Whether the background thread runs.
    running: AtomicBool,
}

/// Opens library files for the job through the source manager.
struct Files(Arc<SourceOpener>);

impl pa_skip::Opener for Files {
    fn open(&self, source_id: i64, path: &str) -> Result<Arc<dyn pa_vfs::ByteSource>, String> {
        let path = VfsPath::parse(path).map_err(|e| e.to_string())?;
        // Through the opener first: it reconnects a stale source.
        self.0.open_file(source_id, &path).map_err(|e| e.to_string())?;
        // A handle of its own, not the playback one.
        let vfs = self.0.vfs(source_id).map_err(|e| e.to_string())?;
        vfs.open(&path).map(Arc::from).map_err(|e| e.to_string())
    }
}

/// Skip segments of one library.
#[frb(opaque)]
pub struct CoreSkip {
    db: PathBuf,
    opener: Arc<SourceOpener>,
    queue: Arc<Queue>,
}

impl CoreSkip {
    /// The service over `library` (its database and sources). The
    /// background thread starts with the first [`analyze`](Self::analyze).
    pub fn open(library: &CoreLibrary) -> Result<CoreSkip, SkipError> {
        Ok(Self {
            db: library.catalog().path().to_path_buf(),
            opener: library.opener(),
            queue: Arc::new(Queue::default()),
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
            })
            .collect())
    }

    /// Queues the season (or folder) of the file for analysis in the
    /// background and returns at once; `false` if the library does not
    /// know the file. Files analysed before are not read again.
    pub fn analyze(&self, source_id: i64, rel_path: String) -> Result<bool, SkipError> {
        let Some(file_id) = self.file_id(source_id, &rel_path)? else {
            return Ok(false);
        };
        let mut pending = self
            .queue
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !pending.contains(&file_id) {
            pending.push_back(file_id);
        }
        drop(pending);
        if self
            .queue
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.spawn_worker();
        }
        self.queue.wake.notify_one();
        Ok(true)
    }

    /// The background thread: one season at a time, until the service is
    /// dropped (the thread holds the only other reference to the queue).
    fn spawn_worker(&self) {
        let queue = Arc::clone(&self.queue);
        let db = self.db.clone();
        let files = Files(Arc::clone(&self.opener));
        let spawned = std::thread::Builder::new()
            .name("pa-skip".into())
            .spawn(move || {
                loop {
                    let next = {
                        let mut pending = queue.pending.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if let Some(id) = pending.pop_front() {
                                break Some(id);
                            }
                            if Arc::strong_count(&queue) == 1 {
                                break None;
                            }
                            pending = queue
                                .wake
                                .wait_timeout(pending, std::time::Duration::from_secs(5))
                                .map(|(g, _)| g)
                                .unwrap_or_else(|e| e.into_inner().0);
                        }
                    };
                    let Some(file_id) = next else {
                        queue.running.store(false, Ordering::Release);
                        return;
                    };
                    let Ok(ffmpeg) = pa_probe::Ffmpeg::shared() else {
                        continue;
                    };
                    let Ok(conn) = pa_library::open(&db) else {
                        continue;
                    };
                    let _ = pa_skip::analyze_season(
                        &conn,
                        ffmpeg,
                        &files,
                        file_id,
                        &pa_skip::AnalyzeOptions::default(),
                    );
                }
            });
        if spawned.is_err() {
            // No thread: the next call tries again.
            self.queue.running.store(false, Ordering::Release);
        }
    }
}
