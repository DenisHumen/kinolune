// SPDX-License-Identifier: MPL-2.0
//! NET-012: a Plex Media Server as a library source, against the plex.tv
//! and server stand-in of `pa-plex` (synthetic answers, no network): the
//! PIN sign-in keeps only the server token; the scan turns the server's
//! libraries into movies, shows, episodes and tracks; the server's music
//! playlist becomes a playlist of the library (MUS-012); files stream
//! through the loopback proxy byte for byte. NET-013: progress both ways,
//! watched marks, Continue Watching, direct play or the transcoder, video
//! playlists.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use pa_catalog::{Catalog, ServerItemState, SourceOpener, SourcesConnector};
use pa_proxy::{Proxy, ProxyOptions};
use pa_scanner::CancelToken;
use pa_secrets::MemoryStore;
use pa_sources::{PlexQuality, PlexSignIn, ReportEvent, SourceKind, SourceManager};
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

/// NET-013 «done when», with the server stand-in in place of a real Plex
/// Media Server: the position played here reaches the server as its
/// timeline (what Plex Web shows under Continue Watching), and a position
/// or "watched" set at the server (as Plex Web would) comes into the
/// library's progress and Continue Watching. Direct play by default,
/// the transcoder on request; video playlists of the server are read-only
/// video playlists here.
#[test]
fn plex_progress_goes_both_ways() {
    let (mock, base) = mock::start();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let cat = Arc::new(Catalog::open(&db).unwrap());
    let manager = Arc::new(SourceManager::open(&db, Arc::new(MemoryStore::new())).unwrap());
    cat.set_connector(Arc::new(SourcesConnector::new(Arc::clone(&manager))));
    let mut sign_in = PlexSignIn::start_at(&base, "pa-catalog tests").unwrap();
    while !sign_in.poll().unwrap() {}
    let source = manager
        .create_plex(sign_in.finish("abc123").unwrap(), None, Vec::new())
        .unwrap();
    let id = source.id;
    cat.scan(Some(id), &CancelToken::new(), &mut |_| {})
        .unwrap();

    let movie = cat
        .movies()
        .unwrap()
        .into_iter()
        .find(|m| m.title == "Big Buck Bunny")
        .unwrap();
    let show = cat.shows().unwrap().remove(0);
    let episodes = cat.episodes(show.id).unwrap();
    let (emo, proog) = (&episodes[0], &episodes[1]);

    // Played here: start, a heartbeat on pause, the stop. The server keeps
    // the position — what Plex Web lists under Continue Watching.
    manager
        .plex_report(id, &movie.rel_path, ReportEvent::Start, 0, false, None)
        .unwrap();
    manager
        .plex_report(
            id,
            &movie.rel_path,
            ReportEvent::Progress,
            250_000,
            true,
            None,
        )
        .unwrap();
    manager
        .plex_report(id, &movie.rel_path, ReportEvent::Stop, 300_000, false, None)
        .unwrap();
    assert_eq!(mock.user_state("101").view_offset, Some(300_000));
    let states: Vec<String> = mock
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|t| t.starts_with("/:/timeline?") && t.contains("ratingKey=101"))
        .filter_map(|t| t.split("state=").nth(1))
        .map(|s| s.split('&').next().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(states, ["playing", "paused", "stopped"]);

    // Set at the server (Plex Web): a position of Proog, newer than ours.
    mock.set_user_state(
        "202",
        mock::UserState {
            view_offset: Some(42_000),
            view_count: 0,
            last_viewed_at: Some(4_000_000_000),
        },
    );
    let import = |cat: &Catalog| {
        let states: Vec<ServerItemState> = manager
            .plex_user_data(id)
            .unwrap()
            .into_iter()
            .map(|(rel_path, m)| ServerItemState {
                rel_path,
                position_ms: m.view_offset_ms.unwrap_or(0),
                duration_ms: m.duration_ms.unwrap_or(0),
                played: m.watched,
                favorite: false,
                at_ms: m.last_viewed_ms.unwrap_or(0),
            })
            .collect();
        cat.import_server_state(id, &states).unwrap()
    };
    let report = import(&cat);
    // Proog's position; Sintel (watched twice) and Big Buck Bunny (our own
    // position, which the server now has) — the library had none of
    // Sintel, so it is marked watched.
    assert!(report.progress >= 1 && report.watched >= 1, "{report:?}");
    let p = cat.progress(proog.id).unwrap().unwrap();
    assert_eq!((p.position_ms, p.watched), (42_000, false));
    assert!(
        cat.continue_watching(10)
            .unwrap()
            .iter()
            .any(|c| c.item.id == proog.id),
        "Proog is in Continue Watching"
    );
    let sintel = cat
        .movies()
        .unwrap()
        .into_iter()
        .find(|m| m.title == "Sintel")
        .unwrap();
    assert!(cat.progress(sintel.id).unwrap().unwrap().watched);
    // Nothing newer the second time.
    assert_eq!(import(&cat).progress, 0);

    // Watched marks and Continue Watching go to the server.
    manager.plex_set_played(id, &emo.rel_path, true).unwrap();
    assert_eq!(mock.user_state("201").view_count, 1);
    manager.plex_set_played(id, &emo.rel_path, false).unwrap();
    assert_eq!(mock.user_state("201").view_count, 0);
    manager
        .plex_remove_from_resume(id, &proog.rel_path)
        .unwrap();
    assert_eq!(mock.user_state("202").view_offset, None);

    // Direct play unless asked; the transcoder on request or above the
    // bit rate asked for (the movie is 8 Mbit/s).
    let direct = manager
        .plex_playback(id, &movie.rel_path, false, PlexQuality::default())
        .unwrap()
        .unwrap();
    assert!(!direct.transcode && direct.url.is_none());
    let capped = manager
        .plex_playback(
            id,
            &movie.rel_path,
            false,
            PlexQuality {
                max_bitrate_kbps: Some(4000),
                resolution: Some((1280, 720)),
            },
        )
        .unwrap()
        .unwrap();
    assert!(capped.transcode);
    assert_eq!(capped.reasons, ["bitrate"]);
    let forced = manager
        .plex_playback(id, &movie.rel_path, true, PlexQuality::default())
        .unwrap()
        .unwrap();
    let url = forced.url.clone().unwrap();
    assert!(url.contains("/video/:/transcode/universal/start.m3u8?"));
    assert!(url.contains("directStream=1"));
    assert!(!format!("{forced:?}").contains("server-token"));
    // The stop of a transcoded play ends its session.
    let session = forced.session.clone().unwrap();
    manager
        .plex_report(
            id,
            &movie.rel_path,
            ReportEvent::Stop,
            10_000,
            false,
            Some(&session),
        )
        .unwrap();
    assert!(
        mock.seen
            .lock()
            .unwrap()
            .iter()
            .any(|t| t.ends_with(&format!("stop?session={session}")))
    );
    // Other sources and non-media paths: nothing.
    assert_eq!(
        manager
            .plex_playback(id, "Movies/none.mkv", true, PlexQuality::default())
            .ok()
            .flatten(),
        None
    );

    // The server's video playlist: its movie and episode in order, the
    // song left out, read-only here.
    let lists = cat.playlists().unwrap();
    let night = lists
        .iter()
        .find(|p| p.name == "Movie Night")
        .unwrap_or_else(|| panic!("{lists:?}"));
    assert_eq!(night.server.as_deref(), Some(source.name.as_str()));
    let order: Vec<_> = cat
        .playlist_items(night.id)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(order, [proog.id, movie.id]);
    assert!(cat.rename_playlist(night.id, "Mine").is_err());
    assert!(cat.add_to_playlist(night.id, emo.id).is_err());
    // Not among the music playlists.
    assert!(
        !cat.music_playlists()
            .unwrap()
            .iter()
            .any(|p| p.name == "Movie Night")
    );
}
