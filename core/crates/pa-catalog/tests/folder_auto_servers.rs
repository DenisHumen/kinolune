// SPDX-License-Identifier: MPL-2.0
//! SYNC-015 against real servers in Docker: three devices with no folder
//! chosen find one. The first device skips a read-only WebDAV share (rclone)
//! and makes the folder on the first writable source (Apache mod_dav); the
//! second, whose first source is a Samba share, joins the folder with the
//! first device's journal instead; the third has only the WebDAV share.
//! Progress then converges through it, and the Samba share stays clean.
//!
//! Run with
//! `PA_W_PACKAGE=pa-catalog PA_W_TEST=folder_auto_servers core/crates/pa-sources/tests/write_servers/run.sh`,
//! which starts the servers of the NET-023 write contract and sets the
//! variables read here.
#![cfg(feature = "server-tests")]

use std::path::PathBuf;
use std::sync::Arc;

use pa_catalog::{Catalog, FolderSyncMode, ItemRef, SourcesConnector};
use pa_scanner::CancelToken;
use pa_secrets::{MemoryStore, SecretString};
use pa_sources::{NewSource, SourceKind, SourceManager};

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!(
            "{name} is not set: use PA_W_PACKAGE=pa-catalog PA_W_TEST=folder_auto_servers \
             tests/write_servers/run.sh"
        )
    })
}

/// The host folder a server serves (`$PA_W_DATA/<dir>`).
fn host(dir: &str) -> PathBuf {
    PathBuf::from(var("PA_W_DATA")).join(dir)
}

struct Device {
    id: &'static str,
    catalog: Arc<Catalog>,
    manager: Arc<SourceManager>,
    _dir: tempfile::TempDir,
}

impl Device {
    fn new(id: &'static str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("library.db");
        let catalog = Arc::new(Catalog::open(&db).unwrap());
        let manager = Arc::new(SourceManager::open(&db, Arc::new(MemoryStore::new())).unwrap());
        catalog.set_connector(Arc::new(SourcesConnector::new(Arc::clone(&manager))));
        Self {
            id,
            catalog,
            manager,
            _dir: dir,
        }
    }

    fn add(&self, kind: SourceKind, name: &str, address: String) -> i64 {
        let id = self
            .manager
            .create(
                NewSource::new(kind, name, address)
                    .with_login(var("PA_W_USER"), SecretString::new(var("PA_W_PASS"))),
            )
            .unwrap()
            .id;
        std::thread::sleep(std::time::Duration::from_millis(5));
        id
    }

    fn dav(&self) -> i64 {
        self.add(SourceKind::WebDav, "NAS", var("PA_W_APACHE"))
    }

    fn scan(&self) {
        self.catalog
            .scan(None, &CancelToken::new(), &mut |_| {})
            .unwrap();
    }

    fn round(&self) -> pa_catalog::FolderSyncRun {
        self.catalog.run_folder_sync(self.id).unwrap()
    }

    fn matrix(&self) -> ItemRef {
        self.catalog
            .movies()
            .unwrap()
            .iter()
            .find(|m| m.title.contains("Matrix"))
            .expect("the movie is in the library")
            .id
    }

    fn position(&self) -> Option<i64> {
        self.catalog
            .progress(self.matrix())
            .unwrap()
            .map(|p| p.position_ms)
    }
}

#[test]
fn three_devices_find_one_folder_on_real_servers() {
    let movie = host("dav").join("Movies/The Matrix 1999.mkv");
    std::fs::create_dir_all(movie.parent().unwrap()).unwrap();
    std::fs::write(&movie, vec![7u8; 300_000]).unwrap();

    let a = Device::new("ka");
    a.add(SourceKind::WebDav, "Read-only", var("PA_W_RCLONE_RO"));
    let dav_a = a.dav();
    a.add(
        SourceKind::Smb,
        "Samba",
        format!("smb://{}/rw", var("PA_W_SMB")),
    );
    let b = Device::new("kb");
    b.add(
        SourceKind::Smb,
        "Samba",
        format!("smb://{}/rw", var("PA_W_SMB")),
    );
    let dav_b = b.dav();
    let c = Device::new("kc");
    let dav_c = c.dav();

    let run = a.round();
    assert_eq!(run.status.mode, FolderSyncMode::Auto);
    assert_eq!(run.status.source_id, Some(dav_a), "{:?}", run.status);
    assert_eq!(run.status.last_error, None);
    assert!(host("dav").join(".kinolune/state/ka.jsonl").exists());
    let run = b.round();
    assert_eq!(run.status.source_id, Some(dav_b), "{:?}", run.status);
    assert_eq!(run.status.devices, 1);
    let run = c.round();
    assert_eq!(run.status.source_id, Some(dav_c), "{:?}", run.status);
    assert_eq!(run.status.devices, 2);
    assert!(!host("smb").join(".kinolune").exists());

    for d in [&a, &b, &c] {
        d.scan();
    }
    let now = pa_catalog::pa_sync::hlc::system_ms() as i64;
    a.catalog
        .save_progress(a.matrix(), 1_234_000, 8_160_000, now, false)
        .unwrap();
    for _ in 0..2 {
        for d in [&a, &b, &c] {
            let run = d.round();
            assert_eq!(run.status.last_error, None, "{}", d.id);
            assert_eq!(run.status.devices, 2, "{}", d.id);
        }
    }
    for d in [&a, &b, &c] {
        assert_eq!(d.position(), Some(1_234_000), "{}", d.id);
    }
    assert!(!host("smb").join(".kinolune").exists());
}
