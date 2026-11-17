// SPDX-License-Identifier: MPL-2.0
//! SYNC-009: profiles travel between paired devices (with the PIN hash
//! sealed by the family key), so progress of two profiles on one film stays
//! apart on every device, and a deleted profile leaves everywhere.

use pa_library::profiles::{self, ProfileSettings};
use pa_library::progress::{self, PositionReport, Thresholds};
use pa_library::rusqlite::Connection;
use pa_model::{MediaKey, MediaKind};
use pa_sync::profiles::{Applied, apply_remote, reconcile, reset};
use pa_sync::seal::SealKey;
use pa_sync::{Op, Replica, outbox};

const T0: i64 = 1_763_000_000_000;

fn kids() -> ProfileSettings {
    ProfileSettings {
        name: "Дети".into(),
        avatar: Some("color:3".into()),
        is_kid: true,
        max_age: Some(12),
        rating_country: "US".into(),
        hide_unrated: true,
        hide_artwork: false,
        restrict_folders: false,
    }
}

fn key() -> MediaKey {
    "tmdb:movie:603".parse().unwrap()
}

fn family() -> SealKey {
    SealKey::new("family-1", 1, &[7; 32])
}

/// Records the profile ops of `replica` into the outbox of `db`; returns
/// them.
fn send(db: &mut Connection, replica: &mut Replica, seal: Option<&SealKey>, now: i64) -> Vec<Op> {
    let tx = db.transaction().unwrap();
    let mut ops = Vec::new();
    reconcile(&tx, seal, &mut |c, body| {
        let op = replica.record(c, body, now)?;
        ops.push(op.clone());
        Ok((op.hlc, op.op_id))
    })
    .unwrap();
    tx.commit().unwrap();
    ops
}

fn receive(db: &mut Connection, replica: &mut Replica, seal: Option<&SealKey>, ops: &[Op]) {
    for op in ops {
        replica.receive(op, T0 + 50_000).unwrap();
        let tx = db.transaction().unwrap();
        apply_remote(&tx, seal, op, T0 + 50_000).unwrap();
        tx.commit().unwrap();
    }
}

fn profile_by_name(db: &Connection, name: &str) -> Option<profiles::Profile> {
    profiles::list(db).unwrap().into_iter().find(|p| p.name == name)
}

#[test]
fn a_new_profile_and_its_pin_reach_the_other_device() {
    let (mut phone_db, mut tv_db) = (
        pa_library::open_in_memory().unwrap(),
        pa_library::open_in_memory().unwrap(),
    );
    let (mut phone, mut tv) = (Replica::new("phone").unwrap(), Replica::new("tv").unwrap());
    let k = profiles::create(&phone_db, &kids(), T0).unwrap();
    profiles::set_pin(&phone_db, k.id, Some("2468"), T0).unwrap();

    let ops = send(&mut phone_db, &mut phone, Some(&family()), T0);
    // The default profile and the new one.
    assert_eq!(ops.len(), 2);
    // Nothing new to send the second time.
    assert!(send(&mut phone_db, &mut phone, Some(&family()), T0 + 1).is_empty());
    // The PIN hash never travels in the clear.
    let text = serde_json::to_string(&ops).unwrap();
    assert!(!text.contains("argon2id$"));

    receive(&mut tv_db, &mut tv, Some(&family()), &ops);
    let on_tv = profile_by_name(&tv_db, "Дети").expect("profile arrived");
    assert_eq!(on_tv.sync_id, k.sync_id);
    assert!(on_tv.is_kid && on_tv.has_pin);
    assert_eq!(on_tv.max_age, Some(12));
    assert!(profiles::verify_pin(&tv_db, on_tv.id, Some("2468")).unwrap());
    assert!(!profiles::verify_pin(&tv_db, on_tv.id, Some("1111")).unwrap());
    // What arrived is not sent back.
    assert!(send(&mut tv_db, &mut tv, Some(&family()), T0 + 60_000).len() <= 1);
}

#[test]
fn without_the_family_key_the_pin_stays_local() {
    let (mut phone_db, mut tv_db) = (
        pa_library::open_in_memory().unwrap(),
        pa_library::open_in_memory().unwrap(),
    );
    let (mut phone, mut tv) = (Replica::new("phone").unwrap(), Replica::new("tv").unwrap());
    let k = profiles::create(&phone_db, &kids(), T0).unwrap();
    profiles::set_pin(&phone_db, k.id, Some("2468"), T0).unwrap();
    let ops = send(&mut phone_db, &mut phone, None, T0);
    receive(&mut tv_db, &mut tv, None, &ops);
    let on_tv = profile_by_name(&tv_db, "Дети").unwrap();
    // The profile says it has a PIN, but the hash did not come: no PIN.
    assert!(!on_tv.has_pin);

    // A PIN removed on the phone is removed on the TV too.
    profiles::set_pin(&tv_db, on_tv.id, Some("1357"), T0 + 1).unwrap();
    profiles::set_pin(&phone_db, k.id, None, T0 + 100_000).unwrap();
    let ops = send(&mut phone_db, &mut phone, None, T0 + 100_000);
    receive(&mut tv_db, &mut tv, None, &ops);
    assert!(!profile_by_name(&tv_db, "Дети").unwrap().has_pin);
}

#[test]
fn two_profiles_on_one_film_keep_their_own_progress() {
    let (mut phone_db, mut tv_db) = (
        pa_library::open_in_memory().unwrap(),
        pa_library::open_in_memory().unwrap(),
    );
    let (mut phone, mut tv) = (Replica::new("phone").unwrap(), Replica::new("tv").unwrap());
    let k = profiles::create(&phone_db, &kids(), T0).unwrap();
    let ops = send(&mut phone_db, &mut phone, Some(&family()), T0);
    receive(&mut tv_db, &mut tv, Some(&family()), &ops);

    let th = Thresholds::default();
    let report = |profile_id, position_ms| PositionReport {
        profile_id,
        key: key(),
        kind: MediaKind::Movie,
        position_ms,
        duration_ms: Some(8_160_000),
    };
    progress::report_position(&mut phone_db, &mut phone, &th, &report(1, 1_000_000), T0 + 1)
        .unwrap();
    progress::report_position(&mut phone_db, &mut phone, &th, &report(k.id, 5_000_000), T0 + 2)
        .unwrap();
    for p in outbox::due(&phone_db, i64::MAX, 100).unwrap() {
        tv.receive_into(&tv_db, &p.op, T0 + 60_000).unwrap();
    }
    let kid_on_tv = profile_by_name(&tv_db, "Дети").unwrap();
    let main = progress::get(&tv_db, 1, &key()).unwrap().unwrap();
    let kid = progress::get(&tv_db, kid_on_tv.id, &key()).unwrap().unwrap();
    assert_eq!(main.position_ms, 1_000_000);
    assert_eq!(kid.position_ms, 5_000_000);
}

#[test]
fn a_deleted_profile_leaves_and_an_older_upsert_does_not_bring_it_back() {
    let (mut phone_db, mut tv_db) = (
        pa_library::open_in_memory().unwrap(),
        pa_library::open_in_memory().unwrap(),
    );
    let (mut phone, mut tv) = (Replica::new("phone").unwrap(), Replica::new("tv").unwrap());
    let k = profiles::create(&phone_db, &kids(), T0).unwrap();
    let created = send(&mut phone_db, &mut phone, Some(&family()), T0);
    receive(&mut tv_db, &mut tv, Some(&family()), &created);
    assert!(profile_by_name(&tv_db, "Дети").is_some());

    profiles::delete(&phone_db, k.id).unwrap();
    let deleted = send(&mut phone_db, &mut phone, Some(&family()), T0 + 10_000);
    assert_eq!(deleted.len(), 1);
    receive(&mut tv_db, &mut tv, Some(&family()), &deleted);
    assert!(profile_by_name(&tv_db, "Дети").is_none());
    // The delete that arrived is not sent back.
    assert!(send(&mut tv_db, &mut tv, Some(&family()), T0 + 60_000).len() <= 1);

    // A late copy of the old upsert changes nothing.
    let tx = tv_db.transaction().unwrap();
    let upsert = created
        .iter()
        .find(|o| matches!(&o.body, pa_sync::OpBody::ProfileUpsert { profile, .. } if *profile == k.sync_id))
        .unwrap();
    assert_eq!(
        apply_remote(&tx, Some(&family()), upsert, T0 + 70_000).unwrap(),
        Applied::Unchanged
    );
    tx.commit().unwrap();
    assert!(profile_by_name(&tv_db, "Дети").is_none());
}

#[test]
fn leaving_the_family_forgets_what_was_sent() {
    let mut db = pa_library::open_in_memory().unwrap();
    let mut phone = Replica::new("phone").unwrap();
    profiles::create(&db, &kids(), T0).unwrap();
    assert_eq!(send(&mut db, &mut phone, None, T0).len(), 2);
    reset(&db).unwrap();
    assert_eq!(send(&mut db, &mut phone, None, T0 + 1).len(), 2);
}

#[test]
fn the_default_profile_is_never_deleted_by_an_op() {
    let body = pa_sync::OpBody::ProfileDelete {
        profile: "default".into(),
    };
    assert!(body.validate().is_err());
}
