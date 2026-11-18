// SPDX-License-Identifier: MPL-2.0
//! NET-018: the client against minidlna in Docker with a generated media
//! library (`tests/minidlna/run.sh`). Needs `PA_DLNA_LOCATION`,
//! `PA_DLNA_IFACE` and `PA_DLNA_MEDIA` from `tests/minidlna/setup.sh`.
#![cfg(feature = "server-tests")]

use std::path::PathBuf;
use std::time::Duration;

use pa_dlna::{DlnaFs, SSDP_ADDR, SearchOptions, discover};
use pa_vfs::{ByteSource, EntryKind, Vfs, VfsPath};
use url::Url;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set: run tests/minidlna/run.sh"))
}

fn names(fs: &DlnaFs, path: &str) -> Vec<(String, EntryKind, u64)> {
    let mut v: Vec<_> = fs
        .list(&VfsPath::parse(path).unwrap())
        .unwrap()
        .into_iter()
        .map(|e| (e.name, e.metadata.kind, e.metadata.size))
        .collect();
    v.sort();
    v
}

#[test]
fn ssdp_finds_minidlna() {
    let opts = SearchOptions {
        wait: Duration::from_secs(3),
        interfaces: vec![env("PA_DLNA_IFACE").parse().unwrap()],
        target: SSDP_ADDR.into(),
    };
    let servers = discover(&opts).unwrap();
    let ours = servers
        .iter()
        .find(|s| s.location.as_str() == env("PA_DLNA_LOCATION"))
        .unwrap_or_else(|| panic!("minidlna not found in {servers:?}"));
    assert_eq!(ours.name, "Kinolune Test DLNA");
    assert!(ours.udn.starts_with("uuid:"));
    assert!(ours.control_url.as_str().ends_with("/ctl/ContentDir"));
}

#[test]
fn browses_the_folders_and_streams_files() {
    let fs = DlnaFs::new(Url::parse(&env("PA_DLNA_LOCATION")).unwrap()).unwrap();
    assert_eq!(fs.server().unwrap().name, "Kinolune Test DLNA");
    // «Browse Folders» is the root: each file once, in its folder.
    assert_eq!(
        names(&fs, "/"),
        [
            ("Movies".to_owned(), EntryKind::Dir, 0),
            ("Music".to_owned(), EntryKind::Dir, 0)
        ]
    );
    let media = PathBuf::from(env("PA_DLNA_MEDIA"));
    let movie =
        std::fs::read(media.join("Movies/Test Movie (2020)/Test Movie (2020).mp4")).unwrap();
    let song = std::fs::read(media.join("Music/Test Artist/01 - Test Song.mp3")).unwrap();
    assert_eq!(
        names(&fs, "/Movies/Test Movie (2020)"),
        [(
            "Test Movie (2020).mp4".to_owned(),
            EntryKind::File,
            movie.len() as u64
        )]
    );
    // An audio item is named by its tag title.
    assert_eq!(
        names(&fs, "/Music/Test Artist"),
        [(
            "Test Song.mp3".to_owned(),
            EntryKind::File,
            song.len() as u64
        )]
    );

    let f = fs
        .open(&VfsPath::parse("/Movies/Test Movie (2020)/Test Movie (2020).mp4").unwrap())
        .unwrap();
    let mut all = vec![0; movie.len()];
    f.read_exact_at(0, &mut all).unwrap();
    assert_eq!(all, movie);
    let mut part = vec![0; 1000];
    f.read_exact_at(5000, &mut part).unwrap();
    assert_eq!(part, movie[5000..6000]);
}
