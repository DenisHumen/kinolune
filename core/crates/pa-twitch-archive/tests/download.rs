// SPDX-License-Identifier: MPL-2.0
//! TWI-017 "done when": a 3-hour VOD downloads with resume after the
//! process is killed, and its duration matches Helix within 2 %.
//!
//! Everything runs against `testing::FakeTwitch` on a loopback port: no
//! Twitch, no Twitch client id. Two modes:
//!
//! - default (CI): 1080 made-up 10-second segments and the `ConcatMuxer`
//!   double, so the download, the kill, the resume, the sidecars and the
//!   storage commit are tested without FFmpeg;
//! - with `PA_TWITCH_ARCHIVE_MEDIA=<dir>` (a real HLS rendition: an
//!   `index.m3u8` and its `.ts` segments, e.g. 3 hours at low bitrate made
//!   with FFmpeg's `hls` muxer) and `PA_FFMPEG_LIB_DIR` (our LGPL FFmpeg),
//!   the same flow remuxes for real into MKV and probes the result, then
//!   downloads one chapter as MP4.
//!
//! The kill test runs itself as a child process (`kill_child_entry`) and
//! kills it with SIGKILL in the middle of the segment downloads.

#![cfg(all(pa_twitch_archive, pa_twitch_native))]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use pa_jobs::{JobQueue, JobState, JobStore, QueueConfig};
use pa_storage::{StorageConfig, StorageManager, Target, TargetKind};
use pa_twitch::chapters::{Chapter, ChapterGame, ChapterSource};
use pa_twitch::playback::{GqlSettings, PlayerChoice, TwitchFeatures};
use pa_twitch::{AccessToken, ClientId, HelixClient, HelixConfig, ReqwestTransport};
use pa_twitch_archive::fetch::RetryPolicy;
use pa_twitch_archive::testing::{
    ConcatMuxer, FakeSegment, FakeTwitch, FakeVod, segments_from_dir,
};
use pa_twitch_archive::{
    Container, DownloadRequest, FfmpegMuxer, HelixAccess, Muxer, Quality, SegmentBitmap, TimeRange,
    TwitchSource, VodDownloader,
};
use serde_json::Value;

const CHILD_DB: &str = "PA_TWITCH_ARCHIVE_CHILD_DB";
const CHILD_BASE: &str = "PA_TWITCH_ARCHIVE_CHILD_BASE";
const CHILD_TARGET: &str = "PA_TWITCH_ARCHIVE_CHILD_TARGET";
const MEDIA_ENV: &str = "PA_TWITCH_ARCHIVE_MEDIA";
const VOD: &str = "777000111";
const HOUR_MS: u64 = 3_600_000;

fn real_media() -> Option<PathBuf> {
    std::env::var_os(MEDIA_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn muxer() -> Arc<dyn Muxer> {
    if real_media().is_some() {
        Arc::new(FfmpegMuxer)
    } else {
        Arc::new(ConcatMuxer)
    }
}

fn chapters(_: &pa_twitch::Video) -> Vec<Chapter> {
    let ch = |name: &str, start: u64, end: Option<u64>| Chapter {
        game: ChapterGame::new("1", name).unwrap(),
        title: Some("Practice run".to_owned()),
        start_ms: start,
        end_ms: end,
        source: ChapterSource::EventSub,
    };
    vec![
        ch("Elden Ring", 0, Some(HOUR_MS)),
        ch("Hades II", HOUR_MS, None),
    ]
}

fn features(archive: bool) -> TwitchFeatures {
    TwitchFeatures {
        // Made-up GQL settings: the fake server accepts anything.
        player: PlayerChoice::Native(GqlSettings {
            client_id: "not-a-twitch-client-id".to_owned(),
            playback_access_token_hash: "made-up-hash".to_owned(),
        }),
        chapters_gql: false,
        chat_replay: true,
        archive,
    }
}

fn source(base: &str, archive: bool) -> TwitchSource {
    let transport = Arc::new(ReqwestTransport::new(Duration::from_secs(30)).unwrap());
    let mut config = HelixConfig::new(ClientId::new("madeup").unwrap());
    config.base_url = format!("{base}/helix");
    let helix = HelixClient::new(config, transport.clone());
    helix.set_access_token(AccessToken::new("made-up-token"));
    TwitchSource::new(
        features(archive),
        HelixAccess::Client(Arc::new(helix)),
        transport,
    )
    .unwrap()
    .with_endpoints(&format!("{base}/gql"), base)
    .with_chapters(Box::new(chapters))
    .with_chat_replay(Box::new(|v| {
        Ok(Some(
            format!(r#"{{"video":{{"id":"{}"}},"comments":[]}}"#, v.id).into_bytes(),
        ))
    }))
}

fn queue_config() -> QueueConfig {
    QueueConfig {
        progress_interval: Duration::ZERO,
        tick_interval: Duration::from_millis(20),
        ..QueueConfig::default()
    }
}

/// Queue + storage over `db` with the downloader registered.
fn open(db: &Path, target: &Path, base: &str, archive: bool) -> (JobQueue, StorageManager) {
    let queue = JobQueue::open(db, queue_config()).unwrap();
    let storage = StorageManager::open(db, StorageConfig::default()).unwrap();
    storage
        .set_target(&Target::new("downloads", TargetKind::Downloads, target))
        .unwrap();
    storage.attach(&queue);
    VodDownloader::new(Arc::new(source(base, archive)), storage.clone(), muxer())
        .with_retry(RetryPolicy {
            attempts: 3,
            base_delay: Duration::from_millis(10),
        })
        .with_checkpoint_interval(Duration::from_millis(100))
        .register(&queue);
    (queue, storage)
}

/// The VOD: 3 hours, segments 100–105 muted, 2 chapters.
fn vod() -> FakeVod {
    let mut segments = match real_media() {
        Some(dir) => segments_from_dir(&dir).unwrap(),
        None => (0..1080)
            .map(|i| FakeSegment::synthetic(i, 10_000, 4096))
            .collect(),
    };
    for s in &mut segments[100..106] {
        s.muted = true;
    }
    let mut vod = FakeVod::new(VOD, segments);
    vod.helix_duration = Some("3h0m0s".to_owned());
    vod.helix_muted = vec![(1000, 60)];
    vod
}

/// Entry point of the child process; a no-op in a normal test run.
#[test]
fn kill_child_entry() {
    let Ok(db) = std::env::var(CHILD_DB) else {
        return;
    };
    let base = std::env::var(CHILD_BASE).unwrap();
    let target = PathBuf::from(std::env::var(CHILD_TARGET).unwrap());
    let (queue, _storage) = open(Path::new(&db), &target, &base, true);
    let _driver = queue.start();
    // Killed by the parent long before this.
    thread::sleep(Duration::from_secs(600));
    panic!("the parent did not kill the child");
}

fn job_state(db: &Path, id: i64) -> (JobState, usize) {
    let store = JobStore::open(db).unwrap();
    let job = store.get(id).unwrap();
    let done = job
        .checkpoint
        .as_ref()
        .and_then(|c| {
            let hex = c["bitmap"].as_str()?;
            let count = c["plan"]["count"].as_u64()? as usize;
            SegmentBitmap::from_hex(hex, count)
        })
        .map_or(0, |b| b.count());
    (job.state, done)
}

/// Copies a result to `$PA_TWITCH_ARCHIVE_KEEP` for a look with other
/// tools (ffprobe, a player).
fn keep(file: &Path) {
    if let Some(dir) = std::env::var_os("PA_TWITCH_ARCHIVE_KEEP") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(file, dir.join(file.file_name().unwrap())).unwrap();
    }
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn three_hour_vod_resumes_after_kill_and_matches_helix() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let target = dir.path().join("downloads");
    let fake = FakeTwitch::start(vec![vod()]).unwrap();
    fake.set_segment_delay(Duration::from_millis(20));
    fake.fail_once(VOD, 7);
    let n = vod().segments.len();

    let id = {
        let (queue, storage) = open(&db, &target, fake.base(), true);
        let job = DownloadRequest::vod(VOD).to_job("downloads", Some(3 * HOUR_MS));
        storage.enqueue(&queue, &job).unwrap()
    };

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "kill_child_entry",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_DB, &db)
        .env(CHILD_BASE, fake.base())
        .env(CHILD_TARGET, &target)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();

    // Kill -9 once about a third of the segments are on disk.
    let deadline = Instant::now() + Duration::from_secs(300);
    let killed_at = loop {
        let (state, done) = job_state(&db, id);
        if state == JobState::Running && done >= n / 3 {
            child.kill().unwrap();
            child.wait().unwrap();
            break job_state(&db, id).1;
        }
        assert!(Instant::now() < deadline, "the child made no progress");
        assert!(
            child.try_wait().unwrap().is_none(),
            "the child exited early"
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert!(killed_at >= n / 3 && killed_at < n, "{killed_at}");
    let before = fake.segment_requests();
    assert!(before < n, "killed after every segment: {before}");

    // Restart: only the missing segments are fetched.
    let (queue, storage) = open(&db, &target, fake.base(), true);
    assert_eq!(queue.get(id).unwrap().state, JobState::Queued);
    assert!(queue.run_until_idle(Duration::from_secs(900)).unwrap());
    let job = queue.get(id).unwrap();
    assert_eq!(job.state, JobState::Done, "{job:?}");
    // Segments requested twice: those in flight when the child died (at
    // most 8 workers) and the one that failed once.
    let total = fake.segment_requests();
    assert!(total <= n + 8 + 1, "{total} requests for {n} segments");

    let result = job.result.unwrap();
    eprintln!("killed at {killed_at}/{n} segments, {total} segment requests; result: {result}");
    assert!(result["segments_present_at_start"].as_u64().unwrap() >= killed_at as u64);
    let duration = result["duration_ms"].as_u64().unwrap();
    let helix = 3 * HOUR_MS;
    assert!(
        duration.abs_diff(helix) as f64 <= helix as f64 * 0.02,
        "{duration} ms vs {helix} ms"
    );
    assert_eq!(result["helix_duration_ms"].as_u64(), Some(helix));

    // The committed folder.
    let rel = result["rel_path"].as_str().unwrap();
    assert!(
        rel.starts_with(
            "made_up_streamer [4040]/2026/2026-11-12 Elden Ring, Hades II — Practice run_ all bosses [v777000111]"
        ),
        "{rel}"
    );
    let folder = target.join(rel);
    let base = "2026-11-12 Elden Ring, Hades II — Practice run_ all bosses [v777000111]";
    let video = folder.join(format!("{base}.mkv"));
    assert!(video.is_file());
    assert!(folder.join(format!("{base}-thumb.jpg")).is_file());
    assert!(folder.join(format!("{base}.chat.json.gz")).is_file());
    let info = read_json(&folder.join(format!("{base}.info.json")));
    assert_eq!(info["video"]["id"], VOD);
    assert_eq!(info["segments"]["count"], n);
    assert_eq!(info["segments"]["muted"], 6);
    let muted = info["muted"].as_array().unwrap();
    assert_eq!(muted.len(), 1, "{muted:?}");
    let first_muted: u64 = vod().segments[..100].iter().map(|s| s.duration_ms).sum();
    assert_eq!(
        muted[0]["start_ms"].as_u64(),
        Some(first_muted.min(1_000_000))
    );
    assert_eq!(info["chapters"].as_array().unwrap().len(), 2);
    assert!(!target.join(format!(".tmp/job-{id}")).exists());

    let items = storage.items("downloads").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].group_key.as_deref(), Some("twitch:4040"));

    // What went into the file.
    keep(&video);
    let probed = muxer().probe(&video).unwrap();
    assert_eq!(
        probed.tags.get("twitch_video_id").map(String::as_str),
        Some(VOD)
    );
    assert_eq!(
        probed.tags.get("twitch_login").map(String::as_str),
        Some("made_up_streamer")
    );
    assert_eq!(
        probed.tags.get("twitch_categories").map(String::as_str),
        Some("Elden Ring, Hades II")
    );
    assert_eq!(probed.chapters, 2);
    assert_eq!(probed.attachments, 2, "cover and avatar");

    // One chapter only, as MP4 (with real media: a real MP4 remux).
    let mut part = DownloadRequest::vod(VOD);
    part.container = Container::Mp4;
    part.quality = Quality::Source;
    part.range = Some(TimeRange {
        start_ms: HOUR_MS + 5_000,
        end_ms: HOUR_MS + 600_000,
    });
    part.part_label = Some("Hades II".to_owned());
    let id = storage
        .enqueue(&queue, &part.to_job("downloads", Some(3 * HOUR_MS)))
        .unwrap();
    assert!(queue.run_until_idle(Duration::from_secs(600)).unwrap());
    let job = queue.get(id).unwrap();
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let result = job.result.unwrap();
    let expected = result["expected_ms"].as_u64().unwrap();
    let got = result["duration_ms"].as_u64().unwrap();
    assert!((595_000..=615_000).contains(&expected), "{expected}");
    assert!(
        got.abs_diff(expected) as f64 <= expected as f64 * 0.02,
        "{got} vs {expected}"
    );
    let rel = result["rel_path"].as_str().unwrap();
    // TWI-019 default name: the part's category before the title.
    assert!(
        rel.ends_with("/2026-11-12 Hades II — Practice run_ all bosses [v777000111]"),
        "{rel}"
    );
    let file = result["file"].as_str().unwrap();
    assert!(file.ends_with(".mp4"));
    eprintln!("chapter result: {result}");
    keep(&target.join(rel).join(file));
    let probed = muxer().probe(&target.join(rel).join(file)).unwrap();
    assert_eq!(probed.chapters, 1, "only the chapter in range");
    assert_eq!(
        probed.tags.get("twitch_part").map(String::as_str),
        Some("Hades II")
    );
}

/// Runs one job of `request` in-process to the end.
fn run_one(fake: &FakeTwitch, archive: bool, request: &DownloadRequest) -> pa_jobs::Job {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let (queue, storage) = open(&db, &dir.path().join("downloads"), fake.base(), archive);
    let id = storage
        .enqueue(&queue, &request.to_job("downloads", Some(60_000)))
        .unwrap();
    assert!(queue.run_until_idle(Duration::from_secs(60)).unwrap());
    queue.get(id).unwrap()
}

fn small_vod(id: &str) -> FakeVod {
    FakeVod::new(
        id,
        (0..6)
            .map(|i| FakeSegment::synthetic(i, 10_000, 512))
            .collect(),
    )
}

#[test]
fn restricted_vod_fails_with_usher_reason() {
    if real_media().is_some() {
        return;
    }
    let mut vod = small_vod("555");
    vod.restricted = Some("vod_manifest_restricted".to_owned());
    let fake = FakeTwitch::start(vec![vod]).unwrap();
    let job = run_one(&fake, true, &DownloadRequest::vod("555"));
    assert_eq!(job.state, JobState::Failed);
    let error = job.error.unwrap();
    assert!(error.starts_with("usher_403:"), "{error}");
    assert!(error.contains("vod_manifest_restricted"), "{error}");
    assert_eq!(fake.segment_requests(), 0);
}

#[test]
fn archive_flag_off_downloads_nothing() {
    if real_media().is_some() {
        return;
    }
    let fake = FakeTwitch::start(vec![small_vod("556")]).unwrap();
    let job = run_one(&fake, false, &DownloadRequest::vod("556"));
    assert_eq!(job.state, JobState::Failed);
    assert!(job.error.unwrap().starts_with("archive_disabled:"));
    assert!(fake.requests().is_empty(), "{:?}", fake.requests());
}

#[test]
fn live_vod_is_not_downloaded_yet() {
    if real_media().is_some() {
        return;
    }
    let mut vod = small_vod("557");
    vod.live = true;
    let fake = FakeTwitch::start(vec![vod]).unwrap();
    let job = run_one(&fake, true, &DownloadRequest::vod("557"));
    assert_eq!(job.state, JobState::Failed);
    assert!(job.error.unwrap().starts_with("vod_live:"));
}

#[test]
fn wrong_duration_fails_the_check() {
    if real_media().is_some() {
        return;
    }
    let mut vod = small_vod("558");
    // The playlist has 60 s; Helix claims 70 s.
    vod.helix_duration = Some("0h1m10s".to_owned());
    let fake = FakeTwitch::start(vec![vod]).unwrap();
    let job = run_one(&fake, true, &DownloadRequest::vod("558"));
    assert_eq!(job.state, JobState::Failed);
    assert!(job.error.unwrap().starts_with("duration_mismatch:"));
}
