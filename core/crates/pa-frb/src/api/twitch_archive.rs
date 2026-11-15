// SPDX-License-Identifier: MPL-2.0
//! Twitch downloads and recordings (TWI-017, TWI-018, TWI-019; full
//! builds only) for Dart.
//!
//! [`CoreTwitchDownloads`] runs the `pa-jobs` queue of the library database
//! with the `twitch_vod_download` and `twitch_live_record` handlers of
//! `pa-twitch-archive`, behind the `pa-storage` gate of two storage targets:
//! `twitch` (kind `downloads`) and `twitch_live` (kind `recordings`). Jobs
//! survive an app restart and resume from their checkpoints.
//!
//! TWI-019 adds:
//!
//! - name templates of the saved files ([`CoreArchiveSettings`],
//!   `pa_twitch_archive::naming`), the folders and quotas of both targets;
//! - the household's record rules (`twitch_record_rules`) and
//!   [`follow_live`](CoreTwitchDownloads::follow_live): every `stream.online`
//!   (a TWI-009 timeline start or category change) of the running live
//!   monitor goes through `AutoRecord`, which queues a recording when a rule
//!   covers the channel;
//! - the saved files in the library (`twitch_assets`, [`CoreSavedFile`]):
//!   finished jobs are registered, folders of other tools imported
//!   ([`import_folder`](CoreTwitchDownloads::import_folder)).
//!
//! The API is the same in every flavor so the generated bindings are too.
//! Store builds have no downloader code (`cfg(pa_twitch_archive)`,
//! ADR-0041): [`CoreTwitchDownloads::open`] fails there with `store_build`
//! and the app never shows the actions (`AppFlavor.twitchArchive`).

use flutter_rust_bridge::frb;

use super::remote_config::CoreRemoteConfig;
use super::twitch::CoreTwitch;
use super::twitch_live::CoreTwitchLive;

/// Error of a downloads call; Dart receives it as an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDownloadError {
    /// `store_build`, `archive_disabled` (remote config), `native_off`,
    /// `signed_out`, `no_space` (can never fit), `duplicate` (saved
    /// already), `invalid_template`, `storage`, `jobs`, `library`,
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
    /// Queue even if the same VOD (or part) is saved or queued already.
    pub allow_duplicate: bool,
}

/// One download or recording in the list.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreTwitchDownload {
    pub job_id: i64,
    /// `vod` (a download) or `live` (a recording).
    pub kind: String,
    /// Downloads: the VOD.
    pub video_id: String,
    pub part_label: Option<String>,
    /// Recordings: the channel and the broadcast.
    pub user_id: Option<String>,
    pub login: Option<String>,
    pub stream_id: Option<String>,
    /// `queued`, `waiting_resources`, `running`, `paused`, `retrying`,
    /// `done`, `failed`, `cancelled`.
    pub state: String,
    /// 0.0–1.0 when known (downloads).
    pub progress: Option<f64>,
    pub bytes_done: i64,
    pub bytes_total: Option<i64>,
    /// Why it waits (`disk_reserve`, `quota`, …) or is paused (`user`,
    /// `network`, `resources`).
    pub waiting_for: Option<String>,
    /// Last error (`usher_403: …` for restricted VODs).
    pub error: Option<String>,
    /// Folder of the finished file inside its target.
    pub folder: Option<String>,
    /// File name of the video in that folder.
    pub file: Option<String>,
    /// Recordings: gaps so far (outage, pause, restart) and recorded media.
    pub gaps: u32,
    pub media_ms: i64,
    /// Recordings: the profile whose rule started it.
    pub profile_id: Option<i64>,
    pub created_at_ms: i64,
}

/// Folders, quotas and name templates of downloads and recordings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreArchiveSettings {
    pub downloads_dir: String,
    pub recordings_dir: String,
    /// At most this many bytes of saved files; `None` — no quota.
    pub downloads_quota_bytes: Option<u64>,
    pub recordings_quota_bytes: Option<u64>,
    /// Name templates (`pa_twitch_archive::naming`).
    pub folder_template: String,
    pub vod_template: String,
    pub live_template: String,
}

/// Space of a target now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreArchiveUsage {
    /// `downloads` or `recordings`.
    pub kind: String,
    pub path: String,
    pub used: u64,
    pub quota: Option<u64>,
    pub total: Option<u64>,
    pub available: Option<u64>,
    /// Free space is below the reserve (jobs wait or are paused).
    pub low: bool,
}

/// Automatic recording of a favourite channel for a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreRecordRule {
    pub user_id: String,
    pub enabled: bool,
    /// `best`, `1080p60`, `720p`, `480p`, `audio_only`.
    pub quality: String,
    pub with_chat: bool,
    /// Keep the last N recordings of the channel (default 10).
    pub keep_last_n: Option<u32>,
    /// Delete recordings older than this (default 14 days).
    pub retention_days: Option<u32>,
    /// Record only these categories (game ids); `None` — all.
    pub categories: Option<Vec<String>>,
}

/// A saved file in the library: a download, a recording or an import.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreSavedFile {
    pub asset_id: i64,
    /// `vod`, `recording`, `clip`.
    pub kind: String,
    pub user_id: String,
    pub login: Option<String>,
    pub display_name: Option<String>,
    pub video_id: Option<String>,
    pub stream_id: Option<String>,
    pub clip_slug: Option<String>,
    pub title: String,
    pub recorded_at_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub size_bytes: u64,
    /// Absolute path of the video file.
    pub path: String,
    /// `download`, `recording`, `import`.
    pub origin: String,
    pub has_chat: bool,
    pub chapters: u32,
    pub has_ads_marked: bool,
}

/// Result of an import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTwitchImportReport {
    pub added: u32,
    pub duplicates: u32,
    /// Registered before.
    pub known: u32,
    /// Files whose channel could not be found (yt-dlp, streamlink).
    pub no_user: u32,
    /// Video files without a Twitch id.
    pub skipped: u32,
    pub chapters: u32,
}

/// The download and recording queue of one library (full builds).
#[frb(opaque)]
pub struct CoreTwitchDownloads {
    #[cfg(all(pa_twitch_archive, pa_twitch_native))]
    inner: std::sync::Arc<imp::Downloads>,
}

macro_rules! full_only {
    ($full:expr, $store:expr) => {{
        #[cfg(all(pa_twitch_archive, pa_twitch_native))]
        {
            $full
        }
        #[cfg(not(all(pa_twitch_archive, pa_twitch_native)))]
        {
            $store
        }
    }};
}

fn store_build<T>() -> Result<T, CoreDownloadError> {
    Err(CoreDownloadError::new(
        "store_build",
        "Twitch downloads are not in this build",
    ))
}

/// Default name templates (06 §4.3): `(folder, vod, live)`.
#[frb(sync)]
pub fn archive_default_templates() -> Vec<String> {
    full_only!(
        vec![
            pa_twitch_archive::naming::DEFAULT_FOLDER.to_owned(),
            pa_twitch_archive::naming::DEFAULT_VOD.to_owned(),
            pa_twitch_archive::naming::DEFAULT_LIVE.to_owned(),
        ],
        Vec::new()
    )
}

/// Checks name templates and returns example paths `[vod, live]`; fails
/// with `invalid_template` (the message says what is wrong).
#[frb(sync)]
pub fn archive_name_preview(
    folder_template: String,
    vod_template: String,
    live_template: String,
) -> Result<Vec<String>, CoreDownloadError> {
    full_only!(
        {
            let naming = pa_twitch_archive::Naming {
                folder: folder_template,
                vod: vod_template,
                live: live_template,
            };
            naming
                .check()
                .map_err(|e| CoreDownloadError::new("invalid_template", e))?;
            let (vod, live) = naming.preview();
            Ok(vec![vod, live])
        },
        {
            let _ = (folder_template, vod_template, live_template);
            store_build()
        }
    )
}

impl CoreTwitchDownloads {
    /// Whether this build has the downloader at all.
    #[frb(sync)]
    pub fn available() -> bool {
        cfg!(all(pa_twitch_archive, pa_twitch_native))
    }

    /// Opens the queue of `twitch`'s library database and starts it.
    /// Downloads go to `downloads_dir`, recordings to `recordings_dir`
    /// unless the user chose other folders before (kept in the storage
    /// targets). Fails with `archive_disabled` or `native_off` when the
    /// remote config does not allow downloads, and `signed_out` without a
    /// Twitch sign-in.
    pub fn open(
        twitch: &CoreTwitch,
        config: &CoreRemoteConfig,
        downloads_dir: String,
        recordings_dir: String,
    ) -> Result<CoreTwitchDownloads, CoreDownloadError> {
        full_only!(
            Ok(Self {
                inner: imp::Downloads::open(twitch, config, downloads_dir, recordings_dir)?,
            }),
            {
                let _ = (twitch, config, downloads_dir, recordings_dir);
                store_build()
            }
        )
    }

    /// Queues a download; returns its job id. Fails with `duplicate` when
    /// the VOD (or this part) is saved or queued already, unless
    /// `allow_duplicate`.
    pub fn download(&self, request: CoreDownloadRequest) -> Result<i64, CoreDownloadError> {
        full_only!(self.inner.download(request), {
            let _ = request;
            store_build()
        })
    }

    /// Every download, newest first (finished ones are registered in the
    /// library on the way).
    pub fn list(&self) -> Result<Vec<CoreTwitchDownload>, CoreDownloadError> {
        full_only!(self.inner.list(false), Ok(Vec::new()))
    }

    /// Every recording, newest first.
    pub fn recordings(&self) -> Result<Vec<CoreTwitchDownload>, CoreDownloadError> {
        full_only!(self.inner.list(true), Ok(Vec::new()))
    }

    /// `pause`, `resume`, `cancel`, `retry` or `remove` (finished only) of
    /// a download or a recording. A recording that is cancelled while it
    /// runs is not finalised: `stop` instead ends it and keeps what was
    /// recorded.
    pub fn control(&self, job_id: i64, action: String) -> Result<(), CoreDownloadError> {
        full_only!(self.inner.control(job_id, &action), {
            let _ = (job_id, action);
            store_build()
        })
    }

    /// Starts recording a channel now (the profile's rule settings, if
    /// any). Returns the job id; an active recording of the channel is
    /// returned as is.
    pub fn record_now(
        &self,
        profile_id: i64,
        user_id: String,
        login: Option<String>,
    ) -> Result<i64, CoreDownloadError> {
        full_only!(self.inner.record_now(profile_id, &user_id, login), {
            let _ = (profile_id, user_id, login);
            store_build()
        })
    }

    /// Folders, quotas and name templates.
    pub fn settings(&self) -> Result<CoreArchiveSettings, CoreDownloadError> {
        full_only!(self.inner.settings(), store_build())
    }

    /// Changes folders, quotas and templates. Files saved before stay where
    /// they are. Fails with `invalid_template`.
    pub fn set_settings(&self, settings: CoreArchiveSettings) -> Result<(), CoreDownloadError> {
        full_only!(self.inner.set_settings(settings), {
            let _ = settings;
            store_build()
        })
    }

    /// Space of both targets.
    pub fn usage(&self) -> Result<Vec<CoreArchiveUsage>, CoreDownloadError> {
        full_only!(self.inner.usage(), Ok(Vec::new()))
    }

    /// Record rules of a profile.
    pub fn record_rules(&self, profile_id: i64) -> Result<Vec<CoreRecordRule>, CoreDownloadError> {
        full_only!(self.inner.record_rules(profile_id), {
            let _ = profile_id;
            Ok(Vec::new())
        })
    }

    /// Creates or replaces a rule of a profile and applies the retention of
    /// all rules to the recordings target.
    pub fn set_record_rule(
        &self,
        profile_id: i64,
        rule: CoreRecordRule,
    ) -> Result<(), CoreDownloadError> {
        full_only!(self.inner.set_record_rule(profile_id, rule), {
            let _ = (profile_id, rule);
            store_build()
        })
    }

    pub fn delete_record_rule(
        &self,
        profile_id: i64,
        user_id: String,
    ) -> Result<(), CoreDownloadError> {
        full_only!(self.inner.delete_record_rule(profile_id, &user_id), {
            let _ = (profile_id, user_id);
            store_build()
        })
    }

    /// Saved files: `kind` `vod`, `recording`, `clip` or `None` for all.
    pub fn saved(&self, kind: Option<String>) -> Result<Vec<CoreSavedFile>, CoreDownloadError> {
        full_only!(self.inner.saved(kind.as_deref()), {
            let _ = kind;
            Ok(Vec::new())
        })
    }

    /// Imports the saved Twitch files of a folder (TwitchDownloader,
    /// yt-dlp, streamlink, our own) into the library, in place.
    pub fn import_folder(&self, path: String) -> Result<CoreTwitchImportReport, CoreDownloadError> {
        full_only!(self.inner.import_folder(&path), {
            let _ = path;
            store_build()
        })
    }

    /// Removes a saved file from the library (the file stays on disk).
    pub fn forget(&self, asset_id: i64) -> Result<(), CoreDownloadError> {
        full_only!(self.inner.forget(asset_id), {
            let _ = asset_id;
            store_build()
        })
    }

    /// Starts recordings by the rules from the events of `live` (the
    /// running "X is live" monitor, TWI-009): a broadcast start or a
    /// category change of a channel with an enabled rule queues its
    /// recording.
    #[frb(sync)]
    pub fn follow_live(&self, live: &CoreTwitchLive) {
        full_only!(
            {
                let inner = std::sync::Arc::clone(&self.inner);
                live.add_observer(Box::new(move |event| inner.on_live_event(event)));
            },
            {
                let _ = live;
            }
        )
    }
}

#[cfg(all(pa_twitch_archive, pa_twitch_native))]
mod imp {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use pa_jobs::{Job, JobQueue, JobState};
    use pa_library::twitch::{self as lib, AssetKind, RecordRule};
    use pa_storage::{StorageManager, Target, TargetKind};
    use pa_twitch::chapters::{Chapter, ChapterGame, ChapterSource};
    use pa_twitch::playback::TwitchFeatures;
    use pa_twitch_archive::import::FoundKind;
    use pa_twitch_archive::library::{self as alib, UserLookup};
    use pa_twitch_archive::live::{
        AutoDecision, AutoRecord, LiveRecorder, LiveRequest, TWITCH_LIVE_RECORD, TwitchLiveSource,
        quality_of,
    };
    use pa_twitch_archive::{
        Container, DownloadRequest, FfmpegMuxer, Naming, Quality, TimeRange, TwitchSource,
        VodDownloader, kinds,
    };

    use super::{
        CoreArchiveSettings, CoreArchiveUsage, CoreDownloadError, CoreDownloadRequest,
        CoreTwitchImportReport, CoreRecordRule, CoreSavedFile, CoreTwitchDownload,
    };
    use crate::api::remote_config::CoreRemoteConfig;
    use crate::api::twitch::CoreTwitch;

    /// Storage target of Twitch downloads.
    pub(super) const TARGET: &str = "twitch";
    /// Storage target of live recordings.
    pub(super) const LIVE_TARGET: &str = "twitch_live";

    fn err(code: &str) -> impl Fn(String) -> CoreDownloadError + '_ {
        move |m| CoreDownloadError::new(code, m)
    }

    fn lib_err(e: impl std::fmt::Display) -> CoreDownloadError {
        CoreDownloadError::new("library", e)
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }

    pub(super) struct Downloads {
        db: String,
        queue: JobQueue,
        storage: StorageManager,
        source: Arc<TwitchSource>,
        auto: AutoRecord,
        naming: Mutex<Naming>,
    }

    /// Chapters stored for a VOD (TWI-008) in the library at `db`.
    fn stored_chapters(db: &str, video: &pa_twitch::Video) -> Vec<Chapter> {
        let Ok(conn) = pa_library::open(db) else {
            return Vec::new();
        };
        let owner = lib::SegmentOwner::Video(video.id.clone());
        let Ok(segments) = lib::segments(&conn, &owner) else {
            return Vec::new();
        };
        segments
            .into_iter()
            .filter_map(|s| {
                let name = lib::category(&conn, &s.game_id)
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

    /// The user's target if it exists, else a new one at `default_dir`.
    fn ensure_target(
        storage: &StorageManager,
        id: &str,
        kind: TargetKind,
        default_dir: &str,
    ) -> Result<(), CoreDownloadError> {
        if storage.target(id).is_ok() {
            return Ok(());
        }
        let target = Target::new(id, kind, PathBuf::from(default_dir)).reserve_percent(5.0);
        storage
            .set_target(&target)
            .map_err(|e| err("storage")(e.to_string()))
    }

    struct HelixLookup<'a>(&'a TwitchSource);

    impl UserLookup for HelixLookup<'_> {
        fn user_by_login(&self, login: &str) -> Option<(String, String)> {
            self.0.user_by_login(login).ok().flatten()
        }
    }

    impl Downloads {
        pub(super) fn open(
            twitch: &CoreTwitch,
            config: &CoreRemoteConfig,
            downloads_dir: String,
            recordings_dir: String,
        ) -> Result<Arc<Self>, CoreDownloadError> {
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
            let source = Arc::new(
                TwitchSource::new(features, helix, Arc::new(transport))
                    .map_err(|e| CoreDownloadError::new("io", e))?
                    .with_chapters(Box::new(move |v| stored_chapters(&db_for_chapters, v))),
            );
            pa_twitch_archive::ArchiveSource::check_allowed(source.as_ref())
                .map_err(|e| CoreDownloadError::new(e.code, e.message))?;

            // NET-019: one queue per database, shared with offline copies;
            // recordings (TWI-018) register their handler on it too.
            let crate::shared::Jobs { queue, storage } =
                crate::shared::jobs(Path::new(&db)).map_err(err("jobs"))?;
            ensure_target(&storage, TARGET, TargetKind::Downloads, &downloads_dir)?;
            ensure_target(&storage, LIVE_TARGET, TargetKind::Recordings, &recordings_dir)?;
            let _ = storage.clean_tmp(&queue);
            VodDownloader::new(
                Arc::clone(&source) as Arc<dyn pa_twitch_archive::ArchiveSource>,
                storage.clone(),
                Arc::new(FfmpegMuxer),
            )
            .register(&queue);

            // Recordings: chapters from the TWI-008 timeline the live
            // monitor stores, chat over anonymous IRC (TWI-010).
            let db_for_timeline = db.clone();
            let rt = source.runtime_handle();
            let live_source = Arc::new(
                TwitchLiveSource::new(Arc::clone(&source))
                    .with_timeline(Box::new(move |stream_id| {
                        let conn = pa_library::open(&db_for_timeline).ok()?;
                        pa_twitch::chapters::store::load_live_timeline(&conn, stream_id)
                            .ok()
                            .flatten()
                    }))
                    .with_chat(Box::new(move |stream| {
                        let _guard = rt.enter();
                        let login = stream.login.clone();
                        pa_twitch_chat::IrcChatSource::connect(pa_twitch_chat::IrcConfig::new([
                            login,
                        ]))
                        .ok()
                        .map(|s| Arc::new(s) as Arc<dyn pa_twitch_chat::ChatSource>)
                    })),
            );
            LiveRecorder::new(
                Arc::clone(&live_source) as Arc<dyn pa_twitch_archive::live::LiveSource>,
                storage.clone(),
                Arc::new(FfmpegMuxer),
            )
            .register(&queue);
            let auto = AutoRecord::new(live_source, storage.clone(), queue.clone(), LIVE_TARGET);

            let this = Arc::new(Self {
                db,
                queue,
                storage,
                source,
                auto,
                naming: Mutex::new(Naming::default()),
            });
            let _ = this.apply_retention();
            Ok(this)
        }

        fn conn(&self) -> Result<pa_library::rusqlite::Connection, CoreDownloadError> {
            pa_library::open(&self.db).map_err(lib_err)
        }

        fn naming(&self) -> Naming {
            self.naming.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }

        fn apply_retention(&self) -> Result<usize, CoreDownloadError> {
            let rules = lib::record_rules(&self.conn()?, None).map_err(lib_err)?;
            self.auto
                .apply_retention(&rules)
                .map_err(|e| err("storage")(e.to_string()))
        }

        pub(super) fn download(&self, r: CoreDownloadRequest) -> Result<i64, CoreDownloadError> {
            let mut request = DownloadRequest::vod(r.video_id.trim());
            if request.video_id.is_empty() || !request.video_id.bytes().all(|b| b.is_ascii_digit())
            {
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
            request.naming = Some(self.naming());
            if !r.allow_duplicate {
                self.check_duplicate(&request, r.duration_ms)?;
            }
            let job = request.to_job(TARGET, r.duration_ms.and_then(|d| u64::try_from(d).ok()));
            self.storage
                .enqueue(&self.queue, &job)
                .map_err(|e| match e {
                    pa_storage::Error::WontFit { .. } => CoreDownloadError::new("no_space", e),
                    other => CoreDownloadError::new("storage", other),
                })
        }

        /// 06 §4.3: before queueing, no saved file of the same VOD (same
        /// length) and no active job for the same VOD and range.
        fn check_duplicate(
            &self,
            request: &DownloadRequest,
            vod_ms: Option<i64>,
        ) -> Result<(), CoreDownloadError> {
            let active = self
                .queue
                .list(&[
                    JobState::Queued,
                    JobState::WaitingResources,
                    JobState::Running,
                    JobState::Paused,
                    JobState::Retrying,
                ])
                .map_err(|e| err("jobs")(e.to_string()))?;
            let range = serde_json::to_value(request.range).unwrap_or_default();
            if active.iter().any(|j| {
                j.kind == kinds::TWITCH_VOD_DOWNLOAD
                    && j.params["video_id"] == request.video_id.as_str()
                    && j.params.get("range").cloned().unwrap_or_default() == range
            }) {
                return Err(CoreDownloadError::new(
                    "duplicate",
                    "this VOD is being downloaded already",
                ));
            }
            self.register_finished();
            let length = match request.range {
                Some(r) => Some(r.end_ms.saturating_sub(r.start_ms)),
                None => vod_ms.and_then(|d| u64::try_from(d).ok()),
            };
            let saved = alib::saved_asset(&self.conn()?, FoundKind::Vod, &request.video_id, length)
                .map_err(lib_err)?;
            if saved.is_some() {
                return Err(CoreDownloadError::new(
                    "duplicate",
                    "this VOD is saved already",
                ));
            }
            Ok(())
        }

        /// Registers finished downloads and recordings as saved files (once:
        /// the path is the key).
        fn register_finished(&self) {
            let Ok(jobs) = self.queue.list(&[JobState::Done]) else {
                return;
            };
            let Ok(mut conn) = self.conn() else { return };
            for j in jobs {
                let (target, origin) = match j.kind.as_str() {
                    k if k == kinds::TWITCH_VOD_DOWNLOAD => (TARGET, "download"),
                    k if k == TWITCH_LIVE_RECORD => (LIVE_TARGET, "recording"),
                    _ => continue,
                };
                let Some(r) = j.result.as_ref() else { continue };
                let Some(rel) = r["rel_path"].as_str() else {
                    continue;
                };
                let Ok(t) = self.storage.target(target) else {
                    continue;
                };
                let folder = rel.split('/').fold(t.path.clone(), |p, c| p.join(c));
                if !folder.is_dir() {
                    continue;
                }
                let Ok(source) = alib::ensure_source(&conn, &t.path, "Twitch", now_ms()) else {
                    continue;
                };
                let (found, _) = pa_twitch_archive::import::scan(&folder);
                for f in &found {
                    let _ = alib::register(&mut conn, source, &t.path, f, origin, None, now_ms());
                }
            }
        }

        fn row(j: Job) -> CoreTwitchDownload {
            let p = &j.params;
            let r = j.result.as_ref();
            let cp = j.checkpoint.as_ref();
            let live = j.kind == TWITCH_LIVE_RECORD;
            let s = |v: &serde_json::Value| v.as_str().map(str::to_owned);
            CoreTwitchDownload {
                job_id: j.id,
                kind: if live { "live" } else { "vod" }.to_owned(),
                video_id: p["video_id"].as_str().unwrap_or("").to_owned(),
                part_label: s(&p["part_label"]),
                user_id: s(&p["user_id"]),
                login: s(&p["login"]),
                stream_id: s(&p["stream_id"]).or_else(|| r.and_then(|r| s(&r["stream_id"]))),
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
                folder: r.and_then(|r| s(&r["rel_path"])),
                file: r.and_then(|r| s(&r["file"])),
                gaps: r
                    .and_then(|r| r["gaps"].as_u64())
                    .or_else(|| cp.and_then(|c| c["gaps"].as_u64()))
                    .map_or(0, |g| u32::try_from(g).unwrap_or(u32::MAX)),
                media_ms: r
                    .and_then(|r| r["duration_ms"].as_i64())
                    .or_else(|| cp.and_then(|c| c["media_ms"].as_i64()))
                    .unwrap_or(0),
                profile_id: j.owner_profile_id,
                created_at_ms: j.created_at_ms,
            }
        }

        pub(super) fn list(&self, live: bool) -> Result<Vec<CoreTwitchDownload>, CoreDownloadError> {
            self.register_finished();
            let kind = if live {
                TWITCH_LIVE_RECORD
            } else {
                kinds::TWITCH_VOD_DOWNLOAD
            };
            let mut jobs = self
                .queue
                .list(&[])
                .map_err(|e| err("jobs")(e.to_string()))?;
            jobs.retain(|j| j.kind == kind);
            jobs.sort_by_key(|j| std::cmp::Reverse(j.created_at_ms));
            Ok(jobs.into_iter().map(Self::row).collect())
        }

        pub(super) fn control(&self, id: i64, action: &str) -> Result<(), CoreDownloadError> {
            let e = |e: pa_jobs::Error| CoreDownloadError::new("jobs", e);
            let job = self.queue.get(id).map_err(e)?;
            if job.kind != kinds::TWITCH_VOD_DOWNLOAD && job.kind != TWITCH_LIVE_RECORD {
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

        pub(super) fn record_now(
            &self,
            profile_id: i64,
            user_id: &str,
            login: Option<String>,
        ) -> Result<i64, CoreDownloadError> {
            if let Some(id) = self
                .auto
                .active(user_id)
                .map_err(|e| err("jobs")(e.to_string()))?
            {
                return Ok(id);
            }
            pa_twitch_archive::live::LiveSource::check_allowed(
                &TwitchLiveSource::new(Arc::clone(&self.source)),
            )
            .map_err(|e| CoreDownloadError::new(e.code, e.message))?;
            let rule = lib::record_rules(&self.conn()?, Some(profile_id))
                .map_err(lib_err)?
                .into_iter()
                .find(|r| r.user_id == user_id);
            let mut req = LiveRequest::new(user_id);
            req.login = login.filter(|l| !l.is_empty());
            if let Some(rule) = &rule {
                req.quality = quality_of(&rule.quality);
                req.with_chat = rule.with_chat;
            }
            req.naming = Some(self.naming());
            self.storage
                .enqueue(&self.queue, &req.to_job(LIVE_TARGET).owner(profile_id))
                .map_err(|e| match e {
                    pa_storage::Error::WontFit { .. } => CoreDownloadError::new("no_space", e),
                    other => CoreDownloadError::new("storage", other),
                })
        }

        pub(super) fn settings(&self) -> Result<CoreArchiveSettings, CoreDownloadError> {
            let t = |id| {
                self.storage
                    .target(id)
                    .map_err(|e| err("storage")(e.to_string()))
            };
            let (d, l) = (t(TARGET)?, t(LIVE_TARGET)?);
            let n = self.naming();
            Ok(CoreArchiveSettings {
                downloads_dir: d.path.to_string_lossy().into_owned(),
                recordings_dir: l.path.to_string_lossy().into_owned(),
                downloads_quota_bytes: d.quota_bytes,
                recordings_quota_bytes: l.quota_bytes,
                folder_template: n.folder,
                vod_template: n.vod,
                live_template: n.live,
            })
        }

        pub(super) fn set_settings(&self, s: CoreArchiveSettings) -> Result<(), CoreDownloadError> {
            let naming = Naming {
                folder: s.folder_template,
                vod: s.vod_template,
                live: s.live_template,
            };
            naming
                .check()
                .map_err(|e| CoreDownloadError::new("invalid_template", e))?;
            for (id, dir, quota) in [
                (TARGET, &s.downloads_dir, s.downloads_quota_bytes),
                (LIVE_TARGET, &s.recordings_dir, s.recordings_quota_bytes),
            ] {
                let mut t = self
                    .storage
                    .target(id)
                    .map_err(|e| err("storage")(e.to_string()))?;
                let dir = dir.trim();
                if dir.is_empty() || !Path::new(dir).is_absolute() {
                    return Err(CoreDownloadError::new("invalid", "the folder must be absolute"));
                }
                t.path = PathBuf::from(dir);
                t.quota_bytes = quota.filter(|q| *q > 0);
                self.storage
                    .set_target(&t)
                    .map_err(|e| err("storage")(e.to_string()))?;
            }
            *self.naming.lock().unwrap_or_else(|e| e.into_inner()) = naming;
            Ok(())
        }

        pub(super) fn usage(&self) -> Result<Vec<CoreArchiveUsage>, CoreDownloadError> {
            let usage = self
                .storage
                .usage()
                .map_err(|e| err("storage")(e.to_string()))?;
            Ok(usage
                .into_iter()
                .filter_map(|u| {
                    let kind = match u.target_id.as_str() {
                        TARGET => "downloads",
                        LIVE_TARGET => "recordings",
                        _ => return None,
                    };
                    let path = self.storage.target(&u.target_id).ok()?.path;
                    Some(CoreArchiveUsage {
                        kind: kind.to_owned(),
                        path: path.to_string_lossy().into_owned(),
                        used: u.used,
                        quota: u.quota,
                        total: u.total,
                        available: u.available,
                        low: u.below_reserve(),
                    })
                })
                .collect())
        }

        pub(super) fn record_rules(
            &self,
            profile_id: i64,
        ) -> Result<Vec<CoreRecordRule>, CoreDownloadError> {
            Ok(lib::record_rules(&self.conn()?, Some(profile_id))
                .map_err(lib_err)?
                .into_iter()
                .map(|r| CoreRecordRule {
                    user_id: r.user_id,
                    enabled: r.enabled,
                    quality: r.quality,
                    with_chat: r.with_chat,
                    keep_last_n: r.keep_last_n,
                    retention_days: r.retention_days,
                    categories: r.categories,
                })
                .collect())
        }

        pub(super) fn set_record_rule(
            &self,
            profile_id: i64,
            r: CoreRecordRule,
        ) -> Result<(), CoreDownloadError> {
            let rule = RecordRule {
                profile_id,
                user_id: r.user_id,
                quality: if r.quality.trim().is_empty() {
                    "best".to_owned()
                } else {
                    r.quality
                },
                with_chat: r.with_chat,
                retention_days: r.retention_days.filter(|d| *d > 0),
                keep_last_n: r.keep_last_n.filter(|n| *n > 0),
                categories: r.categories.filter(|c| !c.is_empty()),
                replace_with_vod: false,
                enabled: r.enabled,
            };
            lib::upsert_record_rule(&self.conn()?, &rule, now_ms()).map_err(lib_err)?;
            self.apply_retention()?;
            Ok(())
        }

        pub(super) fn delete_record_rule(
            &self,
            profile_id: i64,
            user_id: &str,
        ) -> Result<(), CoreDownloadError> {
            lib::delete_record_rule(&self.conn()?, profile_id, user_id).map_err(lib_err)?;
            self.apply_retention()?;
            Ok(())
        }

        pub(super) fn saved(&self, kind: Option<&str>) -> Result<Vec<CoreSavedFile>, CoreDownloadError> {
            self.register_finished();
            let kind = match kind {
                None | Some("") => None,
                Some("vod") => Some(AssetKind::Vod),
                Some("recording") => Some(AssetKind::Recording),
                Some("clip") => Some(AssetKind::Clip),
                Some(_) => return Err(CoreDownloadError::new("invalid", "unknown kind")),
            };
            Ok(alib::entries(&self.conn()?, kind)
                .map_err(lib_err)?
                .into_iter()
                .map(|e| CoreSavedFile {
                    asset_id: e.asset_id,
                    kind: e.kind.as_str().to_owned(),
                    user_id: e.user_id,
                    login: e.login,
                    display_name: e.display_name,
                    video_id: e.video_id,
                    stream_id: e.stream_id,
                    clip_slug: e.clip_slug,
                    title: e.title,
                    recorded_at_ms: e.recorded_at_ms,
                    duration_ms: e.duration_ms.and_then(|d| i64::try_from(d).ok()),
                    size_bytes: e.size_bytes,
                    path: e.path.to_string_lossy().into_owned(),
                    origin: e.origin,
                    has_chat: e.has_chat,
                    chapters: u32::try_from(e.chapters).unwrap_or(u32::MAX),
                    has_ads_marked: e.has_ads_marked,
                })
                .collect())
        }

        pub(super) fn import_folder(&self, path: &str) -> Result<CoreTwitchImportReport, CoreDownloadError> {
            let root = PathBuf::from(path.trim());
            let name = root
                .file_name()
                .map_or_else(|| "Twitch".to_owned(), |n| n.to_string_lossy().into_owned());
            let mut conn = self.conn()?;
            let r = alib::import_dir(
                &mut conn,
                &root,
                &name,
                "import",
                Some(&HelixLookup(&self.source)),
                now_ms(),
            )
            .map_err(|e| match e {
                alib::LibraryError::Invalid(m) => CoreDownloadError::new("invalid", m),
                other => lib_err(other),
            })?;
            let n = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
            Ok(CoreTwitchImportReport {
                added: n(r.added.len()),
                duplicates: n(r.duplicates.len()),
                known: n(r.known),
                no_user: n(r.no_user.len()),
                skipped: n(r.skipped.len()),
                chapters: n(r.chapters),
            })
        }

        pub(super) fn forget(&self, asset_id: i64) -> Result<(), CoreDownloadError> {
            alib::forget(&self.conn()?, asset_id).map_err(lib_err)?;
            Ok(())
        }

        /// A live monitor event: `AutoRecord` with the household's rules.
        pub(super) fn on_live_event(&self, event: &pa_twitch_live::MonitorEvent) {
            let pa_twitch_live::MonitorEvent::Timeline(change) = event else {
                return;
            };
            let Ok(conn) = self.conn() else { return };
            let Ok(rules) = lib::record_rules(&conn, None) else {
                return;
            };
            let user_id = &change.timeline().user_id;
            if !rules.iter().any(|r| r.enabled && &r.user_id == user_id) {
                return;
            }
            let login = lib::streamer(&conn, user_id)
                .ok()
                .flatten()
                .map(|s| s.login);
            match self.auto.on_change_with(change, &rules, login.as_deref(), &self.naming()) {
                Ok(AutoDecision::Started(id)) => {
                    eprintln!("twitch: recording {user_id} as job {id}");
                }
                Ok(AutoDecision::NotAllowed(code)) => {
                    eprintln!("twitch: not recording {user_id}: {code}");
                }
                Ok(_) => {}
                Err(e) => eprintln!("twitch: auto-record {user_id}: {e}"),
            }
        }
    }
}
