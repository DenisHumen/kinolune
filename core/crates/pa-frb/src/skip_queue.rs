// SPDX-License-Identifier: MPL-2.0
//! The background thread of [`crate::api::skip::CoreSkip`] (PLAY-011):
//! seasons queued by file id, analysed one at a time; marker sidecars
//! (EDL, Plex, Jellyfin; PLAY-015) are imported before the analysis. Kept out of `api` so
//! flutter_rust_bridge does not see it.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use pa_catalog::SourceOpener;
use pa_vfs::VfsPath;

/// Opens library files for the job through the source manager.
struct Files(Arc<SourceOpener>);

impl pa_skip::Opener for Files {
    fn open(&self, source_id: i64, path: &str) -> Result<Arc<dyn pa_vfs::ByteSource>, String> {
        let path = VfsPath::parse(path).map_err(|e| e.to_string())?;
        // Through the opener first: it reconnects a stale source.
        self.0
            .open_file(source_id, &path)
            .map_err(|e| e.to_string())?;
        // A handle of its own, not the playback one.
        let vfs = self.0.vfs(source_id).map_err(|e| e.to_string())?;
        vfs.open(&path).map(Arc::from).map_err(|e| e.to_string())
    }

    fn list(&self, source_id: i64, dir: &str) -> Result<Vec<String>, String> {
        let dir = VfsPath::parse(if dir.is_empty() { "/" } else { dir }).map_err(|e| e.to_string())?;
        let vfs = self.0.vfs(source_id).map_err(|e| e.to_string())?;
        Ok(vfs
            .list(&dir)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|e| e.name)
            .collect())
    }
}

#[derive(Default)]
struct State {
    pending: Mutex<VecDeque<i64>>,
    wake: Condvar,
    /// Whether the background thread runs.
    running: AtomicBool,
}

/// Seasons waiting for analysis. The thread starts with the first one and
/// ends when the queue is dropped.
pub(crate) struct SkipQueue {
    db: PathBuf,
    opener: Arc<SourceOpener>,
    state: Arc<State>,
}

impl SkipQueue {
    pub(crate) fn new(db: PathBuf, opener: Arc<SourceOpener>) -> Self {
        Self {
            db,
            opener,
            state: Arc::new(State::default()),
        }
    }

    /// Queues the season of `file_id` (once).
    pub(crate) fn push(&self, file_id: i64) {
        {
            let mut pending = self.state.pending.lock().unwrap_or_else(|e| e.into_inner());
            if !pending.contains(&file_id) {
                pending.push_back(file_id);
            }
        }
        if self
            .state
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.spawn();
        }
        self.state.wake.notify_one();
    }

    fn spawn(&self) {
        let state = Arc::clone(&self.state);
        let db = self.db.clone();
        let files = Files(Arc::clone(&self.opener));
        let spawned = std::thread::Builder::new()
            .name("pa-skip".into())
            .spawn(move || {
                loop {
                    let next = {
                        let mut pending = state.pending.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if let Some(id) = pending.pop_front() {
                                break Some(id);
                            }
                            // Only this thread holds the state: the queue
                            // was dropped.
                            if Arc::strong_count(&state) == 1 {
                                break None;
                            }
                            pending = state
                                .wake
                                .wait_timeout(pending, Duration::from_secs(5))
                                .map(|(g, _)| g)
                                .unwrap_or_else(|e| e.into_inner().0);
                        }
                    };
                    let Some(file_id) = next else {
                        state.running.store(false, Ordering::Release);
                        return;
                    };
                    let Ok(conn) = pa_library::open(&db) else {
                        continue;
                    };
                    // Markers next to the files first (PLAY-015): cheap,
                    // and they win over what the analysis finds.
                    let _ = pa_skip::import_season(&conn, &files, file_id);
                    let Ok(ffmpeg) = pa_probe::Ffmpeg::shared() else {
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
            // No thread: the next push tries again.
            self.state.running.store(false, Ordering::Release);
        }
    }
}
