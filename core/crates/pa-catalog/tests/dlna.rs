// SPDX-License-Identifier: MPL-2.0
//! NET-018: a DLNA server (minidlna in Docker) as a library source: the
//! scan finds its video and music, files stream through the loopback
//! proxy byte for byte. Run with `core/crates/pa-dlna/tests/minidlna/run.sh`.
#![cfg(feature = "server-tests")]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;

use pa_catalog::{Catalog, SourceOpener, SourcesConnector};
use pa_proxy::{Proxy, ProxyOptions};
use pa_scanner::CancelToken;
use pa_secrets::MemoryStore;
use pa_sources::{NewSource, SourceKind, SourceManager};
use pa_vfs::VfsPath;

fn var(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: use pa-dlna/tests/minidlna/run.sh"))
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
fn dlna_server_is_a_library_source() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let cat = Arc::new(Catalog::open(&db).unwrap());
    let manager = Arc::new(SourceManager::open(&db, Arc::new(MemoryStore::new())).unwrap());
    cat.set_connector(Arc::new(SourcesConnector::new(Arc::clone(&manager))));

    let source = manager
        .create(NewSource::new(
            SourceKind::Dlna,
            "Kinolune Test DLNA",
            var("PA_DLNA_LOCATION"),
        ))
        .unwrap();
    assert_eq!(manager.check(source.id).unwrap().online, Some(true));
    let mut errors = Vec::new();
    cat.scan(Some(source.id), &CancelToken::new(), &mut |u| {
        errors.extend(u.error)
    })
    .unwrap();
    assert_eq!(errors, Vec::<String>::new());

    let conn = pa_library::rusqlite::Connection::open(&db).unwrap();
    let mut stmt = conn
        .prepare("SELECT path FROM files WHERE source_id = ?1 ORDER BY path")
        .unwrap();
    let paths: Vec<String> = stmt
        .query_map([source.id], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|p| p.unwrap().trim_start_matches('/').to_owned())
        .collect();
    assert_eq!(
        paths,
        [
            "Movies/Test Movie (2020)/Test Movie (2020).mp4",
            "Music/Test Artist/Test Song.mp3"
        ]
    );

    let local = std::fs::read(
        PathBuf::from(var("PA_DLNA_MEDIA")).join("Movies/Test Movie (2020)/Test Movie (2020).mp4"),
    )
    .unwrap();
    let proxy = Proxy::start(
        Arc::new(SourceOpener::new(Arc::clone(&cat))),
        ProxyOptions::default(),
    )
    .unwrap();
    let url = proxy.url(&SourceOpener::resource(
        source.id,
        &VfsPath::parse(&paths[0]).unwrap(),
    ));
    let (status, all) = get(&proxy, &url, "bytes=0-");
    assert!(status == 200 || status == 206, "{status}");
    assert_eq!(all, local);
    let (status, part) = get(&proxy, &url, "bytes=1000-4999");
    assert_eq!(status, 206);
    assert_eq!(part, local[1000..5000]);
}
