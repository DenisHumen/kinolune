// SPDX-License-Identifier: MPL-2.0
//! The pa-vfs contract suite against OpenSSH in Docker (NET-006 "done
//! when"), with password and key logins, plus host key pinning. Run with
//! `tests/servers/run.sh`, which starts the server, seeds the fixture and
//! sets the variables read here.
#![cfg(feature = "server-tests")]

use std::sync::{Arc, Mutex};

use pa_vfs::contract::Harness;
use pa_vfs::{ErrorKind, Vfs, VfsPath};
use pa_vfs_sftp::{HostKey, HostKeyTrust, SftpAuth, SftpConfig, SftpFs, is_host_key_changed};

/// Fixture location, next to unrelated data; the source root is `/data`,
/// the user's home.
const BASE: &str = "/media/контракт";

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set: use tests/servers/run.sh"))
}

fn config(root: &str, auth: SftpAuth) -> SftpConfig {
    let addr = var("PA_SFTP_ADDR");
    let (host, port) = addr.rsplit_once(':').expect("host:port");
    SftpConfig::new(host, var("PA_SFTP_USER"))
        .with_port(port.parse().expect("port"))
        .with_root(root)
        .with_auth(auth)
        // 64 KiB exercises both cached and pipelined reads on the 300 KB sample.
        .with_read_ahead(64 * 1024)
}

fn password() -> SftpAuth {
    SftpAuth::Password(var("PA_SFTP_PASS"))
}

fn key(var_name: &str, passphrase: Option<String>) -> SftpAuth {
    SftpAuth::PrivateKey {
        key: std::fs::read_to_string(var(var_name)).expect("read key"),
        passphrase,
    }
}

/// Fingerprints of the server's host keys, as `ssh-keygen -l` prints them.
fn host_fingerprints() -> Vec<String> {
    var("PA_SFTP_HOST_FPS")
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

mod password_login {
    use super::*;
    pa_vfs::vfs_contract_tests!(|| Harness::new(
        SftpFs::new(config("/data", password())).expect("SftpFs::new"),
        VfsPath::parse(BASE).expect("base"),
    ));
}

mod key_login {
    use super::*;
    // Home folder as the root (`~`), ed25519 key.
    pa_vfs::vfs_contract_tests!(|| Harness::new(
        SftpFs::new(config("~", key("PA_SFTP_KEY_ED25519", None))).expect("SftpFs::new"),
        VfsPath::parse(BASE).expect("base"),
    ));
}

#[test]
fn encrypted_ecdsa_key_logs_in() {
    let fs = SftpFs::new(config(
        "/data/media",
        key("PA_SFTP_KEY_ECDSA", Some(var("PA_SFTP_KEY_PASS"))),
    ))
    .unwrap();
    let names: Vec<_> = fs
        .list(&VfsPath::root())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(names.contains(&"контракт".to_owned()), "{names:?}");
    assert!(names.contains(&"other.txt".to_owned()), "{names:?}");
}

#[test]
fn wrong_key_passphrase_is_an_auth_error() {
    let fs = SftpFs::new(config(
        "/data",
        key("PA_SFTP_KEY_ECDSA", Some("not-the-passphrase".into())),
    ))
    .unwrap();
    let e = fs.stat(&VfsPath::root()).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::AuthenticationFailed, "{e}");
}

#[test]
fn wrong_password_is_refused_without_echo() {
    let wrong = "wrong-pass-0451";
    let fs = SftpFs::new(config("/data", SftpAuth::Password(wrong.into()))).unwrap();
    let e = fs.stat(&VfsPath::root()).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::AuthenticationFailed, "{e}");
    assert!(!e.to_string().contains(wrong), "{e}");
    assert!(!format!("{e:?}").contains(wrong), "{e:?}");
}

#[test]
fn anonymous_login_is_refused() {
    let fs = SftpFs::new(config("/data", SftpAuth::None)).unwrap();
    let e = fs.stat(&VfsPath::root()).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::AuthenticationFailed, "{e}");
}

#[test]
fn first_use_learns_and_reports_the_host_key() {
    let seen: Arc<Mutex<Vec<HostKey>>> = Arc::default();
    let store = Arc::clone(&seen);
    let fs = SftpFs::new(
        config("/data", password()).with_host_key(HostKeyTrust::FirstUseWith(Arc::new(
            move |k: &HostKey| store.lock().unwrap().push(k.clone()),
        ))),
    )
    .unwrap();
    assert_eq!(fs.host_key(), None);
    fs.stat(&VfsPath::root()).unwrap();
    fs.list(&VfsPath::root()).unwrap();
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "reported once: {seen:?}");
    assert!(
        host_fingerprints().contains(&seen[0].fingerprint().to_owned()),
        "{} is not one of the server's keys {:?}",
        seen[0],
        host_fingerprints()
    );
    assert_eq!(fs.host_key().as_ref(), Some(&seen[0]));

    // The stored form parses back and works as a pin.
    let pin: HostKey = seen[0].to_string().parse().unwrap();
    let pinned =
        SftpFs::new(config("/data", password()).with_host_key(HostKeyTrust::Pinned(pin))).unwrap();
    assert!(
        pinned
            .stat(&VfsPath::parse(BASE).unwrap())
            .unwrap()
            .is_dir()
    );
}

#[test]
fn a_changed_host_key_is_refused_before_login() {
    let fake: HostKey = "ssh-ed25519 SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        .parse()
        .unwrap();
    // A wrong password too: the host key check must fail first.
    let fs = SftpFs::new(
        config("/data", SftpAuth::Password("never-sent".into()))
            .with_host_key(HostKeyTrust::Pinned(fake.clone())),
    )
    .unwrap();
    let e = fs.list(&VfsPath::root()).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::AuthenticationFailed, "{e}");
    assert!(is_host_key_changed(&e), "{e}");
    let changed = pa_vfs_sftp::host_key_changed(&e).unwrap();
    assert_eq!(changed.expected, fake);
    assert!(host_fingerprints().contains(&changed.actual.fingerprint().to_owned()));
    assert!(e.to_string().contains("host key changed"), "{e}");
    // The pin stays: a second try fails the same way.
    let again = fs.stat(&VfsPath::root()).unwrap_err();
    assert!(is_host_key_changed(&again), "{again}");
}

#[test]
fn a_missing_root_is_not_found() {
    let fs = SftpFs::new(config("/data/no-such-folder", password())).unwrap();
    let e = fs.list(&VfsPath::root()).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotFound, "{e}");
}
