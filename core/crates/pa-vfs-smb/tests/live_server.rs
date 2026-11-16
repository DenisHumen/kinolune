// SPDX-License-Identifier: MPL-2.0
//! The SMB add flow against any real server (UI-029): list the server's
//! shares, list a folder, read bytes, and the plain errors for a wrong
//! password and a missing share. CI runs it on a Windows runner against
//! the runner's own SMB server (`libsmb2.yml`, job `windows-smb`); it
//! works the same against Samba.
//!
//! Skipped (passes without checking anything) unless `PA_SMB2_LIB` and
//! `PA_SMB_LIVE_SERVERS` are set:
//!
//! - `PA_SMB_LIVE_SERVERS`: comma-separated addresses of the same server
//!   (`127.0.0.1,localhost,<hostname>`), each one is tried;
//! - `PA_SMB_LIVE_USER`, `PA_SMB_LIVE_PASS`: an account of that server;
//! - `PA_SMB_LIVE_SHARE`: a share whose root holds `hello.txt` with the
//!   text `kinolune-smb-ok` and a folder `Фильмы` with `sample.bin`, the
//!   bytes 0, 1, …, 255 repeated 4096 times (1 MiB).

use std::time::Duration;

use pa_smb2::{LIB_ENV, Smb2Lib};
use pa_vfs::{ErrorKind, Vfs, VfsPath};
use pa_vfs_smb::{Credentials, SmbConfig, SmbFs, check_folder, list_shares};

const HELLO: &[u8] = b"kinolune-smb-ok";
const SAMPLE_LEN: u64 = 256 * 4096;

struct Live {
    lib: &'static Smb2Lib,
    servers: Vec<String>,
    user: String,
    pass: String,
    share: String,
}

fn live() -> Option<Live> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let (Some(_), Some(servers)) = (var(LIB_ENV), var("PA_SMB_LIVE_SERVERS")) else {
        eprintln!("{LIB_ENV} or PA_SMB_LIVE_SERVERS is not set: skipping");
        return None;
    };
    Some(Live {
        lib: Smb2Lib::shared().expect("load libsmb2"),
        servers: servers
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
        user: var("PA_SMB_LIVE_USER").expect("PA_SMB_LIVE_USER"),
        pass: var("PA_SMB_LIVE_PASS").expect("PA_SMB_LIVE_PASS"),
        share: var("PA_SMB_LIVE_SHARE").expect("PA_SMB_LIVE_SHARE"),
    })
}

impl Live {
    fn config(&self, server: &str, share: &str, pass: &str) -> SmbConfig {
        SmbConfig::new(server, share)
            .with_library(self.lib)
            .with_credentials(Credentials::new(&self.user, pass))
            .with_timeout(Duration::from_secs(20))
            .with_max_connections(2)
    }
}

fn path(p: &str) -> VfsPath {
    VfsPath::parse(p).expect("path")
}

#[test]
fn shares_folders_and_bytes() {
    let Some(live) = live() else { return };
    for server in &live.servers {
        eprintln!("server {server}");
        // Step 2 of the add flow: sign in and list the shares (IPC$).
        let shares = list_shares(&live.config(server, "IPC$", &live.pass))
            .unwrap_or_else(|e| panic!("{server}: list shares: {e}"));
        let names: Vec<_> = shares.iter().map(|s| s.name.as_str()).collect();
        eprintln!("  shares: {names:?}");
        assert!(
            names.iter().any(|n| n.eq_ignore_ascii_case(&live.share)),
            "{server}: {} not in {names:?}",
            live.share
        );
        assert!(
            !names.iter().any(|n| n.ends_with('$')),
            "{server}: administrative shares listed: {names:?}"
        );

        // Step 3: the folder browser.
        let config = live.config(server, &live.share, &live.pass);
        check_folder(&config.clone().with_root("Фильмы"))
            .unwrap_or_else(|e| panic!("{server}: check folder: {e}"));
        let fs = SmbFs::new(config).expect("SmbFs::new");
        let mut root: Vec<_> = fs
            .list(&VfsPath::root())
            .unwrap_or_else(|e| panic!("{server}: list root: {e}"))
            .into_iter()
            .map(|e| (e.name, e.metadata.is_dir()))
            .collect();
        root.sort();
        eprintln!("  root: {root:?}");
        assert!(root.contains(&("hello.txt".into(), false)), "{root:?}");
        assert!(root.contains(&("Фильмы".into(), true)), "{root:?}");

        // Playback and scanning: bytes at any offset.
        let hello = fs.open(&path("/hello.txt")).expect("open hello.txt");
        let mut buf = vec![0; HELLO.len()];
        hello.read_exact_at(0, &mut buf).expect("read hello.txt");
        assert_eq!(buf, HELLO);

        let sample = fs.open(&path("/Фильмы/sample.bin")).expect("open sample");
        assert_eq!(sample.size(), SAMPLE_LEN);
        for offset in [0, 1, 255, 65_536 + 7, SAMPLE_LEN - 300] {
            let mut buf = vec![0; 300.min(usize::try_from(SAMPLE_LEN - offset).unwrap())];
            sample.read_exact_at(offset, &mut buf).expect("read sample");
            for (i, b) in buf.iter().enumerate() {
                assert_eq!(
                    u64::from(*b),
                    (offset + i as u64) % 256,
                    "byte {offset}+{i}"
                );
            }
        }
        let mut whole = vec![0; usize::try_from(SAMPLE_LEN).unwrap()];
        sample.read_exact_at(0, &mut whole).expect("read 1 MiB");
        assert!(
            whole
                .iter()
                .enumerate()
                .all(|(i, b)| usize::from(*b) == i % 256)
        );
    }
}

#[test]
fn plain_errors() {
    let Some(live) = live() else { return };
    let server = &live.servers[0];
    let e =
        list_shares(&live.config(server, "IPC$", "wrong-password")).expect_err("wrong password");
    eprintln!("wrong password: {e}");
    assert_eq!(e.kind(), ErrorKind::AuthenticationFailed, "{e}");

    let e = SmbFs::new(live.config(server, "no_such_share", &live.pass))
        .and_then(|fs| fs.list(&VfsPath::root()))
        .expect_err("missing share");
    eprintln!("missing share: {e}");
    assert_eq!(e.kind(), ErrorKind::NotFound, "{e}");

    let e = check_folder(
        &live
            .config(server, &live.share, &live.pass)
            .with_root("nope"),
    )
    .expect_err("missing folder");
    eprintln!("missing folder: {e}");
    assert_eq!(e.kind(), ErrorKind::NotFound, "{e}");
}
