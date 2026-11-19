// SPDX-License-Identifier: MPL-2.0
//! SYNC-015: the sync folder picks itself. Three devices with the same
//! NAS converge without anyone choosing a folder; existing journals win
//! over the first writable source; read-only, local and non-file sources
//! are not made the folder; a folder away for long gives way to the next
//! and the own ops come back with the device; a reachable folder with
//! other devices is kept; the user's choice and "off" stay.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pa_catalog::{Catalog, Connector, Error, FolderSyncMode, ItemRef, Source};
use pa_library::rusqlite::{Connection, params};
use pa_scanner::CancelToken;
use pa_vfs::{ByteSource, DirEntry, LocalFs, Metadata, Vfs, VfsPath};

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

/// [`LocalFs`] without the write side: a share this account may only read.
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

/// The "network": every source address leads to a folder on this machine;
/// addresses can go down and be read-only.
#[derive(Debug, Default)]
struct Net {
    shares: Mutex<HashMap<String, PathBuf>>,
    down: Mutex<HashSet<String>>,
    read_only: Mutex<HashSet<String>>,
}

impl Net {
    fn share(&self, uri: &str, root: &Path) {
        self.shares
            .lock()
            .unwrap()
            .insert(uri.to_owned(), root.to_owned());
    }
    fn set_down(&self, uri: &str, down: bool) {
        let mut d = self.down.lock().unwrap();
        if down {
            d.insert(uri.to_owned());
        } else {
            d.remove(uri);
        }
    }
}

#[derive(Debug)]
struct NetConnector(Arc<Net>);

impl Connector for NetConnector {
    fn connect(&self, source: &Source) -> Result<Arc<dyn Vfs>, Error> {
        if let Some(root) = &source.local_root
            && source.kind == "local"
        {
            return Ok(Arc::new(LocalFs::new(root).unwrap()));
        }
        if self.0.down.lock().unwrap().contains(&source.uri) {
            return Err(Error::Io(format!(
                "{}: source unavailable: connection refused",
                source.name
            )));
        }
        let root = self
            .0
            .shares
            .lock()
            .unwrap()
            .get(&source.uri)
            .cloned()
            .ok_or_else(|| Error::Io(format!("{}: no route", source.uri)))?;
        let fs = LocalFs::new(&root).unwrap();
        if self.0.read_only.lock().unwrap().contains(&source.uri) {
            Ok(Arc::new(ReadOnly(fs)))
        } else {
            Ok(Arc::new(fs))
        }
    }
}

struct Device {
    id: String,
    catalog: Catalog,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

impl Device {
    fn new(id: &str, net: &Arc<Net>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("library.db");
        let catalog = Catalog::open(&db).unwrap();
        catalog.set_connector(Arc::new(NetConnector(Arc::clone(net))));
        Self {
            id: id.to_owned(),
            catalog,
            db,
            _dir: dir,
        }
    }

    fn smb(&self, name: &str, uri: &str) -> i64 {
        let id = self
            .catalog
            .add_source(None, "smb", name, uri, None)
            .unwrap()
            .id;
        // Sources added in the same millisecond still keep their order.
        std::thread::sleep(std::time::Duration::from_millis(2));
        id
    }

    /// A source of a kind the app adds through the source manager.
    fn other(&self, kind: &str, uri: &str) -> i64 {
        let conn = Connection::open(&self.db).unwrap();
        conn.execute(
            "INSERT INTO sources (kind, name, uri, position, created_at_ms)
             VALUES (?1, ?1, ?2, 0, 1)",
            params![kind, uri],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn scan(&self) {
        self.catalog
            .scan(None, &CancelToken::new(), &mut |_| {})
            .unwrap();
    }

    fn round(&self) -> pa_catalog::FolderSyncRun {
        self.catalog.run_folder_sync(&self.id).unwrap()
    }

    fn folder(&self) -> Option<i64> {
        self.catalog.folder_sync_status().unwrap().source_id
    }

    /// As if `minutes` passed for the automatic pick: the last look and the
    /// start of the failing rounds move back.
    fn age(&self, minutes: i64) {
        let conn = Connection::open(&self.db).unwrap();
        let ms = minutes * 60_000;
        conn.execute(
            "UPDATE sync_folder_choice SET evaluated_ms = evaluated_ms - ?1",
            [ms],
        )
        .unwrap();
        conn.execute(
            "UPDATE sync_folder SET failing_since_ms = failing_since_ms - ?1",
            [ms],
        )
        .unwrap();
    }

    fn matrix(&self) -> ItemRef {
        self.catalog
            .movies()
            .unwrap()
            .iter()
            .find(|m| m.title.contains("Matrix"))
            .unwrap()
            .id
    }

    fn play(&self, position_ms: i64) {
        let now = pa_catalog::pa_sync::hlc::system_ms() as i64;
        self.catalog
            .save_progress(self.matrix(), position_ms, 8_160_000, now, false)
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
    }

    fn position(&self) -> Option<i64> {
        self.catalog
            .progress(self.matrix())
            .unwrap()
            .map(|p| p.position_ms)
    }
}

fn journal(root: &Path, device: &str) -> PathBuf {
    root.join(".kinolune/state").join(format!("{device}.jsonl"))
}

/// A journal of a device that is not under test, so the folder looks used.
fn foreign_journal(root: &Path, device: &str) {
    let path = journal(root, device);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        format!("{{\"kinolune_state\":1,\"device\":\"{device}\",\"gen\":1}}\n"),
    )
    .unwrap();
}

#[test]
fn three_devices_converge_on_a_shared_folder_with_no_configuration() {
    let net = Arc::new(Net::default());
    let nas = tempfile::tempdir().unwrap();
    media(nas.path(), "Movies/The Matrix 1999.mkv", 2);
    net.share("smb://nas/video", nas.path());
    let devices: Vec<Device> = ["phone", "tv", "laptop"]
        .iter()
        .map(|id| {
            let d = Device::new(id, &net);
            d.smb("NAS", "smb://nas/video");
            d.scan();
            d
        })
        .collect();
    for d in &devices {
        let s = d.catalog.folder_sync_status().unwrap();
        assert_eq!((s.mode, s.source_id), (FolderSyncMode::Auto, None));
    }

    // Nobody picks anything: the rounds that run on their own do it.
    devices[0].play(60_000);
    for d in &devices {
        let run = d.round();
        assert_eq!(run.status.mode, FolderSyncMode::Auto);
        assert!(run.status.source_id.is_some(), "{}", d.id);
        assert_eq!(run.status.last_error, None);
    }
    devices[1].play(120_000);
    devices[2].play(180_000);
    for _ in 0..2 {
        for d in &devices {
            let run = d.round();
            assert_eq!(run.status.devices, 2, "{}", d.id);
        }
    }
    for d in &devices {
        assert_eq!(d.position(), Some(180_000), "{}", d.id);
        assert!(journal(nas.path(), &d.id).exists());
    }
}

#[test]
fn existing_journals_win_over_the_first_writable_source() {
    let net = Arc::new(Net::default());
    let (nas1, nas2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    net.share("smb://nas1/a", nas1.path());
    net.share("smb://nas2/b", nas2.path());
    foreign_journal(nas2.path(), "ktv");
    let d = Device::new("kphone", &net);
    d.smb("First", "smb://nas1/a");
    let second = d.smb("Second", "smb://nas2/b");

    let run = d.round();
    assert_eq!(run.status.source_id, Some(second));
    assert_eq!(run.status.devices, 1);
    assert!(journal(nas2.path(), "kphone").exists());
    assert!(!nas1.path().join(".kinolune").exists());
}

#[test]
fn the_first_writable_network_source_gets_the_folder() {
    let net = Arc::new(Net::default());
    let shares: Vec<tempfile::TempDir> = (0..5).map(|_| tempfile::tempdir().unwrap()).collect();
    let d = Device::new("kphone", &net);
    // Earliest first: a local folder, a DLNA server and a Jellyfin server
    // (with journals of others they must not be joined through), a
    // read-only share, then the share that gets the folder.
    let local = d
        .catalog
        .add_source(None, "local", "Videos", shares[0].path().to_str().unwrap(), None)
        .unwrap()
        .id;
    foreign_journal(shares[0].path(), "ktv");
    for (i, kind) in [(1, "dlna"), (2, "jellyfin")] {
        let uri = format!("http://media{i}:8096");
        net.share(&uri, shares[i].path());
        foreign_journal(shares[i].path(), "ktv");
        d.other(kind, &uri);
    }
    std::thread::sleep(std::time::Duration::from_millis(2));
    net.share("smb://ro/share", shares[3].path());
    net.read_only.lock().unwrap().insert("smb://ro/share".into());
    d.smb("Read-only", "smb://ro/share");
    net.share("smb://nas/video", shares[4].path());
    let nas = d.smb("NAS", "smb://nas/video");

    let run = d.round();
    assert_eq!(run.status.source_id, Some(nas));
    assert!(!run.status.read_only);
    assert!(journal(shares[4].path(), "kphone").exists());
    for s in &shares[..4] {
        assert!(!journal(s.path(), "kphone").exists());
    }
    assert_ne!(run.status.source_id, Some(local));
}

#[test]
fn a_read_only_folder_with_journals_is_joined_and_only_receives() {
    let net = Arc::new(Net::default());
    let nas = tempfile::tempdir().unwrap();
    media(nas.path(), "Movies/The Matrix 1999.mkv", 2);
    net.share("smb://nas/video", nas.path());
    // The TV can write the share; the phone's account only reads it.
    net.share("smb://guest@nas/video", nas.path());
    net.read_only
        .lock()
        .unwrap()
        .insert("smb://guest@nas/video".into());
    let tv = Device::new("ktv", &net);
    tv.smb("NAS", "smb://nas/video");
    tv.scan();
    let phone = Device::new("kphone", &net);
    let ro = phone.smb("NAS", "smb://guest@nas/video");
    phone.scan();

    // Nothing to join and nothing to write: no folder yet.
    assert_eq!(phone.round().status.source_id, None);
    // The TV makes the folder (its library sync starts with it), plays.
    tv.round();
    tv.play(90_000);
    tv.round();
    phone.age(6);
    let run = phone.round();
    assert_eq!(run.status.source_id, Some(ro));
    assert!(run.status.read_only);
    assert_eq!(phone.position(), Some(90_000));
    assert!(!journal(nas.path(), "kphone").exists());
}

#[test]
fn a_folder_away_for_long_gives_way_and_the_device_comes_back_with_its_changes() {
    let net = Arc::new(Net::default());
    let (nas1, nas2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    media(nas1.path(), "Movies/The Matrix 1999.mkv", 2);
    net.share("smb://nas1/video", nas1.path());
    net.share("smb://nas2/backup", nas2.path());
    let tv = Device::new("ktv", &net);
    tv.smb("NAS", "smb://nas1/video");
    tv.scan();
    let phone = Device::new("kphone", &net);
    let first = phone.smb("NAS", "smb://nas1/video");
    let second = phone.smb("Backup", "smb://nas2/backup");
    phone.scan();
    tv.round();
    assert_eq!(phone.round().status.source_id, Some(first));
    tv.round();

    // The NAS is away: a status, the folder stays for a while.
    net.set_down("smb://nas1/video", true);
    phone.play(300_000);
    let run = phone.round();
    assert_eq!(run.status.source_id, Some(first));
    assert_eq!(run.status.last_error.as_deref(), Some("unavailable"));
    phone.age(9);
    assert_eq!(phone.round().status.source_id, Some(first));
    // Away for longer than ten minutes: the next source takes over.
    phone.age(2);
    let run = phone.round();
    assert_eq!(run.status.source_id, Some(second));
    assert_eq!(run.status.mode, FolderSyncMode::Auto);
    assert!(journal(nas2.path(), "kphone").exists());
    phone.play(400_000);
    phone.round();

    // The NAS is back; the backup folder has no other devices, so the
    // phone goes back to the TV and brings what it wrote meanwhile.
    net.set_down("smb://nas1/video", false);
    phone.age(6);
    let run = phone.round();
    assert_eq!(run.status.source_id, Some(first));
    assert_eq!(run.status.devices, 1);
    assert_eq!(run.status.pending, 0);
    tv.round();
    assert_eq!(tv.position(), Some(400_000));
}

#[test]
fn a_reachable_folder_with_other_devices_is_kept() {
    let net = Arc::new(Net::default());
    let (nas1, nas2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    net.share("smb://nas1/a", nas1.path());
    net.share("smb://nas2/b", nas2.path());
    foreign_journal(nas1.path(), "ktv");
    let d = Device::new("kphone", &net);
    let first = d.smb("First", "smb://nas1/a");
    let second = d.smb("Second", "smb://nas2/b");
    assert_eq!(d.round().status.source_id, Some(first));

    // Another folder gets more devices later: no move while this one works.
    for other in ["ka", "kb", "kc"] {
        foreign_journal(nas2.path(), other);
    }
    for _ in 0..3 {
        d.age(60);
        assert_eq!(d.round().status.source_id, Some(first));
    }
    // A brief failure does not move it either.
    net.set_down("smb://nas1/a", true);
    d.age(60);
    assert_eq!(d.round().status.source_id, Some(first));
    net.set_down("smb://nas1/a", false);
    assert_eq!(d.round().status.source_id, Some(first));

    // The source is removed: the next one by the same rules, at once.
    d.catalog.remove_source(first).unwrap();
    let run = d.round();
    assert_eq!(run.status.source_id, Some(second));
    assert_eq!(run.status.devices, 3);
}

#[test]
fn two_devices_that_made_different_folders_end_up_in_one() {
    let net = Arc::new(Net::default());
    let (nas1, nas2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    net.share("smb://nas1/a", nas1.path());
    net.share("smb://nas2/b", nas2.path());
    // Added in the opposite order on the two devices.
    let a = Device::new("ka", &net);
    a.smb("One", "smb://nas1/a");
    a.smb("Two", "smb://nas2/b");
    let b = Device::new("kb", &net);
    b.smb("Two", "smb://nas2/b");
    b.smb("One", "smb://nas1/a");
    // Each makes its own folder while the other's NAS is away.
    net.set_down("smb://nas2/b", true);
    a.round();
    net.set_down("smb://nas2/b", false);
    net.set_down("smb://nas1/a", true);
    b.round();
    net.set_down("smb://nas1/a", false);
    assert!(journal(nas1.path(), "ka").exists());
    assert!(journal(nas2.path(), "kb").exists());

    for _ in 0..3 {
        a.age(6);
        b.age(6);
        a.round();
        b.round();
    }
    let uri = |d: &Device| {
        let id = d.folder().unwrap();
        d.catalog
            .sources()
            .unwrap()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap()
            .uri
    };
    assert_eq!(uri(&a), uri(&b));
    assert_eq!(a.round().status.devices, 1);
    assert_eq!(b.round().status.devices, 1);
}

#[test]
fn the_users_choice_and_off_stay_until_auto_again() {
    let net = Arc::new(Net::default());
    let (nas, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    net.share("smb://nas/video", nas.path());
    let d = Device::new("kphone", &net);
    let smb = d.smb("NAS", "smb://nas/video");
    let local = d
        .catalog
        .add_source(None, "local", "Home", home.path().to_str().unwrap(), None)
        .unwrap()
        .id;

    // The user picks a local folder: kept, though auto would not take it.
    let s = d.catalog.set_folder_sync(Some(local), &d.id).unwrap();
    assert_eq!((s.mode, s.source_id), (FolderSyncMode::Manual, Some(local)));
    for _ in 0..2 {
        d.age(60);
        let run = d.round();
        assert_eq!(run.status.mode, FolderSyncMode::Manual);
        assert_eq!(run.status.source_id, Some(local));
    }
    assert!(!nas.path().join(".kinolune").exists());

    // Off: nothing is picked.
    let s = d.catalog.set_folder_sync(None, &d.id).unwrap();
    assert_eq!((s.mode, s.source_id), (FolderSyncMode::Off, None));
    d.age(60);
    let run = d.round();
    assert_eq!((run.status.mode, run.status.source_id), (FolderSyncMode::Off, None));
    assert!(!nas.path().join(".kinolune").exists());

    // Automatic again: the NAS.
    let s = d.catalog.set_folder_sync_auto().unwrap();
    assert_eq!((s.mode, s.source_id), (FolderSyncMode::Auto, None));
    let run = d.round();
    assert_eq!((run.status.mode, run.status.source_id), (FolderSyncMode::Auto, Some(smb)));

    // Picking the same source by hand keeps what was read; back to auto
    // keeps it too.
    let s = d.catalog.set_folder_sync(Some(smb), &d.id).unwrap();
    assert_eq!((s.mode, s.source_id), (FolderSyncMode::Manual, Some(smb)));
    assert!(s.last_sync_ms.is_some());
    let s = d.catalog.set_folder_sync_auto().unwrap();
    assert_eq!((s.mode, s.source_id), (FolderSyncMode::Auto, Some(smb)));

    // A picked source that is removed lets the folder pick itself again.
    d.catalog.set_folder_sync(Some(local), &d.id).unwrap();
    d.catalog.remove_source(local).unwrap();
    assert_eq!(d.catalog.folder_sync_status().unwrap().mode, FolderSyncMode::Auto);
    assert_eq!(d.round().status.source_id, Some(smb));
}
