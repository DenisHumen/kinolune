// SPDX-License-Identifier: MPL-2.0
//! Manual check of the downloader on real Twitch (TWI-014 checks,
//! `docs/dev/twitch-live-checks.md`). Never run in CI: `#[ignore]` and a
//! no-op without `PA_TWITCH_LIVE_TEST=1`.
//!
//! A ~5-minute range of a public VOD at a modest quality is downloaded by
//! the real `VodDownloader` (usher, segments, checkpoint, remux through our
//! LGPL FFmpeg from `PA_FFMPEG_LIB_DIR`), the process is killed with SIGKILL
//! in the middle of the segments, the job resumes, and the MKV is probed:
//! duration, chapter, tags. Everything lives in a temporary directory that
//! is deleted at the end.
//!
//! There are no Helix credentials: the VOD's description (what
//! `ArchiveSource::video` reads from Helix) comes from GQL `VideoMetadata`
//! instead, and there is no avatar. GQL client-id and hashes come only from
//! the environment, as in `pa-twitch/tests/real_twitch.rs`:
//! `PA_TWITCH_GQL_CLIENT_ID`, `PA_TWITCH_HASH_PLAYBACK_ACCESS_TOKEN`,
//! `PA_TWITCH_HASH_CHAPTERS`, `PA_TWITCH_HASH_VIDEO_METADATA`,
//! `PA_TWITCH_VOD_ID`; optional `PA_TWITCH_RANGE_START_MS` (default: the
//! middle of the VOD), `PA_TWITCH_RANGE_MS` (300000) and
//! `PA_TWITCH_MAX_HEIGHT` (360).

#![cfg(all(pa_twitch_archive, pa_twitch_native))]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use pa_jobs::{JobQueue, JobState, JobStore, QueueConfig};
use pa_remote_config::{ClientContext, RemoteConfig};
use pa_storage::{StorageConfig, StorageManager, Target, TargetKind};
use pa_twitch::chapters::{Chapter, ChapterResolver, ChaptersGql, VodChaptersRequest};
use pa_twitch::playback::{Flavor, MasterPlaylist, TwitchFeatures};
use pa_twitch::testing::FixtureTransport;
use pa_twitch::{
    AccessToken, ClientId, HelixClient, HelixConfig, HttpRequest, HttpResponse, Method,
    ReqwestTransport, Transport, Video,
};
use pa_twitch_archive::{
    ArchiveError, ArchiveSource, Container, DownloadRequest, FfmpegMuxer, HelixAccess, Muxer,
    Quality, SegmentBitmap, TimeRange, TwitchSource, VodDownloader,
};

const CHILD: &str = "PA_TWITCH_REAL_CHILD_DIR";

fn enabled() -> bool {
    std::env::var("PA_TWITCH_LIVE_TEST").as_deref() == Ok("1")
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

fn ctx() -> ClientContext {
    ClientContext {
        app_version: "0.1.0".into(),
        platform: "linux".into(),
    }
}

/// Unsigned test config: GQL, chapters and archive on, values from env.
fn config() -> RemoteConfig {
    let json = serde_json::json!({
        "schema": 1,
        "serial": 1,
        "flags": {
            "twitch.gql_playback": true,
            "twitch.chapters_gql": true,
            "twitch.chat_replay": false,
            "twitch.archive": true,
        },
        "kill_switches": [],
        "twitch": {
            "gql_client_id": env("PA_TWITCH_GQL_CLIENT_ID"),
            "persisted_queries": {
                "PlaybackAccessToken": env("PA_TWITCH_HASH_PLAYBACK_ACCESS_TOKEN"),
                "VideoPlayer_ChapterSelectButtonVideo": env("PA_TWITCH_HASH_CHAPTERS"),
                "VideoMetadata": env("PA_TWITCH_HASH_VIDEO_METADATA"),
            },
            "player": "native",
        },
    });
    RemoteConfig::parse(json.to_string().as_bytes()).unwrap()
}

fn transport() -> Arc<ReqwestTransport> {
    Arc::new(ReqwestTransport::new(Duration::from_secs(30)).unwrap())
}

/// The VOD as Helix would describe it, from GQL `VideoMetadata`.
fn video_from_metadata(rt: &tokio::runtime::Runtime) -> Video {
    let body = serde_json::json!({
        "operationName": "VideoMetadata",
        "variables": { "channelLogin": "", "videoID": env("PA_TWITCH_VOD_ID") },
        "extensions": { "persistedQuery": {
            "version": 1, "sha256Hash": env("PA_TWITCH_HASH_VIDEO_METADATA") } },
    });
    let request = HttpRequest {
        method: Method::Post,
        url: "https://gql.twitch.tv/gql".to_owned(),
        headers: vec![
            ("Client-ID".to_owned(), env("PA_TWITCH_GQL_CLIENT_ID")),
            ("Content-Type".to_owned(), "application/json".to_owned()),
        ],
        body: Some(body.to_string().into_bytes()),
    };
    let bytes = rt.block_on(transport().send(request)).unwrap().body;
    let answer: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let v = &answer["data"]["video"];
    let secs = v["lengthSeconds"].as_u64().unwrap();
    serde_json::from_value(serde_json::json!({
        "id": env("PA_TWITCH_VOD_ID"),
        "user_id": v["owner"]["id"],
        "user_login": v["owner"]["login"],
        "user_name": v["owner"]["displayName"],
        "title": v["title"],
        "created_at": v["createdAt"],
        "published_at": v["publishedAt"],
        "url": format!("https://www.twitch.tv/videos/{}", env("PA_TWITCH_VOD_ID")),
        "viewable": "public",
        "type": v["broadcastType"].as_str().unwrap().to_ascii_lowercase(),
        "duration": format!("{}h{}m{}s", secs / 3600, secs / 60 % 60, secs % 60),
    }))
    .unwrap()
}

/// `TwitchSource` for usher and HTTP; the VOD and its chapters from GQL.
struct RealSource {
    inner: TwitchSource,
    video: Video,
    chapters: Vec<Chapter>,
}

impl ArchiveSource for RealSource {
    fn check_allowed(&self) -> Result<(), ArchiveError> {
        self.inner.check_allowed()
    }
    fn video(&self, video_id: &str) -> Result<Video, ArchiveError> {
        assert_eq!(video_id, self.video.id);
        Ok(self.video.clone())
    }
    fn avatar_url(&self, _: &str) -> Result<Option<String>, ArchiveError> {
        Ok(None) // Helix only
    }
    fn chapters(&self, _: &Video) -> Vec<Chapter> {
        self.chapters.clone()
    }
    fn master_playlist(&self, video_id: &str) -> Result<MasterPlaylist, ArchiveError> {
        self.inner.master_playlist(video_id)
    }
    fn get(&self, url: &str) -> Result<HttpResponse, ArchiveError> {
        self.inner.get(url)
    }
}

fn source() -> RealSource {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let cfg = config();
    let video = video_from_metadata(&rt);
    // A Helix client that fails any request: none may be needed.
    let fixture = Arc::new(FixtureTransport::new());
    let helix = HelixClient::new(
        HelixConfig::new(ClientId::new("testclientid0000000000000000000").unwrap()),
        fixture,
    );
    helix.set_access_token(AccessToken::new("test-user-token-not-real"));
    let chapters = rt
        .block_on(
            ChapterResolver::new(ChaptersGql::resolve_for(&cfg, &ctx(), Flavor::Full), transport())
                .resolve(
                    &helix,
                    VodChaptersRequest {
                        video: &video,
                        timeline: None,
                        stored: None,
                        now_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as i64,
                    },
                ),
        )
        .unwrap()
        .chapters
        .unwrap_or_default();
    let features = TwitchFeatures::resolve_for(&cfg, &ctx(), Flavor::Full);
    let inner = TwitchSource::new(features, HelixAccess::Client(Arc::new(helix)), transport())
        .unwrap();
    RealSource {
        inner,
        video,
        chapters,
    }
}

fn open(dir: &Path) -> (JobQueue, StorageManager) {
    let db = dir.join("library.db");
    let queue = JobQueue::open(
        &db,
        QueueConfig {
            progress_interval: Duration::ZERO,
            tick_interval: Duration::from_millis(20),
            ..QueueConfig::default()
        },
    )
    .unwrap();
    let storage = StorageManager::open(&db, StorageConfig::default()).unwrap();
    storage
        .set_target(&Target::new(
            "downloads",
            TargetKind::Downloads,
            dir.join("downloads"),
        ))
        .unwrap();
    storage.attach(&queue);
    VodDownloader::new(Arc::new(source()), storage.clone(), Arc::new(FfmpegMuxer))
        .with_checkpoint_interval(Duration::from_millis(100))
        .register(&queue);
    (queue, storage)
}

fn progress(db: &Path, id: i64) -> (JobState, usize, usize) {
    let job = JobStore::open(db).unwrap().get(id).unwrap();
    let (done, count) = job
        .checkpoint
        .as_ref()
        .and_then(|c| {
            let count = c["plan"]["count"].as_u64()? as usize;
            let b = SegmentBitmap::from_hex(c["bitmap"].as_str()?, count)?;
            Some((b.count(), count))
        })
        .unwrap_or((0, 0));
    (job.state, done, count)
}

/// The process that gets killed; a no-op in a normal run.
#[test]
#[ignore = "child of real_vod_range_download_resumes_after_kill"]
fn real_child_entry() {
    let Ok(dir) = std::env::var(CHILD) else {
        return;
    };
    let (queue, _storage) = open(Path::new(&dir));
    let _driver = queue.start();
    thread::sleep(Duration::from_secs(600));
    panic!("the parent did not kill the child");
}

#[test]
#[ignore = "real Twitch; PA_TWITCH_LIVE_TEST=1, env values and PA_FFMPEG_LIB_DIR"]
fn real_vod_range_download_resumes_after_kill() {
    if !enabled() {
        eprintln!("skipped: set PA_TWITCH_LIVE_TEST=1");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir: PathBuf = tmp.path().into();
    let db = dir.join("library.db");
    let (len_ms, chapters) = {
        let s = source();
        (pa_twitch::chapters::vod_duration_ms(&s.video), s.chapters)
    };
    let span = env_u64("PA_TWITCH_RANGE_MS").unwrap_or(300_000);
    let start = env_u64("PA_TWITCH_RANGE_START_MS").unwrap_or(len_ms / 2);
    let mut request = DownloadRequest::vod(env("PA_TWITCH_VOD_ID"));
    request.quality = Quality::MaxHeight(env_u64("PA_TWITCH_MAX_HEIGHT").unwrap_or(360) as u32);
    request.container = Container::Mkv;
    request.range = Some(TimeRange {
        start_ms: start,
        end_ms: start + span,
    });
    request.part_label = chapters.first().map(|c| c.game.name.clone());
    request.workers = 4;
    eprintln!(
        "VOD {len_ms} ms, chapters {:?}, range {start}..{} ms",
        chapters
            .iter()
            .map(|c| (&c.game.name, c.start_ms, c.end_ms))
            .collect::<Vec<_>>(),
        start + span
    );
    let id = {
        let (queue, storage) = open(&dir);
        storage
            .enqueue(&queue, &request.to_job("downloads", Some(len_ms)))
            .unwrap()
    };

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "real_child_entry",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(300);
    let (killed_at, count) = loop {
        let (state, done, count) = progress(&db, id);
        if state == JobState::Running && count > 0 && done >= count / 3 {
            child.kill().unwrap();
            child.wait().unwrap();
            let (_, done, count) = progress(&db, id);
            break (done, count);
        }
        assert!(Instant::now() < deadline, "no progress");
        assert!(child.try_wait().unwrap().is_none(), "child exited early");
        thread::sleep(Duration::from_millis(20));
    };
    eprintln!("killed (SIGKILL) with {killed_at}/{count} segments on disk");
    assert!(killed_at < count, "killed after every segment");

    let (queue, _storage) = open(&dir);
    assert_eq!(queue.get(id).unwrap().state, JobState::Queued);
    assert!(queue.run_until_idle(Duration::from_secs(600)).unwrap());
    let job = queue.get(id).unwrap();
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let result = job.result.unwrap();
    eprintln!("result: {result}");
    assert!(result["segments_present_at_start"].as_u64().unwrap() >= killed_at as u64);
    let expected = result["expected_ms"].as_u64().unwrap();
    let got = result["duration_ms"].as_u64().unwrap();
    assert!(
        got.abs_diff(expected) as f64 <= expected as f64 * 0.02,
        "{got} vs {expected}"
    );

    let rel = result["rel_path"].as_str().unwrap();
    let file = dir
        .join("downloads")
        .join(rel)
        .join(result["file"].as_str().unwrap());
    let size = std::fs::metadata(&file).unwrap().len();
    let probed = FfmpegMuxer.probe(&file).unwrap();
    eprintln!(
        "probe: {size} bytes, duration {:?}, chapters {}, attachments {}",
        probed.duration_ms, probed.chapters, probed.attachments
    );
    let mut tags: Vec<_> = probed.tags.iter().collect();
    tags.sort();
    for (k, v) in tags {
        eprintln!("  tag {k} = {v}");
    }
    assert!(file.extension().is_some_and(|e| e == "mkv"));
    assert_eq!(
        probed.tags.get("twitch_video_id").map(String::as_str),
        Some(env("PA_TWITCH_VOD_ID").as_str())
    );
    assert_eq!(probed.chapters, chapters.len().min(1));
    if let Some(keep) = std::env::var_os("PA_TWITCH_ARCHIVE_KEEP") {
        std::fs::copy(&file, PathBuf::from(keep).join("real.mkv")).unwrap();
    }
}
