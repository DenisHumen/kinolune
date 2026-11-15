// SPDX-License-Identifier: MPL-2.0
//! SYNC-010 (and the UI-019 criterion): a phone adds an SMB source with a
//! password; a paired TV gets the source over the LAN service and the
//! password in its own secret store. The password is never in clear text
//! in what the devices store or send: the outbox, the LAN journal, every
//! file under each device's folder (databases with WAL, roster, the
//! encrypted secret file, a `.project_alpha/state` journal if any) and the
//! log output, every byte scanned. Edits and deletion come back the other
//! way, last writer wins.

use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pa_catalog::Catalog;
use pa_catalog::pa_sync::lan::service::{ServiceConfig, SyncLibrary, SyncService};
use pa_catalog::pa_sync::seal::SealKey;
use pa_catalog::pa_sync::{Op, OpId, outbox};
use pa_discovery::DeviceClass;
use pa_secrets::{EncryptedFileStore, SecretKey, SecretStore, SecretString};
use pa_sources::{NewSource, SourceKind, SourceManager, SourceSync, SourceUpdate};

const PASSWORD: &str = "c0rrect-H0rse-SYNC010";

/// Everything every thread logs, at every level.
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
    // One store per device, as in the app: device keys, family key and
    // source passwords.
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
    /// What the app does after the source manager changed a source.
    fn changed(&self) {
        self.catalog.sync_sources().unwrap();
        self.service.flush();
    }

    fn password(&self, id: i64) -> Option<String> {
        let conn = pa_library::open(self.catalog.path()).unwrap();
        let cred: Option<String> = conn
            .query_row("SELECT credential_ref FROM sources WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        cred.and_then(|c| self.store.get(&SecretKey::new(c).unwrap()).unwrap())
            .map(|p| p.expose().to_owned())
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

fn pair(phone: &Device, tv: &Device) {
    let invite = tv.service.invite().unwrap();
    phone.service.join(&invite.qr).unwrap();
    wait("tv running", || tv.service.is_running());
    wait("phone running", || phone.service.is_running());
    let (a, b) = (
        tv.service.sync_addr().unwrap(),
        phone.service.sync_addr().unwrap(),
    );
    tv.service
        .peer_up(phone.service.device_id(), DeviceClass::Mobile, b);
    phone
        .service
        .peer_up(tv.service.device_id(), DeviceClass::Tv, a);
    wait("phone links to the tv", || {
        phone.service.hub().as_deref() == Some(tv.service.device_id())
    });
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

/// The password in the usual encodings a leak could take.
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

#[test]
fn phone_adds_an_smb_source_with_a_password_and_the_tv_gets_both() {
    let logs = LogBuffer::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let phone = device("Phone", DeviceClass::Mobile);
    let tv = device("Living room TV", DeviceClass::Tv);
    pair(&phone, &tv);

    // On the phone: an SMB share with a password that syncs end-to-end.
    let created = phone
        .sources
        .create(
            NewSource::new(SourceKind::Smb, "NAS", "smb://nas.local/media/Movies")
                .with_login("anna", SecretString::new(PASSWORD))
                .with_sync(SourceSync::WithPassword),
        )
        .unwrap();
    phone.catalog.sync_sources().unwrap();
    let queued = phone.catalog.pending_ops(100).unwrap();
    assert_eq!(queued.len(), 1);
    let mut wire: Vec<String> = queued.iter().map(|op| outbox::encode(op).unwrap()).collect();
    assert!(wire[0].contains("\"op\":\"source.upsert\""));
    assert!(wire[0].contains("\"secret\":\"sb1."), "sealed: {}", wire[0]);
    for w in &wire {
        assert_clean("the outbox", w.as_bytes());
    }
    phone.service.flush();

    // On the TV: the source, and the password in the TV's secret store.
    wait("source reaches the tv", || {
        tv.sources
            .list()
            .unwrap()
            .first()
            .is_some_and(|s| s.has_password)
    });
    let on_tv = tv.sources.list().unwrap().remove(0);
    assert_eq!(on_tv.kind, SourceKind::Smb);
    assert_eq!(on_tv.uri, "smb://nas.local/media/Movies");
    assert_eq!(on_tv.username.as_deref(), Some("anna"));
    assert_eq!(on_tv.sync, SourceSync::WithPassword);
    assert_eq!(tv.password(on_tv.id).as_deref(), Some(PASSWORD));
    // The TV's catalog sees it too (it scans the same table).
    assert!(tv.catalog.sources().unwrap().iter().any(|s| s.name == "NAS"));

    // Renamed on the TV: back on the phone, the password stays.
    tv.sources
        .update(
            on_tv.id,
            SourceUpdate {
                name: Some("NAS (TV)".into()),
                ..Default::default()
            },
        )
        .unwrap();
    tv.changed();
    wait("rename reaches the phone", || {
        phone.sources.get(created.id).unwrap().name == "NAS (TV)"
    });
    assert_eq!(phone.password(created.id).as_deref(), Some(PASSWORD));

    // Everything either node holds in its journal now.
    wire.extend(phone.service.journal_frames());
    wire.extend(tv.service.journal_frames());
    assert!(
        wire.iter().any(|f| f.contains("source.upsert")),
        "the journal carried the source"
    );

    // Deleted on the phone: gone on the TV, with its password.
    let tv_cred = {
        let conn = pa_library::open(tv.catalog.path()).unwrap();
        conn.query_row(
            "SELECT credential_ref FROM sources WHERE id = ?1",
            [on_tv.id],
            |r| r.get::<_, String>(0),
        )
        .unwrap()
    };
    phone.sources.delete(created.id).unwrap();
    phone.changed();
    wait("delete reaches the tv", || tv.sources.list().unwrap().is_empty());
    assert!(
        tv.store
            .get(&SecretKey::new(tv_cred).unwrap())
            .unwrap()
            .is_none()
    );
    wire.extend(phone.service.journal_frames());
    wire.extend(tv.service.journal_frames());

    // Scan: journal frames, every file of both devices, the logs.
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
    assert!(!logged.is_empty(), "the log capture works");
    assert_clean("the logs", &logged);
}
