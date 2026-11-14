// SPDX-License-Identifier: MPL-2.0
//! Quotas, free-space reserve, admission and monitoring on a simulated
//! small volume (see `common`).

mod common;

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::{Env, exists, wait_until};
use pa_jobs::{JobContext, JobState, NewJob, PauseReason, RunOutcome, serde_json::json};
use pa_storage::{Error, Fit, NewItem, StorageManager, wait};

const KIND: &str = "offline_copy";

/// A download handler: writes `bytes_total` into the job's tmp folder in
/// 1000-byte chunks with checkpoints, resumes from the file length, and
/// commits the file to `params.name`. While `hold` is set it stops after
/// the first chunk and waits to be paused.
fn register_writer(env: &Env, hold: Arc<AtomicBool>) {
    let storage: StorageManager = env.storage.clone();
    env.queue.register(KIND, move |ctx: &JobContext| {
        let job = ctx.job();
        let target = job.target_storage.clone().unwrap();
        let total = job.bytes_total.unwrap();
        let tmp = storage.job_tmp_dir(&target, job.id).unwrap().join("part");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&tmp)
            .unwrap();
        let mut done = file.metadata().unwrap().len();
        while done < total {
            let n = (total - done).min(1000);
            file.write_all(&vec![0u8; n as usize]).unwrap();
            done += n;
            ctx.save_checkpoint(&json!({}), done).unwrap();
            while hold.load(Ordering::SeqCst) {
                if ctx.stop_requested() {
                    return RunOutcome::Interrupted;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        drop(file);
        let name = ctx.params()["name"].as_str().unwrap();
        storage
            .commit_file(&tmp, &NewItem::new(target, name), Some(job.id))
            .unwrap();
        RunOutcome::Done(None)
    });
}

fn download(name: &str, bytes: u64) -> NewJob {
    NewJob::new(KIND, json!({ "name": name }))
        .target_storage("dl")
        .bytes_total(bytes)
}

#[test]
fn reserve_holds_jobs_until_the_trash_is_emptied() {
    let env = Env::new(10_000, |t| t.reserve_bytes(1000));
    register_writer(&env, Arc::new(AtomicBool::new(false)));
    let a = env.queue.enqueue(&download("a.bin", 5000)).unwrap();
    let b = env.queue.enqueue(&download("b.bin", 5000)).unwrap();
    env.idle();

    // A fitted; B did not, both before A started (A's 5000 bytes were
    // reserved) and after A's file landed.
    assert_eq!(env.queue.get(a).unwrap().state, JobState::Done);
    let job_b = env.queue.get(b).unwrap();
    assert_eq!(job_b.state, JobState::WaitingResources);
    assert_eq!(job_b.wait_reason.as_deref(), Some(wait::DISK_RESERVE));
    assert!(env.target_path().join("a.bin").exists());

    // A trashed file still takes disk space.
    let item = env.storage.items("dl").unwrap()[0].id;
    env.storage.trash_item(item).unwrap();
    env.idle();
    assert_eq!(env.queue.get(b).unwrap().state, JobState::WaitingResources);

    // Emptying the trash frees it and B runs.
    let purged = env.storage.purge_trash(None).unwrap();
    assert_eq!(purged.purged, vec![item]);
    assert_eq!(purged.purged_bytes, 5000);
    env.idle();
    assert_eq!(env.queue.get(b).unwrap().state, JobState::Done);
    assert_eq!(std::fs::metadata(env.target_path().join("b.bin")).unwrap().len(), 5000);
}

#[test]
fn quota_counts_kept_items_not_the_trash() {
    let env = Env::new(1_000_000, |t| t.quota_bytes(6000));
    register_writer(&env, Arc::new(AtomicBool::new(false)));
    let a = env.queue.enqueue(&download("a.bin", 4000)).unwrap();
    let b = env.queue.enqueue(&download("b.bin", 4000)).unwrap();
    env.idle();
    assert_eq!(env.queue.get(a).unwrap().state, JobState::Done);
    let job_b = env.queue.get(b).unwrap();
    assert_eq!(job_b.state, JobState::WaitingResources);
    assert_eq!(job_b.wait_reason.as_deref(), Some(wait::QUOTA));

    let usage = &env.storage.usage().unwrap()[0];
    assert_eq!((usage.used, usage.quota), (4000, Some(6000)));

    let item = env.storage.items("dl").unwrap()[0].id;
    env.storage.trash_item(item).unwrap();
    env.idle();
    assert_eq!(env.queue.get(b).unwrap().state, JobState::Done);
    let usage = &env.storage.usage().unwrap()[0];
    assert_eq!((usage.used, usage.trash), (4000, 4000));
}

#[test]
fn quota_in_percent_and_jobs_that_never_fit_are_refused() {
    let env = Env::new(100_000, |t| t.quota_percent(10.0).quota_bytes(50_000));
    // The smaller limit wins: 10 % of 100 000.
    assert_eq!(env.storage.check_fit("dl", 10_000), Fit::Fits);
    assert_eq!(env.storage.check_fit("dl", 10_001), Fit::Never(wait::TOO_LARGE));
    let err = env
        .storage
        .enqueue(&env.queue, &download("big.bin", 20_000))
        .unwrap_err();
    assert!(matches!(err, Error::WontFit { reason: wait::TOO_LARGE, .. }));
    let err = env
        .storage
        .enqueue(&env.queue, &download("x", 1).target_storage("nope"))
        .unwrap_err();
    assert!(matches!(err, Error::WontFit { reason: wait::NO_TARGET, .. }));
    assert!(env.queue.list(&[]).unwrap().is_empty());
    // One that does not fit now is queued.
    env.fs.set_external(&env.volume, 95_000);
    assert_eq!(env.storage.check_fit("dl", 8000), Fit::NotNow(wait::DISK_RESERVE));
    env.storage
        .enqueue(&env.queue, &download("later.bin", 8000))
        .unwrap();
    assert_eq!(env.queue.list(&[]).unwrap().len(), 1);
}

#[test]
fn reserve_in_percent_and_unavailable_volume() {
    let env = Env::new(100_000, |t| t.reserve_percent(5.0).reserve_bytes(1000));
    let usage = &env.storage.usage().unwrap()[0];
    assert_eq!(usage.reserve, 5000);
    assert!(!usage.below_reserve());
    assert_eq!(env.storage.check_fit("dl", 95_000), Fit::Fits);
    assert_eq!(env.storage.check_fit("dl", 95_001), Fit::Never(wait::TOO_LARGE));

    env.fs.remove_volume(&env.volume);
    assert_eq!(env.storage.check_fit("dl", 1), Fit::NotNow(wait::DISK_UNAVAILABLE));
    let usage = &env.storage.usage().unwrap()[0];
    assert!(usage.below_reserve() && usage.available.is_none());
}

#[test]
fn monitor_pauses_on_low_space_and_resumes_when_freed() {
    let env = Env::new(10_000, |t| t.reserve_bytes(2000));
    let hold = Arc::new(AtomicBool::new(true));
    register_writer(&env, Arc::clone(&hold));
    let id = env.queue.enqueue(&download("rec.bin", 3000)).unwrap();
    assert_eq!(env.queue.tick().unwrap(), 1);
    wait_until(|| env.queue.get(id).unwrap().bytes_done == 1000);

    // Enough space: nothing to do.
    let report = env.storage.check(&env.queue).unwrap();
    assert!(report.paused.is_empty());
    assert_eq!(report.usage[0].pending, 2000);

    // Another program fills the disk: 10 000 - 1000 - 7500 < reserve.
    env.fs.set_external(&env.volume, 7500);
    let report = env.storage.check(&env.queue).unwrap();
    assert_eq!(report.paused, vec![id]);
    assert!(report.usage[0].below_reserve());
    wait_until(|| env.queue.get(id).unwrap().state == JobState::Paused);
    assert_eq!(
        env.queue.get(id).unwrap().pause_reason,
        Some(PauseReason::Resources)
    );
    // A second check does not pause it again.
    assert!(env.storage.check(&env.queue).unwrap().paused.is_empty());

    // The scheduler parks it in waiting_resources while space is short.
    env.queue.tick().unwrap();
    let job = env.queue.get(id).unwrap();
    assert_eq!(job.state, JobState::WaitingResources);
    assert_eq!(job.wait_reason.as_deref(), Some(wait::DISK_RESERVE));
    assert_eq!(job.bytes_done, 1000);

    // Space is freed: it continues from its checkpoint and finishes.
    env.fs.set_external(&env.volume, 0);
    hold.store(false, Ordering::SeqCst);
    env.idle();
    let job = env.queue.get(id).unwrap();
    assert_eq!(job.state, JobState::Done);
    assert_eq!(job.attempts, 2);
    assert_eq!(std::fs::metadata(env.target_path().join("rec.bin")).unwrap().len(), 3000);
    // The tmp folder is empty, the reservation is gone.
    assert_eq!(env.storage.usage().unwrap()[0].pending, 0);
}

#[test]
fn background_monitor_pauses_a_job_that_crosses_the_quota() {
    let env = Env::new(1_000_000, |t| t.quota_bytes(5000));
    let hold = Arc::new(AtomicBool::new(true));
    register_writer(&env, Arc::clone(&hold));
    env.add_item("old.bin", None, 3000);
    let id = env.queue.enqueue(&download("grow.bin", 1000)).unwrap();
    env.queue.tick().unwrap();
    wait_until(|| env.queue.get(id).unwrap().bytes_done == 1000);
    assert!(env.storage.check(&env.queue).unwrap().paused.is_empty());

    // Another file lands meanwhile: 4500 kept + 1000 written > 5000.
    env.add_item("other.bin", None, 1500);
    let (tx, rx) = std::sync::mpsc::channel();
    let monitor = env.storage.start_monitor(
        env.queue.clone(),
        std::time::Duration::from_millis(10),
        move |report| {
            if !report.paused.is_empty() {
                let _ = tx.send(report.paused.clone());
            }
        },
    );
    let paused = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
    drop(monitor);
    assert_eq!(paused, vec![id]);
    wait_until(|| env.queue.get(id).unwrap().state == JobState::Paused);
    env.queue.tick().unwrap();
    let job = env.queue.get(id).unwrap();
    assert_eq!(job.state, JobState::WaitingResources);
    assert_eq!(job.wait_reason.as_deref(), Some(wait::QUOTA));

    // Retention frees the quota; the job finishes.
    let other = env.storage.items("dl").unwrap()[0].id;
    env.storage.trash_item(other).unwrap();
    hold.store(false, Ordering::SeqCst);
    env.idle();
    assert_eq!(env.queue.get(id).unwrap().state, JobState::Done);
}

#[test]
fn startup_removes_tmp_of_jobs_that_are_gone() {
    let env = Env::new(1_000_000, |t| t);
    let live = env.queue.enqueue(&download("live.bin", 10)).unwrap();
    let live_dir = env.storage.job_tmp_dir("dl", live).unwrap();
    std::fs::write(live_dir.join("part"), b"resume me").unwrap();
    let dead_dir = env.storage.job_tmp_dir("dl", 999).unwrap();
    std::fs::write(dead_dir.join("part"), b"orphan").unwrap();
    let tmp = env.target_path().join(pa_storage::TMP_DIR);
    std::fs::write(tmp.join("xdev-1-0"), b"half copy").unwrap();

    assert_eq!(env.storage.clean_tmp(&env.queue).unwrap(), 2);
    assert!(exists(&live_dir.join("part")));
    assert!(!exists(&dead_dir));
    assert!(!exists(&tmp.join("xdev-1-0")));

    // Once the job is finished its folder goes too.
    env.queue.cancel(live).unwrap();
    assert_eq!(env.storage.clean_tmp(&env.queue).unwrap(), 1);
    assert!(!exists(&live_dir));
    assert_eq!(env.storage.clean_tmp_except(&HashSet::new()).unwrap(), 0);
}

#[test]
fn targets_survive_reopen_and_validate() {
    let env = Env::new(1_000_000, |t| t.quota_bytes(1234).reserve_percent(2.5));
    let reopened =
        StorageManager::open(&env.db, pa_storage::StorageConfig::default()).unwrap();
    let target = reopened.target("dl").unwrap();
    assert_eq!(target.quota_bytes, Some(1234));
    assert_eq!(target.reserve_percent, 2.5);
    assert!(target.path.is_dir());

    let bad = pa_storage::Target::new("Bad Id", pa_storage::TargetKind::Library, "/x");
    assert!(env.storage.set_target(&bad).is_err());
    let relative = pa_storage::Target::new("rel", pa_storage::TargetKind::Library, "rel/x");
    assert!(env.storage.set_target(&relative).is_err());
    assert!(env.storage.remove_target("dl").unwrap());
    assert!(env.storage.targets().is_empty());
    assert!(!env.storage.remove_target("dl").unwrap());
}
