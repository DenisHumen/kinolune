// SPDX-License-Identifier: MPL-2.0
//! SYNC-007 against real servers in Docker: three devices sync through
//! journals in one folder (`pa_sync::folder`), each over another protocol
//! — Samba, Apache mod_dav (appends) or rclone serve webdav (no appends) —
//! plus SFTP and the host folder underneath, offline and online in any
//! order, and converge. A read-only share only receives.
//!
//! Run with `PA_W_TEST=folder_sync_servers tests/write_servers/run.sh`,
//! which starts the servers of the NET-023 write contract and sets the
//! variables read here.
#![cfg(feature = "server-tests")]

use std::path::PathBuf;
use std::time::Duration;

use pa_library::rusqlite::Connection;
use pa_model::{MediaKey, MediaKind};
use pa_sync::folder::{self, Cursors, Folder};
use pa_sync::{Op, OpBody, Replica, SyncState};
use pa_vfs::{LocalFs, Vfs};
use pa_vfs_sftp::{SftpAuth, SftpConfig, SftpFs};
use pa_vfs_smb::{SmbConfig, SmbFs};
use pa_vfs_webdav::{WebDavConfig, WebDavFs};

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} is not set: use PA_W_TEST=folder_sync_servers tests/write_servers/run.sh")
    })
}

fn smb(share: &str) -> Box<dyn Vfs> {
    Box::new(
        SmbFs::new(
            SmbConfig::new(var("PA_W_SMB"), share)
                .with_credentials(pa_vfs_smb::Credentials::new(
                    var("PA_W_USER"),
                    var("PA_W_PASS"),
                ))
                .with_timeout(Duration::from_secs(60)),
        )
        .expect("SmbFs::new"),
    )
}

fn dav(v: &str) -> Box<dyn Vfs> {
    Box::new(
        WebDavFs::new(
            WebDavConfig::new(var(v)).with_credentials(pa_vfs_webdav::Credentials::new(
                var("PA_W_USER"),
                var("PA_W_PASS"),
            )),
        )
        .expect("WebDavFs::new"),
    )
}

fn sftp(root: &str) -> Box<dyn Vfs> {
    let addr = var("PA_W_SFTP");
    let (host, port) = addr.rsplit_once(':').expect("host:port");
    Box::new(
        SftpFs::new(
            SftpConfig::new(host, var("PA_W_USER"))
                .with_port(port.parse().expect("port"))
                .with_root(root)
                .with_auth(SftpAuth::Password(var("PA_W_PASS"))),
        )
        .expect("SftpFs::new"),
    )
}

/// The host folder a server serves (`$PA_W_DATA/<dir>`).
fn host(dir: &str) -> PathBuf {
    PathBuf::from(var("PA_W_DATA")).join(dir)
}

fn local(dir: &str) -> Box<dyn Vfs> {
    Box::new(LocalFs::new(host(dir)).expect("LocalFs::new"))
}

/// Empties the journals folder and opens it to every server's user (each
/// runs as another uid; the folder is shared like on a NAS).
fn fresh_state(dir: &str) {
    use std::os::unix::fs::PermissionsExt;
    let state = host(dir).join(".kinolune/state");
    let _ = std::fs::remove_dir_all(&state);
    std::fs::create_dir_all(&state).unwrap();
    for d in [host(dir).join(".kinolune"), state] {
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o777)).unwrap();
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const KEYS: [&str; 3] = ["tmdb:movie:603", "tmdb:episode:1399:1:1", "twitch:vod:123"];

fn key(i: u64) -> MediaKey {
    KEYS[usize::try_from(i).unwrap()].parse().unwrap()
}

fn body(rng: &mut Rng) -> OpBody {
    let profile = "default".to_owned();
    match rng.below(4) {
        0 | 1 => OpBody::ProgressUpsert {
            profile,
            key: key(rng.below(3)),
            kind: MediaKind::Movie,
            position_ms: rng.below(5) * 1000,
            duration_ms: Some(10_000),
        },
        2 => OpBody::Watched {
            profile,
            key: key(rng.below(3)),
        },
        _ => OpBody::Unwatch {
            profile,
            key: key(rng.below(3)),
        },
    }
}

fn outbox_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../pa-library/migrations/202611141200_SYNC-001_sync_outbox.sql"
    ))
    .unwrap();
    conn
}

struct Device {
    id: String,
    vfs: Box<dyn Vfs>,
    replica: Replica,
    conn: Connection,
    unsent: Vec<Op>,
    cursors: Cursors,
    snapshot: u64,
}

impl Device {
    fn new(id: &str, vfs: Box<dyn Vfs>) -> Self {
        Self {
            id: id.to_owned(),
            vfs,
            replica: Replica::new(id).unwrap(),
            conn: outbox_db(),
            unsent: Vec::new(),
            cursors: Cursors::new(),
            snapshot: 0,
        }
    }

    fn sync(&mut self, now: u64, compact: bool) -> folder::ReadReport {
        let f = Folder::new(self.vfs.as_ref());
        let written = f.write(&self.id, &self.unsent, now).unwrap();
        assert!(!written.read_only, "{}: read-only", self.id);
        self.unsent.clear();
        let replica = &mut self.replica;
        let report = f
            .read(&self.id, &mut self.cursors, now, &mut |op| {
                replica
                    .receive(op, i64::try_from(now).unwrap())
                    .map_err(|e| e.to_string())
            })
            .unwrap();
        if (compact || folder::should_compact(written.size, self.snapshot))
            && let Some(size) = f.compact(&self.id, now).unwrap()
        {
            self.snapshot = size;
        }
        report
    }
}

/// Empties the journals folder through the server itself: rclone caches
/// what it saw, so files removed behind its back would still be served.
fn fresh_state_via(vfs: &dyn Vfs) {
    let dir = pa_vfs::VfsPath::parse(folder::DIR).unwrap();
    let w = vfs.as_write().expect("writable");
    if let Ok(entries) = vfs.list(&dir) {
        for e in entries {
            w.remove_file(&e.path).unwrap();
        }
    }
}

/// Random scenarios of three devices, each over its own protocol, on the
/// host folder `dir` (`cleaner`: empty it through this server instead).
fn converge(
    dir: &str,
    make: &[fn() -> Box<dyn Vfs>; 3],
    seeds: std::ops::Range<u64>,
    cleaner: Option<fn() -> Box<dyn Vfs>>,
) {
    for seed in seeds {
        match cleaner {
            Some(c) => fresh_state_via(c().as_ref()),
            None => fresh_state(dir),
        }
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut devices = [
            Device::new("phone", make[0]()),
            Device::new("tv", make[1]()),
            Device::new("laptop", make[2]()),
        ];
        let mut online = [true, true, false];
        let mut all = Vec::new();
        let mut now = pa_sync::hlc::system_ms();
        for _ in 0..30 {
            now += 1 + rng.below(500);
            let d = usize::try_from(rng.below(3)).unwrap();
            match rng.below(8) {
                0..=3 => {
                    let body = body(&mut rng);
                    let at = i64::try_from(now).unwrap();
                    let op = devices[d]
                        .replica
                        .record(&devices[d].conn, body, at)
                        .unwrap();
                    devices[d].unsent.push(op.clone());
                    all.push(op);
                }
                4 | 5 => online[d] = !online[d],
                _ => {}
            }
            if online[d] {
                let compact = rng.below(8) == 0;
                devices[d].sync(now, compact);
            }
        }
        for round in 0..2 {
            now += 1000;
            for dev in &mut devices {
                let r = dev.sync(now, false);
                // A device that was never online writes its journal in
                // the first round.
                if round == 1 {
                    assert_eq!(r.devices.len(), 2, "{dir} seed {seed}: {}", dev.id);
                }
            }
        }
        let mut expected = SyncState::default();
        for op in &all {
            expected.apply(op);
        }
        if devices.iter().any(|d| d.replica.state() != &expected) {
            // What the folder holds, for the report.
            for e in std::fs::read_dir(host(dir).join(".kinolune/state")).unwrap() {
                let p = e.unwrap().path();
                eprintln!("--- {}", p.display());
                eprintln!("{}", std::fs::read_to_string(&p).unwrap_or_default());
            }
            for d in &devices {
                eprintln!("--- cursors of {}: {:?}", d.id, d.cursors);
            }
        }
        for dev in &devices {
            assert_eq!(
                dev.replica.state(),
                &expected,
                "{dir} seed {seed}: {} differs",
                dev.id
            );
        }
        eprintln!("{dir} seed {seed}: {} ops converged", all.len());
    }
}

#[test]
fn samba_sftp_and_the_host_folder_converge() {
    converge(
        "smb",
        &[|| smb("rw"), || sftp("/data/smb"), || local("smb")],
        0..12,
        None,
    );
}

#[test]
fn apache_webdav_sftp_and_the_host_folder_converge() {
    converge(
        "dav",
        &[|| dav("PA_W_APACHE"), || sftp("/data/dav"), || local("dav")],
        100..112,
        None,
    );
}

/// All three through rclone: it caches directory listings (5 min by
/// default), so files written past it (SFTP, the host) show up late.
#[test]
fn rclone_webdav_without_appends_converges() {
    converge(
        "rclone",
        &[
            || dav("PA_W_RCLONE"),
            || dav("PA_W_RCLONE"),
            || dav("PA_W_RCLONE"),
        ],
        200..212,
        Some(|| dav("PA_W_RCLONE")),
    );
}

#[test]
fn read_only_shares_only_receive() {
    for vfs in [smb("ro"), dav("PA_W_RCLONE_RO")] {
        let mut replica = Replica::new("ro-device").unwrap();
        let conn = outbox_db();
        let op = replica
            .record(
                &conn,
                OpBody::Watched {
                    profile: "default".into(),
                    key: key(0),
                },
                1_763_000_000_000,
            )
            .unwrap();
        let f = Folder::new(vfs.as_ref());
        let w = f.write("ro-device", &[op], 1_763_000_000_000).unwrap();
        assert!(w.read_only, "{vfs:?}");
        let mut cursors = Cursors::new();
        let r = f
            .read("ro-device", &mut cursors, 1_763_000_000_000, &mut |_| {
                Ok(true)
            })
            .unwrap();
        assert_eq!(r.ops, 0);
    }
}
