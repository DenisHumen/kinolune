// SPDX-License-Identifier: MPL-2.0
//! The NET-023 write contract against real servers in Docker ("done
//! when"): Samba, Apache mod_dav, rclone serve webdav, OpenSSH (SFTP),
//! vsftpd (FTP) and a local folder. For each: every case of
//! `pa_vfs::write_contract`, a 1 GiB upload broken in the middle and
//! resumed, and a read-only variant; then moves between the sources.
//! Run with `tests/write_servers/run.sh`, which starts the servers and
//! sets the variables read here.
#![cfg(feature = "server-tests")]

use std::time::Duration;

use pa_vfs::write_contract as wc;
use pa_vfs::{LocalFs, Vfs, VfsPath};
use pa_vfs_ftp::{FtpConfig, FtpFs};
use pa_vfs_sftp::{SftpAuth, SftpConfig, SftpFs};
use pa_vfs_smb::{SmbConfig, SmbFs};
use pa_vfs_webdav::{WebDavConfig, WebDavFs};

fn var(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: use tests/write_servers/run.sh"))
}

fn big() -> u64 {
    var("PA_W_BIG").parse().expect("PA_W_BIG")
}

fn host_port(v: &str) -> (String, u16) {
    let addr = var(v);
    let (h, p) = addr.rsplit_once(':').expect("host:port");
    (h.to_owned(), p.parse().expect("port"))
}

fn smb(share: &str) -> SmbFs {
    SmbFs::new(
        SmbConfig::new(var("PA_W_SMB"), share)
            .with_credentials(pa_vfs_smb::Credentials::new(
                var("PA_W_USER"),
                var("PA_W_PASS"),
            ))
            .with_timeout(Duration::from_secs(60)),
    )
    .expect("SmbFs::new")
}

fn dav(v: &str) -> WebDavFs {
    WebDavFs::new(
        WebDavConfig::new(var(v)).with_credentials(pa_vfs_webdav::Credentials::new(
            var("PA_W_USER"),
            var("PA_W_PASS"),
        )),
    )
    .expect("WebDavFs::new")
}

fn sftp(root: &str) -> SftpFs {
    let (host, port) = host_port("PA_W_SFTP");
    SftpFs::new(
        SftpConfig::new(host, var("PA_W_USER"))
            .with_port(port)
            .with_root(root)
            .with_auth(SftpAuth::Password(var("PA_W_PASS"))),
    )
    .expect("SftpFs::new")
}

fn ftp(root: &str) -> FtpFs {
    let (host, port) = host_port("PA_W_FTP");
    FtpFs::new(
        FtpConfig::new(host)
            .with_port(port)
            .with_root(root)
            .with_credentials(pa_vfs_ftp::Credentials::new(
                var("PA_W_USER"),
                var("PA_W_PASS"),
            )),
    )
    .expect("FtpFs::new")
}

fn local() -> LocalFs {
    LocalFs::new(var("PA_W_LOCAL")).expect("LocalFs::new")
}

fn root() -> VfsPath {
    VfsPath::root()
}

/// Big resume with a report on stderr (the PR quotes it).
fn big_resume(name: &str, vfs: &dyn Vfs, expect_resume: bool) {
    let size = big();
    let started = std::time::Instant::now();
    let r = wc::upload_resume(vfs, &root(), size, expect_resume);
    eprintln!(
        "{name}: {size} bytes, .part after the break {} bytes, resumed upload sent {} bytes, {:.1} s",
        r.part_after_break,
        r.sent_on_resume,
        started.elapsed().as_secs_f64()
    );
}

macro_rules! backend {
    ($name:ident, rw: $rw:expr, ro: $ro:expr, ro_dir: $ro_dir:expr, resume: $resume:expr) => {
        mod $name {
            use super::*;

            #[test]
            fn contract() {
                wc::run_all(&$rw, &root(), $resume);
            }

            #[test]
            fn resume_big() {
                big_resume(stringify!($name), &$rw, $resume);
            }

            #[test]
            fn read_only() {
                wc::read_only(&$ro, &VfsPath::parse($ro_dir).expect("path"));
            }
        }
    };
}

backend!(smb_share, rw: smb("rw"), ro: smb("ro"), ro_dir: "/", resume: true);
backend!(sftp_server, rw: sftp("/data/sftp"), ro: sftp("/ro"), ro_dir: "/", resume: true);
backend!(ftp_server, rw: ftp("/ftp"), ro: ftp("/ro"), ro_dir: "/", resume: true);
backend!(webdav_apache, rw: dav("PA_W_APACHE"), ro: dav("PA_W_RCLONE_RO"), ro_dir: "/", resume: true);
// rclone serve webdav ignores `Content-Range` on PUT: a broken upload
// starts over (and must still end up exact).
backend!(webdav_rclone, rw: dav("PA_W_RCLONE"), ro: dav("PA_W_RCLONE_RO"), ro_dir: "/", resume: false);

mod local_folder {
    use super::*;

    #[test]
    fn contract() {
        wc::run_all(&local(), &root(), true);
    }

    #[test]
    fn resume_big() {
        big_resume("local", &local(), true);
    }
}

/// Moves and copies between sources: each pair crosses two protocols.
mod between {
    use super::*;

    #[test]
    fn local_to_smb() {
        wc::between(&local(), &root(), &smb("rw"), &root());
    }

    #[test]
    fn smb_to_sftp() {
        wc::between(&smb("rw"), &root(), &sftp("/data/sftp"), &root());
    }

    #[test]
    fn sftp_to_ftp() {
        wc::between(&sftp("/data/sftp"), &root(), &ftp("/ftp"), &root());
    }

    #[test]
    fn ftp_to_webdav() {
        wc::between(&ftp("/ftp"), &root(), &dav("PA_W_APACHE"), &root());
    }

    #[test]
    fn webdav_to_local() {
        wc::between(&dav("PA_W_RCLONE"), &root(), &local(), &root());
    }

    /// A move to a read-only source fails and keeps the original.
    #[test]
    fn read_only_destination_keeps_the_original() {
        let src = local();
        let dir = wc::scratch(&src, &root(), "to-ro");
        let mut data = std::io::Cursor::new(b"keep me".to_vec());
        let f = pa_vfs::files::upload(
            &src,
            &dir,
            "keep.mkv",
            &mut data,
            pa_vfs::files::Conflict::KeepBoth,
            &mut |_, _| true,
        )
        .expect("upload")
        .path;
        let ro = smb("ro");
        pa_vfs::files::move_between(
            &src,
            &f,
            &ro,
            &root(),
            pa_vfs::files::Conflict::KeepBoth,
            &mut |_, _| true,
        )
        .expect_err("a read-only destination");
        assert_eq!(wc::read_all(&src, &f), b"keep me");
    }
}
