// SPDX-License-Identifier: MPL-2.0
//! Retention rules, the 7-day trash and the `cleanup` job on real files in
//! a temp dir.

mod common;

use common::{DAY, Env, T0, exists};
use pa_jobs::{JobState, NewJob, kinds, serde_json::json};
use pa_storage::{Error, ExpiryReason, RetentionRule, TRASH_DIR};

fn rule(group: Option<&str>) -> RetentionRule {
    RetentionRule::new("dl", group.map(str::to_owned))
}

#[test]
fn keep_last_n_per_group() {
    let env = Env::new(1_000_000, |t| t);
    let mut ch1 = Vec::new();
    for i in 0..5 {
        env.set_now(T0 + i * 1000);
        ch1.push(env.add_item(&format!("ch1/{i}.mp4"), Some("ch1"), 100));
    }
    let ch2: Vec<i64> = (0..2)
        .map(|i| env.add_item(&format!("ch2/{i}.mp4"), Some("ch2"), 100))
        .collect();
    env.storage
        .set_retention(&RetentionRule {
            keep_last: Some(3),
            ..rule(None)
        })
        .unwrap();

    let plan = env.storage.plan_retention().unwrap();
    let ids: Vec<i64> = plan.iter().map(|e| e.item.id).collect();
    assert_eq!(ids, vec![ch1[1], ch1[0]]);
    assert!(plan.iter().all(|e| e.reason == ExpiryReason::KeepLast));

    let report = env.storage.apply_retention().unwrap();
    assert_eq!(report.trashed, vec![ch1[1], ch1[0]]);
    assert!(report.purged.is_empty());
    let kept: Vec<i64> = env
        .storage
        .items("dl")
        .unwrap()
        .iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(kept.len(), 5);
    assert!(ch2.iter().all(|id| kept.contains(id)));
    let root = env.target_path();
    assert!(!exists(&root.join("ch1/0.mp4")));
    assert!(exists(
        &root.join(TRASH_DIR).join(ch1[0].to_string()).join("0.mp4")
    ));
    assert_eq!(env.storage.usage().unwrap()[0].trash, 200);
    // Applying again changes nothing.
    assert!(env.storage.apply_retention().unwrap().trashed.is_empty());
}

#[test]
fn group_rule_overrides_the_default_and_favourites_are_kept() {
    let env = Env::new(1_000_000, |t| t);
    let fav = env.add_item("ch1/fav.mp4", Some("ch1"), 10);
    env.storage.set_favorite(fav, true).unwrap();
    env.advance(1);
    let a = env.add_item("ch1/a.mp4", Some("ch1"), 10);
    env.advance(1);
    let b = env.add_item("ch1/b.mp4", Some("ch1"), 10);
    env.advance(1);
    let loose = env.add_item("loose.mp4", None, 10);

    env.storage
        .set_retention(&RetentionRule {
            keep_last: Some(0),
            ..rule(None)
        })
        .unwrap();
    env.storage
        .set_retention(&RetentionRule {
            keep_last: Some(1),
            ..rule(Some("ch1"))
        })
        .unwrap();
    // Replacing a rule keeps one per (target, group).
    env.storage
        .set_retention(&RetentionRule {
            keep_last: Some(1),
            max_age_days: Some(365),
            ..rule(Some("ch1"))
        })
        .unwrap();
    assert_eq!(env.storage.retention_rules().unwrap().len(), 2);

    let ids: Vec<i64> = env
        .storage
        .plan_retention()
        .unwrap()
        .iter()
        .map(|e| e.item.id)
        .collect();
    // ch1 keeps b (newest) and the favourite; the ungrouped item falls
    // under the default `keep_last 0` (the NULL group sorts first).
    assert_eq!(ids, vec![loose, a]);
    let _ = b;

    // Without keep_favorites the favourite counts like any other item.
    env.storage
        .set_retention(&RetentionRule {
            keep_last: Some(1),
            keep_favorites: false,
            ..rule(Some("ch1"))
        })
        .unwrap();
    let ids: Vec<i64> = env
        .storage
        .plan_retention()
        .unwrap()
        .iter()
        .map(|e| e.item.id)
        .collect();
    assert_eq!(ids, vec![loose, a, fav]);

    assert!(env.storage.remove_retention("dl", None).unwrap());
    assert!(!env.storage.remove_retention("dl", None).unwrap());
}

#[test]
fn max_age_and_watched_after() {
    let env = Env::new(1_000_000, |t| t);
    let old = env.add_item("old.mp4", None, 10);
    env.set_now(T0 + 20 * DAY);
    let watched_long_ago = env.add_item("w1.mp4", None, 10);
    let watched_recently = env.add_item("w2.mp4", None, 10);
    let unwatched = env.add_item("u.mp4", None, 10);
    env.set_now(T0 + 35 * DAY);
    env.storage
        .set_watched(watched_long_ago, Some(T0 + 31 * DAY))
        .unwrap();
    env.storage
        .set_watched(watched_recently, Some(T0 + 34 * DAY))
        .unwrap();
    env.storage
        .set_retention(&RetentionRule {
            max_age_days: Some(30),
            watched_after_days: Some(3),
            ..rule(None)
        })
        .unwrap();

    let plan = env.storage.plan_retention().unwrap();
    let got: Vec<(i64, ExpiryReason)> = plan.iter().map(|e| (e.item.id, e.reason)).collect();
    assert!(got.contains(&(old, ExpiryReason::MaxAge)));
    assert!(got.contains(&(watched_long_ago, ExpiryReason::Watched)));
    assert_eq!(got.len(), 2);

    // Two days later the recent one expires too; the unwatched one stays.
    env.advance(2 * DAY);
    env.storage.apply_retention().unwrap();
    let kept: Vec<i64> = env
        .storage
        .items("dl")
        .unwrap()
        .iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(kept, vec![unwatched]);
    assert!(env.storage.set_watched(9999, None).is_err());
}

#[test]
fn trash_keeps_items_for_seven_days_and_restores() {
    let env = Env::new(1_000_000, |t| t);
    let a = env.add_item("vod/a.mp4", Some("ch"), 100);
    let b = env.add_item("vod/b.mp4", Some("ch"), 100);
    let root = env.target_path();

    env.storage.trash_item(a).unwrap();
    env.advance(DAY);
    env.storage.trash_item(b).unwrap();
    assert_eq!(env.storage.trash().unwrap().len(), 2);
    assert_eq!(env.storage.trash().unwrap()[0].id, b);
    assert_eq!(env.storage.usage().unwrap()[0].used, 0);

    // Restore b, but not over a new file at its path.
    std::fs::write(root.join("vod/b.mp4"), b"new").unwrap();
    assert!(matches!(env.storage.restore(b), Err(Error::Exists(_))));
    std::fs::remove_file(root.join("vod/b.mp4")).unwrap();
    let restored = env.storage.restore(b).unwrap();
    assert_eq!(restored.trashed_at_ms, None);
    assert_eq!(
        std::fs::metadata(root.join("vod/b.mp4")).unwrap().len(),
        100
    );
    assert!(!exists(&root.join(TRASH_DIR).join(b.to_string())));

    // Six days after trashing, `a` is still there.
    env.advance(5 * DAY);
    let report = env.storage.apply_retention().unwrap();
    assert!(report.purged.is_empty());
    assert!(exists(
        &root.join(TRASH_DIR).join(a.to_string()).join("a.mp4")
    ));

    // After seven days it is gone for good.
    env.advance(DAY);
    let report = env.storage.apply_retention().unwrap();
    assert_eq!(report.purged, vec![a]);
    assert_eq!(report.purged_bytes, 100);
    assert!(!exists(&root.join(TRASH_DIR).join(a.to_string())));
    assert!(matches!(env.storage.item(a), Err(Error::NotFound(_))));
    assert_eq!(env.storage.items("dl").unwrap().len(), 1);
}

#[test]
fn missing_file_is_trashed_and_bad_paths_are_refused() {
    let env = Env::new(1_000_000, |t| t);
    let a = env.add_item("a.mp4", None, 10);
    std::fs::remove_file(env.target_path().join("a.mp4")).unwrap();
    assert!(env.storage.trash_item(a).unwrap().trashed_at_ms.is_some());
    // A new item may reuse the path of a trashed one.
    env.add_item("a.mp4", None, 10);

    let src = env.dir.path().join("x");
    std::fs::write(&src, b"x").unwrap();
    for bad in ["../escape", "/abs", ".trash/1/x", ".tmp/x"] {
        let item = pa_storage::NewItem::new("dl", bad);
        assert!(env.storage.commit_file(&src, &item, None).is_err(), "{bad}");
    }
    let item = pa_storage::NewItem::new("dl", "a.mp4");
    assert!(matches!(
        env.storage.commit_file(&src, &item, None),
        Err(Error::Exists(_))
    ));
    assert!(src.exists());
}

#[test]
fn cleanup_job_applies_retention() {
    let env = Env::new(1_000_000, |t| t);
    for i in 0..4 {
        env.advance(1);
        env.add_item(&format!("{i}.mp4"), None, 10);
    }
    env.storage
        .set_retention(&RetentionRule {
            keep_last: Some(1),
            ..rule(None)
        })
        .unwrap();
    let id = env
        .queue
        .enqueue(&NewJob::new(kinds::CLEANUP, json!({})))
        .unwrap();
    env.idle();
    let job = env.queue.get(id).unwrap();
    assert_eq!(job.state, JobState::Done);
    assert_eq!(job.result.unwrap()["trashed"], 3);
    assert_eq!(env.storage.items("dl").unwrap().len(), 1);
    assert_eq!(env.storage.trash().unwrap().len(), 3);
}
