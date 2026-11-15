// SPDX-License-Identifier: MPL-2.0
//! The pa-vfs contract suite against vsftpd in Docker (NET-006 "done
//! when"): plain FTP, explicit FTPS (`AUTH TLS`) and implicit FTPS, with a
//! pinned self-signed certificate. Run with `tests/servers/run.sh`, which
//! starts the servers, seeds the fixture and sets the variables read here.
#![cfg(feature = "server-tests")]

use pa_vfs::contract::Harness;
use pa_vfs::{ByteSource, ErrorKind, Vfs, VfsPath};
use pa_vfs_ftp::{Credentials, FtpConfig, FtpFs, Security, TlsTrust};

/// Fixture location, next to unrelated data; vsftpd chroots the user to
/// the data folder, so the login folder is `/`.
const BASE: &str = "/media/контракт";

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set: use tests/servers/run.sh"))
}

fn config(var_name: &str, security: Security) -> FtpConfig {
    let addr = var(var_name);
    let (host, port) = addr.rsplit_once(':').expect("host:port");
    let mut c = FtpConfig::new(host)
        .with_port(port.parse().expect("port"))
        .with_security(security)
        .with_credentials(Credentials::new(var("PA_FTP_USER"), var("PA_FTP_PASS")))
        // 64 KiB exercises both cached and streamed reads on the 300 KB sample.
        .with_read_ahead(64 * 1024);
    if security != Security::Plain {
        c = c.with_tls(TlsTrust::Pinned(var("PA_FTP_PIN").parse().expect("pin")));
    }
    c
}

fn server(var_name: &str, security: Security, root: &str) -> Harness {
    let fs = FtpFs::new(config(var_name, security).with_root(root)).expect("FtpFs::new");
    Harness::new(fs, VfsPath::parse(BASE).expect("base"))
}

mod plain {
    use super::*;
    pa_vfs::vfs_contract_tests!(|| server("PA_FTP_PLAIN", Security::Plain, ""));
}

mod explicit_tls {
    use super::*;
    pa_vfs::vfs_contract_tests!(|| server("PA_FTP_EXPLICIT", Security::Explicit, "/"));
}

mod implicit_tls {
    use super::*;
    pa_vfs::vfs_contract_tests!(|| server("PA_FTP_IMPLICIT", Security::Implicit, ""));
}

#[test]
fn a_subfolder_root() {
    // Absolute, and relative to the login folder (`/` in the chroot).
    for root in ["/media/контракт/dir", "media/контракт/dir"] {
        let fs = FtpFs::new(config("PA_FTP_PLAIN", Security::Plain).with_root(root)).unwrap();
        let mut names: Vec<_> = fs
            .list(&VfsPath::root())
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        assert_eq!(names, ["nested.txt", "sub"], "{root}");
    }
}

#[test]
fn wrong_password_is_refused_without_echo() {
    let wrong = "wrong-pass-0451";
    for (v, s) in [
        ("PA_FTP_PLAIN", Security::Plain),
        ("PA_FTP_EXPLICIT", Security::Explicit),
    ] {
        let fs =
            FtpFs::new(config(v, s).with_credentials(Credentials::new(var("PA_FTP_USER"), wrong)))
                .unwrap();
        let e = fs.stat(&VfsPath::root()).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::AuthenticationFailed, "{s:?}: {e}");
        assert!(!e.to_string().contains(wrong), "{e}");
    }
}

#[test]
fn anonymous_login_is_refused() {
    let mut c = config("PA_FTP_PLAIN", Security::Plain);
    c.credentials = None;
    let e = FtpFs::new(c).unwrap().stat(&VfsPath::root()).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::AuthenticationFailed, "{e}");
}

#[test]
fn plain_login_is_refused_where_tls_is_required() {
    // The explicit server demands TLS before the password.
    let e = FtpFs::new(config("PA_FTP_EXPLICIT", Security::Plain))
        .unwrap()
        .stat(&VfsPath::root())
        .unwrap_err();
    assert_ne!(e.kind(), ErrorKind::NotFound, "{e}");
}

#[test]
fn an_unpinned_self_signed_certificate_is_refused() {
    let fs = FtpFs::new(config("PA_FTP_EXPLICIT", Security::Explicit).with_tls(TlsTrust::System))
        .unwrap();
    let e = fs.stat(&VfsPath::root()).unwrap_err();
    assert!(
        e.to_string().contains("TLS: invalid peer certificate"),
        "{e}"
    );
    let other_pin = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
    let fs = FtpFs::new(
        config("PA_FTP_IMPLICIT", Security::Implicit)
            .with_tls(TlsTrust::Pinned(other_pin.parse().unwrap())),
    )
    .unwrap();
    assert!(fs.stat(&VfsPath::root()).is_err());
}

#[test]
fn sequential_reads_continue_one_transfer_and_seeks_restart_it() {
    let fs = FtpFs::new(config("PA_FTP_PLAIN", Security::Plain).with_read_ahead(0)).unwrap();
    let src = fs
        .open(&VfsPath::parse(&format!("{BASE}/sample.bin")).unwrap())
        .unwrap();
    let want = pa_vfs::contract::sample_bytes();
    // Forward in small steps, back to the start, then the tail.
    let mut buf = [0u8; 4096];
    for i in 0..20u64 {
        let off = i * 4096;
        src.read_exact_at(off, &mut buf).unwrap();
        assert_eq!(&buf[..], &want[off as usize..off as usize + 4096]);
    }
    src.read_exact_at(0, &mut buf).unwrap();
    assert_eq!(&buf[..], &want[..4096]);
    let tail = want.len() as u64 - 100;
    let mut end = [0u8; 100];
    src.read_exact_at(tail, &mut end).unwrap();
    assert_eq!(&end[..], &want[want.len() - 100..]);
    // The kept transfer at the start continues after the tail read.
    src.read_exact_at(4096, &mut buf).unwrap();
    assert_eq!(&buf[..], &want[4096..8192]);
}
