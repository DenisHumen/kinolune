// SPDX-License-Identifier: MPL-2.0
//! SYNC-014 (ADR-0137): the player saves its position every second and a
//! save is on disk when it returns (`synchronous = FULL` for progress
//! writes), so killing the app — or pulling the plug — loses at most the
//! second since the last save.
//!
//! The kill test runs a saver in a child process (this test binary again,
//! [`child_saver`]) and kills it with SIGKILL in the middle of its saves.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use pa_catalog::Catalog;
use pa_model::MediaKey;

const CHILD_ENV: &str = "PA_SYNC014_SAVER_DB";
/// The child's pace: faster than the player's second, same writes.
const TICK: Duration = Duration::from_millis(20);
/// Saves the parent waits for before the kill.
const SAVES: u64 = 25;
/// Position step of one save: one second of video.
const STEP_MS: i64 = 1_000;

fn vod() -> MediaKey {
    MediaKey::TwitchVod {
        video_id: "14000".to_owned(),
    }
}

/// The child process: saves a position one second further every [`TICK`]
/// and prints it after the save returned. Does nothing as a plain test.
#[test]
fn child_saver() {
    let Ok(db) = std::env::var(CHILD_ENV) else {
        return;
    };
    let cat = Catalog::open(db).unwrap();
    let mut out = std::io::stdout();
    let mut pos = 0;
    loop {
        pos += STEP_MS;
        cat.save_key_progress(&vod(), pos, 3_600_000, 1_763_000_000_000 + pos, false)
            .unwrap();
        writeln!(out, "saved {pos}").unwrap();
        out.flush().unwrap();
        std::thread::sleep(TICK);
    }
}

#[test]
fn kill_9_loses_at_most_the_save_in_flight() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    // The schema exists before the child starts saving.
    drop(Catalog::open(&db).unwrap());
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["child_saver", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, &db)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut last = 0;
    for line in BufReader::new(stdout).lines() {
        let line = line.unwrap();
        if let Some(pos) = line.strip_prefix("saved ") {
            last = pos.parse::<i64>().unwrap();
            if last >= i64::try_from(SAVES).unwrap() * STEP_MS {
                break;
            }
        }
    }
    // SIGKILL while the child keeps saving: maybe in the middle of a write.
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(last > 0, "the child saved nothing");

    let cat = Catalog::open(&db).unwrap();
    let stored = cat
        .resume_point_for_key(&vod())
        .unwrap()
        .expect("stored")
        .position_ms;
    let stored = i64::try_from(stored).unwrap();
    // Every save that returned is there; at most one more (in flight).
    assert!(stored >= last, "lost a returned save: {stored} < {last}");
    assert!(stored <= last + 2 * STEP_MS, "{stored} vs {last}");
}

/// A progress save leaves the connection on `synchronous = NORMAL`, so
/// scans after it do not pay for a sync per commit.
#[test]
fn a_save_keeps_bulk_writes_on_normal() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let cat = Catalog::open(&db).unwrap();
    cat.save_key_progress(&vod(), 1_000, 3_600_000, 1_763_000_000_000, false)
        .unwrap();
    let mut conn = pa_library::open(&db).unwrap();
    let level = |c: &pa_library::rusqlite::Connection| -> i64 {
        c.pragma_query_value(None, "synchronous", |r| r.get(0))
            .unwrap()
    };
    assert_eq!(level(&conn), 1, "NORMAL by default");
    let inside: i64 = pa_library::durably(&mut conn, |c| {
        Ok::<_, pa_library::rusqlite::Error>(level(c))
    })
    .unwrap();
    assert_eq!(inside, 2, "FULL inside");
    assert_eq!(level(&conn), 1, "NORMAL again");
}

/// What a durable save costs (ADR-0137): `cargo test -p pa-catalog --test
/// progress_durability -- --ignored --nocapture measure`.
#[test]
#[ignore = "a measurement, not a check"]
fn measure_progress_write_cost() {
    use pa_library::progress::{LocalOnly, PositionReport, Thresholds, report_position};
    use std::time::Instant;
    const N: usize = 300;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    drop(Catalog::open(&db).unwrap());
    let mut conn = pa_library::open(&db).unwrap();
    let mut sink = LocalOnly::new("local");
    let th = Thresholds::default();
    let mut run = |conn: &mut pa_library::rusqlite::Connection, full: bool, base: u64| {
        let mut times: Vec<Duration> = Vec::with_capacity(N);
        for i in 0..N as u64 {
            let report = PositionReport {
                profile_id: pa_library::profiles::DEFAULT_PROFILE,
                key: vod(),
                kind: pa_model::MediaKind::TwitchVod,
                position_ms: base + i * 1_000,
                duration_ms: Some(3_600_000),
            };
            let now = 1_763_000_000_000 + i64::try_from(i).unwrap();
            let t = Instant::now();
            if full {
                pa_library::durably(conn, |c| report_position(c, &mut sink, &th, &report, now))
                    .unwrap();
            } else {
                report_position(conn, &mut sink, &th, &report, now).unwrap();
            }
            times.push(t.elapsed());
        }
        times.sort();
        let avg = times.iter().sum::<Duration>() / u32::try_from(N).unwrap();
        (avg, times[N * 95 / 100], times[N - 1])
    };
    let normal = run(&mut conn, false, 0);
    let full = run(&mut conn, true, 1_000_000);
    println!(
        "NORMAL avg/p95/max: {:?} / {:?} / {:?}",
        normal.0, normal.1, normal.2
    );
    println!(
        "FULL   avg/p95/max: {:?} / {:?} / {:?}",
        full.0, full.1, full.2
    );
    let cat = Catalog::open(&db).unwrap();
    let t = Instant::now();
    for i in 0..N {
        let pos = i64::try_from(i).unwrap() * 1_000;
        cat.save_key_progress(&vod(), pos, 3_600_000, 1_764_000_000_000 + pos, false)
            .unwrap();
    }
    println!(
        "Catalog::save_key_progress avg: {:?}",
        t.elapsed() / u32::try_from(N).unwrap()
    );
}
