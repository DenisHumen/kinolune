// SPDX-License-Identifier: MPL-2.0
//! NET-016: the client against a real Navidrome in Docker with a generated
//! library (`tests/navidrome/run.sh`). Needs `PA_SUBSONIC_URL`,
//! `PA_SUBSONIC_USER`, `PA_SUBSONIC_PASS` and `PA_SUBSONIC_MUSIC` from
//! `tests/navidrome/setup.sh`.
#![cfg(feature = "server-tests")]

use std::path::PathBuf;
use std::sync::Arc;

use pa_subsonic::{AlbumListType, AuthMode, Client, Credentials, Error, SearchQuery, SubsonicFs};
use pa_vfs::{ByteSource, EntryKind, Vfs, VfsPath};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set: run tests/navidrome/run.sh"))
}

fn client() -> Client {
    Client::new(
        &env("PA_SUBSONIC_URL"),
        Credentials::password(env("PA_SUBSONIC_USER"), env("PA_SUBSONIC_PASS")),
    )
    .unwrap()
}

fn local(rel: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env("PA_SUBSONIC_MUSIC")).join(rel)).unwrap()
}

#[test]
fn pings_with_a_token_and_refuses_a_wrong_password() {
    let c = client();
    let info = c.ping().unwrap();
    assert_eq!(c.auth_mode(), AuthMode::Token, "Navidrome checks tokens");
    assert_eq!(info.server_type, "navidrome");
    assert!(info.open_subsonic);
    assert!(info.version.starts_with("1.16"), "{info:?}");
    assert!(!c.extensions().unwrap().is_empty());
    // Navidrome 0.58 has no API keys yet; the call itself works.
    let _ = c.supports_api_keys().unwrap();

    // The hex-encoded password works too (servers without token auth).
    c.set_auth_mode(AuthMode::HexPassword);
    c.ping().unwrap();

    let wrong = Client::new(
        &env("PA_SUBSONIC_URL"),
        Credentials::password(env("PA_SUBSONIC_USER"), "not-the-password"),
    )
    .unwrap();
    assert_eq!(wrong.ping().unwrap_err(), Error::Unauthorized);
    // An API key Navidrome does not know is refused, not a crash.
    let key = Client::new(&env("PA_SUBSONIC_URL"), Credentials::api_key("no-such-key")).unwrap();
    assert!(key.ping().is_err());
}

#[test]
fn browses_artists_albums_and_tracks() {
    let c = client();
    let mut artists: Vec<_> = c.artists().unwrap();
    artists.sort_by(|a, b| a.name.cmp(&b.name));
    let names: Vec<_> = artists.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Kinolune Other Artist", "Kinolune Test Artist"]);

    let test = c.artist(&artists[1].id).unwrap();
    assert_eq!(test.albums.len(), 1);
    let album = c.album(&test.albums[0].id).unwrap();
    assert_eq!(album.display_name(), "Kinolune Test Album");
    assert_eq!(album.year, Some(2021));
    let tracks: Vec<_> = album
        .songs
        .iter()
        .map(|s| (s.track, s.title.as_str()))
        .collect();
    assert_eq!(tracks, [(Some(1), "First Song"), (Some(2), "Second Song")]);
    let first = c.song(&album.songs[0].id).unwrap();
    assert_eq!(first.suffix.as_deref(), Some("mp3"));
    assert_eq!(
        first.size,
        Some(local("Kinolune Test Artist/Kinolune Test Album/01 - First Song.mp3").len() as i64)
    );
    assert!((5..=7).contains(&first.duration.unwrap()), "{first:?}");
    assert!(album.cover_art.is_some());
    let cover = c
        .cover_art(album.cover_art.as_deref().unwrap(), Some(300))
        .unwrap();
    assert_eq!(&cover[..2], &[0xFF, 0xD8], "a JPEG");

    let albums = c.album_list(AlbumListType::AlphabeticalByName, 10, 0).unwrap();
    assert_eq!(albums.len(), 2);
    let (_, found, songs) = c.search(&SearchQuery::new("Closing")).unwrap();
    assert!(found.is_empty());
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0].disc_number, Some(2));
    assert_eq!(c.album("no-such-album").unwrap_err(), Error::NotFound);

    assert_eq!(c.all_songs().unwrap().len(), 4);
}

#[test]
fn the_library_reads_as_files() {
    let fs = SubsonicFs::new(Arc::new(client()));
    let root: Vec<_> = fs
        .list(&VfsPath::root())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(root, ["Kinolune Other Artist", "Kinolune Test Artist"]);
    let album = VfsPath::parse("/Kinolune Test Artist/Kinolune Test Album").unwrap();
    let mut files: Vec<_> = fs
        .list(&album)
        .unwrap()
        .into_iter()
        .map(|e| (e.name, e.metadata.kind))
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            ("01 - First Song.mp3".to_owned(), EntryKind::File),
            ("02 - Second Song.mp3".to_owned(), EntryKind::File),
            ("cover.jpg".to_owned(), EntryKind::File),
        ]
    );
    let double: Vec<_> = fs
        .list(&VfsPath::parse("/Kinolune Other Artist/Kinolune Double Album").unwrap())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .filter(|n| n.ends_with(".flac"))
        .collect();
    assert_eq!(double, ["01 - Opening.flac", "2-01 - Closing.flac"]);

    // The original file, byte for byte, with ranges.
    let path = album.join("02 - Second Song.mp3").unwrap();
    let want = local("Kinolune Test Artist/Kinolune Test Album/02 - Second Song.mp3");
    let f = fs.open(&path).unwrap();
    assert_eq!(f.size(), want.len() as u64);
    let mut got = vec![0u8; want.len()];
    let mut at = 0;
    while at < got.len() {
        let n = f.read_at(at as u64, &mut got[at..]).unwrap();
        assert!(n > 0);
        at += n;
    }
    assert_eq!(got, want);
    let mut mid = vec![0u8; 500];
    f.read_at(1234, &mut mid).unwrap();
    assert_eq!(mid, want[1234..1734]);

    let cover = fs.open(&album.join("cover.jpg").unwrap()).unwrap();
    let mut head = [0u8; 2];
    cover.read_at(0, &mut head).unwrap();
    assert_eq!(head, [0xFF, 0xD8]);
    let id = fs.song_at(&path).unwrap().unwrap().song_id;
    assert_eq!(fs.path_of(&id).unwrap(), Some(path));
}

#[test]
fn scrobbles_reach_the_server() {
    let c = client();
    let (_, _, songs) = c.search(&SearchQuery::new("Opening")).unwrap();
    let song = &songs[0];
    let before = c.song(&song.id).unwrap().play_count.unwrap_or(0);

    c.scrobble(&song.id, false, None).unwrap();
    let playing = c.now_playing().unwrap();
    assert!(playing.iter().any(|s| s.id == song.id), "{playing:?}");

    c.scrobble(&song.id, true, Some(1_700_000_000_000)).unwrap();
    let after = c.song(&song.id).unwrap();
    assert_eq!(after.play_count, Some(before + 1));
    assert_eq!(c.scrobble("no-such-song", true, None).unwrap_err(), Error::NotFound);
}
