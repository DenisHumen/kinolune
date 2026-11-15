// SPDX-License-Identifier: MPL-2.0
//! Live recordings first (TWI-018): on low space the monitor empties the
//! trash, then pauses the other jobs, and pauses a live recording only at
//! the hard reserve; the gate admits a live recording below the reserve.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::{Env, exists, wait_until};
use pa_jobs::{
    JobContext, JobId, JobState, NewJob, PauseReason, RunOutcome, priority, serde_json::json,
};
use pa_storage::{LiveReserve, StorageManager, wait};

const DOWNLOAD: &str = "offline_copy";
const LIVE: &str = "twitch_live_record";

/// Writes 1000 bytes into the job's tmp folder, then holds until stopped
/// (or, once `hold` is cleared, finishes without committing).
fn register(env: &Env, kind: &str, hold: Arc<AtomicBool>) {
    let storage: StorageManager = env.storage.clone();
    env.queue.register(kind, move |ctx: &JobContext| {
        let job = ctx.job();
        let tmp = storage
            .job_tmp_dir(job.target_storage.as_deref().unwrap(), job.id)
            .unwrap()
            .join("part");
        if !tmp.exists() {
            std::fs::write(&tmp, vec![0u8; 1000]).unwrap();
        }
        ctx.save_checkpoint(&json!({}), 1000).unwrap();
        while hold.load(Ordering::SeqCst) {
            if ctx.stop_requested() {
                return RunOutcome::Interrupted;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        RunOutcome::Done(None)
    });
}

fn download() -> NewJob {
    NewJob::new(DOWNLOAD, json!({}))
        .target_storage("dl")
        .bytes_total(2000)
}

fn live() -> NewJob {
    NewJob::new(LIVE, json!({}))
        .priority(priority::LIVE)
        .target_storage("dl")
        .bytes_total(50_000)
}

fn state(env: &Env, id: JobId) -> JobState {
    env.queue.get(id).unwrap().state
}

#[test]
fn hard_reserve_is_a_share_of_the_reserve_with_a_cap() {
    let r = LiveReserve::default();
    assert_eq!(r.of(0), 0);
    assert_eq!(r.of(20_000), 5000);
    assert_eq!(r.of(100 << 30), 2 << 30);
    let all = LiveReserve {
        share: 2.0,
        max_bytes: u64::MAX,
    };
    assert_eq!(all.of(7), 7);
}

#[test]
fn low_space_empties_the_trash_then_pauses_others_then_live_at_the_hard_reserve() {
    // Volume 100 000, reserve 20 000, hard reserve 5000.
    let env = Env::new(100_000, |t| t.reserve_bytes(20_000));
    let hold = Arc::new(AtomicBool::new(true));
    register(&env, DOWNLOAD, Arc::clone(&hold));
    register(&env, LIVE, Arc::clone(&hold));
    let trashed = env.add_item("old/rec.bin", Some("twitch:1"), 10_000);
    env.storage.trash_item(trashed).unwrap();

    let dl = env.queue.enqueue(&download()).unwrap();
    let rec = env.storage.enqueue(&env.queue, &live()).unwrap();
    assert_eq!(env.queue.tick().unwrap(), 2);
    wait_until(|| [dl, rec].iter().all(|&id| env.queue.get(id).unwrap().bytes_done == 1000));
    let usage = &env.storage.usage().unwrap()[0];
    assert_eq!((usage.reserve, usage.hard_reserve), (20_000, 5000));
    // 100 000 - 10 000 trash - 2 × 1000 tmp.
    assert_eq!(usage.available, Some(88_000));

    // 1. Another program fills the disk to 15 000 free: below the reserve.
    //    The trash goes first and that is enough; nothing is paused.
    env.fs.set_external(&env.volume, 73_000);
    let report = env.storage.check(&env.queue).unwrap();
    assert_eq!(report.purged, vec![trashed]);
    assert_eq!(report.purged_bytes, 10_000);
    assert!(report.paused.is_empty(), "{report:?}");
    assert!(!exists(&env.target_path().join(".trash").join(trashed.to_string())));
    assert!(env.storage.trash().unwrap().is_empty());
    assert_eq!(report.usage[0].available, Some(25_000));

    // 2. 12 000 free, no trash left: the download is paused, the live
    //    recording keeps going.
    env.fs.set_external(&env.volume, 86_000);
    let report = env.storage.check(&env.queue).unwrap();
    assert_eq!(report.paused, vec![dl]);
    assert!(report.live_paused.is_empty() && report.purged.is_empty());
    wait_until(|| state(&env, dl) == JobState::Paused);
    assert_eq!(state(&env, rec), JobState::Running);
    assert_eq!(
        env.queue.get(dl).unwrap().pause_reason,
        Some(PauseReason::Resources)
    );

    // 3. 5000 free: the hard reserve. Now the live recording stops too.
    env.fs.set_external(&env.volume, 93_000);
    let report = env.storage.check(&env.queue).unwrap();
    assert_eq!(report.paused, vec![rec]);
    assert_eq!(report.live_paused, vec![rec]);
    wait_until(|| state(&env, rec) == JobState::Paused);

    // Both wait for space.
    env.queue.tick().unwrap();
    assert_eq!(state(&env, rec), JobState::WaitingResources);
    assert_eq!(state(&env, dl), JobState::WaitingResources);
    assert_eq!(
        env.queue.get(rec).unwrap().wait_reason.as_deref(),
        Some(wait::DISK_RESERVE)
    );

    // 4. 9000 free: above the hard reserve but below the reserve. The gate
    //    admits the live recording, not the download.
    env.fs.set_external(&env.volume, 89_000);
    env.queue.tick().unwrap();
    wait_until(|| state(&env, rec) == JobState::Running);
    assert_eq!(state(&env, dl), JobState::WaitingResources);
    // The monitor leaves it running there.
    assert!(env.storage.check(&env.queue).unwrap().paused.is_empty());

    // 5. Space is back: everything finishes.
    env.fs.set_external(&env.volume, 0);
    hold.store(false, Ordering::SeqCst);
    env.idle();
    assert_eq!(state(&env, rec), JobState::Done);
    assert_eq!(state(&env, dl), JobState::Done);
}

#[test]
fn without_a_live_recording_the_trash_is_kept() {
    // ADR-0062: the trash is not emptied early for ordinary jobs.
    let env = Env::new(100_000, |t| t.reserve_bytes(20_000));
    let hold = Arc::new(AtomicBool::new(true));
    register(&env, DOWNLOAD, Arc::clone(&hold));
    let trashed = env.add_item("old.bin", None, 10_000);
    env.storage.trash_item(trashed).unwrap();
    let dl = env.queue.enqueue(&download()).unwrap();
    env.queue.tick().unwrap();
    wait_until(|| env.queue.get(dl).unwrap().bytes_done == 1000);
    env.fs.set_external(&env.volume, 73_000);
    let report = env.storage.check(&env.queue).unwrap();
    assert!(report.purged.is_empty());
    assert_eq!(report.paused, vec![dl]);
    assert_eq!(env.storage.trash().unwrap().len(), 1);
    hold.store(false, Ordering::SeqCst);
}

#[test]
fn a_waiting_live_recording_gets_the_trash_emptied() {
    let env = Env::new(100_000, |t| t.reserve_bytes(20_000));
    register(&env, LIVE, Arc::new(AtomicBool::new(false)));
    let trashed = env.add_item("old.bin", None, 10_000);
    env.storage.trash_item(trashed).unwrap();
    // 4000 free: even a live recording may not start.
    env.fs.set_external(&env.volume, 86_000);
    let rec = env.storage.enqueue(&env.queue, &live()).unwrap();
    env.queue.tick().unwrap();
    assert_eq!(state(&env, rec), JobState::WaitingResources);

    let report = env.storage.check(&env.queue).unwrap();
    assert_eq!(report.purged, vec![trashed]);
    env.idle();
    assert_eq!(state(&env, rec), JobState::Done);
}

#[test]
fn live_recording_passes_the_quota_by_the_hard_reserve_only() {
    let env = Env::new(1_000_000, |t| t.quota_bytes(10_000).reserve_bytes(8000));
    let hold = Arc::new(AtomicBool::new(true));
    register(&env, DOWNLOAD, Arc::clone(&hold));
    register(&env, LIVE, Arc::clone(&hold));
    env.add_item("kept.bin", None, 9500);
    let dl = env.queue.enqueue(&download()).unwrap();
    let rec = env.storage.enqueue(&env.queue, &live()).unwrap();
    env.queue.tick().unwrap();
    // The download does not fit the quota; the live recording starts.
    assert_eq!(state(&env, dl), JobState::WaitingResources);
    assert_eq!(
        env.queue.get(dl).unwrap().wait_reason.as_deref(),
        Some(wait::QUOTA)
    );
    wait_until(|| env.queue.get(rec).unwrap().bytes_done == 1000);
    // 9500 + 1000 > 10 000, but within quota + 2000 (hard reserve).
    assert!(env.storage.check(&env.queue).unwrap().paused.is_empty());
    // 2000 more kept: 11 500 + 1000 > 10 000 + 2000.
    env.add_item("more.bin", None, 2000);
    let report = env.storage.check(&env.queue).unwrap();
    assert_eq!(report.live_paused, vec![rec]);
    hold.store(false, Ordering::SeqCst);
}
