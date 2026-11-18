// SPDX-License-Identifier: MPL-2.0
//! NET-018: DLNA servers and M3U playlists as sources without a login.

use std::sync::Arc;

use pa_secrets::{MemoryStore, SecretString};
use pa_sources::{NewSource, SourceKind, SourceManager, normalize_address, playlist_file};

fn manager() -> SourceManager {
    SourceManager::new(
        pa_library::open_in_memory().unwrap(),
        Arc::new(MemoryStore::new()),
    )
}

#[test]
fn addresses_of_dlna_servers_and_playlists() {
    assert_eq!(
        normalize_address(SourceKind::Dlna, " http://192.168.1.5:8200/rootDesc.xml ").unwrap(),
        "http://192.168.1.5:8200/rootDesc.xml"
    );
    assert_eq!(
        normalize_address(SourceKind::Playlist, "https://cdn.example/list.m3u8?t=1").unwrap(),
        "https://cdn.example/list.m3u8?t=1"
    );
    let file = normalize_address(SourceKind::Playlist, "/home/media/list.m3u").unwrap();
    assert_eq!(file, "m3u+file:///home/media/list.m3u");
    assert_eq!(
        normalize_address(SourceKind::Playlist, &file).unwrap(),
        file,
        "a stored address normalizes to itself"
    );
    assert_eq!(
        playlist_file(&file).unwrap(),
        std::path::PathBuf::from("/home/media/list.m3u")
    );
    assert_eq!(playlist_file("https://cdn.example/list.m3u8"), None);
    for (kind, bad) in [
        (SourceKind::Dlna, "rtsp://192.168.1.5/x"),
        (SourceKind::Dlna, "192.168.1.5:8200"),
        (SourceKind::Dlna, "http://u:p@192.168.1.5/d.xml"),
        (SourceKind::Playlist, "ftp://h/list.m3u"),
        (SourceKind::Playlist, "relative/list.m3u"),
        (SourceKind::Playlist, "http://u:secret@h/list.m3u"),
    ] {
        let e = normalize_address(kind, bad).unwrap_err().to_string();
        assert!(!e.contains("secret"), "{e}");
    }
}

#[test]
fn open_sources_have_no_login() {
    let m = manager();
    let dlna = m
        .create(NewSource::new(
            SourceKind::Dlna,
            "NAS",
            "http://192.168.1.5:8200/rootDesc.xml",
        ))
        .unwrap();
    assert_eq!(dlna.kind, SourceKind::Dlna);
    assert!(!dlna.has_password);
    assert_eq!("dlna".parse::<SourceKind>().unwrap(), SourceKind::Dlna);
    assert_eq!(
        "playlist".parse::<SourceKind>().unwrap(),
        SourceKind::Playlist
    );
    assert!(SourceKind::Dlna.is_open() && SourceKind::Playlist.is_open());
    assert!(!SourceKind::Subsonic.is_open());
    assert!(
        m.create(
            NewSource::new(SourceKind::Playlist, "L", "https://cdn.example/l.m3u")
                .with_login("anna", SecretString::new("pw"))
        )
        .is_err()
    );
    // Connecting does not touch the network yet.
    assert!(m.connect(dlna.id).is_ok());
    let list = m
        .create(NewSource::new(
            SourceKind::Playlist,
            "List",
            "/nonexistent/list.m3u",
        ))
        .unwrap();
    let vfs = m.connect(list.id).unwrap();
    assert_eq!(
        vfs.list(&pa_vfs::VfsPath::root()).unwrap_err().kind(),
        pa_vfs::ErrorKind::NotFound
    );
}
