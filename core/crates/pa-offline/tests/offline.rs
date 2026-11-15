// SPDX-License-Identifier: MPL-2.0
//! NET-019 on a simulated share: a movie with subtitles and a poster is
//! copied, linked to the source file and served by the stream opener with
//! the share offline; a dropped connection resumes from the offset (in the
//! same run and after a retry); a killed process resumes from the `.part`
//! file; seasons and albums queue one job per item; watched copies are
//! deleted by the retention rule; Wi-Fi only holds the jobs on a metered
//! network. Real Samba and WebDAV: `tests/servers.rs`.

mod common;

use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use common::{Env, content, open, put, test_copy_config};
use pa_catalog::{ItemRef, SourceOpener};
use pa_jobs::{Conditions, JobState, Network};
use pa_offline::{DownloadOptions, DownloadState, Selection, TARGET};
use pa_vfs::{ByteSource, VfsPath};

const MOVIE: &str = "Movies/Dune (2021)/Dune.2021.1080p.WEB-DL.mkv";

fn movie_env(size: usize) -> (tempfile::TempDir, Env, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let env = open(dir.path(), test_copy_config());
    let data = content(size, 7);
    put(&env.share, MOVIE, &data);
    put(
        &env.share,
        "Movies/Dune (2021)/Dune.2021.1080p.WEB-DL.en.srt",
        b"1\n00:00:01,000 --> 00:00:02,000\nHi\n",
    );
    put(
        &env.share,
        "Movies/Dune (2021)/poster.jpg",
        &content(3000, 1),
    );
    env.scan();
    (dir, env, data)
}

fn movie(env: &Env) -> ItemRef {
    let movies = env.catalog.movies().unwrap();
    assert_eq!(movies.len(), 1, "{movies:?}");
    movies[0].id
}

fn read_all(file: &dyn ByteSource) -> Vec<u8> {
    let mut out = vec![0u8; usize::try_from(file.size()).unwrap()];
    file.read_exact_at(0, &mut out).unwrap();
    out
}

/// Ticks the queue until the job is in `state` (retries are due later, so
/// `run_until_idle` alone returns early).
fn wait_for(env: &Env, job: i64, state: JobState) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        env.queue.run_until_idle(Duration::from_secs(5)).unwrap();
        let now = env.queue.get(job).unwrap().state;
        if now == state {
            return;
        }
        assert!(Instant::now() < deadline, "job {job} is {now}, not {state}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[test]
fn a_movie_is_copied_with_its_sidecars_and_plays_offline() {
    let (_dir, env, data) = movie_env(700_000);
    let item = movie(&env);
    let jobs = env
        .offline
        .download(&Selection::Item(item), &DownloadOptions::default())
        .unwrap();
    assert_eq!(jobs.len(), 1);
    wait_for(&env, jobs[0], JobState::Done);

    let copies = env.offline.copies().unwrap();
    assert_eq!(copies.len(), 1);
    let copy = &copies[0];
    assert_eq!(copy.item, item.to_string());
    assert!(copy.size_bytes >= data.len() as u64);
    assert!(copy.folder.starts_with(&env.offline_dir));
    assert_eq!(std::fs::read(copy.folder.join(MOVIE)).unwrap(), data);
    assert!(copy.folder.join("Movies/Dune (2021)/poster.jpg").is_file());
    assert!(
        copy.folder
            .join("Movies/Dune (2021)/Dune.2021.1080p.WEB-DL.en.srt")
            .is_file()
    );
    // Linked, not a new item: the library still has one movie.
    assert_eq!(env.catalog.movies().unwrap().len(), 1);
    let state = env.offline.state(&item.to_string()).unwrap().unwrap();
    assert_eq!(state.state, DownloadState::Done);
    // Queued again: nothing new.
    let again = env
        .offline
        .download(&Selection::Item(item), &DownloadOptions::default())
        .unwrap();
    assert!(again.is_empty());

    // The server goes away: the opener plays the copy, subtitles and art
    // included.
    env.ctl.offline.store(true, Ordering::SeqCst);
    let opener = SourceOpener::new(env.catalog.clone());
    let file = opener
        .open_file(env.source, &VfsPath::parse(MOVIE).unwrap())
        .unwrap();
    assert_eq!(read_all(file.as_ref()), data);
    for rel in [
        "Movies/Dune (2021)/Dune.2021.1080p.WEB-DL.en.srt",
        "Movies/Dune (2021)/poster.jpg",
    ] {
        assert!(
            opener
                .open_file(env.source, &VfsPath::parse(rel).unwrap())
                .is_ok(),
            "{rel}"
        );
    }
    let before = env.ctl.read.load(Ordering::SeqCst);
    let _ = read_all(file.as_ref());
    assert_eq!(
        env.ctl.read.load(Ordering::SeqCst),
        before,
        "read the share"
    );

    // Deleted: the network file again (and offline, nothing).
    let freed = env.offline.delete_copy(copy.id).unwrap();
    assert!(freed > 0);
    assert!(!copy.folder.exists());
    assert!(env.offline.copies().unwrap().is_empty());
    assert!(
        opener
            .open_file(env.source, &VfsPath::parse(MOVIE).unwrap())
            .is_err()
    );
}

#[test]
fn a_dropped_connection_resumes_in_the_same_run() {
    let mut config = test_copy_config();
    config.reconnect_window = Duration::from_secs(20);
    let dir = tempfile::tempdir().unwrap();
    let env = open(dir.path(), config);
    let data = content(2_000_000, 3);
    put(&env.share, MOVIE, &data);
    env.scan();
    env.ctl.read.store(0, Ordering::SeqCst);
    env.ctl.cut_after.store(600_000, Ordering::SeqCst);
    let job = env
        .offline
        .download(&Selection::Item(movie(&env)), &DownloadOptions::default())
        .unwrap()[0];
    let ctl = env.ctl.clone();
    let back = std::thread::spawn(move || {
        while !ctl.offline.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(400));
        ctl.offline.store(false, Ordering::SeqCst);
    });
    wait_for(&env, job, JobState::Done);
    back.join().unwrap();
    let j = env.queue.get(job).unwrap();
    assert_eq!(j.attempts, 1, "reconnected within the run");
    let first = env.ctl.first_reads.lock().unwrap().clone();
    assert!(first.len() >= 2, "{first:?}");
    assert!(first[1] >= 500_000, "resumed at {first:?}");
    let copy = &env.offline.copies().unwrap()[0];
    assert_eq!(std::fs::read(copy.folder.join(MOVIE)).unwrap(), data);
    assert!(env.ctl.read.load(Ordering::SeqCst) < data.len() as u64 + 200_000);
}

#[test]
fn a_long_outage_retries_and_resumes_from_the_offset() {
    let dir = tempfile::tempdir().unwrap();
    let env = open(dir.path(), test_copy_config());
    let data = content(2_000_000, 4);
    put(&env.share, MOVIE, &data);
    env.scan();
    env.ctl.read.store(0, Ordering::SeqCst);
    env.ctl.cut_after.store(900_000, Ordering::SeqCst);
    let job = env
        .offline
        .download(&Selection::Item(movie(&env)), &DownloadOptions::default())
        .unwrap()[0];
    // Longer than the reconnect window: the queue retries it.
    wait_for(&env, job, JobState::Retrying);
    let j = env.queue.get(job).unwrap();
    assert!(j.bytes_done >= 800_000, "{}", j.bytes_done);
    assert!(
        j.error.as_deref().unwrap_or("").contains("unavailable"),
        "{:?}",
        j.error
    );
    let progress = env.offline.state(&j_item(&env)).unwrap().unwrap();
    assert_eq!(progress.state, DownloadState::Retrying);
    env.ctl.offline.store(false, Ordering::SeqCst);
    wait_for(&env, job, JobState::Done);
    let copy = &env.offline.copies().unwrap()[0];
    assert_eq!(std::fs::read(copy.folder.join(MOVIE)).unwrap(), data);
    assert!(env.ctl.read.load(Ordering::SeqCst) < data.len() as u64 + 200_000);
}

fn j_item(env: &Env) -> String {
    movie(env).to_string()
}

#[test]
fn pause_resume_and_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let env = open(dir.path(), test_copy_config());
    let data = content(3_000_000, 5);
    put(&env.share, MOVIE, &data);
    env.scan();
    env.ctl.throttle_ms.store(5, Ordering::SeqCst);
    let item = movie(&env);
    let job = env
        .offline
        .download(&Selection::Item(item), &DownloadOptions::default())
        .unwrap()[0];
    let driver = env.queue.start();
    let deadline = Instant::now() + Duration::from_secs(30);
    while env.queue.get(job).unwrap().bytes_done < 500_000 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    env.offline.control(job, "pause").unwrap();
    while env.queue.get(job).unwrap().state != JobState::Paused {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let paused_at = env.queue.get(job).unwrap().bytes_done;
    env.ctl.throttle_ms.store(0, Ordering::SeqCst);
    env.offline.control(job, "resume").unwrap();
    while env.queue.get(job).unwrap().state != JobState::Done {
        assert!(
            Instant::now() < deadline,
            "{:?}",
            env.queue.get(job).unwrap()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(driver);
    assert!(paused_at > 0);
    let copy = &env.offline.copies().unwrap()[0];
    assert_eq!(std::fs::read(copy.folder.join(MOVIE)).unwrap(), data);

    // A cancelled download leaves nothing behind.
    env.offline.delete_item(&item.to_string()).unwrap();
    env.ctl.throttle_ms.store(5, Ordering::SeqCst);
    let job = env
        .offline
        .download(&Selection::Item(item), &DownloadOptions::default())
        .unwrap()[0];
    let driver = env.queue.start();
    while env.queue.get(job).unwrap().bytes_done < 300_000 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    env.offline.control(job, "cancel").unwrap();
    std::thread::sleep(Duration::from_millis(300));
    drop(driver);
    assert!(env.offline.downloads().unwrap().is_empty());
    let tmp = env.offline_dir.join(".tmp").join(format!("job-{job}"));
    assert!(!tmp.exists(), "{tmp:?} left");
}

/// Child process of the kill test: downloads slowly until killed.
#[test]
#[ignore = "run by a_killed_process_resumes_from_the_part_file"]
fn kill_child_entry() {
    let Ok(dir) = std::env::var("PA_OFFLINE_KILL_DIR") else {
        return;
    };
    let env = open(std::path::Path::new(&dir), test_copy_config());
    env.ctl.throttle_ms.store(4, Ordering::SeqCst);
    let job: i64 = std::env::var("PA_OFFLINE_KILL_JOB")
        .unwrap()
        .parse()
        .unwrap();
    let _ = job;
    let _driver = env.queue.start();
    std::thread::sleep(Duration::from_secs(120));
    panic!("the parent did not kill the child");
}

#[test]
fn a_killed_process_resumes_from_the_part_file() {
    let dir = tempfile::tempdir().unwrap();
    let data = content(8_000_000, 9);
    let (job, item) = {
        let env = open(dir.path(), test_copy_config());
        put(&env.share, MOVIE, &data);
        env.scan();
        let item = movie(&env);
        let job = env
            .offline
            .download(&Selection::Item(item), &DownloadOptions::default())
            .unwrap()[0];
        (job, item)
    };
    let exe = std::env::current_exe().unwrap();
    let mut child = Command::new(exe)
        .args(["--exact", "kill_child_entry", "--ignored", "--nocapture"])
        .env("PA_OFFLINE_KILL_DIR", dir.path())
        .env("PA_OFFLINE_KILL_JOB", job.to_string())
        .spawn()
        .unwrap();
    let part = dir
        .path()
        .join("offline/.tmp")
        .join(format!("job-{job}"))
        .join("out")
        .join(format!("{MOVIE}.part"));
    let deadline = Instant::now() + Duration::from_secs(60);
    let killed_at = loop {
        let len = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if len >= 2_500_000 {
            child.kill().unwrap();
            break len;
        }
        assert!(Instant::now() < deadline, "child made no progress");
        std::thread::sleep(Duration::from_millis(5));
    };
    child.wait().unwrap();

    // The next start: the job left `running` goes back to the queue and
    // continues from the length of the `.part` file.
    let env = open(dir.path(), test_copy_config());
    env.storage.clean_tmp(&env.queue).unwrap();
    assert!(
        part.exists(),
        "the startup cleanup kept the running job's tmp"
    );
    env.ctl.read.store(0, Ordering::SeqCst);
    wait_for(&env, job, JobState::Done);
    let first = env.ctl.first_reads.lock().unwrap().clone();
    assert!(first[0] >= killed_at - 200_000, "{first:?} vs {killed_at}");
    let read = env.ctl.read.load(Ordering::SeqCst);
    assert!(
        read <= data.len() as u64 - killed_at + 300_000,
        "read {read} after the kill at {killed_at}"
    );
    let copy = &env.offline.copies().unwrap()[0];
    assert_eq!(copy.item, item.to_string());
    assert_eq!(std::fs::read(copy.folder.join(MOVIE)).unwrap(), data);
}

#[test]
fn a_season_queues_one_job_per_episode_and_retention_deletes_watched() {
    let dir = tempfile::tempdir().unwrap();
    let env = open(dir.path(), test_copy_config());
    for (e, name) in [(1, "Secrets"), (2, "Lies"), (3, "Past")] {
        put(
            &env.share,
            &format!("Shows/Dark/Season 1/Dark.S01E0{e}.{name}.1080p.mkv"),
            &content(100_000, e),
        );
    }
    put(
        &env.share,
        "Shows/Dark/Season 2/Dark.S02E01.Ghosts.1080p.mkv",
        &content(100_000, 9),
    );
    env.scan();
    let show = env.catalog.shows().unwrap()[0].id;
    let jobs = env
        .offline
        .download(
            &Selection::Season { show, season: 1 },
            &DownloadOptions::default(),
        )
        .unwrap();
    assert_eq!(jobs.len(), 3);
    env.run();
    let copies = env.offline.copies().unwrap();
    assert_eq!(copies.len(), 3);
    // The whole show: only the missing episode is new.
    let more = env
        .offline
        .download(&Selection::Item(show), &DownloadOptions::default())
        .unwrap();
    assert_eq!(more.len(), 1);
    env.run();
    assert_eq!(env.offline.copies().unwrap().len(), 4);

    // Watch one, then the cleanup trashes it with "delete watched".
    let episodes = env.catalog.episodes(show).unwrap();
    let first = &episodes[0];
    env.catalog
        .save_progress(
            first.id,
            3_000_000,
            3_000_000,
            pa_jobs::system_now_ms() - 1000,
            true,
        )
        .unwrap();
    env.offline.set_delete_watched(Some(0)).unwrap();
    assert_eq!(
        env.offline.settings().unwrap().delete_watched_after_days,
        Some(0)
    );
    let report = env.offline.cleanup().unwrap();
    assert_eq!(report.trashed.len(), 1, "{report:?}");
    let left: Vec<String> = env
        .offline
        .copies()
        .unwrap()
        .into_iter()
        .map(|c| c.item)
        .collect();
    assert_eq!(left.len(), 3);
    assert!(!left.contains(&first.id.to_string()));
    env.offline.set_delete_watched(None).unwrap();
    assert_eq!(
        env.offline.settings().unwrap().delete_watched_after_days,
        None
    );
}

#[test]
fn wifi_only_waits_on_a_metered_network() {
    let (_dir, env, _) = movie_env(200_000);
    assert_eq!(env.offline.set_wifi_only(true).unwrap(), 0);
    env.queue.set_conditions(Conditions {
        network: Network::Metered,
        on_battery: false,
    });
    let job = env
        .offline
        .download(&Selection::Item(movie(&env)), &DownloadOptions::default())
        .unwrap()[0];
    env.run();
    let j = env.queue.get(job).unwrap();
    assert_ne!(j.state, JobState::Done);
    assert_eq!(j.network, pa_jobs::NetworkPolicy::Unmetered);
    // Wi-Fi only off: the same job goes on mobile data.
    assert_eq!(env.offline.set_wifi_only(false).unwrap(), 1);
    wait_for(&env, job, JobState::Done);
}

#[test]
fn the_folder_does_not_move_under_copies_and_local_items_are_refused() {
    let (dir, env, _) = movie_env(100_000);
    let item = movie(&env);
    let job = env
        .offline
        .download(&Selection::Item(item), &DownloadOptions::default())
        .unwrap()[0];
    assert!(matches!(
        env.offline.set_folder(&dir.path().join("elsewhere")),
        Err(pa_offline::Error::Busy(_))
    ));
    wait_for(&env, job, JobState::Done);
    assert!(matches!(
        env.offline.set_folder(&dir.path().join("elsewhere")),
        Err(pa_offline::Error::Busy("copies"))
    ));
    assert_eq!(env.offline.folder().unwrap(), env.offline_dir);
    assert!(env.storage.target(TARGET).is_ok());

    // A local folder source: nothing to download.
    let local = dir.path().join("local");
    put(&local, "Film (2000)/Film.2000.mkv", &content(1000, 1));
    let id = env
        .catalog
        .add_source(None, "local", "Disk", local.to_str().unwrap(), None)
        .unwrap()
        .id;
    let mut errors = Vec::new();
    env.catalog
        .scan(Some(id), &pa_catalog::CancelToken::new(), &mut |u| {
            errors.extend(u.error)
        })
        .unwrap();
    let film = env
        .catalog
        .movies()
        .unwrap()
        .into_iter()
        .find(|m| m.source_id == id)
        .unwrap();
    assert!(matches!(
        env.offline
            .download(&Selection::Item(film.id), &DownloadOptions::default()),
        Err(pa_offline::Error::Local(_))
    ));
}
