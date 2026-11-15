// SPDX-License-Identifier: MPL-2.0
//! NET-014: a Jellyfin server as a library source. Signing in keeps only
//! the token; the scan turns the server's libraries into movies, shows,
//! episodes and tracks with the server's posters; files stream through
//! the loopback proxy byte for byte; playback reports, favorites and
//! "Continue Watching" reach the server. Run with
//! `core/crates/pa-jellyfin/tests/server/run.sh`.
#![cfg(feature = "server-tests")]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;

use pa_catalog::{Catalog, SourceOpener, SourcesConnector};
use pa_proxy::{Proxy, ProxyOptions};
use pa_scanner::CancelToken;
use pa_secrets::{MemoryStore, SecretString};
use pa_sources::pa_jellyfin::DeviceCapabilities;
use pa_sources::{JellyfinAccount, ReportEvent, SourceKind, SourceManager};
use pa_vfs::VfsPath;

fn var(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: use pa-jellyfin/tests/server/run.sh"))
}

fn get(proxy: &Proxy, url: &str, range: &str) -> (u16, Vec<u8>) {
    let rest = url.strip_prefix("http://").unwrap();
    let path = &rest[rest.find('/').unwrap()..];
    let mut s = TcpStream::connect(proxy.local_addr()).unwrap();
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: x\r\nRange: {range}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut reply = Vec::new();
    s.read_to_end(&mut reply).unwrap();
    let split = reply.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&reply[..split]).into_owned();
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        reply[split + 4..].to_vec(),
    )
}

#[test]
fn jellyfin_server_is_a_library_source() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let cat = Arc::new(Catalog::open(&db).unwrap());
    let manager = Arc::new(SourceManager::open(&db, Arc::new(MemoryStore::new())).unwrap());
    cat.set_connector(Arc::new(SourcesConnector::new(Arc::clone(&manager))));

    let password = SecretString::new(var("PA_JELLYFIN_PASS"));
    let account = JellyfinAccount::sign_in(
        &[var("PA_JELLYFIN_URL")],
        "pa-catalog tests",
        &var("PA_JELLYFIN_USER"),
        &password,
    )
    .unwrap();
    let token = account.session.token.expose().to_owned();
    let source = manager.create_jellyfin(account, None, Vec::new()).unwrap();
    assert_eq!(source.kind, SourceKind::Jellyfin);
    assert!(source.has_password);
    // The database holds neither the password nor the token.
    let raw = std::fs::read(&db).unwrap();
    for secret in [password.expose(), token.as_str()] {
        assert!(!raw.windows(secret.len()).any(|w| w == secret.as_bytes()));
    }
    let id = source.id;
    assert!(manager.check(id).unwrap().online == Some(true));

    let mut errors = Vec::new();
    cat.scan(Some(id), &CancelToken::new(), &mut |u| errors.extend(u.error))
        .unwrap();
    assert_eq!(errors, Vec::<String>::new());

    let mut movies: Vec<_> = cat.movies().unwrap();
    movies.sort_by(|a, b| a.title.cmp(&b.title));
    let titles: Vec<_> = movies.iter().map(|m| (m.title.as_str(), m.year)).collect();
    assert_eq!(
        titles,
        [
            ("Kinolune Second Film", Some(2019)),
            ("Kinolune Test Movie", Some(2020))
        ]
    );
    // The server's poster is the local poster of the movie.
    assert!(
        movies[1]
            .local_poster
            .as_deref()
            .is_some_and(|p| p.ends_with("/poster.jpg")),
        "{:?}",
        movies[1].local_poster
    );
    let shows = cat.shows().unwrap();
    assert_eq!(shows.len(), 1);
    assert_eq!(shows[0].title, "Kinolune Test Show");
    let episodes = cat.episodes(shows[0].id).unwrap();
    assert_eq!(
        episodes.iter().map(|e| (e.season, e.episode)).collect::<Vec<_>>(),
        [(Some(1), Some(1)), (Some(1), Some(2))]
    );
    let mut tracks: Vec<_> = cat
        .tracks()
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    tracks.sort();
    assert_eq!(tracks, ["First Song", "Second Song"]);

    // Streaming through the proxy gives the server's file byte for byte.
    let film = &movies[0];
    let local = std::fs::read(
        PathBuf::from(var("PA_JELLYFIN_MEDIA"))
            .join("Movies/Kinolune Second Film (2019)/Kinolune Second Film (2019).mp4"),
    )
    .unwrap();
    let proxy = Proxy::start(
        Arc::new(SourceOpener::new(Arc::clone(&cat))),
        ProxyOptions::default(),
    )
    .unwrap();
    let url = proxy.url(&SourceOpener::resource(
        id,
        &VfsPath::parse(&film.rel_path).unwrap(),
    ));
    let (status, body) = get(&proxy, &url, "bytes=1000-200999");
    assert_eq!(status, 206);
    assert_eq!(body, local[1000..201_000]);
    let (_, all) = get(&proxy, &url, "bytes=0-");
    assert_eq!(all, local);

    // The device plays it directly; reports reach the server.
    let rel = film.rel_path.as_str();
    let play = manager
        .jellyfin_playback(id, rel, DeviceCapabilities::libmpv(), 0, false)
        .unwrap()
        .expect("a Jellyfin file");
    assert!(play.url.is_none());
    for (event, pos) in [
        (ReportEvent::Start, 0),
        (ReportEvent::Progress, 4_000),
        (ReportEvent::Stop, 6_000),
    ] {
        manager
            .jellyfin_report(id, rel, event, pos, false, Some(&play.plan))
            .unwrap();
    }
    manager.jellyfin_set_favorite(id, rel, true).unwrap();
    let state = |rel: &str| {
        manager
            .jellyfin_user_data(id)
            .unwrap()
            .into_iter()
            .find(|(p, _)| p == rel)
            .and_then(|(_, m)| m.user_data)
            .unwrap()
    };
    let data = state(rel);
    assert!((data.position_ms() - 6_000).abs() <= 1_000, "{data:?}");
    assert!(data.is_favorite);

    manager.jellyfin_remove_from_resume(id, rel).unwrap();
    manager.jellyfin_set_favorite(id, rel, false).unwrap();
    let data = state(rel);
    assert_eq!(data.playback_position_ticks, 0);
    assert!(!data.is_favorite);

    // A forced transcode gives an HLS URL for the player.
    let hls = manager
        .jellyfin_playback(id, rel, DeviceCapabilities::libmpv(), 0, true)
        .unwrap()
        .unwrap();
    assert!(hls.url.as_deref().is_some_and(|u| u.contains(".m3u8")));
    manager
        .jellyfin_report(id, rel, ReportEvent::Stop, 0, false, Some(&hls.plan))
        .unwrap();

    // A local folder is not a Jellyfin file: nothing to do.
    let local_dir = tempfile::tempdir().unwrap();
    let other = manager
        .create(pa_sources::NewSource::new(
            SourceKind::Local,
            "Local",
            local_dir.path().to_string_lossy(),
        ))
        .unwrap()
        .id;
    assert!(
        manager
            .jellyfin_playback(other, "a.mkv", DeviceCapabilities::libmpv(), 0, false)
            .unwrap()
            .is_none()
    );

    manager.delete(id).unwrap();
    assert!(manager.get(id).is_err());
}
