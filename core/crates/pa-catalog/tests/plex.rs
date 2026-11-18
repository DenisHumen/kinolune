// SPDX-License-Identifier: MPL-2.0
//! NET-012: a Plex Media Server as a library source, against the plex.tv
//! and server stand-in of `pa-plex` (synthetic answers, no network): the
//! PIN sign-in keeps only the server token; the scan turns the server's
//! libraries into movies, shows, episodes and tracks; the server's music
//! playlist becomes a playlist of the library (MUS-012); files stream
//! through the loopback proxy byte for byte.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use pa_catalog::{Catalog, SourceOpener, SourcesConnector};
use pa_proxy::{Proxy, ProxyOptions};
use pa_scanner::CancelToken;
use pa_secrets::MemoryStore;
use pa_sources::{PlexSignIn, SourceKind, SourceManager};
use pa_vfs::VfsPath;

#[path = "../../pa-plex/tests/support/mock.rs"]
mod mock;

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
fn plex_server_is_a_library_source() {
    let (_mock, base) = mock::start();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let cat = Arc::new(Catalog::open(&db).unwrap());
    let manager = Arc::new(SourceManager::open(&db, Arc::new(MemoryStore::new())).unwrap());
    cat.set_connector(Arc::new(SourcesConnector::new(Arc::clone(&manager))));

    let mut sign_in = PlexSignIn::start_at(&base, "pa-catalog tests").unwrap();
    while !sign_in.poll().unwrap() {}
    let account = sign_in.finish("abc123").unwrap();
    let source = manager.create_plex(account, None, Vec::new()).unwrap();
    assert_eq!(source.kind, SourceKind::Plex);
    // The database holds no token.
    let raw = std::fs::read(&db).unwrap();
    for secret in ["server-token", "account-token"] {
        assert!(!raw.windows(secret.len()).any(|w| w == secret.as_bytes()));
    }
    let id = source.id;

    let mut errors = Vec::new();
    cat.scan(Some(id), &CancelToken::new(), &mut |u| {
        errors.extend(u.error)
    })
    .unwrap();
    assert_eq!(errors, Vec::<String>::new());

    let mut movies = cat.movies().unwrap();
    movies.sort_by(|a, b| a.title.cmp(&b.title));
    let titles: Vec<_> = movies.iter().map(|m| (m.title.as_str(), m.year)).collect();
    assert_eq!(
        titles,
        [("Big Buck Bunny", Some(2008)), ("Sintel", Some(2010))]
    );
    assert!(
        movies[0]
            .local_poster
            .as_deref()
            .is_some_and(|p| p.ends_with("/poster.jpg")),
        "{:?}",
        movies[0].local_poster
    );
    let shows = cat.shows().unwrap();
    assert_eq!(shows.len(), 1);
    assert_eq!(shows[0].title, "Elephants Dream");
    let episodes = cat.episodes(shows[0].id).unwrap();
    assert_eq!(
        episodes
            .iter()
            .map(|e| (e.season, e.episode))
            .collect::<Vec<_>>(),
        [(Some(1), Some(1)), (Some(1), Some(2))]
    );
    let mut tracks: Vec<_> = cat.tracks().unwrap().into_iter().map(|t| t.title).collect();
    tracks.sort();
    assert_eq!(tracks, ["Closing", "Opening"]);

    // MUS-012: the server's music playlist, in its order.
    let playlists = cat.music_playlists().unwrap();
    let road = playlists
        .iter()
        .find(|p| p.name == "Road Trip")
        .unwrap_or_else(|| panic!("{playlists:?}"));
    assert_eq!(road.server.as_deref(), Some(source.name.as_str()));
    let order: Vec<String> = cat
        .music_playlist_items(road.id)
        .unwrap()
        .into_iter()
        .map(|i| i.title)
        .collect();
    assert_eq!(order, ["Opening", "Closing"]);

    // Streaming through the proxy gives the server's file byte for byte.
    let proxy = Proxy::start(
        Arc::new(SourceOpener::new(Arc::clone(&cat))),
        ProxyOptions::default(),
    )
    .unwrap();
    let url = proxy.url(&SourceOpener::resource(
        id,
        &VfsPath::parse(&movies[0].rel_path).unwrap(),
    ));
    let (status, body) = get(&proxy, &url, "bytes=1000-200999");
    assert_eq!(status, 206);
    assert_eq!(body.len(), 200_000);
    assert!(
        body.iter()
            .enumerate()
            .all(|(i, b)| *b == mock::byte(1000 + i as u64))
    );
    let (_, all) = get(&proxy, &url, "bytes=0-");
    assert_eq!(all.len(), 300_000);
}
