// SPDX-License-Identifier: MPL-2.0
//! SYNC-010: the source list between two devices without a network: ops
//! go from one library to the other by hand. Last writer wins by HLC,
//! deletion propagates and wins over older upserts, the password travels
//! only sealed and lands in the receiver's secret store.

use std::path::Path;
use std::sync::Arc;

use pa_library::rusqlite::Connection;
use pa_secrets::{MemoryStore, SecretKey, SecretStore, SecretString};
use pa_sources::sync::{self, Applied};
use pa_sources::{
    NewSource, PasswordChange, SourceKind, SourceManager, SourceSync, SourceUpdate,
};
use pa_sync::seal::SealKey;
use pa_sync::{Op, Replica, outbox};

const PASSWORD: &str = "Tr0ub4dor&3-sync010";

struct Device {
    manager: SourceManager,
    conn: Connection,
    store: Arc<MemoryStore>,
    replica: Replica,
    seal: Option<SealKey>,
    clock_ms: i64,
    _dir: tempfile::TempDir,
}

fn family_key(epoch: u32) -> SealKey {
    SealKey::new("family-1", epoch, &[0x42; 32])
}

fn device(id: &str) -> Device {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let store = Arc::new(MemoryStore::new());
    let manager = SourceManager::open(&db, Arc::clone(&store) as Arc<dyn SecretStore>).unwrap();
    Device {
        manager,
        conn: pa_library::open(&db).unwrap(),
        store,
        replica: Replica::new(id).unwrap(),
        seal: Some(family_key(1)),
        clock_ms: 1_763_000_000_000,
        _dir: dir,
    }
}

impl Device {
    /// Records ops for local changes and returns them.
    fn outgoing(&mut self) -> Vec<Op> {
        self.clock_ms += 1_000;
        let now = self.clock_ms;
        let tx = self.conn.transaction().unwrap();
        let replica = &mut self.replica;
        sync::reconcile(
            &tx,
            Some(self.store.as_ref() as &dyn SecretStore),
            self.seal.as_ref(),
            &mut |c, body| replica.record(c, body, now).map(|op| (op.hlc, op.op_id)),
        )
        .unwrap();
        tx.commit().unwrap();
        let ops: Vec<Op> = outbox::due(&self.conn, i64::MAX, 100)
            .unwrap()
            .into_iter()
            .map(|p| p.op)
            .collect();
        for op in &ops {
            outbox::ack(&self.conn, op.op_id).unwrap();
        }
        ops
    }

    fn apply(&mut self, op: &Op) -> Applied {
        self.clock_ms += 1_000;
        self.replica.receive(op, self.clock_ms).unwrap();
        let tx = self.conn.transaction().unwrap();
        let applied = sync::apply_remote(
            &tx,
            Some(self.store.as_ref() as &dyn SecretStore),
            self.seal.as_ref(),
            op,
        )
        .unwrap();
        tx.commit().unwrap();
        applied
    }

    fn password_of(&self, id: i64) -> Option<String> {
        let cred: Option<String> = self
            .conn
            .query_row("SELECT credential_ref FROM sources WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        cred.and_then(|c| self.store.get(&SecretKey::new(c).unwrap()).unwrap())
            .map(|p| p.expose().to_owned())
    }
}

fn smb(sync: SourceSync, password: &str) -> NewSource {
    NewSource::new(SourceKind::Smb, "NAS", "smb://nas.local/media/Movies")
        .with_login("anna", SecretString::new(password))
        .with_sync(sync)
}

fn bytes_under(dir: &Path) -> Vec<u8> {
    let mut all = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            all.extend(bytes_under(&path));
        } else {
            all.extend(std::fs::read(&path).unwrap());
        }
    }
    all
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn password_travels_sealed_and_lands_in_the_receivers_store() {
    let mut phone = device("phone");
    let mut tv = device("tv");
    let created = phone
        .manager
        .create(smb(SourceSync::WithPassword, PASSWORD))
        .unwrap();
    assert_eq!(created.sync, SourceSync::WithPassword);

    let ops = phone.outgoing();
    assert_eq!(ops.len(), 1);
    let wire = pa_sync::outbox::encode(&ops[0]).unwrap();
    assert!(!wire.contains(PASSWORD), "op in clear: {wire}");
    assert!(wire.contains("\"secret\":\"sb1."), "sealed container: {wire}");
    assert_eq!(tv.apply(&ops[0]), Applied::Changed);

    let tv_sources = tv.manager.list().unwrap();
    assert_eq!(tv_sources.len(), 1);
    let s = &tv_sources[0];
    assert_eq!(
        (s.kind, s.name.as_str(), s.uri.as_str(), s.username.as_deref()),
        (SourceKind::Smb, "NAS", "smb://nas.local/media/Movies", Some("anna"))
    );
    assert!(s.has_password);
    assert_eq!(s.sync, SourceSync::WithPassword);
    assert_eq!(tv.password_of(s.id).as_deref(), Some(PASSWORD));

    // Applied ops are not echoed back.
    assert!(tv.outgoing().is_empty());
    assert!(phone.outgoing().is_empty());

    // Not in the databases (WAL included), not in the outbox.
    for dev in [&phone, &tv] {
        dev.conn
            .execute_batch("PRAGMA wal_checkpoint(FULL);")
            .unwrap();
        assert!(!contains(&bytes_under(dev._dir.path()), PASSWORD));
    }
}

#[test]
fn settings_only_keeps_the_receivers_own_password() {
    let mut phone = device("phone");
    let mut tv = device("tv");
    phone
        .manager
        .create(smb(SourceSync::Settings, PASSWORD))
        .unwrap();
    let ops = phone.outgoing();
    assert!(!outbox::encode(&ops[0]).unwrap().contains("secret"));
    tv.apply(&ops[0]);
    let id = tv.manager.list().unwrap()[0].id;
    assert!(!tv.manager.get(id).unwrap().has_password);
    tv.manager
        .update(
            id,
            SourceUpdate {
                password: PasswordChange::Set(SecretString::new("tv-own")),
                ..Default::default()
            },
        )
        .unwrap();
    // A settings change from the phone does not touch it.
    let pid = phone.manager.list().unwrap()[0].id;
    phone
        .manager
        .update(
            pid,
            SourceUpdate {
                name: Some("NAS 2".into()),
                ..Default::default()
            },
        )
        .unwrap();
    tv.outgoing(); // the TV's own password change: settings only, no secret
    phone.clock_ms = tv.clock_ms + 60_000;
    for op in phone.outgoing() {
        tv.apply(&op);
    }
    assert_eq!(tv.manager.get(id).unwrap().name, "NAS 2");
    assert_eq!(tv.password_of(id).as_deref(), Some("tv-own"));
}

#[test]
fn last_writer_wins_and_deletion_propagates() {
    let mut phone = device("phone");
    let mut tv = device("tv");
    phone
        .manager
        .create(smb(SourceSync::WithPassword, PASSWORD))
        .unwrap();
    let create = phone.outgoing().remove(0);
    tv.apply(&create);
    let pid = phone.manager.list().unwrap()[0].id;
    let tid = tv.manager.list().unwrap()[0].id;

    // Both edit; the TV edits later (its clock is ahead), so it wins.
    phone
        .manager
        .update(pid, SourceUpdate { name: Some("Phone name".into()), ..Default::default() })
        .unwrap();
    let from_phone = phone.outgoing().remove(0);
    tv.clock_ms = phone.clock_ms + 60_000;
    tv.manager
        .update(tid, SourceUpdate { name: Some("TV name".into()), ..Default::default() })
        .unwrap();
    let from_tv = tv.outgoing().remove(0);
    assert_eq!(phone.apply(&from_tv), Applied::Changed);
    assert_eq!(tv.apply(&from_phone), Applied::Unchanged, "older edit loses");
    assert_eq!(phone.apply(&from_phone), Applied::Unchanged, "duplicate");
    assert_eq!(phone.manager.get(pid).unwrap().name, "TV name");
    assert_eq!(tv.manager.get(tid).unwrap().name, "TV name");
    // The password survives a settings edit on the other device.
    assert_eq!(tv.password_of(tid).as_deref(), Some(PASSWORD));

    // Delete on the TV: the phone loses the source and its password.
    let phone_cred: String = phone
        .conn
        .query_row("SELECT credential_ref FROM sources WHERE id = ?1", [pid], |r| r.get(0))
        .unwrap();
    tv.manager.delete(tid).unwrap();
    let delete = tv.outgoing();
    assert_eq!(delete.len(), 1);
    assert!(matches!(delete[0].body, pa_sync::OpBody::SourceDelete { .. }));
    assert_eq!(phone.apply(&delete[0]), Applied::Deleted);
    assert!(phone.manager.list().unwrap().is_empty());
    assert!(
        phone
            .store
            .get(&SecretKey::new(phone_cred).unwrap())
            .unwrap()
            .is_none()
    );
    // The phone does not send the delete back.
    assert!(phone.outgoing().is_empty());

    // A late, older upsert does not bring it back; a newer one does.
    assert_eq!(phone.apply(&from_tv), Applied::Unchanged);
    assert_eq!(phone.apply(&create), Applied::Unchanged);
    assert!(phone.manager.list().unwrap().is_empty());
}

#[test]
fn a_newer_upsert_restores_a_deleted_source() {
    let mut phone = device("phone");
    let mut tv = device("tv");
    phone
        .manager
        .create(smb(SourceSync::Settings, PASSWORD))
        .unwrap();
    tv.apply(&phone.outgoing()[0]);
    let tid = tv.manager.list().unwrap()[0].id;
    tv.manager.delete(tid).unwrap();
    let delete = tv.outgoing().remove(0);

    // The phone edits after the delete (its clock is ahead): the edit wins.
    let pid = phone.manager.list().unwrap()[0].id;
    phone.clock_ms = tv.clock_ms + 60_000;
    phone
        .manager
        .update(pid, SourceUpdate { name: Some("Still here".into()), ..Default::default() })
        .unwrap();
    let edit = phone.outgoing().remove(0);
    assert_eq!(phone.apply(&delete), Applied::Unchanged);
    assert_eq!(tv.apply(&edit), Applied::Changed);
    assert_eq!(tv.manager.list().unwrap()[0].name, "Still here");
}

#[test]
fn another_key_epoch_keeps_the_local_password() {
    let mut phone = device("phone");
    let mut tv = device("tv");
    tv.seal = Some(family_key(2));
    phone
        .manager
        .create(smb(SourceSync::WithPassword, PASSWORD))
        .unwrap();
    tv.apply(&phone.outgoing()[0]);
    let s = &tv.manager.list().unwrap()[0];
    assert_eq!(s.username.as_deref(), Some("anna"), "settings still apply");
    assert!(!s.has_password, "the container does not open");

    // After the key rotation reaches the phone, it seals again.
    phone.seal = Some(family_key(2));
    let again = phone.outgoing();
    assert_eq!(again.len(), 1);
    tv.apply(&again[0]);
    let s = &tv.manager.list().unwrap()[0];
    assert_eq!(tv.password_of(s.id).as_deref(), Some(PASSWORD));
}

#[test]
fn opt_in_rules() {
    let mut phone = device("phone");
    let dir = tempfile::tempdir().unwrap();
    let local = NewSource::new(SourceKind::Local, "Here", dir.path().to_str().unwrap())
        .with_sync(SourceSync::Settings);
    assert!(phone.manager.create(local).is_err(), "local folders never sync");

    // Off: nothing goes out; switching it on later sends it.
    let s = phone.manager.create(smb(SourceSync::Off, PASSWORD)).unwrap();
    assert!(phone.outgoing().is_empty());
    phone
        .manager
        .update(s.id, SourceUpdate { sync: Some(SourceSync::Settings), ..Default::default() })
        .unwrap();
    assert_eq!(phone.outgoing().len(), 1);
    // Deleting a source that does not sync sends nothing.
    let other = phone
        .manager
        .create(NewSource::new(SourceKind::WebDav, "Dav", "https://dav.local/m"))
        .unwrap();
    phone.manager.delete(other.id).unwrap();
    assert!(phone.outgoing().is_empty());
}

#[test]
fn a_hand_added_copy_is_adopted_not_duplicated() {
    let mut phone = device("phone");
    let mut tv = device("tv");
    tv.manager
        .create(NewSource::new(SourceKind::Smb, "Mine", "smb://nas.local/media/Movies"))
        .unwrap();
    phone
        .manager
        .create(smb(SourceSync::WithPassword, PASSWORD))
        .unwrap();
    tv.apply(&phone.outgoing()[0]);
    let all = tv.manager.list().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "NAS");
    assert_eq!(tv.password_of(all[0].id).as_deref(), Some(PASSWORD));
}

#[test]
fn leaving_the_family_resends_everything_later() {
    let mut phone = device("phone");
    phone
        .manager
        .create(smb(SourceSync::Settings, PASSWORD))
        .unwrap();
    assert_eq!(phone.outgoing().len(), 1);
    assert!(phone.outgoing().is_empty());
    sync::reset(&phone.conn).unwrap();
    assert_eq!(phone.outgoing().len(), 1);
}
