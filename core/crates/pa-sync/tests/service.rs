// SPDX-License-Identifier: MPL-2.0
//! UI-019 end to end in one process: two apps pair by QR code and by a
//! typed code (found over mDNS on a private port), then a progress update
//! and a "watched" mark on one appear in the other's library; unpairing
//! stops it.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pa_discovery::{DeviceClass, Interfaces, MdnsConfig};
use pa_library::progress::{self, PositionReport, Thresholds};
use pa_library::rusqlite::Connection;
use pa_model::{MediaKey, MediaKind};
use pa_secrets::MemoryStore;
use pa_sync::input::{InputAction, InputField};
use pa_sync::lan::service::{
    InputRequest, RemoteInput, RemoteMusic, ServiceConfig, ServiceEvent, SyncLibrary, SyncService,
};
use pa_sync::music::MusicAction;
use pa_sync::{Op, OpId, Replica, outbox};

/// A library like the app's catalog: one database and a replica while
/// sync is on.
struct TestLibrary {
    conn: Mutex<Connection>,
    replica: Mutex<Option<Replica>>,
}

impl TestLibrary {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            conn: Mutex::new(pa_library::open_in_memory().unwrap()),
            replica: Mutex::new(None),
        })
    }

    /// The player reports a position (what `Catalog::save_progress` does).
    fn play(&self, key: &MediaKey, position_ms: u64) {
        let mut conn = self.conn.lock().unwrap();
        let mut replica = self.replica.lock().unwrap();
        let replica = replica.as_mut().expect("sync is on");
        let report = PositionReport {
            profile_id: 1,
            key: key.clone(),
            kind: MediaKind::Movie,
            position_ms,
            duration_ms: Some(6_000_000),
        };
        progress::report_position(
            &mut conn,
            replica,
            &Thresholds::default(),
            &report,
            now_ms(),
        )
        .unwrap();
    }

    fn position(&self, key: &MediaKey) -> Option<(u64, bool)> {
        let conn = self.conn.lock().unwrap();
        progress::get(&conn, 1, key)
            .unwrap()
            .map(|p| (p.position_ms, p.watched))
    }
}

impl SyncLibrary for TestLibrary {
    fn enable(&self, device_id: &str) -> Result<(), String> {
        let mut r = self.replica.lock().unwrap();
        if r.as_ref().is_none_or(|r| r.device_id() != device_id) {
            *r = Some(Replica::new(device_id).map_err(|e| e.to_string())?);
        }
        Ok(())
    }

    fn disable(&self) -> Result<(), String> {
        *self.replica.lock().unwrap() = None;
        Ok(())
    }

    fn pending(&self) -> Result<Vec<Op>, String> {
        let conn = self.conn.lock().unwrap();
        outbox::compact(&conn).map_err(|e| e.to_string())?;
        Ok(outbox::due(&conn, i64::MAX, 256)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|p| p.op)
            .collect())
    }

    fn ack(&self, op_id: OpId) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        outbox::ack(&conn, op_id).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn apply(&self, op: &Op) -> Result<bool, String> {
        let conn = self.conn.lock().unwrap();
        let mut r = self.replica.lock().unwrap();
        let Some(r) = r.as_mut() else {
            return Ok(false);
        };
        r.receive_into(&conn, op, now_ms())
            .map_err(|e| e.to_string())
    }
}

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn private_port() -> u16 {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let n = nanos.wrapping_add(NEXT.fetch_add(7919, Ordering::Relaxed)) ^ std::process::id();
    30_000 + u16::try_from(n % 20_000).unwrap()
}

struct App {
    service: SyncService,
    library: Arc<TestLibrary>,
    _dir: tempfile::TempDir,
}

fn app(name: &str, class: DeviceClass, mdns: Option<MdnsConfig>) -> App {
    let dir = tempfile::tempdir().unwrap();
    let library = TestLibrary::new();
    let config = ServiceConfig {
        bind_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
        mdns,
        flush_interval: Duration::from_millis(200),
        find_timeout: Duration::from_secs(5),
        ..ServiceConfig::new(dir.path(), name, class)
    };
    let service = SyncService::open(
        config,
        Arc::new(MemoryStore::new()),
        Arc::clone(&library) as Arc<dyn SyncLibrary>,
    )
    .unwrap();
    App {
        service,
        library,
        _dir: dir,
    }
}

/// Without mDNS the devices learn each other's sync address by hand.
fn introduce(a: &App, b: &App) {
    let (pa, pb) = (
        a.service.sync_addr().unwrap(),
        b.service.sync_addr().unwrap(),
    );
    a.service
        .peer_up(b.service.device_id(), DeviceClass::Mobile, pb);
    b.service
        .peer_up(a.service.device_id(), DeviceClass::Desktop, pa);
}

fn wait(what: &str, limit: Duration, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(start.elapsed() < limit, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Waits up to 5 s for an event matching `ok`, skipping others. Events
/// are emitted after the state change they report, so a test that has
/// seen the state must still wait for its event.
fn saw(events: &Receiver<ServiceEvent>, what: &str, ok: impl Fn(&ServiceEvent) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(e) if ok(&e) => return,
            Ok(_) => {}
            Err(_) => panic!("no {what}"),
        }
    }
}

fn movie(n: u32) -> MediaKey {
    format!("tmdb:movie:{n}").parse().unwrap()
}

#[test]
fn pair_by_qr_then_progress_and_watched_sync() {
    let pc = app("Desktop", DeviceClass::Desktop, None);
    let phone = app("Phone", DeviceClass::Mobile, None);
    let pc_events = pc.service.subscribe();
    assert!(pc.service.devices().is_empty());
    assert!(!pc.service.start().unwrap(), "nothing to sync alone");

    // A wrong code costs an attempt but keeps the invitation.
    let invite = pc.service.invite().unwrap();
    assert_eq!(invite.code.len(), 6);
    assert!(invite.qr.starts_with("projectalpha://pair?"));
    let wrong = if invite.code == "000000" {
        "111111"
    } else {
        "000000"
    };
    let qr_wrong = invite
        .qr
        .replace(&format!("c={}", invite.code), &format!("c={wrong}"));
    assert_eq!(
        phone.service.join(&qr_wrong).unwrap_err().code,
        "wrong_code"
    );
    assert!(pc.service.invite_addr().is_some());

    let inviter = phone.service.join(&invite.qr).unwrap();
    assert_eq!(inviter.device_id, pc.service.device_id());
    assert_eq!(inviter.name, "Desktop");
    assert!(phone.service.is_paired());
    wait("pc sees the pairing", Duration::from_secs(5), || {
        pc.service.is_paired() && pc.service.is_running()
    });
    assert!(phone.service.is_running());
    saw(
        &pc_events,
        "Paired event",
        |e| matches!(e, ServiceEvent::Paired { device_id, .. } if device_id == phone.service.device_id()),
    );

    let names: Vec<String> = pc.service.devices().into_iter().map(|d| d.name).collect();
    assert_eq!(names, ["Desktop", "Phone"]);
    assert!(pc.service.devices()[0].this_device);
    assert_eq!(
        phone.service.devices()[1].fingerprint,
        pc.service.fingerprint()
    );

    introduce(&pc, &phone);
    wait(
        "phone links to the desktop hub",
        Duration::from_secs(10),
        || phone.service.hub().as_deref() == Some(pc.service.device_id()),
    );
    wait("both online", Duration::from_secs(5), || {
        pc.service.devices().iter().all(|d| d.online)
            && phone.service.devices().iter().all(|d| d.online)
    });

    // Progress on the desktop shows up on the phone…
    let phone_events = phone.service.subscribe();
    pc.library.play(&movie(603), 1_200_000);
    pc.service.flush();
    wait(
        "progress reaches the phone",
        Duration::from_secs(10),
        || phone.library.position(&movie(603)) == Some((1_200_000, false)),
    );
    // The event follows the write the wait above saw.
    saw(&phone_events, "LibraryChanged", |e| {
        *e == ServiceEvent::LibraryChanged
    });
    // …and the other way round, with "watched" past 95 %.
    phone.library.play(&movie(604), 5_900_000);
    phone.service.flush();
    wait(
        "watched reaches the desktop",
        Duration::from_secs(10),
        || pc.library.position(&movie(604)) == Some((5_900_000, true)),
    );
    // The phone's ops were acknowledged by its hub.
    wait("outbox drained", Duration::from_secs(5), || {
        let conn = phone.library.conn.lock().unwrap();
        outbox::len(&conn).unwrap() == 0
    });

    // Unpairing the phone on the desktop: the desktop is alone, stops, and
    // its later changes stay local.
    pc.service.unpair(phone.service.device_id()).unwrap();
    assert_eq!(pc.service.devices().len(), 1);
    assert!(!pc.service.is_paired());
    assert!(!pc.service.is_running());
}

#[test]
fn pair_by_typed_code_over_mdns() {
    let mdns = MdnsConfig {
        port: Some(private_port()),
        interfaces: Interfaces::LoopbackOnly,
        multicast_lock: None,
    };
    let tv = app("TV", DeviceClass::Tv, Some(mdns.clone()));
    let phone = app("Phone", DeviceClass::Mobile, Some(mdns));
    let invite = tv.service.invite().unwrap();
    assert_eq!(
        phone.service.join("12345").unwrap_err().code,
        "invalid_code"
    );
    let inviter = phone.service.join(&invite.code).unwrap();
    assert_eq!(inviter.device_id, tv.service.device_id());
    wait("tv paired", Duration::from_secs(5), || {
        tv.service.is_running()
    });
    // The two find each other over mDNS and sync.
    wait("phone links to the TV", Duration::from_secs(15), || {
        phone.service.hub().as_deref() == Some(tv.service.device_id())
    });
    phone.library.play(&movie(1), 600_000);
    phone.service.flush();
    wait("progress reaches the TV", Duration::from_secs(10), || {
        tv.library.position(&movie(1)) == Some((600_000, false))
    });
    // A paired device cannot join another family before it unpairs.
    let other = app("Other", DeviceClass::Desktop, None);
    let qr = other.service.invite().unwrap().qr;
    assert_eq!(phone.service.join(&qr).unwrap_err().code, "already_paired");
    // Leaving: the phone forgets the family.
    phone.service.unpair(phone.service.device_id()).unwrap();
    assert!(phone.service.devices().is_empty());
    assert!(!phone.service.is_running());
}

/// SYNC-003: heartbeats reach the other device without touching the
/// outbox, "playing elsewhere" follows the state, a takeover reaches the
/// other device, and a pull before playing returns within its timeout.
#[test]
fn playback_session_heartbeat_and_takeover() {
    use pa_sync::handoff::{PULL_TIMEOUT_MS, ResumePoint};
    use pa_sync::proto::PlayState;

    let pc = app("Desktop", DeviceClass::Desktop, None);
    let phone = app("Phone", DeviceClass::Mobile, None);
    phone
        .service
        .join(&pc.service.invite().unwrap().qr)
        .unwrap();
    wait("pc running", Duration::from_secs(5), || {
        pc.service.is_running()
    });
    introduce(&pc, &phone);
    wait("phone links to the hub", Duration::from_secs(10), || {
        phone.service.hub().as_deref() == Some(pc.service.device_id())
    });
    let pc_events = pc.service.subscribe();
    let key = movie(603);

    // The phone plays: the desktop sees it playing, nothing is queued.
    phone
        .service
        .heartbeat(
            "default",
            &key,
            120_000,
            Some(6_000_000),
            PlayState::Playing,
        )
        .unwrap();
    wait(
        "heartbeat reaches the desktop",
        Duration::from_secs(5),
        || pc.service.playing_elsewhere().len() == 1,
    );
    let now = pc.service.playing_elsewhere()[0].clone();
    assert_eq!(now.hlc.device, phone.service.device_id());
    assert_eq!(now.ttl_ms, Some(pa_sync::handoff::PLAYING_TTL_MS));
    assert!(phone.service.playing_elsewhere().is_empty(), "not its own");
    let remote = pc.service.remote_playback("default", &key).unwrap();
    assert_eq!(remote.position_ms, 120_000);
    assert!(pc.service.remote_playback("kids", &key).is_none());
    for a in [&pc, &phone] {
        let conn = a.library.conn.lock().unwrap();
        assert_eq!(outbox::len(&conn).unwrap(), 0, "ephemeral");
        assert!(progress::get(&conn, 1, &key).unwrap().is_none());
    }
    // The node caches the heartbeat before the service reports it.
    saw(
        &pc_events,
        "Playback event",
        |e| matches!(e, ServiceEvent::Playback(u) if u.position_ms == 120_000),
    );

    // The desktop takes over: the phone hears it; its paused heartbeat
    // ends "playing elsewhere" but stays the resume point.
    let phone_events = phone.service.subscribe();
    pc.service.start_playing("default", &key).unwrap();
    let start = Instant::now();
    let takeover = loop {
        let left = Duration::from_secs(5).saturating_sub(start.elapsed());
        if let ServiceEvent::Takeover(t) = phone_events.recv_timeout(left).expect("takeover") {
            break t;
        }
    };
    assert_eq!(takeover.key, key);
    assert_eq!(takeover.hlc.device, pc.service.device_id());
    // The phone's clock has seen the desktop's stamp: its pause orders after.
    phone
        .service
        .heartbeat("default", &key, 125_000, Some(6_000_000), PlayState::Paused)
        .unwrap();
    wait("pause reaches the desktop", Duration::from_secs(5), || {
        pc.service
            .remote_playback("default", &key)
            .is_some_and(|u| u.state == PlayState::Paused)
    });
    let paused = pc.service.remote_playback("default", &key).unwrap();
    assert!(paused.hlc > takeover.hlc);
    assert!(pc.service.playing_elsewhere().is_empty());
    assert_eq!(
        ResumePoint::from_update(&paused).position_at(paused.hlc.wall_ms + 60_000),
        125_000
    );

    // A pull before playing: the phone asks its hub, which answers fast;
    // the hub itself has nobody to ask.
    let t = Instant::now();
    assert!(
        phone
            .service
            .refresh(Duration::from_millis(PULL_TIMEOUT_MS))
    );
    assert!(t.elapsed() < Duration::from_millis(PULL_TIMEOUT_MS + 200));
    assert!(pc.service.refresh(Duration::from_millis(PULL_TIMEOUT_MS)));
}

/// Waits up to 5 s for an `input` event matching `ok` and returns it.
fn input_event(
    events: &Receiver<ServiceEvent>,
    what: &str,
    ok: impl Fn(&RemoteInput) -> bool,
) -> RemoteInput {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(ServiceEvent::Input(i)) if ok(&i) => return i,
            Ok(_) => {}
            Err(_) => panic!("no {what}"),
        }
    }
}

fn text_of(i: &RemoteInput) -> Option<&str> {
    i.text.as_deref().map(String::as_str)
}

#[test]
fn remote_input_from_tv_to_phone() {
    // UI-027: a field focused on the TV opens input on the phone; text and
    // a password typed there reach the TV (the password sealed); a cancel
    // on either side ends the session on the other.
    let tv = app("TV", DeviceClass::Tv, None);
    let phone = app("Phone", DeviceClass::Mobile, None);
    let request = |field, label: Option<&str>, secret, text: &str| InputRequest {
        field,
        label: label.map(str::to_owned),
        secret,
        text: text.to_owned(),
    };
    assert_eq!(
        tv.service
            .input_request(&request(InputField::Search, None, false, ""))
            .unwrap(),
        None,
        "nobody to ask before pairing"
    );
    phone
        .service
        .join(&tv.service.invite().unwrap().qr)
        .unwrap();
    wait("tv running", Duration::from_secs(5), || {
        tv.service.is_running()
    });
    introduce(&tv, &phone);
    wait("phone links to the tv", Duration::from_secs(10), || {
        phone.service.hub().as_deref() == Some(tv.service.device_id())
    });
    let tv_events = tv.service.subscribe();
    let phone_events = phone.service.subscribe();

    // Search: the request carries what the field holds.
    let search = tv
        .service
        .input_request(&request(InputField::Search, Some("Search"), false, "mat"))
        .unwrap()
        .unwrap();
    let req = input_event(&phone_events, "search request", |i| {
        i.action == InputAction::Request
    });
    assert_eq!(req.session, search);
    assert_eq!(req.device_id, tv.service.device_id());
    assert_eq!(req.device_name.as_deref(), Some("TV"));
    assert_eq!(req.field, Some(InputField::Search));
    assert_eq!(req.label.as_deref(), Some("Search"));
    assert_eq!(text_of(&req), Some("mat"));
    let to = Some(tv.service.device_id());
    phone
        .service
        .input_send(&search, to, InputAction::Open, None, false)
        .unwrap();
    let open = input_event(&tv_events, "open", |i| i.action == InputAction::Open);
    assert_eq!(open.device_name.as_deref(), Some("Phone"));
    for text in ["matr", "matrix"] {
        phone
            .service
            .input_send(&search, to, InputAction::Text, Some(text), false)
            .unwrap();
    }
    let typed = input_event(&tv_events, "text", |i| text_of(i) == Some("matrix"));
    assert_eq!(typed.session, search);
    phone
        .service
        .input_send(&search, to, InputAction::Submit, Some("matrix"), false)
        .unwrap();
    input_event(&tv_events, "submit", |i| i.action == InputAction::Submit);

    // A password: the request is empty and secret, the text comes back
    // sealed, nothing prints or keeps it.
    let password = tv
        .service
        .input_request(&request(
            InputField::Password,
            Some("Password"),
            true,
            "old",
        ))
        .unwrap()
        .unwrap();
    let req = input_event(&phone_events, "password request", |i| i.session == password);
    assert!(req.secret);
    assert!(req.text.is_none(), "a secret field starts empty");
    phone
        .service
        .input_send(&password, to, InputAction::Text, Some("hunter2"), true)
        .unwrap();
    let got = input_event(&tv_events, "password", |i| i.session == password);
    assert_eq!(text_of(&got), Some("hunter2"));
    assert!(got.secret);
    assert!(!format!("{got:?}").contains("hunter2"));
    for a in [&tv, &phone] {
        assert!(
            a.service
                .journal_frames()
                .iter()
                .all(|f| !f.contains("hunter2") && !f.contains("\"input\"")),
            "ephemeral"
        );
        let conn = a.library.conn.lock().unwrap();
        assert_eq!(outbox::len(&conn).unwrap(), 0);
    }

    // Cancel on the TV (the field lost focus) closes the phone's sheet.
    tv.service
        .input_send(&password, None, InputAction::Cancel, None, false)
        .unwrap();
    input_event(&phone_events, "cancel from the tv", |i| {
        i.session == password && i.action == InputAction::Cancel
    });
    // And the other way round.
    let pin = tv
        .service
        .input_request(&request(InputField::Pin, None, true, ""))
        .unwrap()
        .unwrap();
    input_event(&phone_events, "pin request", |i| i.session == pin);
    phone
        .service
        .input_send(&pin, to, InputAction::Cancel, None, false)
        .unwrap();
    input_event(&tv_events, "cancel from the phone", |i| {
        i.session == pin && i.action == InputAction::Cancel
    });
}

/// SYNC-014 (ADR-0137): a progress save goes out at once (the library calls
/// `flush` after it), not with the next batch: with the default 2 s batch
/// interval both directions — desktop hub to phone and phone to hub — see
/// every position in under a second.
#[test]
fn progress_reaches_the_other_device_within_a_second() {
    let slow = |name: &str, class: DeviceClass| {
        let dir = tempfile::tempdir().unwrap();
        let library = TestLibrary::new();
        let config = ServiceConfig {
            bind_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            mdns: None,
            ..ServiceConfig::new(dir.path(), name, class)
        };
        assert_eq!(config.flush_interval, Duration::from_secs(2));
        let service = SyncService::open(
            config,
            Arc::new(MemoryStore::new()),
            Arc::clone(&library) as Arc<dyn SyncLibrary>,
        )
        .unwrap();
        App {
            service,
            library,
            _dir: dir,
        }
    };
    let pc = slow("Desktop", DeviceClass::Desktop);
    let phone = slow("Phone", DeviceClass::Mobile);
    let invite = pc.service.invite().unwrap();
    phone.service.join(&invite.qr).unwrap();
    wait("pc sees the pairing", Duration::from_secs(5), || {
        pc.service.is_paired() && pc.service.is_running()
    });
    introduce(&pc, &phone);
    wait("phone links to the hub", Duration::from_secs(10), || {
        phone.service.hub().as_deref() == Some(pc.service.device_id())
    });
    let mut worst = Duration::ZERO;
    for (from, to) in [(&pc, &phone), (&phone, &pc)] {
        for second in 1..=5u64 {
            let position = 60_000 + second * 1_000;
            let start = Instant::now();
            from.library.play(&movie(700), position);
            from.service.flush();
            while to.library.position(&movie(700)).map(|p| p.0) != Some(position) {
                assert!(
                    start.elapsed() < Duration::from_secs(1),
                    "{position} took over a second"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            worst = worst.max(start.elapsed());
        }
    }
    println!("worst LAN progress latency: {worst:?}");
}

/// Waits up to 5 s for a `music` event matching `ok` and returns it.
fn music_event(
    events: &Receiver<ServiceEvent>,
    what: &str,
    ok: impl Fn(&RemoteMusic) -> bool,
) -> RemoteMusic {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(ServiceEvent::Music(m)) if ok(&m) => return m,
            Ok(_) => {}
            Err(_) => panic!("no {what}"),
        }
    }
}

#[test]
fn music_handoff_push_and_pull() {
    // MUS-015: the phone hands its queue to the TV (sealed, only the TV
    // gets it); the TV pulls it back and the phone answers in the same
    // session.
    let tv = app("TV", DeviceClass::Tv, None);
    let phone = app("Phone", DeviceClass::Mobile, None);
    assert_eq!(
        phone
            .service
            .music_send(tv.service.device_id(), MusicAction::Pull, None, None)
            .unwrap(),
        None,
        "nobody to hand to before pairing"
    );
    phone
        .service
        .join(&tv.service.invite().unwrap().qr)
        .unwrap();
    wait("tv running", Duration::from_secs(5), || {
        tv.service.is_running()
    });
    introduce(&tv, &phone);
    wait("phone links to the tv", Duration::from_secs(10), || {
        phone.service.hub().as_deref() == Some(tv.service.device_id())
    });
    let tv_events = tv.service.subscribe();
    let phone_events = phone.service.subscribe();

    let queue = r#"{"v":1,"tracks":[{"title":"Song","artist":"Band"}],"positionMs":61000}"#;
    let push = phone
        .service
        .music_send(tv.service.device_id(), MusicAction::Push, Some(queue), None)
        .unwrap()
        .unwrap();
    let got = music_event(&tv_events, "push", |m| m.action == MusicAction::Push);
    assert_eq!(got.session, push);
    assert_eq!(got.device_id, phone.service.device_id());
    assert_eq!(got.device_name.as_deref(), Some("Phone"));
    assert_eq!(got.queue.as_deref().map(String::as_str), Some(queue));
    assert!(!format!("{got:?}").contains("Song"));

    // A push without a queue is refused before it leaves.
    assert!(
        phone
            .service
            .music_send(tv.service.device_id(), MusicAction::Push, None, None)
            .is_err()
    );

    let pull = tv
        .service
        .music_send(phone.service.device_id(), MusicAction::Pull, None, None)
        .unwrap()
        .unwrap();
    let asked = music_event(&phone_events, "pull", |m| m.action == MusicAction::Pull);
    assert_eq!(asked.session, pull);
    assert_eq!(asked.queue, None);
    phone
        .service
        .music_send(
            tv.service.device_id(),
            MusicAction::Push,
            Some(queue),
            Some(&pull),
        )
        .unwrap();
    let back = music_event(&tv_events, "answer", |m| m.session == pull);
    assert_eq!(back.action, MusicAction::Push);

    // Frames for another device never reach this one.
    phone
        .service
        .music_send("someone-else", MusicAction::Pull, None, None)
        .unwrap();
    assert!(
        tv_events
            .recv_timeout(Duration::from_millis(500))
            .map(|e| !matches!(e, ServiceEvent::Music(_)))
            .unwrap_or(true)
    );
}

/// Phone and tablet of one family (paired by QR code, linked by hand).
fn family_of_two(mdns: Option<MdnsConfig>) -> (App, App) {
    let phone = app("Phone", DeviceClass::Mobile, mdns.clone());
    let tablet = app("Tablet", DeviceClass::Mobile, mdns);
    let invite = phone.service.invite().unwrap();
    tablet.service.join(&invite.qr).unwrap();
    wait("family of two runs", Duration::from_secs(5), || {
        phone.service.is_running() && tablet.service.is_running()
    });
    (phone, tablet)
}

/// SYNC-015: a new TV asks to join, the phone allows it with one tap
/// (after a stranger tried the invitation), and the TV gets progress.
/// Without the tap it gets nothing; a declined device is not shown again;
/// a flood of requests is capped.
#[test]
fn join_family_without_a_code_after_one_tap() {
    let (phone, tablet) = family_of_two(None);
    introduce(&phone, &tablet);
    let family = phone.service.family_id().unwrap();
    let tv = app("Living room TV", DeviceClass::Tv, None);
    let phone_events = phone.service.subscribe();

    // The TV asks; the phone sees it with the same short code.
    let shown = tv.service.join_ask(&family).unwrap();
    assert_eq!(shown.short_code.len(), 9);
    let txt = tv.service.join_ask_txt().unwrap();
    assert!(phone.service.note_join_request("Living room TV abc", &txt));
    assert!(!phone.service.note_join_request("Living room TV abc", &txt));
    saw(&phone_events, "JoinRequestsChanged", |e| {
        matches!(e, ServiceEvent::JoinRequestsChanged)
    });
    let requests = phone.service.join_requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.name, "Living room TV");
    assert_eq!(request.device_id, tv.service.device_id());
    assert_eq!(request.kind, pa_pairing::DeviceKind::Tv);
    assert_eq!(request.short_code, shown.short_code);
    assert!(!request.duplicate_name);

    // Without a tap: nothing reaches the TV.
    assert!(
        tv.service
            .join_ask_wait(Duration::from_millis(600), None)
            .unwrap()
            .is_none()
    );
    assert!(tv.service.devices().is_empty());
    assert!(!tv.service.is_running());

    // One tap. A stranger on the LAN knows the (public) code and races
    // to the invitation first: refused, it is not the allowed key.
    phone.service.join_allow(&request.request_id).unwrap();
    assert!(phone.service.join_requests().is_empty());
    assert_eq!(
        phone
            .service
            .join_allow(&request.request_id)
            .unwrap_err()
            .code,
        "expired"
    );
    let addr = phone.service.invite_addr().unwrap();
    let ask = pa_sync::lan::join::JoinAsk::from_txt(|k| {
        txt.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    })
    .unwrap();
    let stranger = pa_pairing::DeviceIdentity::generate("kstranger").unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let raced = rt.block_on(pa_sync::lan::pair::join(
        addr,
        pa_sync::lan::pair::JoinAs {
            identity: &stranger,
            name: "Living room TV",
            kind: pa_pairing::DeviceKind::Tv,
        },
        &ask.code(),
        None,
    ));
    assert!(raced.is_err(), "a stranger must not join");
    assert!(
        phone
            .service
            .devices()
            .iter()
            .all(|d| d.name != "Living room TV")
    );

    // The TV joins the family it asked for.
    let inviter = tv
        .service
        .join_ask_wait(Duration::from_secs(10), Some(addr))
        .unwrap()
        .expect("joined");
    assert_eq!(inviter.device_id, phone.service.device_id());
    assert!(tv.service.is_paired());
    assert_eq!(tv.service.family_id().as_deref(), Some(family.as_str()));
    assert!(tv.service.join_ask_txt().is_none());
    wait("phone admits the TV", Duration::from_secs(5), || {
        phone.service.devices().len() == 3
    });

    // Progress of the phone reaches the TV (the phone restarts its node
    // after admitting it: introduce them until they link).
    phone.library.play(&movie(15), 420_000);
    phone.service.flush();
    wait("progress reaches the TV", Duration::from_secs(15), || {
        introduce(&phone, &tv);
        std::thread::sleep(Duration::from_millis(200));
        tv.library.position(&movie(15)) == Some((420_000, false))
    });

    // A declined device is not shown again; a flood is capped.
    let pc = app("PC", DeviceClass::Desktop, None);
    pc.service.join_ask(&family).unwrap();
    let pc_txt = pc.service.join_ask_txt().unwrap();
    assert!(phone.service.note_join_request("PC 1", &pc_txt));
    let id = phone.service.join_requests()[0].request_id.clone();
    phone.service.join_decline(&id);
    assert!(!phone.service.note_join_request("PC 1", &pc_txt));
    assert!(phone.service.join_requests().is_empty());
    let mut flood = Vec::new();
    for n in 0..10 {
        let fake = app(&format!("Fake {n}"), DeviceClass::Mobile, None);
        fake.service.join_ask(&family).unwrap();
        phone
            .service
            .note_join_request(&format!("Fake {n}"), &fake.service.join_ask_txt().unwrap());
        flood.push(fake);
    }
    assert_eq!(
        phone.service.join_requests().len(),
        pa_sync::lan::join::MAX_PENDING
    );
    // Another family's request is not shown at all.
    let other = app("Other", DeviceClass::Desktop, None);
    other
        .service
        .join_ask("ffeeddccbbaa99887766554433221100")
        .unwrap();
    assert!(
        !tablet
            .service
            .note_join_request("Other 1", &other.service.join_ask_txt().unwrap())
    );
    // A paired device does not ask.
    assert_eq!(
        tablet.service.join_ask(&family).unwrap_err().code,
        "already_paired"
    );
}

/// SYNC-015 over mDNS: the new device sees the family, asks, every
/// trusted device shows the request, one allows it, the request goes away
/// everywhere and progress syncs.
#[test]
fn join_family_found_over_mdns() {
    let mdns = MdnsConfig {
        port: Some(private_port()),
        interfaces: Interfaces::LoopbackOnly,
        multicast_lock: None,
    };
    let (phone, tablet) = family_of_two(Some(mdns.clone()));
    let family = phone.service.family_id().unwrap();
    let tv = app("TV", DeviceClass::Tv, Some(mdns));
    let mut nearby = Vec::new();
    wait("TV sees the family", Duration::from_secs(15), || {
        nearby = tv.service.families_nearby();
        nearby.first().is_some_and(|f| f.devices.len() == 2)
    });
    assert_eq!(nearby[0].family_id, family);
    assert_eq!(nearby[0].devices, ["Phone", "Tablet"]);

    let shown = tv.service.join_ask(&family).unwrap();
    wait(
        "both members show the request",
        Duration::from_secs(15),
        || {
            [&phone, &tablet].iter().all(|m| {
                m.service
                    .join_requests()
                    .first()
                    .is_some_and(|r| r.short_code == shown.short_code)
            })
        },
    );
    let id = tablet.service.join_requests()[0].request_id.clone();
    tablet.service.join_allow(&id).unwrap();
    let inviter = tv
        .service
        .join_ask_wait(Duration::from_secs(15), None)
        .unwrap()
        .expect("joined over mDNS");
    assert_eq!(inviter.device_id, tablet.service.device_id());
    wait(
        "the request is gone everywhere",
        Duration::from_secs(15),
        || phone.service.join_requests().is_empty() && tablet.service.join_requests().is_empty(),
    );
    tablet.library.play(&movie(16), 90_000);
    tablet.service.flush();
    wait("progress reaches the TV", Duration::from_secs(20), || {
        tv.library.position(&movie(16)) == Some((90_000, false))
    });
}

/// SYNC-015 for the second device: a phone alone offers itself on mDNS,
/// a new TV sees «Phone» and asks; the phone allows it and founds the
/// family the TV asked for.
#[test]
fn second_device_joins_a_device_alone_over_mdns() {
    let mdns = MdnsConfig {
        port: Some(private_port()),
        interfaces: Interfaces::LoopbackOnly,
        multicast_lock: None,
    };
    let phone = app("Phone", DeviceClass::Mobile, Some(mdns.clone()));
    assert!(!phone.service.start().unwrap());
    let tv = app("TV", DeviceClass::Tv, Some(mdns));
    let mut nearby = Vec::new();
    wait("TV sees the phone", Duration::from_secs(15), || {
        nearby = tv.service.families_nearby();
        !nearby.is_empty()
    });
    assert_eq!(nearby[0].devices, ["Phone"]);
    assert_eq!(nearby[0].family_id, phone.service.offered_family_id());
    let shown = tv.service.join_ask(&nearby[0].family_id).unwrap();
    wait(
        "the phone shows the request",
        Duration::from_secs(15),
        || {
            phone
                .service
                .join_requests()
                .first()
                .is_some_and(|r| r.short_code == shown.short_code && r.name == "TV")
        },
    );
    let id = phone.service.join_requests()[0].request_id.clone();
    phone.service.join_allow(&id).unwrap();
    let inviter = tv
        .service
        .join_ask_wait(Duration::from_secs(15), None)
        .unwrap()
        .expect("joined");
    assert_eq!(inviter.device_id, phone.service.device_id());
    assert_eq!(phone.service.family_id(), Some(nearby[0].family_id.clone()));
    wait("both run", Duration::from_secs(5), || {
        phone.service.is_running() && tv.service.is_running()
    });
    // A paired device offers nothing any more.
    assert!(tv.service.families_nearby().is_empty());
}
