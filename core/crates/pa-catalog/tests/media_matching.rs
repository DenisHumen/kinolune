// SPDX-License-Identifier: MPL-2.0
//! SYNC-004: the same film or episode on two devices is one item, whatever
//! the paths, sources and versions.
//!
//! The main test runs two instances (each with its own catalog, keys and
//! LAN node on loopback): a phone with a 1080p web copy of "The Matrix" and
//! a 720p episode of "Dark" in its download folders, and a NAS that holds
//! the film in 4K plus the phone's 1080p copy and the episode in 4K, in
//! another folder layout and another source. The phone recognised the film
//! (TMDB), the NAS did not; both know the show. Progress, "watched",
//! "Continue watching" and handoff (heartbeat, takeover) still meet.
//!
//! The other tests work on two catalogs without a network (ops passed by
//! hand): another cut of a film, progress from before recognition and
//! from peers without SYNC-004, AniDB keys and the ed2k check.

use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pa_catalog::pa_sync::handoff::{self, ResumePoint};
use pa_catalog::pa_sync::hlc::system_ms;
use pa_catalog::pa_sync::lan::service::{ServiceConfig, ServiceEvent, SyncLibrary, SyncService};
use pa_catalog::pa_sync::proto::PlayState;
use pa_catalog::pa_sync::{Op, OpBody, OpId};
use pa_catalog::{Catalog, Ed2kOptions, ItemRef};
use pa_discovery::DeviceClass;
use pa_library::rusqlite::{Connection, params};
use pa_model::MediaKey;
use pa_scanner::CancelToken;
use pa_secrets::MemoryStore;

const MATRIX_MS: i64 = 8_160_000;
const DARK_MS: i64 = 3_060_000;

struct Lib(Arc<Catalog>);

impl SyncLibrary for Lib {
    fn enable(&self, device_id: &str) -> Result<(), String> {
        self.0.enable_sync(device_id).map_err(|e| e.to_string())
    }
    fn disable(&self) -> Result<(), String> {
        self.0.disable_sync().map_err(|e| e.to_string())
    }
    fn pending(&self) -> Result<Vec<Op>, String> {
        self.0.pending_ops(512).map_err(|e| e.to_string())
    }
    fn ack(&self, op_id: OpId) -> Result<(), String> {
        self.0.ack_op(op_id).map(drop).map_err(|e| e.to_string())
    }
    fn apply(&self, op: &Op) -> Result<bool, String> {
        self.0.apply_remote(op).map_err(|e| e.to_string())
    }
}

/// Content of a test file: `len` bytes from `seed`.
fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn write(root: &Path, rel: &str, data: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

/// A catalog over `files` (path, content) in a source named `source`.
fn library(dir: &Path, source: &str, files: &[(&str, Vec<u8>)]) -> Arc<Catalog> {
    let root = dir.join(source.to_lowercase());
    for (rel, data) in files {
        write(&root, rel, data);
    }
    let catalog = Arc::new(Catalog::open(dir.join("db/library.db")).unwrap());
    catalog
        .add_source(None, "local", source, root.to_str().unwrap(), None)
        .unwrap();
    catalog
        .scan(None, &CancelToken::new(), &mut |_| {})
        .unwrap();
    catalog
}

/// Sets the duration of the files whose path ends with `suffix` (what
/// probing would find; the test files are not real video).
fn set_duration(cat: &Catalog, suffix: &str, ms: i64) {
    let conn = Connection::open(cat.path()).unwrap();
    let n = conn
        .execute(
            "UPDATE files SET duration_ms = ?2 WHERE path LIKE '%' || ?1",
            params![suffix, ms],
        )
        .unwrap();
    assert_eq!(n, 1, "{suffix}");
}

fn movie(cat: &Catalog, title: &str) -> ItemRef {
    let movies = cat.movies().unwrap();
    movies
        .iter()
        .find(|m| m.title.contains(title))
        .unwrap_or_else(|| panic!("no {title} in {movies:?}"))
        .id
}

fn episode(cat: &Catalog, show: &str) -> ItemRef {
    let shows = cat.shows().unwrap();
    let show = shows
        .iter()
        .find(|s| s.title.contains(show))
        .unwrap_or_else(|| panic!("no {show} in {shows:?}"));
    cat.episodes(show.id).unwrap()[0].id
}

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out: {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn now() -> i64 {
    i64::try_from(system_ms()).unwrap()
}

struct Device {
    catalog: Arc<Catalog>,
    service: SyncService,
    _dir: tempfile::TempDir,
}

fn device(name: &str, class: DeviceClass, source: &str, files: &[(&str, Vec<u8>)]) -> Device {
    let dir = tempfile::tempdir().unwrap();
    let catalog = library(dir.path(), source, files);
    let config = ServiceConfig {
        bind_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
        mdns: None,
        flush_interval: Duration::from_millis(200),
        ..ServiceConfig::new(dir.path().join("sync"), name, class)
    };
    let service = SyncService::open(
        config,
        Arc::new(MemoryStore::new()),
        Arc::new(Lib(Arc::clone(&catalog))),
    )
    .unwrap();
    Device {
        catalog,
        service,
        _dir: dir,
    }
}

/// Where `d` continues `item` (what `CoreSync::resume_hint` does): its
/// stored progress and the others' last heartbeat for any key of the item,
/// carried onto its own version.
fn resume(d: &Device, item: ItemRef) -> Option<(u64, String)> {
    let profile = d.catalog.sync_profile().unwrap();
    let keys = d.catalog.sync_keys(item).unwrap().unwrap();
    let local = d.catalog.resume_point(item).unwrap();
    let duration = d.catalog.local_duration_ms(item).unwrap();
    let remote = d
        .service
        .remote_playback_keys(&profile, &keys)
        .map(|u| ResumePoint::from_update(&u).for_version(duration));
    let point = handoff::newest(local.into_iter().chain(remote))?;
    Some((point.position_at(system_ms()), point.device().to_owned()))
}

#[test]
fn same_title_on_two_devices_with_other_paths_sources_and_versions() {
    let matrix_1080 = bytes(150_000, 2);
    let phone = device(
        "Phone",
        DeviceClass::Mobile,
        "Phone",
        &[
            (
                "Download/The.Matrix.1999.1080p.WEB.mkv",
                matrix_1080.clone(),
            ),
            (
                "Download/TV/Dark/Season 1/Dark.S01E01.720p.mkv",
                bytes(150_000, 3),
            ),
        ],
    );
    let nas = device(
        "NAS",
        DeviceClass::Server,
        "Library",
        &[
            (
                "Films/The Matrix (1999)/The.Matrix.1999.2160p.UHD.mkv",
                bytes(160_000, 4),
            ),
            (
                "Films/The Matrix (1999)/The.Matrix.1999.1080p.WEB.mkv",
                matrix_1080,
            ),
            (
                "Series/Dark (2017)/Season 01/Dark.S01E01.2160p.mkv",
                bytes(170_000, 5),
            ),
        ],
    );
    let (p_movie, p_ep) = (
        movie(&phone.catalog, "Matrix"),
        episode(&phone.catalog, "Dark"),
    );
    let (n_movie, n_ep) = (movie(&nas.catalog, "Matrix"), episode(&nas.catalog, "Dark"));
    assert_eq!(
        nas.catalog.versions(n_movie).unwrap().len(),
        2,
        "4K and 1080p"
    );
    // The phone recognised the film, the NAS did not; both know the show.
    phone
        .catalog
        .fix_match(p_movie, None, None, Some(603))
        .unwrap();
    phone
        .catalog
        .fix_match(p_ep, None, None, Some(70523))
        .unwrap();
    nas.catalog
        .fix_match(n_ep, None, None, Some(70523))
        .unwrap();
    set_duration(&phone.catalog, "1080p.WEB.mkv", MATRIX_MS);
    set_duration(&phone.catalog, "S01E01.720p.mkv", DARK_MS);
    set_duration(&nas.catalog, "2160p.UHD.mkv", MATRIX_MS + 400);
    set_duration(&nas.catalog, "1080p.WEB.mkv", MATRIX_MS);
    set_duration(&nas.catalog, "S01E01.2160p.mkv", DARK_MS + 1_000);

    // Different keys for the film: TMDB on the phone, `file:` on the NAS.
    let p_keys = phone.catalog.sync_keys(p_movie).unwrap().unwrap();
    let n_keys = nas.catalog.sync_keys(n_movie).unwrap().unwrap();
    assert_eq!(p_keys.key, MediaKey::TmdbMovie { id: 603 });
    assert!(matches!(n_keys.key, MediaKey::File { .. }));
    assert_eq!(n_keys.aliases.len(), 1, "the other version: {n_keys:?}");
    // One key for the episode: the TMDB episode, other files.
    let ep_key = MediaKey::TmdbEpisode {
        show_id: 70523,
        season: 1,
        episode: 1,
    };
    assert_eq!(phone.catalog.sync_key(p_ep).unwrap(), Some(ep_key.clone()));
    assert_eq!(nas.catalog.sync_key(n_ep).unwrap(), Some(ep_key));

    // Pair; the NAS (a server) is the hub.
    let invite = nas.service.invite().unwrap();
    phone.service.join(&invite.qr).unwrap();
    wait("nas running", || nas.service.is_running());
    let (a, b) = (
        nas.service.sync_addr().unwrap(),
        phone.service.sync_addr().unwrap(),
    );
    nas.service
        .peer_up(phone.service.device_id(), DeviceClass::Mobile, b);
    phone
        .service
        .peer_up(nas.service.device_id(), DeviceClass::Server, a);
    wait("phone links to the NAS", || {
        phone.service.hub().as_deref() == Some(nas.service.device_id())
    });

    // Progress on the phone reaches the NAS's film (4K version) and episode.
    let t = now();
    assert!(
        phone
            .catalog
            .save_progress(p_movie, 600_000, MATRIX_MS, t, false)
            .unwrap()
    );
    assert!(
        phone
            .catalog
            .save_progress(p_ep, 1_200_000, DARK_MS, t + 1, false)
            .unwrap()
    );
    phone.service.flush();
    wait("film progress reaches the NAS", || {
        nas.catalog
            .progress(n_movie)
            .unwrap()
            .is_some_and(|p| p.position_ms == 600_000)
    });
    wait("episode progress reaches the NAS", || {
        nas.catalog
            .progress(n_ep)
            .unwrap()
            .is_some_and(|p| p.position_ms == 1_200_000)
    });
    let cont = nas.catalog.continue_watching(10).unwrap();
    let ids: Vec<ItemRef> = cont.iter().map(|c| c.item.id).collect();
    assert_eq!(ids, [n_ep, n_movie], "each once, newest first");
    let point = nas.catalog.resume_point(n_movie).unwrap().unwrap();
    assert_eq!(point.position_ms, 600_000, "the same cut: the same second");

    // Handoff: the phone plays the film and sends heartbeats under its key;
    // the NAS finds them by its own keys and continues within 2 s.
    let profile = phone.catalog.sync_profile().unwrap();
    let start = 610_000;
    let t0 = system_ms();
    let position = |at: u64| start + at.saturating_sub(t0);
    phone
        .service
        .heartbeat_keys(
            &profile,
            &p_keys,
            position(system_ms()),
            Some(MATRIX_MS as u64),
            PlayState::Playing,
        )
        .unwrap();
    wait("the NAS hears the heartbeat", || {
        nas.service
            .remote_playback_keys(&profile, &n_keys)
            .is_some()
    });
    std::thread::sleep(Duration::from_millis(1_000));
    let (pos, by) = resume(&nas, n_movie).unwrap();
    let error = pos.abs_diff(position(system_ms()));
    assert_eq!(by, phone.service.device_id());
    assert!(error <= 2_000, "resume off by {error} ms");
    let playing = nas.service.playing_elsewhere();
    assert_eq!(
        nas.catalog
            .item_for_keys(&playing[0].key, &playing[0].aliases)
            .unwrap(),
        Some(n_movie),
        "the banner names the NAS's film"
    );

    // Takeover from the NAS, under its `file:` key: the phone knows the film.
    let events = phone.service.subscribe();
    nas.service.start_playing_keys(&profile, &n_keys).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if let ServiceEvent::Takeover(t) = events.recv_timeout(left).expect("takeover") {
            assert_eq!(t.key, n_keys.key);
            assert_eq!(
                phone.catalog.item_for_keys(&t.key, &t.aliases).unwrap(),
                Some(p_movie)
            );
            break;
        }
    }

    // Back the other way: the NAS's position and "watched" reach the phone.
    let t = now();
    assert!(
        nas.catalog
            .save_progress(n_movie, 900_000, MATRIX_MS + 400, t, false)
            .unwrap()
    );
    assert!(
        nas.catalog
            .save_progress(n_ep, 3_000_000, DARK_MS + 1_000, t + 1, true)
            .unwrap()
    );
    nas.service.flush();
    wait("film position reaches the phone", || {
        phone
            .catalog
            .progress(p_movie)
            .unwrap()
            .is_some_and(|p| p.position_ms == 900_000)
    });
    wait("episode watched reaches the phone", || {
        phone
            .catalog
            .progress(p_ep)
            .unwrap()
            .is_some_and(|p| p.watched)
    });
    let cont = phone.catalog.continue_watching(10).unwrap();
    let ids: Vec<ItemRef> = cont.iter().map(|c| c.item.id).collect();
    assert_eq!(ids, [p_movie], "the finished episode left the row");
}

/// Two catalogs with sync on and ops passed by hand.
struct Pair {
    a: Arc<Catalog>,
    b: Arc<Catalog>,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

fn pair(a_files: &[(&str, Vec<u8>)], b_files: &[(&str, Vec<u8>)]) -> Pair {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = library(da.path(), "Phone", a_files);
    let b = library(db.path(), "NAS", b_files);
    a.enable_sync("phone").unwrap();
    b.enable_sync("nas").unwrap();
    Pair {
        a,
        b,
        _dirs: (da, db),
    }
}

/// Moves the queued ops of `from` to `to`.
fn pass(from: &Catalog, to: &Catalog) {
    for op in from.pending_ops(100).unwrap() {
        to.apply_remote(&op).unwrap();
        from.ack_op(op.op_id).unwrap();
    }
}

#[test]
fn progress_follows_the_work_and_carries_exactly_only_for_the_same_cut() {
    let p = pair(
        &[("Movies/The.Matrix.1999.1080p.mkv", bytes(150_000, 2))],
        &[(
            "Movies/The Matrix (1999)/The.Matrix.1999.Extended.2160p.mkv",
            bytes(150_000, 9),
        )],
    );
    let (ma, mb) = (movie(&p.a, "Matrix"), movie(&p.b, "Matrix"));
    p.a.fix_match(ma, None, None, Some(603)).unwrap();
    p.b.fix_match(mb, None, None, Some(603)).unwrap();
    set_duration(&p.a, "1080p.mkv", MATRIX_MS);
    // Thirty minutes longer: another cut.
    let extended = MATRIX_MS + 1_800_000;
    set_duration(&p.b, "2160p.mkv", extended);

    p.a.save_progress(ma, MATRIX_MS / 2, MATRIX_MS, now(), false)
        .unwrap();
    pass(&p.a, &p.b);
    // Half the film there is half the film here.
    let got = p.b.progress(mb).unwrap().unwrap();
    assert_eq!((got.position_ms, got.duration_ms), (extended / 2, extended));
    let point = p.b.resume_point(mb).unwrap().unwrap();
    assert_eq!(point.position_ms, u64::try_from(extended / 2).unwrap());
    let cont = p.b.continue_watching(10).unwrap();
    assert_eq!(cont[0].progress.position_ms, extended / 2);

    // The same cut in another encoding (800 ms longer): the same second.
    let conn = Connection::open(p.b.path()).unwrap();
    conn.execute("UPDATE files SET duration_ms = ?1", [MATRIX_MS + 800])
        .unwrap();
    let got = p.b.progress(mb).unwrap().unwrap();
    assert_eq!(got.position_ms, MATRIX_MS / 2);
    assert_eq!(
        p.b.resume_point(mb).unwrap().unwrap().position_ms,
        u64::try_from(MATRIX_MS / 2).unwrap()
    );
}

#[test]
fn file_progress_from_before_recognition_and_from_old_peers_follows_the_item() {
    let data = bytes(150_000, 2);
    let p = pair(
        &[("Movies/The.Matrix.1999.1080p.mkv", data.clone())],
        &[("Films/Matrix/The Matrix (1999).mkv", data)],
    );
    let (ma, mb) = (movie(&p.a, "Matrix"), movie(&p.b, "Matrix"));
    let file_key = p.a.sync_key(ma).unwrap().unwrap();
    assert!(matches!(file_key, MediaKey::File { .. }));

    // Watched halfway while unrecognised: stored under `file:`.
    let t = now();
    p.a.save_progress(ma, 1_000_000, MATRIX_MS, t, false)
        .unwrap();
    p.a.fix_match(ma, None, None, Some(603)).unwrap();
    assert_eq!(
        p.a.sync_key(ma).unwrap(),
        Some(MediaKey::TmdbMovie { id: 603 })
    );
    assert_eq!(p.a.progress(ma).unwrap().unwrap().position_ms, 1_000_000);
    assert_eq!(p.a.continue_watching(10).unwrap().len(), 1);
    // The next report goes under the TMDB key; the item is in the row once.
    p.a.save_progress(ma, 2_000_000, MATRIX_MS, t + 10, false)
        .unwrap();
    assert_eq!(p.a.progress(ma).unwrap().unwrap().position_ms, 2_000_000);
    let cont = p.a.continue_watching(10).unwrap();
    assert_eq!(cont.len(), 1);
    assert_eq!(cont[0].progress.position_ms, 2_000_000);

    // A peer without SYNC-004 sends `file:` and no aliases: still this film.
    p.b.save_progress(mb, 3_000_000, MATRIX_MS, t + 20, false)
        .unwrap();
    let ops = p.b.pending_ops(10).unwrap();
    assert_eq!(ops.len(), 1);
    let old_peer = Op {
        aliases: Vec::new(),
        ..ops[0].clone()
    };
    assert!(
        matches!(&old_peer.body, OpBody::ProgressUpsert { key, .. } if *key == file_key),
        "{old_peer:?}"
    );
    assert!(p.a.apply_remote(&old_peer).unwrap());
    assert_eq!(p.a.item_for_key(&file_key).unwrap(), Some(ma));
    assert_eq!(p.a.progress(ma).unwrap().unwrap().position_ms, 3_000_000);
}

#[test]
fn anidb_episode_keys_and_the_ed2k_check() {
    // Two releases of one anime episode: different files, no TMDB.
    let p = pair(
        &[(
            "Anime/Hoshizora Relay/Season 1/Hoshizora.Relay.S01E05.mkv",
            bytes(150_000, 6),
        )],
        &[(
            "Shows/Hoshizora Relay/Season 1/Hoshizora Relay - S01E05 [BD].mkv",
            bytes(180_000, 7),
        )],
    );
    let (ea, eb) = (episode(&p.a, "Hoshizora"), episode(&p.b, "Hoshizora"));
    for cat in [&p.a, &p.b] {
        let conn = Connection::open(cat.path()).unwrap();
        conn.execute(
            "INSERT INTO external_ids (work_id, provider, ext_kind, ext_id)
             SELECT work_id, 'anidb', '', '99901' FROM episodes",
            [],
        )
        .unwrap();
    }
    let anidb = MediaKey::AnidbEpisode {
        aid: 99901,
        episode: 5,
    };
    let keys = p.a.sync_keys(ea).unwrap().unwrap();
    assert!(
        matches!(keys.key, MediaKey::File { .. }),
        "v1 peers read `file:`"
    );
    assert_eq!(keys.aliases, std::slice::from_ref(&anidb));
    assert_eq!(p.b.item_for_key(&anidb).unwrap(), Some(eb));

    p.a.save_progress(ea, 400_000, 1_440_000, now(), false)
        .unwrap();
    pass(&p.a, &p.b);
    assert_eq!(p.b.progress(eb).unwrap().unwrap().position_ms, 400_000);
    assert_eq!(p.b.item_for_key(&keys.key).unwrap(), Some(eb), "linked");

    // ed2k: equal size, head and tail (one OSHash), another middle.
    let x = bytes(200_000, 1);
    let mut y = x.clone();
    y[100_000] ^= 0xff;
    let q = pair(
        &[("Movies/Heat.1995.mkv", x.clone())],
        &[("Movies/Heat (1995)/Heat.1995.mkv", y)],
    );
    let r = pair(
        &[("Movies/Heat.1995.mkv", x.clone())],
        &[("Films/Heat 1995.mkv", x)],
    );
    for cat in [&q.a, &q.b, &r.a, &r.b] {
        cat.hash_ed2k(&Ed2kOptions::default(), &CancelToken::new(), &mut |_| {})
            .unwrap();
    }
    let qa = q.a.sync_keys(movie(&q.a, "Heat")).unwrap().unwrap();
    let qb = q.b.sync_keys(movie(&q.b, "Heat")).unwrap().unwrap();
    assert_eq!(qa.key, qb.key, "one OSHash");
    assert_ne!(qa.aliases, qb.aliases, "two ed2k hashes");
    let alias_texts = qa.alias_texts();
    assert!(alias_texts[0].starts_with("ed2k:"));
    // With the sender's ed2k the other file is not it; without, as before.
    assert_eq!(q.b.item_for_keys(&qa.key, &alias_texts).unwrap(), None);
    assert!(q.b.item_for_keys(&qa.key, &[]).unwrap().is_some());
    // The same file passes the check, and its ed2k key alone finds it.
    let ra = r.a.sync_keys(movie(&r.a, "Heat")).unwrap().unwrap();
    let rb_movie = movie(&r.b, "Heat");
    assert_eq!(
        r.b.item_for_keys(&ra.key, &ra.alias_texts()).unwrap(),
        Some(rb_movie)
    );
    assert_eq!(r.b.item_for_key(&ra.aliases[0]).unwrap(), Some(rb_movie));
}
