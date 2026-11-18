// SPDX-License-Identifier: MPL-2.0
//! MUS-012 "done when" on a real Navidrome: an album of the server and
//! the server's playlist (mirrored by the scan) are downloaded through the
//! job queue, and with Navidrome stopped every track plays from the copy
//! through the stream proxy, byte for byte the original file. Run with
//! `core/crates/pa-subsonic/tests/navidrome/run.sh`, which starts the
//! server, makes the library and the playlist, and sets the variables.
#![cfg(feature = "server-tests")]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pa_catalog::{CancelToken, Catalog, SourceOpener, SourcesConnector};
use pa_jobs::{JobQueue, QueueConfig};
use pa_offline::{DownloadOptions, Offline, Selection};
use pa_proxy::{Proxy, ProxyOptions};
use pa_secrets::{MemoryStore, SecretString};
use pa_sources::{NewSource, SourceKind, SourceManager};
use pa_storage::{StorageConfig, StorageManager};
use pa_vfs::VfsPath;

fn var(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: use pa-subsonic/tests/navidrome/run.sh"))
}

fn docker(args: &[&str]) {
    let status = Command::new("docker").args(args).status().unwrap();
    assert!(status.success(), "docker {args:?}");
}

fn get(proxy: &Proxy, url: &str) -> (u16, Vec<u8>) {
    let rest = url.strip_prefix("http://").unwrap();
    let path = &rest[rest.find('/').unwrap()..];
    let mut s = TcpStream::connect(proxy.local_addr()).unwrap();
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: x\r\nRange: bytes=0-\r\nConnection: close\r\n\r\n"
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

/// The original file of the song titled `title` in the server's music
/// folder (the tree names files by tags, not by the server's file names).
fn original(music: &Path, title: &str) -> PathBuf {
    fn walk(dir: &Path, title: &str) -> Option<PathBuf> {
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Some(f) = walk(&p, title) {
                    return Some(f);
                }
            } else if p
                .file_stem()
                .is_some_and(|s| s.to_string_lossy().ends_with(title))
            {
                return Some(p);
            }
        }
        None
    }
    walk(music, title).unwrap_or_else(|| panic!("no file for {title}"))
}

#[test]
fn navidrome_album_and_playlist_play_offline() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let db = dir.join("library.db");
    let music = PathBuf::from(var("PA_SUBSONIC_MUSIC"));
    let catalog = Arc::new(Catalog::open(&db).unwrap());
    let manager = Arc::new(SourceManager::open(&db, Arc::new(MemoryStore::new())).unwrap());
    let source = manager
        .create(
            NewSource::new(SourceKind::Subsonic, "Navidrome", var("PA_SUBSONIC_URL")).with_login(
                var("PA_SUBSONIC_USER"),
                SecretString::new(var("PA_SUBSONIC_PASS")),
            ),
        )
        .unwrap()
        .id;
    catalog.set_connector(Arc::new(SourcesConnector::new(Arc::clone(&manager))));
    let storage = StorageManager::open(&db, StorageConfig::default()).unwrap();
    let queue = JobQueue::open(&db, QueueConfig::default()).unwrap();
    storage.attach(&queue);
    let offline = Offline::new(Arc::clone(&catalog), queue.clone(), storage).unwrap();
    offline.set_folder(&dir.join("offline")).unwrap();
    offline.register();

    let mut errors = Vec::new();
    catalog
        .scan(Some(source), &CancelToken::new(), &mut |u| {
            errors.extend(u.error)
        })
        .unwrap();
    assert_eq!(errors, Vec::<String>::new());

    // The album of "First Song", and the server's playlist.
    let tracks = catalog.music_tracks().unwrap();
    assert_eq!(tracks.len(), 4, "{tracks:?}");
    assert!(tracks.iter().all(|t| !t.local));
    let first = tracks
        .iter()
        .find(|t| t.item.title.contains("First Song"))
        .unwrap();
    let album = first.album_id.unwrap();
    let playlist = catalog
        .music_playlists()
        .unwrap()
        .into_iter()
        .find(|p| p.name == var("PA_SUBSONIC_PLAYLIST"))
        .unwrap_or_else(|| panic!("{:?}", catalog.music_playlists()));
    assert_eq!(playlist.server.as_deref(), Some("Navidrome"));
    assert_eq!(playlist.item_count, 3);

    let album_jobs = offline
        .download(&Selection::Album(album), &DownloadOptions::default())
        .unwrap();
    assert_eq!(album_jobs.len(), 2);
    // The playlist adds only what the album download does not have yet.
    let list_jobs = offline
        .download(
            &Selection::MusicPlaylist(playlist.id),
            &DownloadOptions::default(),
        )
        .unwrap();
    assert_eq!(list_jobs.len(), 1);
    let _driver = queue.start();
    let deadline = Instant::now() + Duration::from_secs(300);
    while offline.copies().unwrap().len() < 3 {
        assert!(Instant::now() < deadline, "{:?}", offline.downloads());
        std::thread::sleep(Duration::from_millis(100));
    }
    // "Downloaded" in the music views.
    let local: Vec<String> = catalog
        .music_tracks()
        .unwrap()
        .into_iter()
        .filter(|t| t.local)
        .map(|t| t.item.title)
        .collect();
    assert_eq!(local.len(), 3, "{local:?}");

    // The server is gone: the album and the playlist play from the copies.
    let container = var("PA_NAVIDROME_CONTAINER");
    docker(&["stop", "-t", "1", &container]);
    let proxy = Proxy::start(
        Arc::new(SourceOpener::new(Arc::clone(&catalog))),
        ProxyOptions::default(),
    )
    .unwrap();
    let items = catalog.music_playlist_items(playlist.id).unwrap();
    assert_eq!(items.len(), 3);
    for t in catalog
        .music_tracks()
        .unwrap()
        .into_iter()
        .filter(|t| t.local)
    {
        let url = proxy.url(&SourceOpener::resource(
            source,
            &VfsPath::parse(&t.item.rel_path).unwrap(),
        ));
        let (status, body) = get(&proxy, &url);
        assert!(matches!(status, 200 | 206), "{}: {status}", t.item.title);
        let title = t.item.title.clone();
        assert_eq!(
            body,
            std::fs::read(original(&music, &title)).unwrap(),
            "{title}"
        );
    }
    // A track that was not downloaded does not play without the server.
    let missing = catalog
        .music_tracks()
        .unwrap()
        .into_iter()
        .find(|t| !t.local)
        .unwrap();
    let url = proxy.url(&SourceOpener::resource(
        source,
        &VfsPath::parse(&missing.item.rel_path).unwrap(),
    ));
    assert!(get(&proxy, &url).0 >= 400);
    eprintln!("Navidrome album and playlist played from the copies with the server stopped");
    drop(proxy);
    docker(&["start", &container]);
}
