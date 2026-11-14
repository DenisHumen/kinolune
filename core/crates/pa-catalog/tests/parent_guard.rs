// SPDX-License-Identifier: MPL-2.0
//! LIB-012 hardening (ADR-0055 amendment): while a supervised profile is
//! active, the core refuses every profile-management call without the
//! parent token; the parent PIN gives one, it expires, and the wrong-PIN
//! backoff lives in the database, so a restart does not reset it.

use std::time::Duration;

use pa_catalog::{Catalog, Error, ItemRef, Profile, ProfileSettings};
use pa_library::profiles::DEFAULT_PROFILE;

const PARENT_PIN: &str = "2468";

fn kid_settings() -> ProfileSettings {
    ProfileSettings {
        name: "Kid".into(),
        is_kid: true,
        max_age: Some(12),
        hide_unrated: true,
        restrict_folders: true,
        ..ProfileSettings::default()
    }
}

fn adult(name: &str) -> ProfileSettings {
    ProfileSettings {
        name: name.into(),
        ..ProfileSettings::default()
    }
}

struct Home {
    dir: tempfile::TempDir,
    cat: Catalog,
    kid: Profile,
    sam: Profile,
}

impl Home {
    fn reopen(self) -> Self {
        let Self { dir, cat, kid, sam } = self;
        drop(cat);
        let cat = Catalog::open(dir.path().join("library.db")).unwrap();
        Self { dir, cat, kid, sam }
    }
}

/// The default profile with the parent PIN, a second adult without a PIN
/// (Sam) and a kid profile, which is active.
fn home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    cat.set_profile_pin(DEFAULT_PROFILE, Some(PARENT_PIN), None)
        .unwrap();
    let sam = cat.create_profile(&adult("Sam"), None).unwrap();
    let kid = cat.create_profile(&kid_settings(), None).unwrap();
    cat.switch_profile(kid.id, None, None).unwrap();
    Home { dir, cat, kid, sam }
}

fn denied<T: std::fmt::Debug>(r: Result<T, Error>, what: &str) {
    assert!(matches!(r, Err(Error::Denied(_))), "{what}: {r:?}");
}

/// Every management call, with `token`; each one must be refused.
fn assert_all_denied(h: &Home, token: Option<&str>) {
    let cat = &h.cat;
    let before = cat.profiles().unwrap();
    let folders = cat.profile_folders(h.kid.id).unwrap();
    denied(cat.create_profile(&adult("New"), token), "create");
    denied(
        cat.update_profile(h.kid.id, &adult("Free"), token),
        "lift own limits",
    );
    denied(
        cat.update_profile(DEFAULT_PROFILE, &adult("Parent"), token),
        "update parent",
    );
    denied(cat.delete_profile(DEFAULT_PROFILE, token), "delete parent");
    denied(cat.delete_profile(h.sam.id, token), "delete other");
    denied(
        cat.reorder_profiles(&[h.kid.id, DEFAULT_PROFILE], token),
        "reorder",
    );
    denied(
        cat.set_profile_pin(DEFAULT_PROFILE, None, token),
        "clear parent PIN",
    );
    denied(
        cat.set_profile_pin(DEFAULT_PROFILE, Some("1111"), token),
        "replace parent PIN",
    );
    denied(
        cat.set_profile_pin(h.kid.id, Some("1357"), token),
        "own PIN",
    );
    denied(cat.set_profile_folders(h.kid.id, &[], token), "white list");
    denied(
        cat.set_user_certification(ItemRef::Movie(1), Some(0), token),
        "parent rating",
    );
    // Leaving the kid profile for one without a PIN.
    denied(cat.switch_profile(h.sam.id, None, token), "leave");
    // Nothing changed.
    let profiles = cat.profiles().unwrap();
    assert_eq!(profiles, before);
    assert_eq!(cat.profile_folders(h.kid.id).unwrap(), folders);
    let kid = profiles.iter().find(|p| p.id == h.kid.id).unwrap();
    assert!(kid.is_supervised());
    assert!(
        profiles
            .iter()
            .find(|p| p.id == DEFAULT_PROFILE)
            .unwrap()
            .is_parent()
    );
    assert_eq!(cat.active_profile().unwrap().id, h.kid.id);
}

impl Home {
    fn kid_now(&self, cat: &Catalog) -> Profile {
        cat.profiles()
            .unwrap()
            .into_iter()
            .find(|p| p.id == self.kid.id)
            .unwrap()
    }
}

#[test]
fn a_kid_profile_is_denied_every_management_call() {
    let h = home();
    assert_all_denied(&h, None);
    assert_all_denied(&h, Some("not-a-token"));
    // The kid's settings are as the parent left them.
    let kid = h.kid_now(&h.cat);
    assert_eq!(kid.max_age, Some(12));
    assert!(kid.restrict_folders && kid.hide_unrated);
}

#[test]
fn the_parent_pin_gives_a_token_that_allows_them() {
    let h = home();
    let cat = &h.cat;
    let wrong = cat.verify_parent_pin("0000").unwrap();
    assert!(!wrong.ok && wrong.token.is_none() && !wrong.locked);
    assert_eq!(wrong.attempts.failures, 1);

    let v = cat.verify_parent_pin(PARENT_PIN).unwrap();
    assert!(v.ok);
    assert_eq!(v.attempts.failures, 0, "a right PIN resets the count");
    let token = v.token.expect("a parent token");
    let t = Some(token.as_str());
    let other = cat.create_profile(&adult("New"), t).unwrap();
    cat.update_profile(other.id, &adult("Renamed"), t).unwrap();
    cat.reorder_profiles(&[h.kid.id, DEFAULT_PROFILE], t)
        .unwrap();
    cat.set_profile_pin(h.kid.id, Some("1357"), t).unwrap();
    cat.set_profile_folders(h.kid.id, &[], t).unwrap();
    cat.delete_profile(other.id, t).unwrap();
    let mut s = ProfileSettings::of(&h.kid_now(cat));
    s.max_age = Some(16);
    cat.update_profile(h.kid.id, &s, t).unwrap();
    assert_eq!(h.kid_now(cat).max_age, Some(16));
    // An item the kid cannot see is still "not found" with the token.
    assert!(matches!(
        cat.set_user_certification(ItemRef::Movie(1), Some(0), t),
        Err(Error::NotFound(_))
    ));

    // The kid's own PIN is right but no parent PIN: no token.
    let own = cat.verify_profile_pin(h.kid.id, "1357", false).unwrap();
    assert!(own.ok && own.token.is_none());
    // App Lock: the profile's PIN or a parent PIN.
    let lock = cat.verify_profile_pin(h.kid.id, PARENT_PIN, true).unwrap();
    assert!(lock.ok && lock.token.is_some());
    assert!(
        !cat.verify_profile_pin(h.kid.id, PARENT_PIN, false)
            .unwrap()
            .ok
    );
    // The parent profile's own PIN check gives a token too.
    let p = cat
        .verify_profile_pin(DEFAULT_PROFILE, PARENT_PIN, false)
        .unwrap();
    let token = p.token.expect("a parent token");

    // A new token replaced the old one.
    denied(cat.create_profile(&adult("X"), t), "replaced token");
    // Leaving with the token works and ends it.
    cat.switch_profile(h.sam.id, None, Some(&token)).unwrap();
    cat.switch_profile(h.kid.id, Some("1357"), None).unwrap();
    assert_all_denied_for_token(cat, &token);
    // The parent profile's own PIN is enough to leave for it.
    cat.switch_profile(DEFAULT_PROFILE, Some(PARENT_PIN), None)
        .unwrap();
}

fn assert_all_denied_for_token(cat: &Catalog, token: &str) {
    denied(
        cat.create_profile(&adult("Y"), Some(token)),
        "token after a switch",
    );
}

#[test]
fn the_token_expires_and_ends_on_request() {
    let h = home();
    let cat = &h.cat;
    cat.set_parent_token_ttl(Duration::from_millis(300));
    let token = cat.verify_parent_pin(PARENT_PIN).unwrap().token.unwrap();
    let t = Some(token.as_str());
    // Each accepted call renews it.
    std::thread::sleep(Duration::from_millis(200));
    cat.reorder_profiles(&[], t).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    cat.reorder_profiles(&[], t).unwrap();
    // Unused for longer than its time: gone.
    std::thread::sleep(Duration::from_millis(400));
    denied(cat.reorder_profiles(&[], t), "expired token");
    assert_all_denied(&h, t);

    let token = cat.verify_parent_pin(PARENT_PIN).unwrap().token.unwrap();
    cat.end_parent_session();
    denied(cat.reorder_profiles(&[], Some(&token)), "ended token");
}

#[test]
fn unsupervised_profiles_need_no_pin() {
    let h = home();
    let cat = &h.cat;
    // The adult without a PIN; the parent PIN exists.
    cat.switch_profile(
        h.sam.id,
        None,
        cat.verify_parent_pin(PARENT_PIN).unwrap().token.as_deref(),
    )
    .unwrap();
    let p = cat.create_profile(&adult("Guest"), None).unwrap();
    cat.update_profile(p.id, &kid_settings(), None).unwrap();
    cat.set_profile_pin(p.id, Some("1111"), None).unwrap();
    cat.set_profile_folders(p.id, &[], None).unwrap();
    cat.reorder_profiles(&[p.id], None).unwrap();
    cat.delete_profile(p.id, None).unwrap();
    // Back to the kid profile: no PIN needed to enter it.
    cat.switch_profile(h.kid.id, None, None).unwrap();

    // Without any parent PIN there is nothing to prove: the kid profile is
    // not guarded (the app does not allow this state, ADR-0059).
    let token = cat.verify_parent_pin(PARENT_PIN).unwrap().token;
    cat.set_profile_pin(DEFAULT_PROFILE, None, token.as_deref())
        .unwrap();
    cat.reorder_profiles(&[], None).unwrap();
    cat.switch_profile(h.sam.id, None, None).unwrap();
}

#[test]
fn wrong_pins_back_off_across_checks_and_restarts() {
    let h = home();
    let cat = &h.cat;
    for i in 1..=4 {
        let v = cat.verify_parent_pin("9999").unwrap();
        assert!(!v.ok && !v.locked);
        assert_eq!(v.attempts.failures, i);
    }
    // Every PIN check counts: the fifth wrong one is a profile's own PIN.
    denied(
        cat.switch_profile(DEFAULT_PROFILE, Some("9999"), None),
        "wrong",
    );
    let a = cat.pin_attempts().unwrap();
    assert_eq!(a.failures, 5);
    let until = a.locked_until_ms.expect("a wait after five wrong PINs");

    // While it runs, the right PIN is not even checked.
    let v = cat.verify_parent_pin(PARENT_PIN).unwrap();
    assert!(!v.ok && v.locked && v.token.is_none());
    let v = cat
        .verify_profile_pin(DEFAULT_PROFILE, PARENT_PIN, true)
        .unwrap();
    assert!(!v.ok && v.locked);
    denied(
        cat.switch_profile(DEFAULT_PROFILE, Some(PARENT_PIN), None),
        "locked switch",
    );
    assert_eq!(cat.pin_attempts().unwrap().failures, 5, "not counted");

    // A restart keeps it.
    let h = h.reopen();
    let cat = &h.cat;
    assert_eq!(cat.active_profile().unwrap().id, h.kid.id);
    let a = cat.pin_attempts().unwrap();
    assert_eq!(a.failures, 5);
    assert_eq!(a.locked_until_ms, Some(until));
    assert!(cat.verify_parent_pin(PARENT_PIN).unwrap().locked);
    assert_all_denied(&h, None);
}
