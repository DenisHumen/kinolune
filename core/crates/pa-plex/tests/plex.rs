// SPDX-License-Identifier: MPL-2.0
//! NET-012 against recorded-shape answers of plex.tv and a Plex Media
//! Server (`tests/fixtures/`), served by a small local HTTP server that
//! checks the tokens and the `X-Plex-*` headers and honours byte ranges:
//! PIN sign-in, Plex Home users and switching with a PIN, servers and the
//! automatic choice of a connection (local → remote → relay), libraries
//! as a file tree with ids, artwork and files, music playlists.

use std::sync::Arc;
use std::time::Duration;

use pa_plex::{
    ClientInfo, ConnectionKind, Error, PlexFs, PlexTv, Server, Token, order_connections,
    probe_connections,
};
use pa_vfs::{ByteSource, Vfs, VfsPath};

mod support;
use support::mock::{byte, start};

fn info() -> ClientInfo {
    ClientInfo::kinolune("Test Device", "test-client")
}

/// Signs in with the PIN and returns the account token.
fn sign_in(tv: &PlexTv) -> Token {
    let pin = tv.create_pin().unwrap();
    assert_eq!(pin.code, "K7Q2");
    let id = pin.id.unwrap();
    // Not linked yet, then linked.
    assert_eq!(tv.check_pin(id).unwrap().auth_token, None);
    let linked = tv.check_pin(id).unwrap();
    Token::new(linked.auth_token.expect("linked"))
}

#[test]
fn pin_sign_in_home_users_and_servers() {
    let (mock, base) = start();
    let tv = PlexTv::with_base(info(), &base);
    let token = sign_in(&tv);
    assert!(matches!(tv.check_pin(1), Err(Error::NotFound)));

    let account = tv.account(&token).unwrap();
    assert_eq!(account.uuid, "owner-uuid");
    assert_eq!(account.display_name(), "Owner");
    assert!(account.home);
    assert!(!format!("{account:?}").contains("account-token"));

    let users = tv.home_users(&token).unwrap();
    assert_eq!(users.len(), 2);
    let kid = users.iter().find(|u| u.uuid == "kid-uuid").unwrap();
    assert!(kid.protected && kid.restricted && !kid.admin);
    assert!(matches!(
        tv.switch_user(&token, "kid-uuid", Some("0000")),
        Err(Error::Forbidden)
    ));
    let kid_token = tv.switch_user(&token, "kid-uuid", Some("1234")).unwrap();
    assert_eq!(kid_token.expose(), "kid-token");

    let servers = tv.servers(&token).unwrap();
    assert_eq!(servers.len(), 1, "players are not servers");
    let s = &servers[0];
    assert_eq!(s.client_identifier, "abc123");
    assert!(s.owned);
    assert_eq!(s.access_token.as_deref(), Some("server-token"));
    assert_eq!(
        tv.servers(&kid_token).unwrap()[0].access_token.as_deref(),
        Some("kid-server-token")
    );
    assert!(matches!(
        tv.servers(&Token::new("revoked")),
        Err(Error::Unauthorized)
    ));
    // Every request named this installation.
    assert!(
        mock.clients
            .lock()
            .unwrap()
            .iter()
            .all(|c| c == "test-client")
    );
}

#[test]
fn picks_the_connection_that_answers_and_reads_the_libraries() {
    let (mock, base) = start();
    let tv = PlexTv::with_base(info(), &base);
    let token = sign_in(&tv);
    let server = tv.servers(&token).unwrap().remove(0);
    let order = order_connections(&server);
    let uris: Vec<String> = order.iter().map(|(u, _)| u.clone()).collect();
    // Local (plex.direct, then its plain IP), remote, relay.
    assert_eq!(order[0].1, ConnectionKind::Local);
    assert_eq!(uris[1], base);
    assert_eq!(order[2].1, ConnectionKind::Remote);
    assert_eq!(order.last().unwrap().1, ConnectionKind::Relay);

    let server_token = Token::new(server.access_token.clone().unwrap());
    let ok = probe_connections(
        &uris,
        &info(),
        Some(&server_token),
        &server.client_identifier,
        Duration::from_secs(2),
    );
    assert_eq!(ok, [false, true, false, false]);
    // Another server's id at that address does not count.
    assert_eq!(
        probe_connections(
            &uris[1..2],
            &info(),
            None,
            "other",
            Duration::from_secs(2)
        ),
        [false]
    );

    // The client falls over to the address that answers.
    let pms = Arc::new(
        Server::with_timeouts(
            &uris,
            info(),
            server_token,
            Duration::from_secs(2),
            Duration::from_secs(20),
        )
        .unwrap(),
    );
    assert_eq!(pms.identity().unwrap().machine_identifier, "abc123");
    assert_eq!(pms.active_address(), base);
    assert!(!format!("{pms:?}").contains("server-token"));

    let fs = PlexFs::new(Arc::clone(&pms));
    let root: Vec<String> = fs
        .list(&VfsPath::root())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(root, ["Movies", "TV Shows", "Music"], "photos left out");

    let p = |s: &str| VfsPath::parse(s).unwrap();
    let movie = p("/Movies/Big Buck Bunny (2008)");
    let names: Vec<String> = fs
        .list(&movie)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(
        names,
        [
            "Big Buck Bunny (2008).mkv",
            "fanart.jpg",
            "movie.nfo",
            "poster.jpg"
        ]
    );
    // Both pages of the movie listing were read.
    assert!(fs.stat(&p("/Movies/Sintel (2010)/Sintel (2010).mp4")).is_ok());
    let starts: Vec<String> = mock
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|t| t.starts_with("/library/sections/1/all"))
        .cloned()
        .collect();
    assert!(starts.iter().any(|t| t.contains("X-Plex-Container-Start=0")));
    assert!(starts.iter().any(|t| t.contains("X-Plex-Container-Start=1")));

    let mut nfo = vec![0; 4096];
    let src = fs.open(&movie.join("movie.nfo").unwrap()).unwrap();
    let n = src.read_at(0, &mut nfo).unwrap();
    let nfo = String::from_utf8_lossy(&nfo[..n]);
    assert!(nfo.contains("<uniqueid type=\"tmdb\">10378</uniqueid>"), "{nfo}");
    assert!(nfo.contains("<uniqueid type=\"imdb\">tt1254207</uniqueid>"));

    let poster = fs.open(&movie.join("poster.jpg").unwrap()).unwrap();
    assert_eq!(poster.size(), 10);

    // The file streams with byte ranges.
    let file = fs.open(&movie.join("Big Buck Bunny (2008).mkv").unwrap()).unwrap();
    assert_eq!(file.size(), 300_000);
    let mut buf = vec![0; 1000];
    file.read_exact_at(123_456, &mut buf).unwrap();
    assert!(buf.iter().enumerate().all(|(i, b)| *b == byte(123_456 + i as u64)));
    let mut tail = vec![0; 100];
    let n = file.read_at(299_950, &mut tail).unwrap();
    assert_eq!(n, 50);
    let media = fs
        .media_at(&movie.join("Big Buck Bunny (2008).mkv").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(media.rating_key, "101");
    assert_eq!(media.view_offset_ms, Some(120_000));

    // Shows and music.
    let ep = p("/TV Shows/Elephants Dream (2006)/Season 01/Elephants Dream S01E02 - Proog.mkv");
    assert!(fs.media_at(&ep).unwrap().unwrap().watched);
    assert!(
        fs.stat(&p("/TV Shows/Elephants Dream (2006)/tvshow.nfo"))
            .is_ok()
    );
    assert!(
        fs.stat(&p("/Music/Test Band/First Album/01 - Opening.flac"))
            .is_ok()
    );
    assert!(fs.stat(&p("/Music/Test Band/First Album/cover.jpg")).is_ok());
    assert_eq!(
        fs.path_of("302").unwrap().unwrap().as_str(),
        "/Music/Test Band/First Album/02 - Closing.mp3"
    );

    // Music playlists.
    let lists = pms.playlists("audio").unwrap();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].title, "Road Trip");
    let items = pms.playlist_items(&lists[0].rating_key).unwrap();
    assert_eq!(
        items.iter().map(|i| i.rating_key.as_str()).collect::<Vec<_>>(),
        ["301", "302"]
    );
}

#[test]
fn a_home_user_sees_their_own_libraries() {
    let (_mock, base) = start();
    let tv = PlexTv::with_base(info(), &base);
    let token = sign_in(&tv);
    let kid = tv.switch_user(&token, "kid-uuid", Some("1234")).unwrap();
    let server = tv.servers(&kid).unwrap().remove(0);
    let pms = Server::new(
        &[base.clone()],
        info(),
        Token::new(server.access_token.unwrap()),
    )
    .unwrap();
    let fs = PlexFs::new(Arc::new(pms));
    let root: Vec<String> = fs
        .list(&VfsPath::root())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(root, ["TV Shows"]);
    // A wrong token is an authentication error, without the token.
    let bad = PlexFs::new(Arc::new(
        Server::new(&[base], info(), Token::new("wrong-secret")).unwrap(),
    ));
    let e = bad.list(&VfsPath::root()).unwrap_err();
    assert_eq!(e.kind(), pa_vfs::ErrorKind::AuthenticationFailed);
    assert!(!e.to_string().contains("wrong-secret"));
}
