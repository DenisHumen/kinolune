// SPDX-License-Identifier: MPL-2.0
//! TWI-018 "done when": a 6-hour broadcast is recorded without gaps through
//! a network blip, and the chat is in sync within ±2 s.
//!
//! Everything runs against `live::testing::FakeLiveTwitch` on a loopback
//! port, on a virtual clock (the recorder sleeps; the clock jumps), through
//! the real HTTP transport, usher/GQL resolver and Helix client: no Twitch,
//! no Twitch client id. The stream has two stitched ad breaks (DATERANGE,
//! `Amazon|…` titles, discontinuities) and a category change.
//!
//! By default the `ConcatMuxer` double stands in for FFmpeg (CI). With
//! `PA_TWITCH_ARCHIVE_LIVE_MEDIA=<dir>` (an HLS rendition with 2-second
//! segments: `index.m3u8` + `.ts`, served in a loop so the timestamps jump
//! back at every lap) and `PA_FFMPEG_LIB_DIR`, the 6-hour test remuxes for
//! real and checks that the folded MKV is as long as the recording.

#![cfg(all(pa_twitch_archive, pa_twitch_native))]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pa_jobs::{JobQueue, JobState, QueueConfig};
use pa_storage::{FakeFs, StorageConfig, StorageManager, Target, TargetKind};
use pa_twitch::chapters::{
    ChapterGame, ChapterSource, LiveTimeline, TimelineChange, TimelineSegment,
};
use pa_twitch::playback::MasterPlaylist;
use pa_twitch::playback::{GqlSettings, PlayerChoice, TwitchFeatures};
use pa_twitch::{AccessToken, ClientId, HelixClient, HelixConfig, HttpResponse, ReqwestTransport};
use pa_twitch_archive::live::testing::{
    FakeLiveChat, FakeLiveStream, FakeLiveTwitch, VirtualClock,
};
use pa_twitch_archive::live::{
    AutoDecision, AutoRecord, LiveChat, LiveClock, LiveRecorder, LiveRequest, LiveSettings,
    LiveSource, LiveStops, LiveStream, TwitchLiveSource,
};
use pa_twitch_archive::testing::{ConcatMuxer, segments_from_dir};
use pa_twitch_archive::{ArchiveError, FfmpegMuxer, HelixAccess, Muxer, TwitchSource};
use serde_json::Value;

const T0: i64 = 1_800_000_000_000;
const MEDIA_ENV: &str = "PA_TWITCH_ARCHIVE_LIVE_MEDIA";

fn real_media() -> Option<PathBuf> {
    std::env::var_os(MEDIA_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn features(archive: bool) -> TwitchFeatures {
    TwitchFeatures {
        // Made-up GQL settings: the fake server accepts anything.
        player: PlayerChoice::Native(GqlSettings {
            client_id: "not-a-twitch-client-id".to_owned(),
            playback_access_token_hash: "made-up-hash".to_owned(),
        }),
        chapters_gql: false,
        chat_replay: false,
        archive,
    }
}

fn twitch_source(fake: &FakeLiveTwitch, archive: bool) -> TwitchSource {
    let transport = Arc::new(ReqwestTransport::new(Duration::from_secs(10)).unwrap());
    let mut config = HelixConfig::new(ClientId::new("madeup").unwrap());
    config.base_url = fake.helix_url();
    let helix = HelixClient::new(config, transport.clone());
    helix.set_access_token(AccessToken::new("made-up-token"));
    TwitchSource::new(
        features(archive),
        HelixAccess::Client(Arc::new(helix)),
        transport,
    )
    .unwrap()
    .with_endpoints(&fake.gql_url(), fake.base())
}

/// The Twitch source plus the made-up chat on the virtual clock and an
/// archive flag the test can flip.
struct TestSource {
    inner: TwitchLiveSource,
    clock: Arc<VirtualClock>,
    stream: FakeLiveStream,
    chat_every_ms: i64,
    allowed: AtomicBool,
    /// Runs before every request of the recorder (on its thread).
    on_get: std::sync::Mutex<Option<Box<dyn FnMut() + Send>>>,
}

impl LiveSource for TestSource {
    fn check_allowed(&self) -> Result<(), ArchiveError> {
        if !self.allowed.load(Ordering::SeqCst) {
            return Err(ArchiveError::fatal("archive_disabled", "flag off"));
        }
        self.inner.check_allowed()
    }
    fn stream(&self, user_id: &str) -> Result<Option<LiveStream>, ArchiveError> {
        self.inner.stream(user_id)
    }
    fn master_playlist(&self, login: &str) -> Result<Option<MasterPlaylist>, ArchiveError> {
        self.inner.master_playlist(login)
    }
    fn get(&self, url: &str) -> Result<HttpResponse, ArchiveError> {
        if let Some(hook) = self.on_get.lock().unwrap().as_mut() {
            hook();
        }
        self.inner.get(url)
    }
    fn timeline(&self, stream_id: &str) -> Option<LiveTimeline> {
        self.inner.timeline(stream_id)
    }
    fn avatar_url(&self, user_id: &str) -> Option<String> {
        self.inner.avatar_url(user_id)
    }
    fn chat(&self, _stream: &LiveStream) -> Option<Box<dyn LiveChat>> {
        Some(Box::new(FakeLiveChat::new(
            Arc::clone(&self.clock),
            self.stream.clone(),
            self.chat_every_ms,
        )))
    }
}

/// The TWI-008 timeline of the fake broadcast: one segment per category.
fn timeline_of(stream: &FakeLiveStream) -> LiveTimeline {
    let mut t = LiveTimeline::new(&stream.stream_id, &stream.user_id, stream.start_ms);
    let cats = &stream.categories;
    for (n, (from, id, name)) in cats.iter().enumerate() {
        t.segments.push(TimelineSegment {
            game: ChapterGame::new(id, name),
            title: Some(stream.title.clone()),
            started_at_ms: stream.pdt(*from),
            ended_at_ms: cats.get(n + 1).map(|(next, _, _)| stream.pdt(*next)),
            source: ChapterSource::EventSub,
        });
    }
    t
}

struct Env {
    _dir: tempfile::TempDir,
    target: PathBuf,
    queue: JobQueue,
    storage: StorageManager,
    source: Arc<TestSource>,
    fs: Arc<FakeFs>,
    volume: PathBuf,
    stops: LiveStops,
}

fn env(
    fake: &FakeLiveTwitch,
    stream: &FakeLiveStream,
    clock: &Arc<VirtualClock>,
    with_timeline: bool,
    settings: LiveSettings,
    muxer: Arc<dyn Muxer>,
) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let volume = dir.path().join("volume");
    std::fs::create_dir_all(&volume).unwrap();
    let fs = Arc::new(FakeFs::new());
    fs.add_volume(&volume, 1 << 40);
    let queue = JobQueue::open(
        &db,
        QueueConfig {
            // Progress is saved at most once a second of real time: the
            // virtual 6 hours pass in seconds.
            tick_interval: Duration::from_millis(10),
            ..QueueConfig::default()
        },
    )
    .unwrap();
    let storage = StorageManager::open(
        &db,
        StorageConfig {
            fs: fs.clone(),
            ..StorageConfig::default()
        },
    )
    .unwrap();
    let target = volume.join("recordings");
    storage
        .set_target(
            &Target::new("recordings", TargetKind::Recordings, &target).reserve_bytes(1 << 30),
        )
        .unwrap();
    storage.attach(&queue);
    let mut inner = TwitchLiveSource::new(Arc::new(twitch_source(fake, true)));
    if with_timeline {
        let t = timeline_of(stream);
        inner = inner.with_timeline(Box::new(move |id| (id == t.stream_id).then(|| t.clone())));
    }
    let source = Arc::new(TestSource {
        inner,
        clock: Arc::clone(clock),
        stream: stream.clone(),
        chat_every_ms: 7000,
        allowed: AtomicBool::new(true),
        on_get: std::sync::Mutex::new(None),
    });
    // Checkpoints are database commits; on the virtual clock a 10-second
    // interval would be thousands of them.
    let settings = LiveSettings {
        checkpoint_every: Duration::from_secs(600),
        ..settings
    };
    let recorder = LiveRecorder::new(source.clone(), storage.clone(), muxer)
        .with_clock(clock.clone())
        .with_settings(settings);
    let stops = recorder.stops();
    recorder.register(&queue);
    Env {
        _dir: dir,
        target,
        queue,
        storage,
        source,
        fs,
        volume,
        stops,
    }
}

fn request(stream: &FakeLiveStream) -> LiveRequest {
    let mut r = LiveRequest::new(&stream.user_id);
    r.login = Some(stream.login.clone());
    r.stream_id = Some(stream.stream_id.clone());
    r
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn read_chat(path: &Path) -> Value {
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Folder of the committed recording.
fn recording_dir(env: &Env, result: &Value) -> PathBuf {
    env.target.join(result["rel_path"].as_str().unwrap())
}

fn file_in(dir: &Path, suffix: &str) -> PathBuf {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(suffix))
        .unwrap_or_else(|| panic!("no *{suffix} in {}", dir.display()))
}

/// Checks every chat message against where it belongs: the message sent
/// at `t` is at `t − start − (media missing before t)` in the recording.
fn check_chat(chat: &Value, stream: &FakeLiveStream, every_ms: i64, gaps: &[(i64, i64)]) -> usize {
    let comments = chat["comments"].as_array().unwrap();
    let mut checked = 0;
    for c in comments {
        let body = c["message"]["body"].as_str().unwrap();
        let n: i64 = body.split_whitespace().nth(1).unwrap().parse().unwrap();
        let sent = FakeLiveChat::sent_at(stream.start_ms, every_ms, n);
        if gaps.iter().any(|(a, b)| sent >= *a && sent < *b) {
            assert_eq!(c["outside_recording"], true, "{c}");
            continue;
        }
        let missing: i64 = gaps
            .iter()
            .filter(|(_, b)| *b <= sent)
            .map(|(a, b)| b - a)
            .sum();
        let expected = sent - stream.start_ms - missing;
        let got = (c["content_offset_seconds"].as_f64().unwrap() * 1000.0).round() as i64;
        assert!(
            (got - expected).abs() <= 2000,
            "message {n}: at {got} ms, belongs at {expected} ms"
        );
        checked += 1;
    }
    checked
}

#[test]
fn six_hour_broadcast_with_ads_and_a_network_blip() {
    const HOUR: i64 = 3_600_000;
    let media = real_media().map(|d| segments_from_dir(&d).unwrap());
    let start = T0 + 60_000;
    let mut stream = FakeLiveStream::new(start, 6 * 1800);
    // Stitched ads at 1:00 (30 s) and 3:00 (60 s).
    stream.ads = vec![(1800, 15), (5400, 30)];
    // The network is gone for 20 s at 2:00:10; the window is 30 s.
    stream.blips = vec![(start + 2 * HOUR + 10_000, start + 2 * HOUR + 30_000)];
    stream
        .categories
        .push((4500, "1002".to_owned(), "Hades II".to_owned()));
    if let Some(m) = media.clone() {
        stream.media = m;
    }
    // The recorder starts 3 s after the broadcast (stream.online).
    let clock = Arc::new(VirtualClock::new(start + 3000));
    let fake = FakeLiveTwitch::start(stream.clone(), Arc::clone(&clock)).unwrap();
    let muxer: Arc<dyn Muxer> = if media.is_some() {
        Arc::new(FfmpegMuxer)
    } else {
        Arc::new(ConcatMuxer)
    };
    let settings = LiveSettings {
        fsync: false,
        ..LiveSettings::default()
    };
    let env = env(&fake, &stream, &clock, true, settings, muxer);
    let id = env
        .storage
        .enqueue(&env.queue, &request(&stream).to_job("recordings"))
        .unwrap();
    assert!(env.queue.run_until_idle(Duration::from_secs(900)).unwrap());
    let job = env.queue.get(id).unwrap();
    assert_eq!(job.state, JobState::Done, "{:?}", job.error);
    let result = job.result.unwrap();

    // Every segment, no gap, through the blip.
    assert!(fake.dropped() > 0, "the blip did not happen");
    assert_eq!(result["segments"], 10_800, "{result}");
    assert_eq!(result["gaps"], 0, "{result}");
    assert_eq!(result["missing_ms"], 0);
    assert_eq!(result["expected_ms"], 6 * HOUR);
    assert_eq!(result["end"], "offline");
    assert_eq!(fake.served_segments(), 10_800);
    let got = result["duration_ms"].as_u64().unwrap() as f64;
    assert!(
        (got - (6 * HOUR) as f64).abs() <= 6.0 * HOUR as f64 * 0.02,
        "{result}"
    );

    let dir = recording_dir(&env, &result);
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.ends_with(" [s90001]"), "{name}");
    assert!(
        // TWI-019 default name: categories before the title.
        name.contains(" LIVE Elden Ring, Hades II — Speedrun practice_ all bosses"),
        "{name}"
    );
    assert!(
        dir.parent()
            .unwrap()
            .parent()
            .unwrap()
            .ends_with("madeupstreamer [4242]")
    );
    let info = read_json(&file_in(&dir, ".info.json"));
    assert_eq!(info["format"], "kinolune.twitch.live/1");
    // Ads are recorded as served, and marked.
    assert_eq!(info["segments"]["ad"], 45);
    assert_eq!(info["ad_breaks"].as_array().unwrap().len(), 2);
    assert_eq!(
        info["ads"],
        serde_json::json!([
            {"start_ms": HOUR, "end_ms": HOUR + 30_000},
            {"start_ms": 3 * HOUR, "end_ms": 3 * HOUR + 60_000},
        ])
    );
    assert_eq!(info["segments"]["discontinuities"], 4);
    assert_eq!(info["segments"]["parts"], 6);
    // Chapters from the timeline, on the recording's clock.
    assert_eq!(info["chapter_source"], "timeline");
    assert_eq!(
        info["chapters"],
        serde_json::json!([
            {"start_ms": 0, "end_ms": 9_000_000, "title": "Elden Ring"},
            {"start_ms": 9_000_000, "end_ms": 6 * HOUR, "title": "Hades II"},
        ])
    );
    assert!(file_in(&dir, "-thumb.jpg").exists());
    // A copy for a look with other tools (ffprobe, a player).
    if let Some(keep) = std::env::var_os("PA_TWITCH_ARCHIVE_KEEP") {
        let keep = PathBuf::from(keep);
        std::fs::create_dir_all(&keep).unwrap();
        for f in std::fs::read_dir(&dir).unwrap() {
            let f = f.unwrap().path();
            std::fs::copy(&f, keep.join(f.file_name().unwrap())).unwrap();
        }
    }

    if media.is_none() {
        let mux = read_json(&PathBuf::from(format!(
            "{}.mux.json",
            file_in(&dir, ".mkv").display()
        )));
        let meta = mux["ffmetadata"].as_str().unwrap();
        for tag in [
            "TWITCH_STREAM_ID=90001\n",
            "TWITCH_USER_ID=4242\n",
            "TWITCH_LOGIN=madeupstreamer\n",
            "TWITCH_VIDEO_TYPE=live_recording\n",
            "TWITCH_CATEGORIES=Elden Ring, Hades II\n",
            "TWITCH_ADS_MS=3600000-3630000,10800000-10860000\n",
        ] {
            assert!(meta.contains(tag), "{tag} not in {meta}");
        }
        assert!(!meta.contains("TWITCH_GAPS_MS"));
        assert_eq!(meta.matches("[CHAPTER]").count(), 2);
        assert_eq!(mux["attachments"][0]["name"], "cover.jpg");
    }

    // Chat: in sync within ±2 s; only what arrived during the blip is lost.
    let chat = read_chat(&file_in(&dir, ".chat.json.gz"));
    let checked = check_chat(&chat, &stream, 7000, &[]);
    let sent = (6 * HOUR / 7000) as usize;
    assert!(checked + 4 >= sent && checked < sent, "{checked} of {sent}");

    let items = env.storage.items("recordings").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].group_key.as_deref(), Some("twitch:4242"));
    // Nothing left behind in tmp.
    assert_eq!(
        std::fs::read_dir(env.target.join(".tmp")).unwrap().count(),
        0
    );
}

#[test]
fn long_outage_and_a_storage_pause_are_gaps() {
    const MIN: i64 = 60_000;
    let start = T0;
    // 20 minutes; a 60-second outage at 5:00 (longer than the window).
    let mut stream = FakeLiveStream::new(start, 600);
    stream.blips = vec![(start + 5 * MIN, start + 6 * MIN)];
    stream
        .categories
        .push((300, "1002".to_owned(), "Hades II".to_owned()));
    let clock = Arc::new(VirtualClock::new(start + 1000));
    let fake = FakeLiveTwitch::start(stream.clone(), Arc::clone(&clock)).unwrap();
    let env = env(
        &fake,
        &stream,
        &clock,
        false,
        LiveSettings::default(),
        Arc::new(ConcatMuxer),
    );
    let id = env
        .storage
        .enqueue(&env.queue, &request(&stream).to_job("recordings"))
        .unwrap();
    let _driver = env.queue.start();
    // At 12:00 the disk is full: the monitor pauses the live recording at
    // the hard reserve (there is nothing else to pause).
    while clock.now_ms() < start + 12 * MIN {
        std::thread::sleep(Duration::from_millis(1));
    }
    // 1 TiB volume, reserve 1 GiB, hard reserve 256 MiB.
    env.fs.set_external(&env.volume, (1 << 40) - (200 << 20));
    let report = env.storage.check(&env.queue).unwrap();
    assert_eq!(report.live_paused, vec![id]);
    while env.queue.get(id).unwrap().state == JobState::Running {
        std::thread::sleep(Duration::from_millis(1));
    }
    // Paused for 2 minutes of the broadcast; then space is back.
    let paused_at = clock.now_ms();
    clock.advance(2 * MIN);
    env.fs.set_external(&env.volume, 0);
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    while !env.queue.get(id).unwrap().state.is_terminal() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let job = env.queue.get(id).unwrap();
    assert_eq!(job.state, JobState::Done, "{:?}", job.error);
    let result = job.result.unwrap();
    assert_eq!(result["gaps"], 2, "{result}");
    let dir = recording_dir(&env, &result);
    let info = read_json(&file_in(&dir, ".info.json"));
    let gaps = info["gaps"].as_array().unwrap();
    assert_eq!(gaps[0]["reason"], "window");
    assert_eq!(gaps[1]["reason"], "paused");
    // The outage: 60 s minus the 30-s window, give or take the backoff.
    let outage = gaps[0]["missing_ms"].as_i64().unwrap();
    assert!((20_000..=40_000).contains(&outage), "{outage}");
    let pause = gaps[1]["missing_ms"].as_i64().unwrap();
    assert!((60_000..=150_000).contains(&pause), "{pause}");
    let media = info["recorded"]["media_ms"].as_i64().unwrap();
    assert_eq!(media + outage + pause, 20 * MIN, "{info}");
    // Gaps are tagged; chapters come from what Helix showed.
    let mux = read_json(&PathBuf::from(format!(
        "{}.mux.json",
        file_in(&dir, ".mkv").display()
    )));
    assert!(
        mux["ffmetadata"]
            .as_str()
            .unwrap()
            .contains("TWITCH_GAPS_MS=")
    );
    assert_eq!(info["chapter_source"], "helix_polling");
    let chapters = info["chapters"].as_array().unwrap();
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[1]["title"], "Hades II");
    // Helix is polled once a minute: the boundary is at most that late
    // (10:00 of the broadcast, minus the outage before it).
    let boundary = chapters[1]["start_ms"].as_i64().unwrap();
    let ideal = 10 * MIN - outage;
    assert!(
        (ideal..=ideal + 61_000).contains(&boundary),
        "{boundary} vs {ideal}"
    );

    // Chat stays in sync across the gaps.
    let gap_span = |g: &Value| {
        let pdt = |k: &str| pa_twitch::parse_rfc3339_ms(g[k].as_str().unwrap()).unwrap();
        (pdt("from_pdt"), pdt("to_pdt"))
    };
    let spans: Vec<(i64, i64)> = gaps.iter().map(gap_span).collect();
    assert!(spans[1].0 <= paused_at);
    let chat = read_chat(&file_in(&dir, ".chat.json.gz"));
    assert!(check_chat(&chat, &stream, 7000, &spans) > 100);
}

#[test]
fn archive_flag_off_records_nothing() {
    let stream = FakeLiveStream::new(T0, 30);
    let clock = Arc::new(VirtualClock::new(T0 + 1000));
    let fake = FakeLiveTwitch::start(stream.clone(), Arc::clone(&clock)).unwrap();
    let env = env(
        &fake,
        &stream,
        &clock,
        false,
        LiveSettings::default(),
        Arc::new(ConcatMuxer),
    );
    env.source.allowed.store(false, Ordering::SeqCst);
    let id = env
        .storage
        .enqueue(&env.queue, &request(&stream).to_job("recordings"))
        .unwrap();
    assert!(env.queue.run_until_idle(Duration::from_secs(60)).unwrap());
    let job = env.queue.get(id).unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert!(job.error.unwrap().contains("archive_disabled"));
    assert_eq!(fake.requests(), 0, "nothing may reach Twitch");

    // The rules do not even queue one.
    let auto = AutoRecord::new(
        env.source.clone(),
        env.storage.clone(),
        env.queue.clone(),
        "recordings",
    );
    let rule = rule(&stream, None);
    let started = TimelineChange::Started(timeline_of(&stream));
    assert_eq!(
        auto.on_change(&started, &[rule], Some(&stream.login))
            .unwrap(),
        AutoDecision::NotAllowed("archive_disabled")
    );
}

#[test]
fn flag_turned_off_mid_broadcast_keeps_what_was_recorded() {
    let stream = FakeLiveStream::new(T0, 600);
    let clock = Arc::new(VirtualClock::new(T0 + 1000));
    let fake = FakeLiveTwitch::start(stream.clone(), Arc::clone(&clock)).unwrap();
    let env = env(
        &fake,
        &stream,
        &clock,
        false,
        LiveSettings::default(),
        Arc::new(ConcatMuxer),
    );
    let id = env
        .storage
        .enqueue(&env.queue, &request(&stream).to_job("recordings"))
        .unwrap();
    let _driver = env.queue.start();
    while clock.now_ms() < T0 + 5 * 60_000 {
        std::thread::sleep(Duration::from_millis(1));
    }
    env.source.allowed.store(false, Ordering::SeqCst);
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !env.queue.get(id).unwrap().state.is_terminal() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let job = env.queue.get(id).unwrap();
    assert_eq!(job.state, JobState::Done, "{:?}", job.error);
    let result = job.result.unwrap();
    assert_eq!(result["end"], "archive_disabled");
    let recorded = result["expected_ms"].as_i64().unwrap();
    // Stopped within a minute (the flag is checked once a minute).
    assert!(
        (5 * 60_000..=6 * 60_000 + 30_000).contains(&recorded),
        "{recorded}"
    );
    assert!(clock.now_ms() < T0 + 7 * 60_000);
}

fn rule(
    stream: &FakeLiveStream,
    categories: Option<Vec<String>>,
) -> pa_library::twitch::RecordRule {
    pa_library::twitch::RecordRule {
        profile_id: 1,
        user_id: stream.user_id.clone(),
        quality: "720p60".to_owned(),
        with_chat: true,
        retention_days: Some(14),
        keep_last_n: Some(10),
        categories,
        replace_with_vod: false,
        enabled: true,
    }
}

#[test]
fn rules_start_one_recording_per_channel() {
    let stream = FakeLiveStream::new(T0, 30);
    let clock = Arc::new(VirtualClock::new(T0));
    let fake = FakeLiveTwitch::start(stream.clone(), Arc::clone(&clock)).unwrap();
    let env = env(
        &fake,
        &stream,
        &clock,
        false,
        LiveSettings::default(),
        Arc::new(ConcatMuxer),
    );
    let auto = AutoRecord::new(
        env.source.clone(),
        env.storage.clone(),
        env.queue.clone(),
        "recordings",
    );
    let started = TimelineChange::Started(timeline_of(&stream));

    // Another channel's rule, a disabled rule: nothing.
    let mut other = rule(&stream, None);
    other.user_id = "777".to_owned();
    let mut off = rule(&stream, None);
    off.enabled = false;
    assert_eq!(
        auto.on_change(&started, &[other, off], None).unwrap(),
        AutoDecision::NoRule
    );
    // A rule for other categories waits for an update into one of them.
    let filtered = rule(&stream, Some(vec!["1002".to_owned()]));
    assert_eq!(
        auto.on_change(&started, std::slice::from_ref(&filtered), None)
            .unwrap(),
        AutoDecision::CategoryFiltered
    );
    let mut later = timeline_of(&stream);
    later.segments[0].ended_at_ms = Some(T0 + 10_000);
    later.segments.push(TimelineSegment {
        game: ChapterGame::new("1002", "Hades II"),
        title: None,
        started_at_ms: T0 + 10_000,
        ended_at_ms: None,
        source: ChapterSource::EventSub,
    });
    let AutoDecision::Started(id) = auto
        .on_change(
            &TimelineChange::Updated(later),
            std::slice::from_ref(&filtered),
            Some(&stream.login),
        )
        .unwrap()
    else {
        panic!("not started");
    };
    let job = env.queue.get(id).unwrap();
    assert_eq!(job.priority, pa_jobs::priority::LIVE);
    assert_eq!(job.owner_profile_id, Some(1));
    assert_eq!(job.target_storage.as_deref(), Some("recordings"));
    assert_eq!(
        job.params["quality"],
        serde_json::json!({"max_height": 720})
    );
    assert_eq!(job.params["stream_id"], stream.stream_id);
    assert_eq!(job.params["login"], stream.login);
    // The same broadcast again (EventSub and polling both report it).
    assert_eq!(
        auto.on_change(&started, &[rule(&stream, None)], None)
            .unwrap(),
        AutoDecision::AlreadyRecording(id)
    );
    assert_eq!(
        auto.on_change(
            &TimelineChange::Ended(timeline_of(&stream)),
            &[rule(&stream, None)],
            None
        )
        .unwrap(),
        AutoDecision::Ignored
    );
    // Retention of the channel's group from the rules.
    assert_eq!(auto.apply_retention(&[rule(&stream, None)]).unwrap(), 1);
    let rules = env.storage.retention_rules().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].group_key.as_deref(), Some("twitch:4242"));
    assert_eq!(
        (rules[0].keep_last, rules[0].max_age_days),
        (Some(10), Some(14))
    );
    let _ = clock.now_ms();
}

/// Waits (real time) until the job is terminal.
fn wait_terminal(env: &Env, id: i64) -> pa_jobs::Job {
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    while !env.queue.get(id).unwrap().state.is_terminal() {
        assert!(std::time::Instant::now() < deadline, "the job did not end");
        std::thread::sleep(Duration::from_millis(5));
    }
    env.queue.get(id).unwrap()
}

/// TWI-035: "Stop" of the app ends a recording in the middle of the
/// broadcast and keeps it, finalised like the end of the broadcast: one
/// MKV with tags, the cover and the channel's avatar, chat and chapters.
#[test]
fn stop_finalises_what_was_recorded() {
    const MIN: i64 = 60_000;
    let stream = FakeLiveStream::new(T0, 1800);
    let clock = Arc::new(VirtualClock::new(T0 + 1000));
    let fake = FakeLiveTwitch::start(stream.clone(), Arc::clone(&clock)).unwrap();
    let env = env(
        &fake,
        &stream,
        &clock,
        true,
        LiveSettings::default(),
        Arc::new(ConcatMuxer),
    );
    let id = env
        .storage
        .enqueue(&env.queue, &request(&stream).to_job("recordings"))
        .unwrap();
    let _driver = env.queue.start();
    while clock.now_ms() < T0 + 5 * MIN {
        std::thread::sleep(Duration::from_millis(1));
    }
    env.stops.request(id);
    let job = wait_terminal(&env, id);
    assert_eq!(job.state, JobState::Done, "{:?}", job.error);
    assert!(!env.stops.is_requested(id), "the request is used up");
    let result = job.result.unwrap();
    assert_eq!(result["end"], "stopped");
    let recorded = result["expected_ms"].as_i64().unwrap();
    // Five minutes, give or take the segments still in flight.
    assert!(
        (4 * MIN + 30_000..=6 * MIN).contains(&recorded),
        "{recorded}"
    );
    // The broadcast goes on; the recorder does not.
    assert!(clock.now_ms() < T0 + 7 * MIN);

    let dir = recording_dir(&env, &result);
    let mkv = file_in(&dir, ".mkv");
    let mux = read_json(&PathBuf::from(format!("{}.mux.json", mkv.display())));
    let meta = mux["ffmetadata"].as_str().unwrap();
    for tag in [
        "TWITCH_STREAM_ID=90001\n",
        "TWITCH_LOGIN=madeupstreamer\n",
        "TWITCH_CATEGORIES=Elden Ring\n",
        "TWITCH_STARTED_AT=",
    ] {
        assert!(meta.contains(tag), "{tag} not in {meta}");
    }
    assert!(meta.contains(&format!("title={}", stream.title)), "{meta}");
    let names: Vec<&str> = mux["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["cover.jpg", "avatar.jpg"]);
    let info = read_json(&file_in(&dir, ".info.json"));
    assert_eq!(info["end"], "stopped");
    assert!(file_in(&dir, ".chat.json.gz").exists());
    assert_eq!(env.storage.items("recordings").unwrap().len(), 1);
}

/// TWI-035: a paused recording that is stopped is finalised when it runs
/// again (the app resumes it), without recording more.
#[test]
fn stopping_a_paused_recording_keeps_it() {
    const MIN: i64 = 60_000;
    let stream = FakeLiveStream::new(T0, 1800);
    let clock = Arc::new(VirtualClock::new(T0 + 1000));
    let fake = FakeLiveTwitch::start(stream.clone(), Arc::clone(&clock)).unwrap();
    let env = env(
        &fake,
        &stream,
        &clock,
        false,
        LiveSettings::default(),
        Arc::new(ConcatMuxer),
    );
    let id = env
        .storage
        .enqueue(&env.queue, &request(&stream).to_job("recordings"))
        .unwrap();
    let _driver = env.queue.start();
    while clock.now_ms() < T0 + 3 * MIN {
        std::thread::sleep(Duration::from_millis(1));
    }
    env.queue.pause(id).unwrap();
    while env.queue.get(id).unwrap().state != JobState::Paused {
        std::thread::sleep(Duration::from_millis(1));
    }
    let served = fake.served_segments();
    env.stops.request(id);
    env.queue.resume(id).unwrap();
    let job = wait_terminal(&env, id);
    assert_eq!(job.state, JobState::Done, "{:?}", job.error);
    let result = job.result.unwrap();
    assert_eq!(result["end"], "stopped");
    assert_eq!(fake.served_segments(), served, "nothing recorded after");
    assert_eq!(result["segments"].as_u64().unwrap() as usize, served);
}

/// TWI-035: stopped before the first segment: nothing to keep.
#[test]
fn stop_before_anything_was_recorded_fails_cleanly() {
    let stream = FakeLiveStream::new(T0, 600);
    let clock = Arc::new(VirtualClock::new(T0 + 1000));
    let fake = FakeLiveTwitch::start(stream.clone(), Arc::clone(&clock)).unwrap();
    let env = env(
        &fake,
        &stream,
        &clock,
        false,
        LiveSettings::default(),
        Arc::new(ConcatMuxer),
    );
    let id = env
        .storage
        .enqueue(&env.queue, &request(&stream).to_job("recordings"))
        .unwrap();
    env.stops.request(id);
    assert!(env.queue.run_until_idle(Duration::from_secs(60)).unwrap());
    let job = env.queue.get(id).unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert!(job.error.unwrap().contains("stopped"));
    assert_eq!(fake.served_segments(), 0);
    assert!(env.storage.items("recordings").unwrap().is_empty());
}

/// TWI-035: the disk fills up in the middle of the broadcast (a write fails
/// with ENOSPC: the second part file is `/dev/full`). The recording stops
/// cleanly and keeps what it has as MPEG-TS, without a remux that would
/// need the same space again.
#[cfg(target_os = "linux")]
#[test]
fn a_full_disk_stops_and_keeps_the_recording() {
    const MIN: i64 = 60_000;
    let stream = FakeLiveStream::new(T0, 1800);
    let clock = Arc::new(VirtualClock::new(T0 + 1000));
    let fake = FakeLiveTwitch::start(stream.clone(), Arc::clone(&clock)).unwrap();
    let env = env(
        &fake,
        &stream,
        &clock,
        false,
        LiveSettings::default(),
        Arc::new(ConcatMuxer),
    );
    let mut req = request(&stream);
    req.part_minutes = 4;
    let id = env
        .storage
        .enqueue(&env.queue, &req.to_job("recordings"))
        .unwrap();
    let tmp = env.storage.job_tmp_dir("recordings", id).unwrap();
    let hook_clock = Arc::clone(&clock);
    let mut armed = false;
    *env.source.on_get.lock().unwrap() = Some(Box::new(move || {
        if !armed && hook_clock.now_ms() >= T0 + MIN {
            armed = true;
            std::os::unix::fs::symlink("/dev/full", tmp.join("part-0002.ts")).unwrap();
        }
    }));
    let _driver = env.queue.start();
    let job = wait_terminal(&env, id);
    assert_eq!(job.state, JobState::Done, "{:?}", job.error);
    let result = job.result.unwrap();
    assert_eq!(result["end"], "disk_full");
    let recorded = result["expected_ms"].as_i64().unwrap();
    assert!((4 * MIN..=5 * MIN).contains(&recorded), "{recorded}");
    assert!(clock.now_ms() < T0 + 6 * MIN, "stopped at once");
    let dir = recording_dir(&env, &result);
    let ts = file_in(&dir, ".ts");
    assert!(std::fs::metadata(&ts).unwrap().len() > 0);
    assert_eq!(
        result["file"].as_str().unwrap(),
        ts.file_name().unwrap().to_str().unwrap()
    );
    assert!(
        std::fs::read_dir(&dir).unwrap().all(|e| !e
            .unwrap()
            .path()
            .to_string_lossy()
            .ends_with(".mkv")),
        "no remux"
    );
    let info = read_json(&file_in(&dir, ".info.json"));
    assert_eq!(info["end"], "disk_full");
    assert_eq!(info["file"]["container"], "ts");
    assert!(
        info["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().starts_with("disk_full"))
    );
    assert_eq!(env.storage.items("recordings").unwrap().len(), 1);
}
