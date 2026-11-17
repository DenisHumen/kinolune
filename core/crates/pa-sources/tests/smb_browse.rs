// SPDX-License-Identifier: MPL-2.0
//! Picking a folder on an SMB server (UI-028) against Samba in Docker:
//! share listing, finding the owner's typed path (`home` +
//! `<user>/smb_shared/torrents/downloads`), guest logon and each error the
//! add flow tells apart. Run with `tests/smb_server/run.sh`.
#![cfg(feature = "server-tests")]

use std::sync::Arc;

use pa_secrets::{MemoryStore, SecretString};
use pa_sources::{
    Error, NewSource, SmbLogin, SmbPlace, SourceKind, SourceManager, smb_find, smb_list_folder,
    smb_shares,
};
use pa_vfs::ErrorKind;

fn server() -> String {
    std::env::var("PA_SMB_SERVER").expect("PA_SMB_SERVER is not set: use tests/smb_server/run.sh")
}

fn pass() -> String {
    std::env::var("PA_SMB_PASS").expect("PA_SMB_PASS")
}

fn login(user: &str, password: &str) -> SmbLogin {
    SmbLogin {
        server: server(),
        username: Some(user.into()),
        password: Some(SecretString::new(password)),
        ..Default::default()
    }
}

fn alex() -> SmbLogin {
    login("alex", &pass())
}

fn guest() -> SmbLogin {
    SmbLogin {
        server: server(),
        ..Default::default()
    }
}

fn kind(e: &Error) -> ErrorKind {
    e.source_kind()
        .unwrap_or_else(|| panic!("not a backend error: {e}"))
}

fn place(share: &str, folder: &str) -> SmbPlace {
    SmbPlace {
        share: share.into(),
        folder: folder.into(),
    }
}

#[test]
fn lists_folder_shares_only() {
    let names: Vec<String> = smb_shares(&alex())
        .expect("shares")
        .into_iter()
        .map(|s| s.name)
        .collect();
    for want in ["smb_shared", "public", "private"] {
        assert!(
            names.iter().any(|n| n == want),
            "{want} missing in {names:?}"
        );
    }
    assert!(!names.iter().any(|n| n.ends_with('$')), "{names:?}");
}

#[test]
fn browses_into_folders() {
    let root = smb_list_folder(&alex(), "smb_shared", "").expect("share root");
    assert_eq!(
        root.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["torrents"]
    );
    let deep = smb_list_folder(&alex(), "smb_shared", "torrents/downloads").expect("folder");
    assert_eq!(deep[0].name, "film.mkv");
    // The user's home share is not listed, but opens.
    let home = smb_list_folder(&alex(), "alex", "").expect("home share");
    assert!(home.iter().any(|e| e.name == "smb_shared"));
}

#[test]
fn finds_the_owners_typed_path() {
    // What the owner typed in alpha.7: share `home`, folder
    // `<user>/smb_shared/torrents/downloads`. `[homes]` (share `alex`) and
    // `smb_shared` lead to the same folder; the more specific share wins.
    let found = smb_find(&alex(), "home", "alex/smb_shared/torrents/downloads").expect("find");
    assert_eq!(found, [place("smb_shared", "torrents/downloads")]);
    // A full disk path in one field works too.
    let found = smb_find(&alex(), "", "/home/alex/smb_shared/torrents/downloads").expect("find");
    assert_eq!(found, [place("smb_shared", "torrents/downloads")]);
    // Without the user in the path: the home share of the user.
    let found = smb_find(&alex(), "home", "Documents").expect("find");
    assert_eq!(found, [place("alex", "Documents")]);
    // A correct address stays as it is.
    let found = smb_find(&alex(), "smb_shared", "torrents").expect("find");
    assert_eq!(found, [place("smb_shared", "torrents")]);
    // Nothing fits.
    assert!(
        smb_find(&alex(), "home", "alex/nope")
            .expect("find")
            .is_empty()
    );
}

#[test]
fn guest_sees_the_guest_share() {
    let names: Vec<String> = smb_shares(&guest())
        .expect("guest shares")
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert!(names.iter().any(|n| n == "public"), "{names:?}");
    let music = smb_list_folder(&guest(), "public", "Music").expect("guest listing");
    assert_eq!(music[0].name, "song.flac");
}

#[test]
fn wrong_password() {
    let e = smb_shares(&login("alex", "wrong-password")).expect_err("logon");
    assert_eq!(kind(&e), ErrorKind::AuthenticationFailed, "{e}");
    assert!(!e.to_string().contains("wrong-password"), "{e}");
    let e = smb_find(&login("alex", "wrong-password"), "home", "alex").expect_err("logon");
    assert_eq!(kind(&e), ErrorKind::AuthenticationFailed, "{e}");
}

#[test]
fn server_unreachable() {
    let mut l = alex();
    l.server = "127.0.0.1:1".into();
    let e = smb_shares(&l).expect_err("unreachable");
    assert!(
        matches!(kind(&e), ErrorKind::Unavailable | ErrorKind::TimedOut),
        "{e}"
    );
}

#[test]
fn missing_share_is_not_a_missing_folder() {
    let e = smb_list_folder(&alex(), "home", "alex").expect_err("no share");
    assert_eq!(kind(&e), ErrorKind::NotFound, "{e}");
    assert!(e.to_string().contains(pa_vfs_smb::SHARE_NOT_FOUND), "{e}");
}

#[test]
fn missing_folder_inside_a_share() {
    let e = smb_list_folder(&alex(), "smb_shared", "nope").expect_err("no folder");
    assert_eq!(kind(&e), ErrorKind::NotFound, "{e}");
    assert!(!e.to_string().contains(pa_vfs_smb::SHARE_NOT_FOUND), "{e}");
}

#[test]
fn share_of_another_user_is_denied() {
    let e = smb_list_folder(&alex(), "private", "").expect_err("denied");
    assert_eq!(kind(&e), ErrorKind::PermissionDenied, "{e}");
}

/// The owner's kind of server (UI-031): login works, the share list is
/// refused, no `[homes]`, the films are in a share named `shared`.
fn hidden(user: &str, password: &str) -> SmbLogin {
    SmbLogin {
        server: std::env::var("PA_SMB_HIDDEN_SERVER")
            .expect("PA_SMB_HIDDEN_SERVER is not set: use tests/smb_server/run.sh"),
        ..login(user, password)
    }
}

#[test]
fn hidden_share_list_is_not_a_failed_login() {
    let e = smb_shares(&hidden("alex", &pass())).expect_err("no list");
    assert_eq!(kind(&e), ErrorKind::PermissionDenied, "{e}");
    assert!(
        e.to_string().contains(pa_vfs_smb::SHARE_LIST_REFUSED),
        "{e}"
    );
    // A wrong password is still a wrong password.
    let e = smb_shares(&hidden("alex", "wrong-password")).expect_err("logon");
    assert_eq!(kind(&e), ErrorKind::AuthenticationFailed, "{e}");
    assert!(
        !e.to_string().contains(pa_vfs_smb::SHARE_LIST_REFUSED),
        "{e}"
    );
    // And a dead server is a dead server.
    let mut l = hidden("alex", &pass());
    l.server = "127.0.0.1:1".into();
    let e = smb_shares(&l).expect_err("unreachable");
    assert!(
        matches!(kind(&e), ErrorKind::Unavailable | ErrorKind::TimedOut),
        "{e}"
    );
}

#[test]
fn hidden_shares_are_guessed_from_common_names() {
    // Nothing typed: the common name `shared` opens; the user's home share
    // does not exist and is not offered.
    let found = smb_find(&hidden("alex", &pass()), "", "").expect("find");
    assert_eq!(found, [place("shared", "")]);
    // The disk path the owner knows: `shared` + the tail found in it.
    let found = smb_find(
        &hidden("alex", &pass()),
        "",
        "/home/alex/smb_shared/torrents/downloads",
    )
    .expect("find");
    assert_eq!(found, [place("shared", "torrents/downloads")]);
    // The right address typed in step 1 keeps working.
    let found = smb_find(&hidden("alex", &pass()), "shared", "torrents/downloads").expect("find");
    assert_eq!(found, [place("shared", "torrents/downloads")]);
    // A typed share name opens and lists like on any server.
    let root = smb_list_folder(&hidden("alex", &pass()), "shared", "").expect("share");
    assert!(root.iter().any(|e| e.name == "torrents"), "{root:?}");
    let e = smb_list_folder(&hidden("alex", &pass()), "films", "").expect_err("no share");
    assert!(e.to_string().contains(pa_vfs_smb::SHARE_NOT_FOUND), "{e}");
    // A wrong password stops the guessing.
    let e = smb_find(&hidden("alex", "wrong-password"), "", "").expect_err("logon");
    assert_eq!(kind(&e), ErrorKind::AuthenticationFailed, "{e}");
}

#[test]
fn listed_server_needs_no_guessing() {
    // With a share list nothing typed means nothing to find.
    assert!(smb_find(&alex(), "", "").expect("find").is_empty());
}

#[test]
fn broken_source_is_diagnosed_and_fixed() {
    let dir = tempfile::tempdir().unwrap();
    let m =
        SourceManager::open(dir.path().join("library.db"), Arc::new(MemoryStore::new())).unwrap();
    let address = format!("smb://{}/home/alex/smb_shared/torrents/downloads", server());
    let src = m
        .create(
            NewSource::new(SourceKind::Smb, "Films", address)
                .with_login("alex", SecretString::new(pass())),
        )
        .unwrap();
    let status = m.check(src.id).unwrap();
    assert_eq!(status.online, Some(false));
    let error = status.last_error.unwrap();
    assert!(error.contains(pa_vfs_smb::SHARE_NOT_FOUND), "{error}");
    // "Choose folder" on the broken source: the stored login finds it.
    let login = m.smb_login_of(src.id).unwrap();
    let found = smb_find(&login, "home", "alex/smb_shared/torrents/downloads").unwrap();
    assert_eq!(found, [place("smb_shared", "torrents/downloads")]);
}
