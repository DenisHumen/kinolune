// SPDX-License-Identifier: MPL-2.0
//! NET-016: a Subsonic server (Navidrome) as a library source. The source
//! keeps its password in the secret store only; the scan turns the
//! server's songs into tracks with their tags; files stream through the
//! loopback proxy byte for byte; scrobbles reach the server. Run with
//! `core/crates/pa-subsonic/tests/navidrome/run.sh`.
#![cfg(feature = "server-tests")]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;

use pa_catalog::{Catalog, SourceOpener, SourcesConnector};
use pa_proxy::{Proxy, ProxyOptions};
use pa_scanner::CancelToken;
use pa_secrets::{MemoryStore, SecretString};
use pa_sources::{NewSource, SourceKind, SourceManager};
use pa_vfs::VfsPath;

fn var(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: use pa-subsonic/tests/navidrome/run.sh"))
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
fn subsonic_server_is_a_music_source() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let cat = Arc::new(Catalog::open(&db).unwrap());
    let manager = Arc::new(SourceManager::open(&db, Arc::new(MemoryStore::new())).unwrap());
    cat.set_connector(Arc::new(SourcesConnector::new(Arc::clone(&manager))));

    let password = SecretString::new(var("PA_SUBSONIC_PASS"));
    let info = pa_sources::subsonic_probe(
        &var("PA_SUBSONIC_URL"),
        Some(&var("PA_SUBSONIC_USER")),
        &password,
    )
    .unwrap();
    assert_eq!(info.server_type, "navidrome");
    let wrong = pa_sources::subsonic_probe(
        &var("PA_SUBSONIC_URL"),
        Some(&var("PA_SUBSONIC_USER")),
        &SecretString::new("wrong"),
    )
    .unwrap_err();
    assert_eq!(
        wrong.source_kind(),
        Some(pa_vfs::ErrorKind::AuthenticationFailed)
    );

    let source = manager
        .create(
            NewSource::new(SourceKind::Subsonic, "Navidrome", var("PA_SUBSONIC_URL"))
                .with_login(var("PA_SUBSONIC_USER"), password.clone()),
        )
        .unwrap();
    assert!(source.has_password);
    // The database never holds the password.
    let raw = std::fs::read(&db).unwrap();
    let secret = password.expose().as_bytes();
    assert!(!raw.windows(secret.len()).any(|w| w == secret));
    let id = source.id;
    assert_eq!(manager.check(id).unwrap().online, Some(true));

    let mut errors = Vec::new();
    cat.scan(Some(id), &CancelToken::new(), &mut |u| {
        errors.extend(u.error)
    })
    .unwrap();
    assert_eq!(errors, Vec::<String>::new());

    let mut tracks = cat.tracks().unwrap();
    tracks.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    let seen: Vec<_> = tracks
        .iter()
        .map(|t| {
            (
                t.artist.as_deref().unwrap_or(""),
                t.album.as_deref().unwrap_or(""),
                t.track_number,
                t.title.as_str(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            ("Kinolune Other Artist", "Kinolune Double Album", Some(1), "Opening"),
            ("Kinolune Other Artist", "Kinolune Double Album", Some(1), "Closing"),
            ("Kinolune Test Artist", "Kinolune Test Album", Some(1), "First Song"),
            ("Kinolune Test Artist", "Kinolune Test Album", Some(2), "Second Song"),
        ]
    );
    assert!(tracks.iter().all(|t| t.duration_ms.is_some_and(|d| d > 4_000)));

    // Streaming through the proxy gives the server's file byte for byte.
    let song = &tracks[2];
    let local = std::fs::read(
        PathBuf::from(var("PA_SUBSONIC_MUSIC"))
            .join("Kinolune Test Artist/Kinolune Test Album/01 - First Song.mp3"),
    )
    .unwrap();
    let proxy = Proxy::start(
        Arc::new(SourceOpener::new(Arc::clone(&cat))),
        ProxyOptions::default(),
    )
    .unwrap();
    let url = proxy.url(&SourceOpener::resource(
        id,
        &VfsPath::parse(&song.rel_path).unwrap(),
    ));
    let (status, body) = get(&proxy, &url, "bytes=1000-20999");
    assert_eq!(status, 206);
    assert_eq!(body, local[1000..21_000]);
    let (_, all) = get(&proxy, &url, "bytes=0-");
    assert_eq!(all, local);

    // Scrobbles: "now playing", then played.
    let fs = manager.subsonic(id).unwrap();
    let song_id = fs
        .song_at(&VfsPath::parse(&song.rel_path).unwrap())
        .unwrap()
        .unwrap()
        .song_id;
    let before = fs.client().song(&song_id).unwrap().play_count.unwrap_or(0);
    assert!(
        manager
            .subsonic_scrobble(id, &song.rel_path, false, None)
            .unwrap()
    );
    assert!(
        fs.client()
            .now_playing()
            .unwrap()
            .iter()
            .any(|s| s.id == song_id)
    );
    assert!(
        manager
            .subsonic_scrobble(id, &song.rel_path, true, None)
            .unwrap()
    );
    assert_eq!(
        fs.client().song(&song_id).unwrap().play_count,
        Some(before + 1)
    );
    // A path that is not a song: nothing to scrobble.
    assert!(
        !manager
            .subsonic_scrobble(id, "Kinolune Test Artist", true, None)
            .unwrap()
    );

    manager.delete(id).unwrap();
    assert!(manager.get(id).is_err());
}
