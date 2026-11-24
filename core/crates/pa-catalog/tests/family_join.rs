// SPDX-License-Identifier: MPL-2.0
//! SYNC-015: a second device joins the family in the LAN without a code.
//! The phone (alone, a kid's profile with a PIN) and a new TV with the
//! same files. The TV asks to join; until the phone allows it, the TV gets
//! nothing: no family, no profiles, no progress. After one "Allow" on the
//! phone the TV is in the family, gets the profiles (PIN as the same hash)
//! and the phone's progress lands in its "Continue watching". Folder sync
//! is on its automatic mode on the TV without anybody touching it.

use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pa_catalog::pa_sync::lan::service::{ServiceConfig, SyncLibrary, SyncService};
use pa_catalog::pa_sync::seal::SealKey;
use pa_catalog::pa_sync::{Op, OpId};
use pa_catalog::{Catalog, FolderSyncMode, ItemRef, ProfileSettings};
use pa_discovery::DeviceClass;
use pa_scanner::CancelToken;
use pa_secrets::MemoryStore;

const KID_PIN: &str = "1357";

struct Lib(Arc<Catalog>);

impl SyncLibrary for Lib {
    fn enable(&self, device_id: &str) -> Result<(), String> {
        self.0.enable_sync(device_id).map_err(|e| e.to_string())
    }
    fn disable(&self) -> Result<(), String> {
        self.0.disable_sync().map_err(|e| e.to_string())
    }
    fn pending(&self) -> Result<Vec<Op>, String> {
        self.0.pending_ops(512).map_err(|e| e.to_string())
    }
    fn ack(&self, op_id: OpId) -> Result<(), String> {
        self.0.ack_op(op_id).map(drop).map_err(|e| e.to_string())
    }
    fn apply(&self, op: &Op) -> Result<bool, String> {
        self.0.apply_remote(op).map_err(|e| e.to_string())
    }
    fn set_seal_key(&self, key: Option<SealKey>) {
        self.0.set_seal_key(key).unwrap();
    }
}

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

struct Device {
    catalog: Arc<Catalog>,
    service: SyncService,
    _dir: tempfile::TempDir,
}

fn device(name: &str, class: DeviceClass) -> Device {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    media(&root, "Movies/The Matrix 1999.mkv", 2);
    let catalog = Arc::new(Catalog::open(dir.path().join("db/library.db")).unwrap());
    catalog
        .add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    catalog
        .scan(None, &CancelToken::new(), &mut |_| {})
        .unwrap();
    let config = ServiceConfig {
        bind_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
        mdns: None,
        flush_interval: Duration::from_millis(200),
        ..ServiceConfig::new(dir.path().join("sync"), name, class)
    };
    let service = SyncService::open(
        config,
        Arc::new(MemoryStore::new()),
        Arc::new(Lib(Arc::clone(&catalog))),
    )
    .unwrap();
    Device {
        catalog,
        service,
        _dir: dir,
    }
}

fn matrix(cat: &Catalog) -> ItemRef {
    let movies = cat.movies().unwrap();
    movies
        .iter()
        .find(|m| m.title.contains("Matrix"))
        .unwrap()
        .id
}

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out: {what}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn link(a: &Device, b: &Device) {
    if let (Some(pa), Some(pb)) = (a.service.sync_addr(), b.service.sync_addr()) {
        a.service
            .peer_up(b.service.device_id(), DeviceClass::Tv, pb);
        b.service
            .peer_up(a.service.device_id(), DeviceClass::Mobile, pa);
    }
}

#[test]
fn second_device_joins_after_one_tap_and_gets_profiles_and_progress() {
    // The phone, alone: a kid's profile with a PIN.
    let phone = device("Phone", DeviceClass::Mobile);
    let kid = phone
        .catalog
        .create_profile(
            &ProfileSettings {
                name: "Kids".into(),
                is_kid: true,
                max_age: Some(6),
                ..ProfileSettings::default()
            },
            None,
        )
        .unwrap();
    phone
        .catalog
        .set_profile_pin(kid.id, Some(KID_PIN), None)
        .unwrap();
    let phone_kid_hash: String = pa_library::open(phone.catalog.path())
        .unwrap()
        .query_row(
            "SELECT pin_hash FROM profiles WHERE id = ?1",
            [kid.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!phone.service.start().unwrap(), "alone: nothing runs");

    // The new TV asks to join the family the phone offers (its id is the
    // hash of the phone's key: the phone founds it on "Allow").
    let tv = device("Living room TV", DeviceClass::Tv);
    assert!(tv.catalog.adopt_family_profiles().unwrap());
    assert_eq!(
        tv.catalog.folder_sync_status().unwrap().mode,
        FolderSyncMode::Auto
    );
    assert!(phone.service.family_id().is_none());
    // What the phone announces as `fam` (the TV reads it from mDNS:
    // `families_nearby`).
    let offered = phone.service.offered_family_id();
    let shown = tv.service.join_ask(&offered).unwrap();
    let txt = tv.service.join_ask_txt().unwrap();
    assert!(phone.service.note_join_request("Living room TV x-j", &txt));
    let request = phone.service.join_requests().remove(0);
    assert_eq!(request.short_code, shown.short_code);
    assert_eq!(request.name, "Living room TV");

    // Without the tap: nothing.
    assert!(
        tv.service
            .join_ask_wait(Duration::from_millis(800), None)
            .unwrap()
            .is_none()
    );
    assert!(!tv.service.is_paired() && !tv.service.is_running());
    assert!(
        tv.catalog
            .profiles()
            .unwrap()
            .iter()
            .all(|p| p.name != "Kids")
    );
    assert!(tv.catalog.continue_watching(10).unwrap().is_empty());

    // One tap on the phone.
    phone.service.join_allow(&request.request_id).unwrap();
    let addr = phone.service.invite_addr().unwrap();
    let inviter = tv
        .service
        .join_ask_wait(Duration::from_secs(15), Some(addr))
        .unwrap()
        .expect("joined");
    assert_eq!(inviter.name, "Phone");
    assert!(tv.service.is_paired());
    wait("both run", || {
        tv.service.is_running() && phone.service.is_running()
    });

    // Profiles: the kid's PIN as the same hash.
    wait("profiles reach the TV", || {
        link(&phone, &tv);
        tv.catalog
            .profiles()
            .unwrap()
            .iter()
            .any(|p| p.name == "Kids" && p.has_pin)
    });
    let tv_kid = tv
        .catalog
        .profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.name == "Kids")
        .unwrap();
    let tv_kid_hash: String = pa_library::open(tv.catalog.path())
        .unwrap()
        .query_row(
            "SELECT pin_hash FROM profiles WHERE id = ?1",
            [tv_kid.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tv_kid_hash, phone_kid_hash);

    // Progress: played on the phone, continued on the TV.
    let now = pa_catalog::pa_sync::hlc::system_ms() as i64;
    assert!(
        phone
            .catalog
            .save_progress(matrix(&phone.catalog), 1_234_000, 8_160_000, now, false)
            .unwrap()
    );
    phone.service.flush();
    let movie_tv = matrix(&tv.catalog);
    wait("progress reaches the TV", || {
        link(&phone, &tv);
        tv.catalog
            .progress(movie_tv)
            .unwrap()
            .is_some_and(|p| p.position_ms == 1_234_000)
    });
    assert_eq!(
        tv.catalog.continue_watching(10).unwrap()[0].item.id,
        movie_tv
    );

    // Folder sync: still automatic, nobody set anything.
    assert_eq!(
        tv.catalog.folder_sync_status().unwrap().mode,
        FolderSyncMode::Auto
    );
}
