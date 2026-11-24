// SPDX-License-Identifier: MPL-2.0
//! UI-046: a new TV is set up from a phone in one go. The TV shows its
//! setup code (QR); the phone, which has an SMB share with a password that
//! does not sync, a local folder and two profiles (one locked by a PIN),
//! opens a setup invitation with that code; the TV joins and gets the
//! sealed bundle: the share with its password (into the TV's secret store
//! only) and the phone's settings. Profiles come over sync with the PIN as
//! a hash; the TV's fresh first profile takes the phone's instead of
//! overwriting it. After that the TV can scan the share without a single
//! question. The password is never in clear text in what either device
//! stores or sends (outbox, LAN journal, every file under its folder) nor
//! in the logs, every byte scanned. A wrong code, a used-up offer and a
//! bundle opened by another device fail.

use std::collections::BTreeMap;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pa_catalog::pa_sync::lan::service::{ServiceConfig, SyncLibrary, SyncService};
use pa_catalog::pa_sync::seal::SealKey;
use pa_catalog::pa_sync::setup::{SetupRequest, Zeroizing};
use pa_catalog::pa_sync::{Op, OpId};
use pa_catalog::{Catalog, ProfileSettings};
use pa_discovery::DeviceClass;
use pa_secrets::{EncryptedFileStore, SecretKey, SecretStore, SecretString};
use pa_sources::setup::SetupBundle;
use pa_sources::{NewSource, SourceKind, SourceManager, SourceSync};

const PASSWORD: &str = "n3w-TV-from-Ph0ne-UI046";
const KID_PIN: &str = "2468";

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

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

struct Device {
    catalog: Arc<Catalog>,
    sources: SourceManager,
    store: Arc<EncryptedFileStore>,
    service: SyncService,
    dir: tempfile::TempDir,
}

fn device(name: &str, class: DeviceClass) -> Device {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("db/library.db");
    let store = Arc::new(EncryptedFileStore::open(dir.path().join("secrets")).unwrap());
    let catalog = Arc::new(Catalog::open(&db).unwrap());
    catalog.set_source_secrets(Arc::clone(&store) as Arc<dyn SecretStore>);
    let sources = SourceManager::open(&db, Arc::clone(&store) as Arc<dyn SecretStore>).unwrap();
    let config = ServiceConfig {
        bind_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
        mdns: None,
        flush_interval: Duration::from_millis(200),
        ..ServiceConfig::new(dir.path().join("sync"), name, class)
    };
    let service = SyncService::open(
        config,
        Arc::clone(&store) as Arc<dyn SecretStore>,
        Arc::new(Lib(Arc::clone(&catalog))),
    )
    .unwrap();
    Device {
        catalog,
        sources,
        store,
        service,
        dir,
    }
}

impl Device {
    fn password(&self, id: i64) -> Option<String> {
        let conn = pa_library::open(self.catalog.path()).unwrap();
        let cred: Option<String> = conn
            .query_row(
                "SELECT credential_ref FROM sources WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        cred.and_then(|c| self.store.get(&SecretKey::new(c).unwrap()).unwrap())
            .map(|p| p.expose().to_owned())
    }

    /// What the phone's app hands the core: its sources and settings.
    fn bundle(&self, settings: &BTreeMap<String, String>) -> Zeroizing<Vec<u8>> {
        let sources = self.sources.export_setup().unwrap();
        Zeroizing::new(
            SetupBundle::new(sources, settings.clone())
                .to_json()
                .unwrap(),
        )
    }
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

fn files_under(dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(&path, out);
        } else if let Ok(bytes) = std::fs::read(&path) {
            out.push((path.display().to_string(), bytes));
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn needles() -> Vec<Vec<u8>> {
    let utf16: Vec<u8> = PASSWORD.encode_utf16().flat_map(u16::to_le_bytes).collect();
    vec![
        PASSWORD.as_bytes().to_vec(),
        utf16,
        PASSWORD.to_ascii_lowercase().into_bytes(),
        serde_json::to_string(PASSWORD).unwrap().into_bytes(),
    ]
}

fn assert_clean(what: &str, bytes: &[u8]) {
    for n in needles() {
        assert!(!find(bytes, &n), "password in clear text in {what}");
    }
}

fn settings() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("ui_locale".to_owned(), "ru".to_owned()),
        ("playback_skip_seconds".to_owned(), "15".to_owned()),
    ])
}

#[test]
fn a_new_tv_gets_sources_passwords_profiles_and_settings_from_the_phone() {
    let logs = LogBuffer::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    // The phone: a share whose password stays on the phone (sync off), a
    // local folder, its first profile renamed and a kid's profile with a
    // PIN. Not paired with anything yet.
    let phone = device("Phone", DeviceClass::Mobile);
    let smb = phone
        .sources
        .create(
            NewSource::new(SourceKind::Smb, "NAS", "smb://nas.local/media/Movies")
                .with_login("anna", SecretString::new(PASSWORD)),
        )
        .unwrap();
    assert_eq!(smb.sync, SourceSync::Off);
    let folder = tempfile::tempdir().unwrap();
    phone
        .sources
        .create(NewSource::new(
            SourceKind::Local,
            "Downloads",
            folder.path().to_str().unwrap(),
        ))
        .unwrap();
    let first = phone
        .catalog
        .profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.sync_id == "default")
        .unwrap();
    phone
        .catalog
        .update_profile(
            first.id,
            &ProfileSettings {
                name: "Anna".into(),
                ..ProfileSettings::default()
            },
            None,
        )
        .unwrap();
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

    // The new TV: shows its code. Nothing else is set up on it.
    let tv = device("Living room TV", DeviceClass::Tv);
    let offer = tv.service.setup_offer().unwrap();
    let request = SetupRequest::parse(&offer.qr).unwrap();
    assert_eq!(request.code(), offer.code);
    assert_eq!(request.name(), Some("Living room TV"));
    assert!(tv.catalog.adopt_family_profiles().unwrap());

    // A phone with another code: the TV does not take it, and it does not
    // try that invitation again.
    let stranger = device("Stranger", DeviceClass::Mobile);
    let wrong = if offer.code == "000000" { "111111" } else { "000000" };
    let stranger_invite = stranger
        .service
        .invite_setup(wrong, phone.bundle(&BTreeMap::new()))
        .unwrap();
    assert!(
        tv.service
            .setup_wait(Duration::from_millis(300), Some(stranger_invite.addr))
            .unwrap()
            .is_none()
    );
    assert!(!tv.service.is_paired());
    stranger.service.cancel_invite();

    // The phone scans the QR code.
    let settings = settings();
    phone
        .service
        .invite_setup(&offer.qr, phone.bundle(&settings))
        .unwrap();
    let addr = phone.service.invite_addr().unwrap();
    let received = tv
        .service
        .setup_wait(Duration::from_secs(15), Some(addr))
        .unwrap()
        .expect("the phone answered");
    assert_eq!(received.inviter.name, "Phone");
    assert!(!format!("{received:?}").contains(PASSWORD));
    let bundle = SetupBundle::from_json(received.bundle.as_ref().expect("the bundle")).unwrap();
    assert_eq!(bundle.settings, settings);
    // Only the share: a local path means nothing on the TV.
    assert_eq!(bundle.sources.len(), 1);
    let report = tv.sources.import_setup(&bundle.sources).unwrap();
    assert_eq!((report.added, report.updated, report.skipped), (1, 0, 0));
    drop(bundle);
    drop(received);

    // The share on the TV, with its password in the TV's store, ready to
    // be scanned and played without a question.
    let on_tv = tv.sources.list().unwrap().remove(0);
    assert_eq!(on_tv.kind, SourceKind::Smb);
    assert_eq!(on_tv.uri, "smb://nas.local/media/Movies");
    assert_eq!(on_tv.username.as_deref(), Some("anna"));
    assert!(on_tv.has_password && on_tv.wants_auto_scan());
    assert_eq!(on_tv.sync, SourceSync::Off);
    assert_eq!(tv.password(on_tv.id).as_deref(), Some(PASSWORD));
    assert_eq!(phone.password(smb.id).as_deref(), Some(PASSWORD));
    // The same bundle again: nothing doubles.
    let again = tv
        .sources
        .import_setup(&SetupBundle::from_json(&phone.bundle(&settings)).unwrap().sources)
        .unwrap();
    assert_eq!((again.added, again.skipped), (0, 1));

    // The offer is used up; the phone's invitation closed.
    assert!(tv.service.is_paired() && phone.service.is_paired());
    assert!(tv.service.setup_offer().is_err());
    wait("the invitation closes", || {
        phone.service.invite_addr().is_none()
    });

    // Profiles over sync: the kid's PIN as the same hash, the phone's
    // first profile not overwritten by the TV's fresh one.
    wait("both nodes run", || {
        tv.service.is_running() && phone.service.is_running()
    });
    let (a, b) = (
        tv.service.sync_addr().unwrap(),
        phone.service.sync_addr().unwrap(),
    );
    tv.service
        .peer_up(phone.service.device_id(), DeviceClass::Mobile, b);
    phone
        .service
        .peer_up(tv.service.device_id(), DeviceClass::Tv, a);
    wait("profiles reach the tv", || {
        let ps = tv.catalog.profiles().unwrap();
        ps.iter().any(|p| p.name == "Kids" && p.has_pin)
            && ps.iter().any(|p| p.sync_id == "default" && p.name == "Anna")
    });
    let tv_kid = tv
        .catalog
        .profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.name == "Kids")
        .unwrap();
    assert!(tv_kid.is_kid && tv_kid.max_age == Some(6));
    let tv_kid_hash: String = pa_library::open(tv.catalog.path())
        .unwrap()
        .query_row(
            "SELECT pin_hash FROM profiles WHERE id = ?1",
            [tv_kid.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tv_kid_hash, phone_kid_hash, "stored as the same hash");
    assert!(!tv_kid_hash.contains(KID_PIN));
    assert!(
        tv.catalog
            .verify_profile_pin(tv_kid.id, KID_PIN, false)
            .unwrap()
            .ok
    );
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        phone
            .catalog
            .profiles()
            .unwrap()
            .into_iter()
            .find(|p| p.sync_id == "default")
            .unwrap()
            .name,
        "Anna"
    );

    // Nothing in clear text: journals, outboxes, every file, the logs.
    let mut wire = phone.service.journal_frames();
    wire.extend(tv.service.journal_frames());
    for frame in &wire {
        assert_clean("the sync journal", frame.as_bytes());
    }
    for dev in [&phone, &tv] {
        let conn = pa_library::open(dev.catalog.path()).unwrap();
        let rows: Vec<String> = conn
            .prepare("SELECT payload FROM sync_outbox")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for row in rows {
            assert_clean("sync_outbox", row.as_bytes());
        }
        let mut files = Vec::new();
        files_under(dev.dir.path(), &mut files);
        assert!(files.iter().any(|(p, _)| p.ends_with("library.db")));
        for (path, bytes) in &files {
            assert_clean(path, bytes);
        }
    }
    let logged = logs.0.lock().unwrap().clone();
    assert!(!logged.is_empty(), "the logs were captured");
    assert_clean("the logs", &logged);
}

#[test]
fn a_bundle_opens_only_on_the_device_it_was_sealed_for() {
    let phone = device("Phone", DeviceClass::Mobile);
    phone
        .sources
        .create(
            NewSource::new(SourceKind::WebDav, "Cloud", "https://dav.example/media")
                .with_login("anna", SecretString::new(PASSWORD)),
        )
        .unwrap();
    let tv = device("TV", DeviceClass::Tv);
    let offer = tv.service.setup_offer().unwrap();
    // Typed instead of scanned: the 6 digits work the same.
    phone
        .service
        .invite_setup(&offer.code, phone.bundle(&BTreeMap::new()))
        .unwrap();
    let addr = phone.service.invite_addr().unwrap();
    let received = tv
        .service
        .setup_wait(Duration::from_secs(15), Some(addr))
        .unwrap()
        .unwrap();
    let bundle = SetupBundle::from_json(received.bundle.as_ref().unwrap()).unwrap();
    assert_eq!(bundle.sources[0].kind, "webdav");
    assert_eq!(
        bundle.sources[0].password.as_ref().map(SecretString::expose),
        Some(PASSWORD)
    );

    // Without an offer the TV does not look for phones.
    let other = device("Other TV", DeviceClass::Tv);
    assert_eq!(
        other
            .service
            .setup_wait(Duration::from_millis(100), Some(addr))
            .unwrap_err()
            .code,
        "expired"
    );
    // A cancelled offer is gone too.
    other.service.setup_offer().unwrap();
    other.service.setup_cancel();
    assert!(
        other
            .service
            .setup_wait(Duration::from_millis(100), None)
            .is_err()
    );
}
