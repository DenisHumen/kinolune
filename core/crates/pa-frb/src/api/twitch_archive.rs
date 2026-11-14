// SPDX-License-Identifier: MPL-2.0
//! Twitch VOD downloads (TWI-017, full builds only) for Dart: the
//! minimal "Download" action and the downloads list.
//!
//! [`CoreTwitchDownloads`] runs the `pa-jobs` queue of the library database
//! with the `twitch_vod_download` handler of `pa-twitch-archive`, behind the
//! `pa-storage` gate of one storage target (`twitch`, kind `downloads`,
//! the folder the app passes). Downloads survive an app restart and resume
//! from their segment bitmap.
//!
//! The API is the same in every flavor so the generated bindings are too.
//! Store builds have no downloader code (`cfg(pa_twitch_archive)`,
//! ADR-0041): [`CoreTwitchDownloads::open`] fails there with `store_build`
//! and the app never shows the action (`AppFlavor.twitchArchive`).

use flutter_rust_bridge::frb;

use super::remote_config::CoreRemoteConfig;
use super::twitch::CoreTwitch;

/// Error of a downloads call; Dart receives it as an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDownloadError {
    /// `store_build`, `archive_disabled` (remote config), `native_off`,
    /// `signed_out`, `no_space` (can never fit), `storage`, `jobs`,
    /// `invalid`.
    pub code: String,
    pub message: String,
}

impl CoreDownloadError {
    fn new(code: &str, message: impl ToString) -> Self {
        Self {
            code: code.into(),
            message: message.to_string(),
        }
    }
}

/// What to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDownloadRequest {
    pub video_id: String,
    /// `source` (default), `audio_only` or a maximum height such as `720`.
    pub quality: String,
    /// `mkv` (default) or `mp4`.
    pub container: String,
    /// Only a part (a chapter), ms from the VOD start.
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    /// Name of the part for the file name (the chapter's category).
    pub part_label: Option<String>,
    /// VOD length from Helix, ms: sizes the free-space estimate.
    pub duration_ms: Option<i64>,
}

/// One download in the list.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreTwitchDownload {
    pub job_id: i64,
    pub video_id: String,
    pub part_label: Option<String>,
    /// `queued`, `waiting_resources`, `running`, `paused`, `retrying`,
    /// `done`, `failed`, `cancelled`.
    pub state: String,
    /// 0.0–1.0 when known.
    pub progress: Option<f64>,
    pub bytes_done: i64,
    pub bytes_total: Option<i64>,
    /// Why it waits (`disk_reserve`, `quota`, …) or is paused (`user`,
    /// `network`, `resources`).
    pub waiting_for: Option<String>,
    /// Last error (`usher_403: …` for restricted VODs).
    pub error: Option<String>,
    /// Folder of the finished download inside the target.
    pub folder: Option<String>,
    /// File name of the video in that folder.
    pub file: Option<String>,
    pub created_at_ms: i64,
}

/// The download queue of one library (full builds).
#[frb(opaque)]
pub struct CoreTwitchDownloads {
    #[cfg(all(pa_twitch_archive, pa_twitch_native))]
    inner: imp::Downloads,
}

impl CoreTwitchDownloads {
    /// Whether this build has the downloader at all.
    #[frb(sync)]
    pub fn available() -> bool {
        cfg!(all(pa_twitch_archive, pa_twitch_native))
    }

    /// Opens the queue of `twitch`'s library database and starts it.
    /// Downloads go to `downloads_dir` (created if needed). Fails with
    /// `archive_disabled` or `native_off` when the remote config does not
    /// allow downloads, and `signed_out` without a Twitch sign-in.
    pub fn open(
        twitch: &CoreTwitch,
        config: &CoreRemoteConfig,
        downloads_dir: String,
    ) -> Result<CoreTwitchDownloads, CoreDownloadError> {
        #[cfg(all(pa_twitch_archive, pa_twitch_native))]
        {
            Ok(Self {
                inner: imp::Downloads::open(twitch, config, downloads_dir)?,
            })
        }
        #[cfg(not(all(pa_twitch_archive, pa_twitch_native)))]
        {
            let _ = (twitch, config, downloads_dir);
            Err(CoreDownloadError::new(
                "store_build",
                "Twitch downloads are not in this build",
            ))
        }
    }

    /// Queues a download; returns its job id.
    pub fn download(&self, request: CoreDownloadRequest) -> Result<i64, CoreDownloadError> {
        #[cfg(all(pa_twitch_archive, pa_twitch_native))]
        {
            self.inner.download(request)
        }
        #[cfg(not(all(pa_twitch_archive, pa_twitch_native)))]
        {
            let _ = request;
            Err(CoreDownloadError::new("store_build", "no downloads"))
        }
    }

    /// Every download, newest first.
    pub fn list(&self) -> Result<Vec<CoreTwitchDownload>, CoreDownloadError> {
        #[cfg(all(pa_twitch_archive, pa_twitch_native))]
        {
            self.inner.list()
        }
        #[cfg(not(all(pa_twitch_archive, pa_twitch_native)))]
        {
            Ok(Vec::new())
        }
    }

    /// `pause`, `resume`, `cancel`, `retry` or `remove` (finished only).
    pub fn control(&self, job_id: i64, action: String) -> Result<(), CoreDownloadError> {
        #[cfg(all(pa_twitch_archive, pa_twitch_native))]
        {
            self.inner.control(job_id, &action)
        }
        #[cfg(not(all(pa_twitch_archive, pa_twitch_native)))]
        {
            let _ = (job_id, action);
            Err(CoreDownloadError::new("store_build", "no downloads"))
        }
    }
}

#[cfg(all(pa_twitch_archive, pa_twitch_native))]
mod imp {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use pa_jobs::{JobQueue, JobState, QueueConfig, QueueDriver};
    use pa_storage::{MonitorDriver, StorageConfig, StorageManager, Target, TargetKind};
    use pa_twitch::chapters::{Chapter, ChapterGame, ChapterSource};
    use pa_twitch::playback::TwitchFeatures;
    use pa_twitch_archive::{
        Container, DownloadRequest, FfmpegMuxer, Quality, TimeRange, TwitchSource, VodDownloader,
        kinds,
    };

    use super::{CoreDownloadError, CoreDownloadRequest, CoreTwitchDownload};
    use crate::api::remote_config::CoreRemoteConfig;
    use crate::api::twitch::CoreTwitch;

    /// Storage target of Twitch downloads.
    pub(super) const TARGET: &str = "twitch";

    fn err(code: &str) -> impl Fn(String) -> CoreDownloadError + '_ {
        move |m| CoreDownloadError::new(code, m)
    }

    pub(super) struct Downloads {
        queue: JobQueue,
        storage: StorageManager,
        // Stop the scheduler and the monitor when the object is dropped.
        _driver: QueueDriver,
        _monitor: MonitorDriver,
    }

    /// Chapters stored for a VOD (TWI-008) in the library at `db`.
    fn stored_chapters(db: &str, video: &pa_twitch::Video) -> Vec<Chapter> {
        let Ok(conn) = pa_library::open(db) else {
            return Vec::new();
        };
        let owner = pa_library::twitch::SegmentOwner::Video(video.id.clone());
        let Ok(segments) = pa_library::twitch::segments(&conn, &owner) else {
            return Vec::new();
        };
        segments
            .into_iter()
            .filter_map(|s| {
                let name = pa_library::twitch::category(&conn, &s.game_id)
                    .ok()
                    .flatten()
                    .map(|c| c.name)
                    .unwrap_or_default();
                Some(Chapter {
                    game: ChapterGame::new(&s.game_id, &name)?,
                    title: s.title_at_time,
                    start_ms: s.start_ms,
                    end_ms: s.end_ms,
                    source: ChapterSource::parse(&s.source).unwrap_or(ChapterSource::Fallback),
                })
            })
            .collect()
    }

    impl Downloads {
        pub(super) fn open(
            twitch: &CoreTwitch,
            config: &CoreRemoteConfig,
            downloads_dir: String,
        ) -> Result<Self, CoreDownloadError> {
            let (cfg, ctx) = config.config_and_context();
            let features = TwitchFeatures::resolve(&cfg, &ctx);
            if !features.archive {
                return Err(CoreDownloadError::new(
                    "archive_disabled",
                    "the remote config does not allow Twitch downloads",
                ));
            }
            let (helix, db) = twitch
                .archive_access()
                .map_err(|e| CoreDownloadError::new("signed_out", e.message))?;
            let transport = pa_twitch::ReqwestTransport::new(Duration::from_secs(30))
                .map_err(|e| CoreDownloadError::new("io", e.0))?;
            let db_for_chapters = db.clone();
            let source = TwitchSource::new(features, helix, Arc::new(transport))
                .map_err(|e| CoreDownloadError::new("io", e))?
                .with_chapters(Box::new(move |v| stored_chapters(&db_for_chapters, v)));
            pa_twitch_archive::ArchiveSource::check_allowed(&source)
                .map_err(|e| CoreDownloadError::new(e.code, e.message))?;

            let queue = JobQueue::open(&db, QueueConfig::default())
                .map_err(|e| err("jobs")(e.to_string()))?;
            let storage = StorageManager::open(&db, StorageConfig::default())
                .map_err(|e| err("storage")(e.to_string()))?;
            let target = Target::new(TARGET, TargetKind::Downloads, PathBuf::from(downloads_dir))
                .reserve_percent(5.0);
            storage
                .set_target(&target)
                .map_err(|e| err("storage")(e.to_string()))?;
            storage.attach(&queue);
            let _ = storage.clean_tmp(&queue);
            VodDownloader::new(Arc::new(source), storage.clone(), Arc::new(FfmpegMuxer))
                .register(&queue);
            let driver = queue.start();
            let monitor = storage.start_monitor(queue.clone(), Duration::from_secs(30), |_| {});
            Ok(Self {
                queue,
                storage,
                _driver: driver,
                _monitor: monitor,
            })
        }

        pub(super) fn download(&self, r: CoreDownloadRequest) -> Result<i64, CoreDownloadError> {
            let mut request = DownloadRequest::vod(r.video_id.trim());
            if request.video_id.is_empty() || !request.video_id.bytes().all(|b| b.is_ascii_digit()) {
                return Err(CoreDownloadError::new("invalid", "bad video id"));
            }
            request.quality = match r.quality.as_str() {
                "" | "source" => Quality::Source,
                "audio_only" => Quality::AudioOnly,
                h => Quality::MaxHeight(
                    h.trim_end_matches('p')
                        .parse()
                        .map_err(|_| CoreDownloadError::new("invalid", "bad quality"))?,
                ),
            };
            request.container = match r.container.as_str() {
                "" | "mkv" => Container::Mkv,
                "mp4" => Container::Mp4,
                _ => return Err(CoreDownloadError::new("invalid", "bad container")),
            };
            request.range = match (r.start_ms, r.end_ms) {
                (Some(s), Some(e)) if s >= 0 && e > s => Some(TimeRange {
                    start_ms: s as u64,
                    end_ms: e as u64,
                }),
                (None, None) => None,
                _ => return Err(CoreDownloadError::new("invalid", "bad range")),
            };
            request.part_label = r.part_label.filter(|l| !l.trim().is_empty());
            let job = request.to_job(TARGET, r.duration_ms.and_then(|d| u64::try_from(d).ok()));
            self.storage
                .enqueue(&self.queue, &job)
                .map_err(|e| match e {
                    pa_storage::Error::WontFit { .. } => CoreDownloadError::new("no_space", e),
                    other => CoreDownloadError::new("storage", other),
                })
        }

        pub(super) fn list(&self) -> Result<Vec<CoreTwitchDownload>, CoreDownloadError> {
            let mut jobs = self
                .queue
                .list(&[])
                .map_err(|e| err("jobs")(e.to_string()))?;
            jobs.retain(|j| j.kind == kinds::TWITCH_VOD_DOWNLOAD);
            jobs.sort_by_key(|j| std::cmp::Reverse(j.created_at_ms));
            Ok(jobs
                .into_iter()
                .map(|j| {
                    let p = &j.params;
                    let r = j.result.as_ref();
                    CoreTwitchDownload {
                        job_id: j.id,
                        video_id: p["video_id"].as_str().unwrap_or("").to_owned(),
                        part_label: p["part_label"].as_str().map(str::to_owned),
                        state: j.state.to_string(),
                        progress: j.progress,
                        bytes_done: i64::try_from(j.bytes_done).unwrap_or(i64::MAX),
                        bytes_total: j.bytes_total.map(|b| i64::try_from(b).unwrap_or(i64::MAX)),
                        waiting_for: j
                            .wait_reason
                            .clone()
                            .or_else(|| j.pause_reason.map(|r| r.to_string())),
                        error: (j.state == JobState::Failed || j.state == JobState::Retrying)
                            .then(|| j.error.clone())
                            .flatten(),
                        folder: r.and_then(|r| r["rel_path"].as_str()).map(str::to_owned),
                        file: r.and_then(|r| r["file"].as_str()).map(str::to_owned),
                        created_at_ms: j.created_at_ms,
                    }
                })
                .collect())
        }

        pub(super) fn control(&self, id: i64, action: &str) -> Result<(), CoreDownloadError> {
            let e = |e: pa_jobs::Error| CoreDownloadError::new("jobs", e);
            let job = self.queue.get(id).map_err(e)?;
            if job.kind != kinds::TWITCH_VOD_DOWNLOAD {
                return Err(CoreDownloadError::new("invalid", "not a download"));
            }
            match action {
                "pause" => self.queue.pause(id).map(|_| ()),
                "resume" => self.queue.resume(id).map(|_| ()),
                "cancel" => self.queue.cancel(id).map(|_| ()),
                "retry" => self.queue.retry(id).map(|_| ()),
                "remove" => self.queue.remove(id),
                _ => return Err(CoreDownloadError::new("invalid", "unknown action")),
            }
            .map_err(e)
        }
    }
}
