// SPDX-License-Identifier: MPL-2.0
//! SYNC-007 done-when: three devices that sync through journals in a shared
//! folder, offline and online in any order, converge. Also: secrets never
//! reach the folder, read-only folders only receive, cut and broken lines
//! are skipped, a backend without append, compaction into snapshots.

use std::io::Read;
use std::path::Path;

use pa_library::rusqlite::Connection;
use pa_model::{MediaKey, MediaKind};
use pa_sync::folder::{self, Cursors, Folder};
use pa_sync::{Op, OpBody, Replica, SyncState};
use pa_vfs::{
    ByteSource, DirEntry, ErrorKind, LocalFs, Metadata, Vfs, VfsError, VfsPath, VfsWrite,
};

/// xorshift64*: deterministic scenarios without a dependency.
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

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

const KEYS: [&str; 3] = ["tmdb:movie:603", "tmdb:episode:1399:1:1", "twitch:vod:123"];

fn key(i: u64) -> MediaKey {
    KEYS[usize::try_from(i).unwrap()].parse().unwrap()
}

/// An in-memory database with only the outbox table: migrating a whole
/// library per device would make the scenarios slow.
fn outbox_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../pa-library/migrations/202611141200_SYNC-001_sync_outbox.sql"
    ))
    .unwrap();
    conn
}

fn random_body(rng: &mut Rng) -> OpBody {
    let profile = if rng.chance(70) { "default" } else { "kid" }.to_owned();
    match rng.below(6) {
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
        3 => OpBody::Unwatch {
            profile,
            key: key(rng.below(3)),
        },
        4 => OpBody::Favorite {
            profile,
            target: format!("tmdb:tv:{}", rng.below(2)),
            favorite: rng.chance(50),
        },
        _ => OpBody::Setting {
            key: format!("ui.theme{}", rng.below(2)),
            value: Some(format!("\"{}\"", rng.below(3))),
        },
    }
}

/// One device: its replica and outbox database, the ops not in the folder
/// yet, its cursors and the size of its last snapshot.
struct Device {
    id: String,
    replica: Replica,
    conn: Connection,
    unsent: Vec<Op>,
    cursors: Cursors,
    snapshot: u64,
    skew_ms: i64,
}

impl Device {
    fn new(id: &str, skew_ms: i64) -> Self {
        Self {
            id: id.to_owned(),
            replica: Replica::new(id).unwrap(),
            conn: outbox_db(),
            unsent: Vec::new(),
            cursors: Cursors::new(),
            snapshot: 0,
            skew_ms,
        }
    }

    fn clock(&self, now_ms: u64) -> u64 {
        now_ms.saturating_add_signed(self.skew_ms)
    }

    fn record(&mut self, body: OpBody, now_ms: u64) -> Op {
        let now = i64::try_from(self.clock(now_ms)).unwrap();
        let op = self.replica.record(&self.conn, body, now).unwrap();
        self.unsent.push(op.clone());
        op
    }

    /// One online round: write what is new, read the others, compact.
    fn sync(&mut self, vfs: &dyn Vfs, now_ms: u64, compact: bool) -> folder::ReadReport {
        let f = Folder::new(vfs);
        let now = self.clock(now_ms);
        let written = f.write(&self.id, &self.unsent, now).unwrap();
        if !written.read_only {
            self.unsent.clear();
        }
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

/// [`LocalFs`] whose backend cannot append (`write_file` at an offset), as
/// some WebDAV servers.
#[derive(Debug)]
struct NoAppend(LocalFs);

impl Vfs for NoAppend {
    fn list(&self, dir: &VfsPath) -> pa_vfs::Result<Vec<DirEntry>> {
        self.0.list(dir)
    }
    fn stat(&self, path: &VfsPath) -> pa_vfs::Result<Metadata> {
        self.0.stat(path)
    }
    fn open(&self, path: &VfsPath) -> pa_vfs::Result<Box<dyn ByteSource>> {
        self.0.open(path)
    }
    fn as_write(&self) -> Option<&dyn VfsWrite> {
        Some(self)
    }
}

impl VfsWrite for NoAppend {
    fn create_dir(&self, path: &VfsPath) -> pa_vfs::Result<()> {
        self.0.create_dir(path)
    }
    fn write_file(
        &self,
        path: &VfsPath,
        offset: u64,
        data: &mut dyn Read,
        len: u64,
    ) -> pa_vfs::Result<u64> {
        if offset > 0 {
            return Err(VfsError::new(ErrorKind::Unsupported, "no append"));
        }
        self.0.write_file(path, offset, data, len)
    }
    fn rename(&self, from: &VfsPath, to: &VfsPath) -> pa_vfs::Result<()> {
        self.0.rename(from, to)
    }
    fn remove_file(&self, path: &VfsPath) -> pa_vfs::Result<()> {
        self.0.remove_file(path)
    }
    fn remove_dir(&self, path: &VfsPath) -> pa_vfs::Result<()> {
        self.0.remove_dir(path)
    }
}

/// [`LocalFs`] without the write side: a read-only source.
#[derive(Debug)]
struct ReadOnly(LocalFs);

impl Vfs for ReadOnly {
    fn list(&self, dir: &VfsPath) -> pa_vfs::Result<Vec<DirEntry>> {
        self.0.list(dir)
    }
    fn stat(&self, path: &VfsPath) -> pa_vfs::Result<Metadata> {
        self.0.stat(path)
    }
    fn open(&self, path: &VfsPath) -> pa_vfs::Result<Box<dyn ByteSource>> {
        self.0.open(path)
    }
}

fn journal(root: &Path, device: &str) -> std::path::PathBuf {
    root.join(".kinolune/state").join(format!("{device}.jsonl"))
}

fn append_raw(root: &Path, device: &str, bytes: &[u8]) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(journal(root, device))
        .unwrap();
    f.write_all(bytes).unwrap();
}

fn truth(ops: &[Op]) -> SyncState {
    let mut s = SyncState::default();
    for op in ops {
        s.apply(op);
    }
    s
}

/// One random scenario: devices record changes, go offline and online in
/// any order, compact at random, a crash leaves a cut line; at the end all
/// come online and every state equals the merge of every op.
fn scenario(seed: u64, no_append: bool) {
    let dir = tempfile::tempdir().unwrap();
    let local = || LocalFs::new(dir.path()).unwrap();
    let vfs: Box<dyn Vfs> = if no_append {
        Box::new(NoAppend(local()))
    } else {
        Box::new(local())
    };
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut devices = vec![
        Device::new("phone", 0),
        Device::new("tv", 700),
        Device::new("laptop", -900),
    ];
    let mut online = [true, false, true];
    let mut all = Vec::new();
    let mut now: u64 = 1_763_000_000_000;
    let steps = 20 + rng.below(60);
    for _ in 0..steps {
        now += rng.below(3000);
        let d = usize::try_from(rng.below(3)).unwrap();
        match rng.below(10) {
            0..=4 => {
                for _ in 0..=rng.below(3) {
                    let body = random_body(&mut rng);
                    all.push(devices[d].record(body, now));
                }
            }
            5 | 6 => online[d] = !online[d],
            7 => {
                // A crash in the middle of an append: a cut line.
                let path = journal(dir.path(), &devices[d].id);
                if online[d] && path.exists() {
                    append_raw(dir.path(), &devices[d].id, b"{\"schema_version\":1,\"da");
                }
            }
            _ => {}
        }
        if online[d] {
            let compact = rng.chance(10);
            devices[d].sync(vfs.as_ref(), now, compact);
        }
    }
    // Everyone online: two rounds reach everything.
    for _ in 0..2 {
        now += 1000;
        for dev in &mut devices {
            dev.sync(vfs.as_ref(), now, false);
        }
    }
    let expected = truth(&all);
    for dev in &devices {
        assert!(dev.unsent.is_empty());
        assert_eq!(
            dev.replica.state(),
            &expected,
            "seed {seed}: {} differs",
            dev.id
        );
    }
}

#[test]
fn three_devices_offline_and_online_in_any_order_converge() {
    for seed in 0..500 {
        scenario(seed, false);
    }
}

#[test]
fn a_backend_without_append_converges_too() {
    for seed in 1000..1100 {
        scenario(seed, true);
    }
}

#[test]
fn secrets_never_reach_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let vfs = LocalFs::new(dir.path()).unwrap();
    let mut a = Device::new("a", 0);
    let key = pa_sync::seal::SealKey::new("family", 1, &[7; 32]);
    let password = pa_sync::seal::seal(&key, "source:s1", b"pass-1").unwrap();
    let pin = pa_sync::seal::seal(&key, "profile-pin:p1", b"argon2-hash").unwrap();
    a.record(
        OpBody::SourceUpsert {
            source: "s1".into(),
            config: "{\"kind\":\"smb\",\"address\":\"nas\"}".into(),
            secret: Some(password.clone()),
        },
        1_763_000_000_000,
    );
    a.record(
        OpBody::ProfileUpsert {
            profile: "p1".into(),
            config: "{\"name\":\"Kid\"}".into(),
            pin: Some(pin.clone()),
        },
        1_763_000_000_001,
    );
    a.sync(&vfs, 1_763_000_000_002, false);
    let text = std::fs::read_to_string(journal(dir.path(), "a")).unwrap();
    assert!(text.contains("source.upsert") && text.contains("profile.upsert"));
    assert!(!text.contains(&password) && !text.contains(&pin), "{text}");

    // A secret setting never becomes an op; a crafted one never goes out.
    let op = Op {
        op_id: pa_sync::OpId::new_v7(),
        hlc: a.replica.clock_mut().tick(1_763_000_000_003),
        aliases: Vec::new(),
        body: OpBody::Setting {
            key: "twitch.token".into(),
            value: Some("\"x\"".into()),
        },
    };
    assert!(folder::for_folder(&op).is_none());
}

#[test]
fn a_read_only_folder_only_receives() {
    let dir = tempfile::tempdir().unwrap();
    let rw = LocalFs::new(dir.path()).unwrap();
    let ro = ReadOnly(LocalFs::new(dir.path()).unwrap());
    let now = 1_763_000_000_000;
    let mut a = Device::new("a", 0);
    let mut b = Device::new("b", 0);
    a.record(
        OpBody::Watched {
            profile: "default".into(),
            key: key(0),
        },
        now,
    );
    a.sync(&rw, now, false);
    b.record(
        OpBody::Watched {
            profile: "default".into(),
            key: key(1),
        },
        now,
    );
    let report = b.sync(&ro, now + 1, false);
    assert_eq!(report.changed, 1);
    assert!(b.replica.state().is_watched("default", &key(0)));
    // Its own change waits for a writable folder.
    assert_eq!(b.unsent.len(), 1);
    assert!(!journal(dir.path(), "b").exists());
}

#[test]
fn cut_broken_and_foreign_lines_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let vfs = LocalFs::new(dir.path()).unwrap();
    let now = 1_763_000_000_000;
    let mut a = Device::new("a", 0);
    let mut b = Device::new("b", 0);
    a.sync(&vfs, now, false);
    let op = a.record(
        OpBody::Watched {
            profile: "default".into(),
            key: key(0),
        },
        now,
    );
    // Written by hand below, not by `sync`.
    a.unsent.clear();
    let line = format!("{}\n", pa_sync::outbox::encode(&op).unwrap());
    let (first, rest) = line.split_at(20);
    // Half a line: not read yet.
    append_raw(dir.path(), "a", first.as_bytes());
    let r = b.sync(&vfs, now, false);
    assert_eq!((r.ops, r.skipped), (0, 0));
    append_raw(dir.path(), "a", rest.as_bytes());
    let r = b.sync(&vfs, now, false);
    assert_eq!((r.ops, r.changed), (1, 1));
    assert!(b.replica.state().is_watched("default", &key(0)));

    // Garbage, an op claiming another device, an op from far in the future.
    let mut c = Device::new("c", 0);
    let spoof = c.record(
        OpBody::Watched {
            profile: "default".into(),
            key: key(1),
        },
        now,
    );
    let mut future = Device::new("a", 0);
    let late = future.record(
        OpBody::Watched {
            profile: "default".into(),
            key: key(2),
        },
        now + 24 * 3_600_000,
    );
    append_raw(
        dir.path(),
        "a",
        format!(
            "not json\n{}\n{}\n\u{0}\u{1}\n",
            pa_sync::outbox::encode(&spoof).unwrap(),
            pa_sync::outbox::encode(&late).unwrap()
        )
        .as_bytes(),
    );
    std::fs::write(dir.path().join(".kinolune/state/junk.txt"), "x").unwrap();
    std::fs::write(dir.path().join(".kinolune/state/bad name.jsonl"), "x").unwrap();
    std::fs::write(
        dir.path().join(".kinolune/state/z.jsonl"),
        "{\"kinolune_state\":99,\"device\":\"z\",\"gen\":1}\n",
    )
    .unwrap();
    let r = b.sync(&vfs, now, false);
    assert_eq!(r.ops, 0);
    assert_eq!(r.skipped, 4);
    assert_eq!(r.newer, 1);
    assert!(!b.replica.state().is_watched("default", &key(1)));
    assert!(!b.replica.state().is_watched("default", &key(2)));

    // The device's next append starts on a new line after a cut one.
    append_raw(dir.path(), "a", b"{\"cut");
    a.record(
        OpBody::Watched {
            profile: "default".into(),
            key: key(1),
        },
        now + 5,
    );
    a.sync(&vfs, now + 5, false);
    let r = b.sync(&vfs, now + 6, false);
    assert_eq!((r.ops, r.changed, r.skipped), (1, 1, 1));
    assert!(b.replica.state().is_watched("default", &key(1)));
}

#[test]
fn compaction_keeps_the_newest_op_of_each_register_under_a_new_generation() {
    let dir = tempfile::tempdir().unwrap();
    let vfs = LocalFs::new(dir.path()).unwrap();
    let mut now = 1_763_000_000_000;
    let mut a = Device::new("a", 0);
    let mut b = Device::new("b", 0);
    let progress = |i: u64| OpBody::ProgressUpsert {
        profile: "default".into(),
        key: key(i % 2),
        kind: MediaKind::Movie,
        position_ms: i * 1000,
        duration_ms: None,
    };
    a.sync(&vfs, now, false);
    for i in 0..499 {
        now += 10;
        a.record(progress(i), now);
    }
    a.sync(&vfs, now, false);
    b.sync(&vfs, now, false);
    let before = std::fs::metadata(journal(dir.path(), "a")).unwrap().len();
    // Compaction follows a write with ops: the backend's append is known.
    now += 10;
    a.record(progress(499), now);
    a.sync(&vfs, now, true);
    let text = std::fs::read_to_string(journal(dir.path(), "a")).unwrap();
    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(a.snapshot < before / 10);
    // A new device reads the snapshot; one that read the old journal
    // starts over and stays the same.
    let mut c = Device::new("c", 0);
    c.sync(&vfs, now, false);
    b.sync(&vfs, now, false);
    assert_eq!(c.replica.state(), a.replica.state());
    assert_eq!(b.replica.state(), a.replica.state());
    // Appends after a compaction reach the readers.
    a.record(
        OpBody::Watched {
            profile: "default".into(),
            key: key(2),
        },
        now + 1,
    );
    a.sync(&vfs, now + 1, false);
    let r = b.sync(&vfs, now + 2, false);
    assert_eq!((r.ops, r.changed), (1, 1));
}

#[test]
fn an_interrupted_compaction_is_finished_by_the_next_write() {
    let dir = tempfile::tempdir().unwrap();
    let vfs = LocalFs::new(dir.path()).unwrap();
    let now = 1_763_000_000_000;
    let mut a = Device::new("a", 0);
    a.record(
        OpBody::Watched {
            profile: "default".into(),
            key: key(0),
        },
        now,
    );
    a.sync(&vfs, now, false);
    let j = journal(dir.path(), "a");
    let part = dir.path().join(".kinolune/state/a.jsonl.part");
    let full = std::fs::read(&j).unwrap();

    // The backup was written in full, the journal was cut in the middle of
    // its rewrite: the next write puts the backup back.
    let mut backup = full.clone();
    backup.extend_from_slice(format!("{{\"kinolune_state_end\":{}}}\n", full.len()).as_bytes());
    std::fs::write(&part, &backup).unwrap();
    std::fs::write(&j, &full[..full.len() / 2]).unwrap();
    a.sync(&vfs, now + 1, false);
    assert!(!part.exists());
    assert_eq!(std::fs::read(&j).unwrap(), full);
    let mut b = Device::new("b", 0);
    b.sync(&vfs, now + 2, false);
    assert!(b.replica.state().is_watched("default", &key(0)));

    // The backup itself was cut: the journal was not touched, it stays.
    std::fs::write(&part, &backup[..backup.len() - 5]).unwrap();
    a.sync(&vfs, now + 3, false);
    assert!(!part.exists());
    assert_eq!(std::fs::read(&j).unwrap(), full);
}

#[cfg(unix)]
#[test]
fn compaction_rewrites_in_place_for_readers_holding_the_file() {
    // An SMB reader keeps the journal open; the owner must not delete or
    // rename it (the server refuses, or the reader keeps the old file).
    let dir = tempfile::tempdir().unwrap();
    let vfs = LocalFs::new(dir.path()).unwrap();
    let now = 1_763_000_000_000;
    let mut a = Device::new("a", 0);
    a.sync(&vfs, now, false);
    for i in 0..20 {
        a.record(
            OpBody::ProgressUpsert {
                profile: "default".into(),
                key: key(0),
                kind: MediaKind::Movie,
                position_ms: i * 1000,
                duration_ms: None,
            },
            now + i,
        );
    }
    a.sync(&vfs, now + 20, false);
    let j = journal(dir.path(), "a");
    let held = std::fs::File::open(&j).unwrap();
    let before = held.metadata().unwrap();
    a.record(
        OpBody::Watched {
            profile: "default".into(),
            key: key(1),
        },
        now + 21,
    );
    a.sync(&vfs, now + 22, true);
    let after = std::fs::metadata(&j).unwrap();
    use std::os::unix::fs::MetadataExt;
    assert_eq!(before.ino(), after.ino(), "the journal is the same file");
    assert!(after.len() < before.len(), "compacted");
    assert_eq!(held.metadata().unwrap().len(), after.len());
    assert!(!dir.path().join(".kinolune/state/a.jsonl.part").exists());
}

#[test]
fn every_device_shows_up_even_before_its_first_change() {
    let dir = tempfile::tempdir().unwrap();
    let vfs = LocalFs::new(dir.path()).unwrap();
    let mut a = Device::new("a", 0);
    let mut b = Device::new("b", 0);
    a.sync(&vfs, 1, false);
    let r = b.sync(&vfs, 2, false);
    assert_eq!(r.devices, vec!["a".to_owned()]);
    let r = a.sync(&vfs, 3, false);
    assert_eq!(r.devices, vec!["b".to_owned()]);
}
