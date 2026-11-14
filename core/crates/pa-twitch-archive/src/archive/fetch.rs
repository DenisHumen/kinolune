// SPDX-License-Identifier: MPL-2.0
//! Parallel segment downloads with per-segment retries.

use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use super::source::{ArchiveError, ArchiveSource};

/// Fewest and most parallel segment requests (06 §4.1).
pub const MIN_WORKERS: usize = 4;
pub const MAX_WORKERS: usize = 8;

/// One file to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// Bit in the segment bitmap.
    pub index: usize,
    pub url: String,
    /// Final path; written as `<dest>.part` and renamed when complete.
    pub dest: PathBuf,
}

/// Retry policy of one segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Attempts per segment in one run (at least 1).
    pub attempts: u32,
    /// Delay after the first failure; doubles per attempt.
    pub base_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 4,
            base_delay: Duration::from_millis(500),
        }
    }
}

/// How a fetch pass ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Every task finished.
    Complete,
    /// `stop` returned true or `on_done` asked to stop.
    Stopped,
    /// A segment failed for good in this run.
    Failed(ArchiveError),
}

/// Writes `body` to `dest` through `<dest>.part` and a rename, so a file
/// at `dest` is always complete (a killed process leaves only `.part`).
pub fn write_atomic(dest: &Path, body: &[u8]) -> std::io::Result<()> {
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    {
        let mut f = fs::File::create(&part)?;
        f.write_all(body)?;
    }
    fs::rename(&part, dest)
}

fn content_length(headers: &[(String, String)]) -> Option<u64> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok())
}

/// Sleeps `d` in small steps; false if `abort` was set meanwhile.
fn pause(d: Duration, abort: &AtomicBool) -> bool {
    let until = Instant::now() + d;
    while Instant::now() < until {
        if abort.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50).min(until - Instant::now()));
    }
    !abort.load(Ordering::SeqCst)
}

/// Fetches one task with retries. `Ok(None)`: aborted.
fn fetch_one(
    source: &dyn ArchiveSource,
    task: &Task,
    policy: RetryPolicy,
    abort: &AtomicBool,
) -> Result<Option<u64>, ArchiveError> {
    let mut last = ArchiveError::retry("segment_failed", "not attempted");
    for attempt in 0..policy.attempts.max(1) {
        if attempt > 0 {
            let delay = policy.base_delay.saturating_mul(1 << (attempt - 1).min(16));
            if !pause(delay, abort) {
                return Ok(None);
            }
        }
        if abort.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let response = match source.get(&task.url) {
            Ok(r) => r,
            Err(e) if e.retryable => {
                last = e;
                continue;
            }
            Err(e) => return Err(e),
        };
        match response.status {
            200 => {
                if response.body.is_empty() {
                    last = ArchiveError::retry("segment_empty", format!("{} is empty", task.url));
                    continue;
                }
                if let Some(len) = content_length(&response.headers)
                    && len != response.body.len() as u64
                {
                    last = ArchiveError::retry(
                        "segment_truncated",
                        format!("got {} of {len} bytes", response.body.len()),
                    );
                    continue;
                }
                write_atomic(&task.dest, &response.body)
                    .map_err(|e| ArchiveError::retry("io", e.to_string()))?;
                return Ok(Some(response.body.len() as u64));
            }
            // The CDN refuses or forgot the segment: the playlist may be
            // stale (a new token, re-muted segments). A job retry resolves
            // the VOD again and continues from the bitmap.
            403 | 404 | 410 => {
                return Err(ArchiveError::retry(
                    "segment_gone",
                    format!("segment {} answered {}", task.index, response.status),
                ));
            }
            status => {
                last = ArchiveError::retry(
                    "segment_failed",
                    format!("segment {} answered {status}", task.index),
                );
            }
        }
    }
    Err(last)
}

/// Fetches `tasks` with `workers` threads (clamped to
/// [`MIN_WORKERS`]..=[`MAX_WORKERS`]). `on_done(index, bytes)` runs on the
/// calling thread after each finished file and returns false to stop;
/// `stop` is polled several times a second.
pub fn fetch_all(
    source: &dyn ArchiveSource,
    tasks: Vec<Task>,
    workers: usize,
    policy: RetryPolicy,
    stop: &dyn Fn() -> bool,
    on_done: &mut dyn FnMut(usize, u64) -> bool,
) -> Outcome {
    if tasks.is_empty() {
        return Outcome::Complete;
    }
    let workers = workers.clamp(MIN_WORKERS, MAX_WORKERS).min(tasks.len());
    let total = tasks.len();
    let queue = Mutex::new(tasks.into_iter().collect::<VecDeque<_>>());
    let abort = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel::<(usize, Result<Option<u64>, ArchiveError>)>();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let (queue, abort) = (&queue, &abort);
            scope.spawn(move || {
                loop {
                    if abort.load(Ordering::SeqCst) {
                        break;
                    }
                    let Some(task) = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front()
                    else {
                        break;
                    };
                    let result = fetch_one(source, &task, policy, abort);
                    if tx.send((task.index, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        let mut done = 0;
        let mut outcome = Outcome::Complete;
        loop {
            if stop() && outcome == Outcome::Complete {
                outcome = Outcome::Stopped;
                abort.store(true, Ordering::SeqCst);
            }
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok((index, Ok(Some(bytes)))) => {
                    done += 1;
                    if !on_done(index, bytes) && outcome == Outcome::Complete {
                        outcome = Outcome::Stopped;
                        abort.store(true, Ordering::SeqCst);
                    }
                }
                Ok((_, Ok(None))) => {}
                Ok((_, Err(e))) => {
                    if outcome == Outcome::Complete {
                        outcome = Outcome::Failed(e);
                    }
                    abort.store(true, Ordering::SeqCst);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if outcome == Outcome::Complete && done < total {
            outcome = Outcome::Stopped;
        }
        outcome
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pa_twitch::chapters::Chapter;
    use pa_twitch::playback::MasterPlaylist;
    use pa_twitch::{HttpResponse, Video};
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;

    /// Serves `n.ts` with body `n`; `flaky` fails the first request of a
    /// segment with 503, `gone` answers 404.
    struct Fake {
        calls: Mutex<HashMap<String, usize>>,
        flaky: Vec<usize>,
        gone: Vec<usize>,
        short: Vec<usize>,
        total: AtomicUsize,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                calls: Mutex::new(HashMap::new()),
                flaky: vec![],
                gone: vec![],
                short: vec![],
                total: AtomicUsize::new(0),
            }
        }
    }

    impl ArchiveSource for Fake {
        fn check_allowed(&self) -> Result<(), ArchiveError> {
            Ok(())
        }
        fn video(&self, _: &str) -> Result<Video, ArchiveError> {
            unreachable!()
        }
        fn avatar_url(&self, _: &str) -> Result<Option<String>, ArchiveError> {
            Ok(None)
        }
        fn chapters(&self, _: &Video) -> Vec<Chapter> {
            vec![]
        }
        fn master_playlist(&self, _: &str) -> Result<MasterPlaylist, ArchiveError> {
            unreachable!()
        }
        fn get(&self, url: &str) -> Result<HttpResponse, ArchiveError> {
            self.total.fetch_add(1, Ordering::SeqCst);
            let n: usize = url.trim_end_matches(".ts").parse().unwrap();
            let count = {
                let mut calls = self.calls.lock().unwrap();
                let c = calls.entry(url.to_owned()).or_default();
                *c += 1;
                *c
            };
            let body = format!("segment {n}").into_bytes();
            let mut headers = vec![("Content-Length".to_owned(), body.len().to_string())];
            let status = if self.gone.contains(&n) {
                404
            } else if self.flaky.contains(&n) && count == 1 {
                503
            } else {
                200
            };
            if self.short.contains(&n) && count == 1 {
                headers[0].1 = "999".to_owned();
            }
            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        }
    }

    fn tasks(dir: &Path, n: usize) -> Vec<Task> {
        (0..n)
            .map(|i| Task {
                index: i,
                url: format!("{i}.ts"),
                dest: dir.join(format!("{i:06}.ts")),
            })
            .collect()
    }

    const FAST: RetryPolicy = RetryPolicy {
        attempts: 3,
        base_delay: Duration::from_millis(1),
    };

    #[test]
    fn fetches_all_with_retries() {
        let dir = tempfile::tempdir().unwrap();
        let mut fake = Fake::new();
        fake.flaky = vec![3, 7];
        fake.short = vec![5];
        let mut seen = Vec::new();
        let out = fetch_all(
            &fake,
            tasks(dir.path(), 20),
            6,
            FAST,
            &|| false,
            &mut |i, b| {
                seen.push((i, b));
                true
            },
        );
        assert_eq!(out, Outcome::Complete);
        assert_eq!(seen.len(), 20);
        assert_eq!(fake.total.load(Ordering::SeqCst), 23);
        assert_eq!(
            fs::read(dir.path().join("000007.ts")).unwrap(),
            b"segment 7".to_vec()
        );
        assert!(!dir.path().join("000007.ts.part").exists());
    }

    #[test]
    fn a_gone_segment_fails_the_pass() {
        let dir = tempfile::tempdir().unwrap();
        let mut fake = Fake::new();
        fake.gone = vec![2];
        let out = fetch_all(
            &fake,
            tasks(dir.path(), 40),
            4,
            FAST,
            &|| false,
            &mut |_, _| true,
        );
        match out {
            Outcome::Failed(e) => assert_eq!((e.code, e.retryable), ("segment_gone", true)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stops_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        let mut n = 0;
        let out = fetch_all(
            &fake,
            tasks(dir.path(), 200),
            4,
            FAST,
            &|| false,
            &mut |_, _| {
                n += 1;
                n < 10
            },
        );
        assert_eq!(out, Outcome::Stopped);
        assert!(fake.total.load(Ordering::SeqCst) < 200);
    }
}
