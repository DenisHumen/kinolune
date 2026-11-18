// SPDX-License-Identifier: MPL-2.0
//! NET-012 in the source manager, against the plex.tv and Plex Media
//! Server stand-in of `pa-plex` (synthetic answers): the PIN sign-in, the
//! Home user with a PIN, the automatic choice of the connection, the
//! source with the server token in the secret store only, its libraries
//! as files and its music playlists (MUS-012).

use std::sync::Arc;

use pa_secrets::{MemoryStore, SecretStore};
use pa_sources::pa_plex::ConnectionKind;
use pa_sources::{PlexSignIn, SourceKind, SourceManager};
use pa_vfs::VfsPath;

#[path = "../../pa-plex/tests/support/mock.rs"]
mod mock;

fn names(m: &SourceManager, id: i64, dir: &str) -> Vec<String> {
    let fs = m.connect(id).unwrap();
    fs.list(&VfsPath::parse(dir).unwrap())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect()
}

#[test]
fn signs_in_with_a_code_and_adds_the_server() {
    let (_mock, base) = mock::start();
    let store = Arc::new(MemoryStore::new());
    let m = SourceManager::new(
        pa_library::open_in_memory().unwrap(),
        Arc::clone(&store) as Arc<dyn SecretStore>,
    );
    let mut sign_in = PlexSignIn::start_at(&base, "Test Device").unwrap();
    assert_eq!(sign_in.code(), "K7Q2");
    // Not entered yet, then entered at plex.tv/link.
    assert!(!sign_in.poll().unwrap());
    assert!(sign_in.poll().unwrap());
    assert_eq!(sign_in.account().unwrap().display_name(), "Owner");
    assert_eq!(sign_in.users().len(), 2);
    assert_eq!(sign_in.current_user(), Some("owner-uuid"));
    let server = sign_in
        .suggested_server(Some("http://127.0.0.1:32400"))
        .unwrap()
        .client_identifier
        .clone();
    assert_eq!(server, "abc123");
    assert!(!format!("{sign_in:?}").contains("account-token"));

    let account = sign_in.finish(&server).unwrap();
    // The plex.direct name does not resolve here; its plain LAN IP answers.
    assert_eq!(account.connected, (base.clone(), ConnectionKind::Local));
    assert_eq!(account.addresses.len(), 4);
    let source = m.create_plex(account, None, Vec::new()).unwrap();
    assert_eq!(source.kind, SourceKind::Plex);
    assert_eq!(source.name, "Living Room");
    assert_eq!(source.username.as_deref(), Some("Owner"));
    assert!(source.has_password);
    let o = source.options.plex.clone().unwrap();
    assert_eq!(o.server_id, "abc123");
    assert_eq!(o.alt_addresses.len(), 3);
    // The token is in the secret store, not in the database row.
    let json = serde_json::to_string(&source.options).unwrap();
    assert!(!json.contains("server-token"));
    assert!(!format!("{source:?}").contains("server-token"));

    // The source tries the plex.direct name first and falls over to the
    // address that answers.
    assert_eq!(names(&m, source.id, "/"), ["Movies", "TV Shows", "Music"]);
    assert_eq!(
        names(&m, source.id, "/Movies/Big Buck Bunny (2008)"),
        [
            "Big Buck Bunny (2008).mkv",
            "fanart.jpg",
            "movie.nfo",
            "poster.jpg"
        ]
    );
    m.check(source.id).unwrap();
    assert_eq!(m.get(source.id).unwrap().status.online, Some(true));

    // MUS-012: the server's music playlists as paths of the tree.
    let lists = m.server_playlists(source.id).unwrap().unwrap();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].name, "Road Trip");
    assert_eq!(lists[0].remote_id, "30");
    assert_eq!(
        lists[0].paths,
        [
            "/Music/Test Band/First Album/01 - Opening.flac",
            "/Music/Test Band/First Album/02 - Closing.mp3"
        ]
    );
}

#[test]
fn a_protected_home_user_needs_their_pin() {
    let (_mock, base) = mock::start();
    let m = SourceManager::new(
        pa_library::open_in_memory().unwrap(),
        Arc::new(MemoryStore::new()),
    );
    let mut sign_in = PlexSignIn::start_at(&base, "TV").unwrap();
    assert!(sign_in.finish("abc123").is_err(), "not signed in yet");
    while !sign_in.poll().unwrap() {}
    let wrong = sign_in.switch_user("kid-uuid", Some("9999")).unwrap_err();
    assert_eq!(
        wrong.source_kind(),
        Some(pa_vfs::ErrorKind::PermissionDenied)
    );
    sign_in.switch_user("kid-uuid", Some("1234")).unwrap();
    assert_eq!(sign_in.current_user(), Some("kid-uuid"));
    assert!(sign_in.finish("nope").is_err());
    let account = sign_in.finish("abc123").unwrap();
    assert_eq!(account.user_title, "Kid");
    let source = m
        .create_plex(account, Some("Kids"), Vec::new())
        .unwrap();
    assert_eq!(source.name, "Kids");
    // The kid's server token sees only their library.
    assert_eq!(names(&m, source.id, "/"), ["TV Shows"]);
    assert_eq!(m.server_playlists(source.id).unwrap().unwrap().len(), 1);
    // Back to the account.
    sign_in.switch_user("owner-uuid", None).unwrap();
    assert_eq!(sign_in.current_user(), Some("owner-uuid"));
}
