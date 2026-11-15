// SPDX-License-Identifier: MPL-2.0
//! The ed2k background job (LIB-004, 03 §4.2).
//!
//! ed2k reads the whole file, so it never runs in the scan. A run
//! ([`Catalog::hash_ed2k`], or the [`ED2K_JOB`] job through
//! [`Ed2kHandler`]) hashes the live video files without `files.ed2k`,
//! smallest first, and decides per source by `sources.ed2k_policy`
//! ([`Ed2kPolicy`]) and the run's [`Ed2kTrigger`]:
//!
//! | policy   | `Background`   | `IdleUnmetered` | `OnDemand` |
//! |----------|----------------|-----------------|------------|
//! | `auto`   | local folders  | every source    | every source |
//! | `always` | every source   | every source    | every source |
//! | `never`  | —              | —               | every source |
//!
//! So by default local files are hashed in the background, network files
//! only when the app reports the device idle on Wi-Fi or the user asks.
//! Where background runs happen is the app's choice (the media server and
//! desktops; not phones on battery: [`ed2k_job`] sets the job conditions).
//!
//! A file that changes while it is read is not stored (the scan resets its
//! hashes); a file that cannot be read is retried a day later.
//!
//! LIB-023 (ADR-NNNN): ed2k never runs in the scan or the metadata run and
//! never holds them up. On a network source it reads at most
//! [`Ed2kOptions::network_bytes_per_sec`] (8 MiB/s by default), and a run
//! that stops keeps the MD4 of every whole chunk read
//! (`file_ed2k_progress`), so a 60 GB remux is hashed over several idle
//! periods instead of starting over each time.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pa_jobs::{JobContext, JobHandler, NetworkPolicy, NewJob, RunOutcome, priority};
use pa_library::fingerprints::{self, Ed2kCandidate, Ed2kPolicy};
use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_scanner::CancelToken;
use pa_vfs::{Vfs, VfsPath};
use serde::{Deserialize, Serialize};

use crate::{Catalog, Error, fingerprint, sources};

/// Job type of the ed2k job.
pub const ED2K_JOB: &str = "file_ed2k";

/// Files fetched from the database at once.
const BATCH: usize = 50;

/// Why a run happens (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ed2kTrigger {
    /// A regular background run: local folders, and network sources whose
    /// policy is `always`.
    #[default]
    Background,
    /// The app reports the device idle on an unmetered network (Wi-Fi,
    /// Ethernet): network sources with policy `auto` too.
    IdleUnmetered,
    /// The user asked: every source, policy `never` included.
    OnDemand,
}

impl Ed2kTrigger {
    fn allows(self, policy: Ed2kPolicy, local: bool) -> bool {
        match (self, policy) {
            (Ed2kTrigger::OnDemand, _) => true,
            (_, Ed2kPolicy::Never) => false,
            (_, Ed2kPolicy::Always) | (Ed2kTrigger::IdleUnmetered, Ed2kPolicy::Auto) => true,
            (Ed2kTrigger::Background, Ed2kPolicy::Auto) => local,
        }
    }
}

/// What an ed2k run hashes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ed2kOptions {
    pub trigger: Ed2kTrigger,
    /// Only this source (`None`: every source).
    pub source_id: Option<i64>,
    /// Stop after this many files (`None`: all).
    pub max_files: Option<usize>,
    /// Read speed limit on network sources, bytes per second (`None`: no
    /// limit). Local folders are never limited.
    #[serde(default = "default_network_rate")]
    pub network_bytes_per_sec: Option<u64>,
}

/// Default network read limit of the ed2k job: 8 MiB/s, a fraction of a
/// LAN and of Wi-Fi, so playback and scans keep the bandwidth.
pub const DEFAULT_NETWORK_RATE: u64 = 8 << 20;

#[allow(clippy::unnecessary_wraps)] // serde default of an Option field
fn default_network_rate() -> Option<u64> {
    Some(DEFAULT_NETWORK_RATE)
}

impl Default for Ed2kOptions {
    fn default() -> Self {
        Self {
            trigger: Ed2kTrigger::default(),
            source_id: None,
            max_files: None,
            network_bytes_per_sec: default_network_rate(),
        }
    }
}

/// Progress of an ed2k run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed2kProgress {
    pub source_id: i64,
    pub file_id: i64,
    pub path: String,
    /// Bytes of the current file read so far.
    pub bytes_done: u64,
    pub size_bytes: u64,
    /// Files finished in this run (hashed or failed).
    pub files_done: u64,
}

/// Outcome of an ed2k run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ed2kReport {
    /// Hashes stored.
    pub hashed: u64,
    /// Files that could not be read (retried later).
    pub failed: u64,
    /// Files that changed while they were read (hashed after the next scan).
    pub changed: u64,
    /// Sources that could not be reached.
    pub unavailable_sources: u64,
    /// The run was asked to stop before it finished.
    pub stopped: bool,
}

impl Catalog {
    /// Computes missing ed2k hashes (see the module docs). Runs on its own
    /// connection and does not block scans; `cancel` stops it between two
    /// reads (1 MiB).
    pub fn hash_ed2k(
        &self,
        options: &Ed2kOptions,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(&Ed2kProgress),
    ) -> Result<Ed2kReport, Error> {
        self.run_ed2k(options, &|| cancel.is_cancelled(), on_progress)
    }

    /// Sets the ed2k policy of a source.
    pub fn set_ed2k_policy(&self, source_id: i64, policy: Ed2kPolicy) -> Result<(), Error> {
        if fingerprints::set_ed2k_policy(&self.lock(), source_id, policy)? {
            Ok(())
        } else {
            Err(Error::NotFound(format!("source {source_id}")))
        }
    }

    /// The ed2k policy of a source.
    pub fn ed2k_policy(&self, source_id: i64) -> Result<Ed2kPolicy, Error> {
        fingerprints::ed2k_policy(&self.lock(), source_id)?
            .ok_or_else(|| Error::NotFound(format!("source {source_id}")))
    }

    fn run_ed2k(
        &self,
        options: &Ed2kOptions,
        stop: &dyn Fn() -> bool,
        on_progress: &mut dyn FnMut(&Ed2kProgress),
    ) -> Result<Ed2kReport, Error> {
        let conn = pa_library::open(&self.path)?;
        let mut report = Ed2kReport::default();
        let mut budget = options.max_files.unwrap_or(usize::MAX);
        let all = sources::list(&conn)?;
        for src in all
            .iter()
            .filter(|s| options.source_id.is_none_or(|id| id == s.id))
        {
            let policy = fingerprints::ed2k_policy(&conn, src.id)?.unwrap_or_default();
            if budget == 0 || !options.trigger.allows(policy, src.local_root.is_some()) {
                continue;
            }
            // Nothing to do: do not wake a network share up.
            let now = crate::now_ms();
            if fingerprints::ed2k_candidates(&conn, src.id, now, 1)?.is_empty() {
                continue;
            }
            let vfs = match self.connector().connect(src) {
                Ok(v) => v,
                Err(_) => {
                    report.unavailable_sources += 1;
                    continue;
                }
            };
            let rate = options
                .network_bytes_per_sec
                .filter(|r| *r > 0 && src.local_root.is_none());
            let mut tried = HashSet::new();
            'source: loop {
                let now = crate::now_ms();
                let batch: Vec<Ed2kCandidate> =
                    fingerprints::ed2k_candidates(&conn, src.id, now, tried.len() + BATCH)?
                        .into_iter()
                        .filter(|c| !tried.contains(&c.file_id))
                        .collect();
                if batch.is_empty() {
                    break;
                }
                for file in batch {
                    if budget == 0 {
                        break 'source;
                    }
                    if stop() {
                        report.stopped = true;
                        return Ok(report);
                    }
                    tried.insert(file.file_id);
                    budget -= 1;
                    let mut progress = Ed2kProgress {
                        source_id: src.id,
                        file_id: file.file_id,
                        path: file.path.clone(),
                        bytes_done: 0,
                        size_bytes: file.size_bytes,
                        files_done: report.hashed + report.failed + report.changed,
                    };
                    on_progress(&progress);
                    let mut pacer = Pacer::new(rate);
                    let hash = hash_one(&conn, vfs.as_ref(), &file, stop, &mut |done| {
                        pacer.pace(done, stop);
                        progress.bytes_done = done;
                        on_progress(&progress);
                    });
                    match hash {
                        Ok(None) => {
                            report.stopped = true;
                            return Ok(report);
                        }
                        Ok(Some(h)) if h.size == file.size_bytes => {
                            if fingerprints::set_ed2k(&conn, &file, &h.hash.hash)? {
                                report.hashed += 1;
                            } else {
                                report.changed += 1;
                            }
                        }
                        Ok(Some(_)) => report.changed += 1,
                        Err(_) => {
                            fingerprints::mark_ed2k_failed(&conn, file.file_id, crate::now_ms())?;
                            report.failed += 1;
                        }
                    }
                }
            }
        }
        Ok(report)
    }
}

struct Hashed {
    hash: fingerprint::Ed2kHash,
    size: u64,
}

/// Keeps reads under a rate: sleeps (in short steps, watching `stop`)
/// whenever the bytes read run ahead of the rate since the start.
struct Pacer {
    rate: Option<u64>,
    start: Instant,
    from: Option<u64>,
}

impl Pacer {
    fn new(rate: Option<u64>) -> Self {
        Self {
            rate,
            start: Instant::now(),
            from: None,
        }
    }

    fn pace(&mut self, done: u64, stop: &dyn Fn() -> bool) {
        let Some(rate) = self.rate else { return };
        // A resumed file starts counting where it resumed.
        let from = *self.from.get_or_insert(done);
        let due = Duration::from_secs_f64((done - from) as f64 / rate as f64);
        while self.start.elapsed() < due && !stop() {
            std::thread::sleep((due - self.start.elapsed()).min(Duration::from_millis(200)));
        }
    }
}

/// The saved chunk MD4s of `file`, if they are of this size and mtime.
fn saved_chunks(conn: &Connection, file: &Ed2kCandidate) -> Result<Vec<[u8; 16]>, Error> {
    let blob: Option<Vec<u8>> = conn
        .query_row(
            "SELECT chunk_md4 FROM file_ed2k_progress
             WHERE file_id = ?1 AND size_bytes = ?2 AND mtime_ms IS ?3",
            params![
                file.file_id,
                i64::try_from(file.size_bytes).unwrap_or(i64::MAX),
                file.mtime_ms
            ],
            |r| r.get(0),
        )
        .optional()?;
    Ok(blob
        .unwrap_or_default()
        .chunks_exact(16)
        .map(|c| c.try_into().unwrap_or([0; 16]))
        .collect())
}

fn save_chunks(conn: &Connection, file: &Ed2kCandidate, chunks: &[[u8; 16]]) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO file_ed2k_progress (file_id, size_bytes, mtime_ms, chunk_md4, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (file_id) DO UPDATE SET size_bytes = excluded.size_bytes,
             mtime_ms = excluded.mtime_ms, chunk_md4 = excluded.chunk_md4,
             updated_at_ms = excluded.updated_at_ms",
        params![
            file.file_id,
            i64::try_from(file.size_bytes).unwrap_or(i64::MAX),
            file.mtime_ms,
            chunks.concat(),
            crate::now_ms()
        ],
    )?;
    Ok(())
}

/// Chunks between two progress writes (~76 MB).
const SAVE_EVERY: usize = 8;

fn hash_one(
    conn: &Connection,
    vfs: &dyn Vfs,
    file: &Ed2kCandidate,
    stop: &dyn Fn() -> bool,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<Option<Hashed>, Error> {
    let source = vfs.open(&VfsPath::parse(&file.path)?)?;
    let size = source.size();
    let saved = saved_chunks(conn, file)?;
    let mut last_saved = saved.len();
    let mut save_error = None;
    let step = fingerprint::ed2k_resume(
        &*source,
        saved,
        stop,
        &mut |chunks| {
            if chunks.len() >= last_saved + SAVE_EVERY {
                last_saved = chunks.len();
                if let Err(e) = save_chunks(conn, file, chunks) {
                    save_error.get_or_insert(e);
                }
            }
        },
        on_bytes,
    )?;
    if let Some(e) = save_error {
        return Err(e);
    }
    match step {
        fingerprint::Ed2kStep::Done(hash) => {
            conn.execute(
                "DELETE FROM file_ed2k_progress WHERE file_id = ?1",
                [file.file_id],
            )?;
            Ok(Some(Hashed { hash, size }))
        }
        fingerprint::Ed2kStep::Stopped(chunks) => {
            if !chunks.is_empty() && size == file.size_bytes {
                save_chunks(conn, file, &chunks)?;
            }
            Ok(None)
        }
    }
}

/// A job that runs [`Catalog::hash_ed2k`] with `options`, with conditions
/// that fit its trigger: a background run waits for mains power; an idle
/// run also for an unmetered network; an on-demand run is what the user
/// waits for and runs on any network and power.
pub fn ed2k_job(options: &Ed2kOptions) -> NewJob {
    let params = serde_json::to_value(options).unwrap_or_else(|_| serde_json::json!({}));
    let job = NewJob::new(ED2K_JOB, params).max_attempts(3);
    match options.trigger {
        Ed2kTrigger::Background => job.allow_on_battery(false),
        Ed2kTrigger::IdleUnmetered => job
            .allow_on_battery(false)
            .network(NetworkPolicy::Unmetered),
        Ed2kTrigger::OnDemand => job
            .priority(priority::USER_WAITING)
            .network(NetworkPolicy::Any),
    }
}

/// Runs [`ED2K_JOB`] jobs on a catalog. A paused or cancelled job stops
/// within one read; the next run continues with the files still without a
/// hash (every hash is stored as soon as it is computed).
pub struct Ed2kHandler {
    catalog: Arc<Catalog>,
}

impl Ed2kHandler {
    pub fn new(catalog: Arc<Catalog>) -> Self {
        Self { catalog }
    }

    /// Registers the handler for [`ED2K_JOB`] on `queue`.
    pub fn register(self, queue: &pa_jobs::JobQueue) {
        queue.register(ED2K_JOB, self);
    }
}

impl JobHandler for Ed2kHandler {
    fn run(&self, ctx: &JobContext) -> RunOutcome {
        let options: Ed2kOptions = match serde_json::from_value(ctx.params().clone()) {
            Ok(o) => o,
            Err(e) => return RunOutcome::Fatal(format!("bad_params: {e}")),
        };
        let result = self
            .catalog
            .run_ed2k(&options, &|| ctx.stop_requested(), &mut |p| {
                ctx.report_bytes(p.bytes_done, Some(p.size_bytes));
            });
        match result {
            Ok(r) if r.stopped => RunOutcome::Interrupted,
            Ok(r) => RunOutcome::Done(serde_json::to_value(r).ok()),
            Err(e) => RunOutcome::Retryable(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_and_policy() {
        use Ed2kPolicy::*;
        use Ed2kTrigger::*;
        assert!(Background.allows(Auto, true));
        assert!(!Background.allows(Auto, false));
        assert!(Background.allows(Always, false));
        assert!(!Background.allows(Never, true));
        assert!(IdleUnmetered.allows(Auto, false));
        assert!(!IdleUnmetered.allows(Never, false));
        assert!(OnDemand.allows(Never, false));
    }

    #[test]
    fn job_conditions() {
        let bg = ed2k_job(&Ed2kOptions::default());
        assert_eq!(bg.kind, ED2K_JOB);
        assert!(!bg.allow_on_battery);
        assert_eq!(bg.network, NetworkPolicy::None);
        let idle = ed2k_job(&Ed2kOptions {
            trigger: Ed2kTrigger::IdleUnmetered,
            ..Ed2kOptions::default()
        });
        assert_eq!(idle.network, NetworkPolicy::Unmetered);
        let demand = ed2k_job(&Ed2kOptions {
            trigger: Ed2kTrigger::OnDemand,
            source_id: Some(3),
            ..Ed2kOptions::default()
        });
        assert!(demand.allow_on_battery);
        assert_eq!(demand.priority, priority::USER_WAITING);
        let back: Ed2kOptions = serde_json::from_value(demand.params).unwrap();
        assert_eq!(back.source_id, Some(3));
        // Jobs queued before LIB-023 get the network limit.
        let old: Ed2kOptions = serde_json::from_value(serde_json::json!({"trigger": "idle_unmetered"})).unwrap();
        assert_eq!(old.network_bytes_per_sec, Some(DEFAULT_NETWORK_RATE));
        assert_eq!(back.trigger, Ed2kTrigger::OnDemand);
    }
}
