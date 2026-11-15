// SPDX-License-Identifier: MPL-2.0
//! NET-014: the client against a real Jellyfin in Docker with a generated
//! library (`tests/server/run.sh`). Needs `PA_JELLYFIN_URL`,
//! `PA_JELLYFIN_USER` and `PA_JELLYFIN_PASS` from `tests/server/setup.sh`.
#![cfg(feature = "server-tests")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use pa_jellyfin::{
    Client, DeviceCapabilities, DeviceInfo, Error, Item, ItemQuery, JellyfinFs, PlayMethod,
    PlaybackReport, PlaybackRequest, Session,
};
use pa_vfs::{ByteSource, EntryKind, Vfs, VfsPath};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set: run tests/server/run.sh"))
}

fn client(device_id: &str) -> Client {
    Client::new(
        &[env("PA_JELLYFIN_URL")],
        DeviceInfo::kinolune("pa-jellyfin tests", device_id),
    )
    .unwrap()
}

fn signed_in(device_id: &str) -> (Client, Session) {
    let c = client(device_id);
    let s = c
        .sign_in(&env("PA_JELLYFIN_USER"), &env("PA_JELLYFIN_PASS"))
        .unwrap();
    (c, s)
}

fn movie(c: &Client, user: &str, term: &str) -> Item {
    let found = c
        .items(
            user,
            &ItemQuery {
                types: vec!["Movie".into()],
                recursive: true,
                search: Some(term.into()),
                ..ItemQuery::default()
            },
        )
        .unwrap();
    assert_eq!(found.len(), 1, "{term}: {found:?}");
    found.into_iter().next().unwrap()
}

fn get(url: &str) -> Vec<u8> {
    let mut r = ureq::get(url).call().unwrap();
    assert_eq!(r.status().as_u16(), 200, "GET failed");
    r.body_mut()
        .with_config()
        .limit(64 << 20)
        .read_to_vec()
        .unwrap()
}

#[test]
fn public_info_and_secondary_addresses() {
    // Port 9 on loopback is closed: the client moves on to the real one.
    let c = Client::new(
        &["http://127.0.0.1:9".into(), env("PA_JELLYFIN_URL")],
        DeviceInfo::kinolune("pa-jellyfin tests", "failover"),
    )
    .unwrap();
    let info = c.public_info().unwrap();
    assert!(info.version.starts_with("10.10"), "{}", info.version);
    assert!(!info.id.is_empty());
    assert_eq!(c.active_address(), env("PA_JELLYFIN_URL"));
    // No address at all answers: unreachable, not a panic or a hang.
    let dead = Client::with_timeouts(
        &["http://127.0.0.1:9".into()],
        DeviceInfo::kinolune("t", "dead"),
        Duration::from_secs(2),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(matches!(dead.public_info(), Err(Error::Unreachable(_))));
}

#[test]
fn password_sign_in_users_and_libraries() {
    let bad = client("wrong-password");
    assert_eq!(
        bad.sign_in(&env("PA_JELLYFIN_USER"), "nope").unwrap_err(),
        Error::Unauthorized
    );
    let (c, s) = signed_in("password");
    assert_eq!(s.user.name, env("PA_JELLYFIN_USER"));
    assert!(!s.server_id.is_empty());
    assert!(!format!("{s:?}").contains(s.token.expose()));
    assert_eq!(c.me().unwrap().id, s.user.id);
    let users: Vec<String> = c.users().unwrap().into_iter().map(|u| u.name).collect();
    assert!(users.contains(&"viewer".to_owned()), "{users:?}");
    c.public_users().unwrap();
    let mut libs: Vec<(String, Option<String>)> = c
        .libraries(&s.user.id)
        .unwrap()
        .into_iter()
        .map(|l| (l.name, l.collection_type))
        .collect();
    libs.sort();
    assert_eq!(
        libs,
        [
            ("Movies".to_owned(), Some("movies".to_owned())),
            ("Music".to_owned(), Some("music".to_owned())),
            ("Shows".to_owned(), Some("tvshows".to_owned())),
        ]
    );
    // A token from a stored sign-in works on a new client.
    let again = client("password");
    again.set_token(Some(s.token.clone()));
    assert_eq!(again.me().unwrap().id, s.user.id);
    // Signing out revokes it.
    let (out, _) = signed_in("sign-out");
    let token = out.token().unwrap();
    out.sign_out().unwrap();
    let revoked = client("sign-out");
    revoked.set_token(Some(token));
    assert_eq!(revoked.me().unwrap_err(), Error::Unauthorized);
}

#[test]
fn quick_connect() {
    let tv = client("quick-connect-tv");
    assert!(tv.quick_connect_enabled().unwrap());
    let started = tv.quick_connect_start().unwrap();
    assert!(!started.authenticated);
    assert!(!started.code.is_empty() && !started.secret.is_empty());
    // Not approved yet: no token.
    assert!(tv.sign_in_quick_connect(&started.secret).is_err());

    let (phone, me) = signed_in("quick-connect-phone");
    phone.quick_connect_approve(&started.code).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !tv.quick_connect_state(&started.secret).unwrap().authenticated {
        assert!(Instant::now() < deadline, "never approved");
        std::thread::sleep(Duration::from_millis(200));
    }
    let s = tv.sign_in_quick_connect(&started.secret).unwrap();
    assert_eq!(s.user.id, me.user.id);
    assert_eq!(tv.me().unwrap().name, env("PA_JELLYFIN_USER"));
}

#[test]
fn favorites_played_resume_and_reports() {
    let (c, s) = signed_in("reports");
    let user = &s.user.id;
    let m = movie(&c, user, "Kinolune Test Movie");

    c.set_favorite(user, &m.id, true).unwrap();
    assert!(c.item(user, &m.id).unwrap().user_data.unwrap().is_favorite);
    assert!(c.favorites(user).unwrap().iter().any(|i| i.id == m.id));
    c.set_favorite(user, &m.id, false).unwrap();
    assert!(!c.favorites(user).unwrap().iter().any(|i| i.id == m.id));

    c.set_played(user, &m.id, true).unwrap();
    assert!(c.item(user, &m.id).unwrap().user_data.unwrap().played);
    c.set_played(user, &m.id, false).unwrap();
    assert!(!c.item(user, &m.id).unwrap().user_data.unwrap().played);

    let plan = c
        .playback_plan(user, &PlaybackRequest::new(&m.id, DeviceCapabilities::libmpv()))
        .unwrap();
    assert_eq!(plan.method, PlayMethod::DirectPlay);
    let mut report = PlaybackReport {
        item_id: m.id.clone(),
        media_source_id: plan.media_source_id.clone(),
        play_session_id: plan.play_session_id.clone(),
        method: plan.method,
        position_ms: 0,
        paused: false,
    };
    c.report_start(&report).unwrap();
    report.position_ms = 10_000;
    c.report_progress(&report).unwrap();
    report.position_ms = 20_000;
    report.paused = true;
    c.report_progress(&report).unwrap();
    c.report_stopped(&report).unwrap();

    let data = c.item(user, &m.id).unwrap().user_data.unwrap();
    assert!(
        (data.position_ms() - 20_000).abs() <= 1_000,
        "position {}",
        data.position_ms()
    );
    assert!(!data.played);
    let resume = c.resume(user, 20).unwrap();
    assert!(resume.iter().any(|i| i.id == m.id), "not in Continue Watching");

    c.remove_from_resume(user, &m.id).unwrap();
    assert!(!c.resume(user, 20).unwrap().iter().any(|i| i.id == m.id));
    let data = c.item(user, &m.id).unwrap().user_data.unwrap();
    assert_eq!(data.playback_position_ticks, 0);
}

/// Path of a line of an HLS playlist at `playlist` (both server paths).
fn resolve(playlist: &str, line: &str) -> String {
    if line.starts_with('/') {
        return line.to_owned();
    }
    let dir = playlist
        .split('?')
        .next()
        .unwrap()
        .rsplit_once('/')
        .unwrap()
        .0;
    format!("{dir}/{line}")
}

fn first_uri(playlist: &str) -> String {
    playlist
        .lines()
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .expect("playlist without entries")
        .to_owned()
}

#[test]
fn direct_play_direct_stream_and_transcode() {
    let (c, s) = signed_in("playback");
    let user = &s.user.id;
    let m = movie(&c, user, "Kinolune Test Movie");

    // libmpv plays the mkv as is.
    let direct = c
        .playback_plan(user, &PlaybackRequest::new(&m.id, DeviceCapabilities::libmpv()))
        .unwrap();
    assert_eq!(direct.method, PlayMethod::DirectPlay);
    let bytes = get(&c.url_with_key(&direct.stream_path));
    assert_eq!(bytes.len() as i64, m.media_sources[0].size.unwrap());

    // Only MP4 files: the server repackages the H.264/AAC streams as HLS.
    let mut mp4_only = DeviceCapabilities::libmpv();
    mp4_only.video_containers = vec!["mp4".into()];
    let remux = c
        .playback_plan(user, &PlaybackRequest::new(&m.id, mp4_only))
        .unwrap();
    assert_eq!(remux.method, PlayMethod::DirectStream, "{:?}", remux.transcode_reasons);
    let master = String::from_utf8(get(&c.url_with_key(&remux.stream_path))).unwrap();
    assert!(master.starts_with("#EXTM3U"), "{master}");
    stop(&c, &remux);

    // No H.264 decoder: the server re-encodes; the first segment arrives.
    let mut hevc_only = DeviceCapabilities::libmpv();
    hevc_only.video_codecs = vec!["hevc".into()];
    let transcode = c
        .playback_plan(user, &PlaybackRequest::new(&m.id, hevc_only))
        .unwrap();
    assert_eq!(transcode.method, PlayMethod::Transcode);
    assert!(
        transcode
            .transcode_reasons
            .iter()
            .any(|r| r == "VideoCodecNotSupported"),
        "{:?}",
        transcode.transcode_reasons
    );
    let master = String::from_utf8(get(&c.url_with_key(&transcode.stream_path))).unwrap();
    let variant_path = resolve(&transcode.stream_path, &first_uri(&master));
    let variant = String::from_utf8(get(&c.url_with_key(&variant_path))).unwrap();
    let segment = get(&c.url_with_key(&resolve(&variant_path, &first_uri(&variant))));
    // MPEG-TS packets start with the sync byte.
    assert_eq!(segment.first(), Some(&0x47), "{} bytes", segment.len());
    stop(&c, &transcode);

    // A bitrate limit below the file's: transcode as well.
    let mut slow = DeviceCapabilities::libmpv();
    slow.max_bitrate = Some(150_000);
    let limited = c
        .playback_plan(user, &PlaybackRequest::new(&m.id, slow))
        .unwrap();
    assert_eq!(limited.method, PlayMethod::Transcode);
    stop(&c, &limited);

    // Asked for: transcode even though it plays directly.
    let mut forced = PlaybackRequest::new(&m.id, DeviceCapabilities::libmpv());
    forced.force_transcode = true;
    let forced = c.playback_plan(user, &forced).unwrap();
    assert_eq!(forced.method, PlayMethod::Transcode);
    stop(&c, &forced);
}

fn stop(c: &Client, plan: &pa_jellyfin::PlaybackPlan) {
    c.report_stopped(&PlaybackReport {
        item_id: plan.item_id.clone(),
        media_source_id: plan.media_source_id.clone(),
        play_session_id: plan.play_session_id.clone(),
        method: plan.method,
        position_ms: 0,
        paused: false,
    })
    .unwrap();
}

#[test]
fn trickplay_and_segments() {
    let (c, s) = signed_in("trickplay");
    let m = movie(&c, &s.user.id, "Kinolune Test Movie");
    let source = &m.media_sources[0].id;
    let t = Client::trickplay_of(&m, source, 320).expect("the server made trickplay");
    assert_eq!(t.width, 320);
    assert!(t.thumbnail_count >= 3 && t.sheets() >= 1, "{t:?}");
    let sheet = c.trickplay_sheet(&m.id, source, t.width, 0).unwrap();
    assert_eq!(&sheet[..2], &[0xFF, 0xD8]);
    assert_eq!(
        c.trickplay_sheet(&m.id, source, t.width, t.sheets() + 5)
            .unwrap_err(),
        Error::NotFound
    );
    // No segment provider on a fresh server: an empty list, not an error.
    assert!(c.media_segments(&m.id).unwrap().is_empty());
}

#[test]
fn libraries_as_a_file_tree() {
    let (c, s) = signed_in("tree");
    let user = s.user.id.clone();
    let m = movie(&c, &user, "Kinolune Test Movie");
    let fs = JellyfinFs::new(Arc::new(c), user);
    let names = |p: &str| -> Vec<String> {
        let mut n: Vec<String> = fs
            .list(&VfsPath::parse(p).unwrap())
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        n.sort();
        n
    };
    assert_eq!(names("/"), ["Movies", "Music", "Shows"]);
    assert_eq!(
        names("/Movies"),
        ["Kinolune Second Film (2019)", "Kinolune Test Movie (2020)"]
    );
    let dir = "/Movies/Kinolune Test Movie (2020)";
    let listing = names(dir);
    for want in [
        "Kinolune Test Movie (2020).mkv",
        "Kinolune Test Movie (2020).jellyfin.json",
        "Kinolune Test Movie (2020).trickplay",
        "poster.jpg",
    ] {
        assert!(listing.contains(&want.to_owned()), "{want}: {listing:?}");
    }

    let file = VfsPath::parse(&format!("{dir}/Kinolune Test Movie (2020).mkv")).unwrap();
    let meta = fs.stat(&file).unwrap();
    assert_eq!(meta.kind, EntryKind::File);
    assert_eq!(meta.size as i64, m.media_sources[0].size.unwrap());
    let media = fs.media_at(&file).unwrap().unwrap();
    assert_eq!(media.item_id, m.id);
    assert_eq!(fs.path_of(&m.id).unwrap(), Some(file.clone()));

    // Byte ranges match the file the server streams.
    let whole = get(&fs.client().url_with_key(&pa_jellyfin::static_stream_path(
        &m.id,
        &media.media_source_id,
        false,
    )));
    let reader = fs.open(&file).unwrap();
    assert_eq!(reader.size(), whole.len() as u64);
    for offset in [0u64, 1_000_000, whole.len() as u64 - 100] {
        let mut buf = vec![0u8; 100];
        reader.read_exact_at(offset, &mut buf).unwrap();
        let o = offset as usize;
        assert_eq!(buf, whole[o..o + 100]);
    }

    let poster = fs
        .open(&VfsPath::parse(&format!("{dir}/poster.jpg")).unwrap())
        .unwrap();
    let mut head = [0u8; 2];
    poster.read_exact_at(0, &mut head).unwrap();
    assert_eq!(head, [0xFF, 0xD8]);

    // The server's trickplay reads as a Jellyfin trickplay folder
    // (ADR-0048).
    let vfs: Arc<dyn Vfs> = Arc::new(fs);
    let sidecar = pa_trickplay::sidecar::find(&vfs, &file, media.duration_ms.map(|d| d as u64))
        .unwrap()
        .expect("trickplay sidecar");
    let pa_trickplay::sidecar::Sidecar::Jellyfin(sheets) = sidecar else {
        panic!("not a Jellyfin folder");
    };
    assert!(sheets.tile_at(15_000).unwrap().is_some());

    let season = "/Shows/Kinolune Test Show/Season 01";
    let episodes: Vec<String> = names_of(vfs.as_ref(), season)
        .into_iter()
        .filter(|n| n.ends_with(".mkv"))
        .collect();
    assert_eq!(episodes.len(), 2, "{episodes:?}");
    assert!(episodes[0].starts_with("Kinolune Test Show S01E01"), "{episodes:?}");
    let album = "/Music/Kinolune Test Artist/Kinolune Test Album";
    assert_eq!(
        names_of(vfs.as_ref(), album)
            .into_iter()
            .filter(|n| n.ends_with(".mp3"))
            .collect::<Vec<_>>(),
        ["01 - First Song.mp3", "02 - Second Song.mp3"]
    );
    assert_eq!(
        vfs.list(&VfsPath::parse("/Nope").unwrap()).unwrap_err().kind(),
        pa_vfs::ErrorKind::NotFound
    );
}

fn names_of(vfs: &dyn Vfs, dir: &str) -> Vec<String> {
    let mut n: Vec<String> = vfs
        .list(&VfsPath::parse(dir).unwrap())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    n.sort();
    n
}
