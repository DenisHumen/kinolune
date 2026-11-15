// SPDX-License-Identifier: MPL-2.0
//! [`StorageManager`]: targets, admission gate, monitor, items, trash and
//! retention.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pa_jobs::{
    Admission, Clock, Job, JobContext, JobId, JobQueue, JobState, NewJob, PauseReason,
    ResourceGate, RunOutcome, kinds, priority, serde_json::json, system_now_ms,
};
use pa_library::rusqlite::{Connection, OptionalExtension, Row, params};

use crate::fs::{FsStats, SystemFs, dir_size, move_atomic};
use crate::{
    Error, Expiry, ExpiryReason, Fit, Item, NewItem, RetentionReport, RetentionRule, Target, Usage,
    wait,
};

/// Folder of unfinished files inside a target.
pub const TMP_DIR: &str = ".tmp";
/// Folder of trashed items inside a target.
pub const TRASH_DIR: &str = ".trash";

const DAY_MS: i64 = 86_400_000;

/// Settings of a [`StorageManager`].
#[derive(Clone)]
pub struct StorageConfig {
    pub fs: Arc<dyn FsStats>,
    pub clock: Clock,
    /// How long trashed items are kept before they are deleted for good.
    pub trash_period: Duration,
    /// Where live recordings stop (TWI-018): the hard reserve of a target.
    pub live: LiveReserve,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            fs: Arc::new(SystemFs),
            clock: Arc::new(system_now_ms),
            trash_period: Duration::from_secs(7 * 24 * 3600),
            live: LiveReserve::default(),
        }
    }
}

impl fmt::Debug for StorageConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorageConfig")
            .field("trash_period", &self.trash_period)
            .field("live", &self.live)
            .finish_non_exhaustive()
    }
}

/// The hard reserve of live recordings (jobs at or above
/// [`priority::LIVE`], TWI-018). A broadcast that is not recorded now is
/// lost, so a live recording keeps writing below the reserve of its
/// target, down to `min(reserve × share, max_bytes)` of free space, and
/// past its quota by the same amount. Other jobs stop at the reserve and
/// the quota as before.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveReserve {
    /// Share of the target's reserve, [0, 1].
    pub share: f64,
    /// Upper bound of the hard reserve, bytes.
    pub max_bytes: u64,
}

impl Default for LiveReserve {
    fn default() -> Self {
        Self {
            share: 0.25,
            max_bytes: 2 << 30,
        }
    }
}

impl LiveReserve {
    /// The hard reserve for a (soft) reserve of `reserve` bytes.
    pub fn of(&self, reserve: u64) -> u64 {
        // f64 is exact enough for a reserve; saturates on overflow.
        let share = (reserve as f64 * self.share.clamp(0.0, 1.0)) as u64;
        share.min(self.max_bytes).min(reserve)
    }
}

/// Whether `job` is a live recording (TWI-018).
fn is_live(job: &Job) -> bool {
    job.priority >= priority::LIVE
}

/// Bytes an admitted job still writes (`remaining`) and its expected size
/// (`full`), until the monitor sees it finished or its file is committed.
#[derive(Debug, Clone)]
struct Reservation {
    target: String,
    remaining: u64,
    full: u64,
}

#[derive(Debug, Default)]
struct Cache {
    targets: HashMap<String, Target>,
    /// Bytes of live items per target.
    used: HashMap<String, u64>,
}

struct Inner {
    conn: Mutex<Connection>,
    fs: Arc<dyn FsStats>,
    clock: Clock,
    trash_ms: i64,
    live: LiveReserve,
    cache: RwLock<Cache>,
    reservations: Mutex<HashMap<JobId, Reservation>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn to_i64(v: u64) -> Result<i64, Error> {
    i64::try_from(v).map_err(|_| Error::Invalid("size"))
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

const ITEM_COLUMNS: &str = "id, target_id, rel_path, group_key, size_bytes, profile_id, \
     favorite, watched_at_ms, created_at_ms, trashed_at_ms";

fn item_from_row(r: &Row<'_>) -> pa_library::rusqlite::Result<Item> {
    Ok(Item {
        id: r.get(0)?,
        target_id: r.get(1)?,
        rel_path: r.get(2)?,
        group_key: r.get(3)?,
        size_bytes: to_u64(r.get(4)?),
        profile_id: r.get(5)?,
        favorite: r.get(6)?,
        watched_at_ms: r.get(7)?,
        created_at_ms: r.get(8)?,
        trashed_at_ms: r.get(9)?,
    })
}

impl Inner {
    fn now(&self) -> i64 {
        (self.clock)()
    }

    /// Reloads targets and used bytes from the database.
    fn reload(&self, conn: &Connection) -> Result<(), Error> {
        let mut cache = Cache::default();
        let mut stmt = conn.prepare(
            "SELECT id, kind, path, quota_bytes, quota_percent, reserve_bytes, reserve_percent \
             FROM storage_targets",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<f64>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, f64>(6)?,
            ))
        })?;
        for row in rows {
            let (id, kind, path, quota_bytes, quota_percent, reserve_bytes, reserve_percent) = row?;
            let kind = kind.parse().map_err(|_| Error::Invalid("storage kind"))?;
            let target = Target {
                id: id.clone(),
                kind,
                path: PathBuf::from(path),
                quota_bytes: quota_bytes.map(to_u64),
                quota_percent,
                reserve_bytes: to_u64(reserve_bytes),
                reserve_percent,
            };
            cache.targets.insert(id, target);
        }
        let mut stmt = conn.prepare(
            "SELECT target_id, sum(size_bytes) FROM storage_items \
             WHERE trashed_at_ms IS NULL GROUP BY target_id",
        )?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (id, used) = row?;
            cache.used.insert(id, to_u64(used));
        }
        *self.cache.write().unwrap_or_else(|e| e.into_inner()) = cache;
        Ok(())
    }

    fn target(&self, id: &str) -> Result<Target, Error> {
        self.cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .targets
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NoTarget(id.to_owned()))
    }

    /// (remaining, full) bytes of admitted jobs on `target`, except `skip`.
    fn pending(&self, target: &str, skip: Option<JobId>) -> (u64, u64) {
        lock(&self.reservations)
            .iter()
            .filter(|(id, r)| r.target == target && Some(**id) != skip)
            .fold((0u64, 0u64), |(a, b), (_, r)| {
                (a.saturating_add(r.remaining), b.saturating_add(r.full))
            })
    }

    /// Does a job that still writes `remaining` bytes of `full` fit?
    fn fit(&self, target_id: &str, remaining: u64, full: u64, skip: Option<JobId>) -> Fit {
        let (target, used) = {
            let cache = self.cache.read().unwrap_or_else(|e| e.into_inner());
            let Some(target) = cache.targets.get(target_id) else {
                return Fit::Never(wait::NO_TARGET);
            };
            (
                target.clone(),
                cache.used.get(target_id).copied().unwrap_or(0),
            )
        };
        let Ok(space) = self.fs.space(&target.path) else {
            return Fit::NotNow(wait::DISK_UNAVAILABLE);
        };
        let reserve = target.reserve(space.total);
        let quota = target.quota(space.total);
        if full > space.total.saturating_sub(reserve) || quota.is_some_and(|q| full > q) {
            return Fit::Never(wait::TOO_LARGE);
        }
        let (pending_remaining, pending_full) = self.pending(target_id, skip);
        if space.available.saturating_sub(pending_remaining) < remaining.saturating_add(reserve) {
            return Fit::NotNow(wait::DISK_RESERVE);
        }
        if quota.is_some_and(|q| used.saturating_add(pending_full).saturating_add(full) > q) {
            return Fit::NotNow(wait::QUOTA);
        }
        Fit::Fits
    }

    /// Admission of a live recording: free space above the hard reserve
    /// and room in the quota plus the same margin. Reservations of other
    /// jobs do not hold it back (the monitor pauses them instead), and its
    /// size is open-ended, so it is never "too large".
    fn fit_live(&self, target_id: &str) -> Fit {
        let (target, used) = {
            let cache = self.cache.read().unwrap_or_else(|e| e.into_inner());
            let Some(target) = cache.targets.get(target_id) else {
                return Fit::Never(wait::NO_TARGET);
            };
            (
                target.clone(),
                cache.used.get(target_id).copied().unwrap_or(0),
            )
        };
        let Ok(space) = self.fs.space(&target.path) else {
            return Fit::NotNow(wait::DISK_UNAVAILABLE);
        };
        let hard = self.live.of(target.reserve(space.total));
        if space.available <= hard {
            return Fit::NotNow(wait::DISK_RESERVE);
        }
        if target
            .quota(space.total)
            .is_some_and(|q| used >= q.saturating_add(hard))
        {
            return Fit::NotNow(wait::QUOTA);
        }
        Fit::Fits
    }
}

/// The admission check of the job queue.
struct StorageGate(Arc<Inner>);

impl ResourceGate for StorageGate {
    fn admit(&self, job: &Job) -> Admission {
        let Some(target) = job.target_storage.as_deref() else {
            return Admission::Admit;
        };
        let full = job.bytes_total.unwrap_or(0).max(job.bytes_done);
        let remaining = full - job.bytes_done;
        let fit = if is_live(job) {
            self.0.fit_live(target)
        } else {
            self.0.fit(target, remaining, full, Some(job.id))
        };
        match fit {
            Fit::Fits => {
                lock(&self.0.reservations).insert(
                    job.id,
                    Reservation {
                        target: target.to_owned(),
                        remaining,
                        full,
                    },
                );
                Admission::Admit
            }
            Fit::NotNow(reason) | Fit::Never(reason) => Admission::Wait(reason.to_owned()),
        }
    }
}

/// Result of one [`StorageManager::check`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MonitorReport {
    pub usage: Vec<Usage>,
    /// Jobs paused by this check (free space below the reserve or quota
    /// exceeded): the app shows a notification. Live recordings among them
    /// are also in [`live_paused`](Self::live_paused).
    pub paused: Vec<JobId>,
    /// Live recordings paused by this check: free space reached the hard
    /// reserve even after the trash was emptied and the other jobs of the
    /// target were paused. The recording has a gap from now on.
    pub live_paused: Vec<JobId>,
    /// Trashed items deleted for good to keep a live recording going.
    pub purged: Vec<i64>,
    /// Bytes freed by that.
    pub purged_bytes: u64,
}

/// Background monitor of [`StorageManager::start_monitor`]; stops when
/// dropped.
pub struct MonitorDriver {
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for MonitorDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MonitorDriver").finish_non_exhaustive()
    }
}

impl Drop for MonitorDriver {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Storage targets, items and retention of one library database.
#[derive(Clone)]
pub struct StorageManager {
    inner: Arc<Inner>,
}

impl fmt::Debug for StorageManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorageManager").finish_non_exhaustive()
    }
}

/// Checks a path relative to a target: plain components only, not inside
/// `.tmp/` or `.trash/`.
fn check_rel_path(rel: &str) -> Result<&Path, Error> {
    let path = Path::new(rel);
    let mut components = path.components().peekable();
    let first = components.peek().copied();
    let plain = components.all(|c| matches!(c, Component::Normal(_)));
    let reserved = matches!(first, Some(Component::Normal(n)) if n == TMP_DIR || n == TRASH_DIR);
    if rel.is_empty() || !plain || reserved {
        return Err(Error::Invalid("relative path"));
    }
    Ok(path)
}

fn remove_any(path: &Path) -> io::Result<()> {
    let result = match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(e) => Err(e),
    };
    match result {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

impl StorageManager {
    /// Opens the library database at `path`.
    pub fn open(path: impl AsRef<Path>, config: StorageConfig) -> Result<Self, Error> {
        Self::new(pa_library::open(path)?, config)
    }

    /// Wraps a migrated library connection.
    pub fn new(conn: Connection, config: StorageConfig) -> Result<Self, Error> {
        let inner = Inner {
            conn: Mutex::new(conn),
            fs: config.fs,
            clock: config.clock,
            trash_ms: i64::try_from(config.trash_period.as_millis()).unwrap_or(i64::MAX),
            live: config.live,
            cache: RwLock::new(Cache::default()),
            reservations: Mutex::new(HashMap::new()),
        };
        inner.reload(&lock(&inner.conn))?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Wires the manager into a queue: the admission gate and the
    /// `cleanup` handler.
    pub fn attach(&self, queue: &JobQueue) {
        queue.set_gate(Some(self.gate()));
        self.register_cleanup(queue);
    }

    // --- Targets -------------------------------------------------------

    /// Adds or replaces a target. Creates its folder if possible (a share
    /// that is offline now is saved anyway).
    pub fn set_target(&self, target: &Target) -> Result<(), Error> {
        let valid_id = !target.id.is_empty()
            && target
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
        if !valid_id {
            return Err(Error::Invalid("storage target id"));
        }
        if !target.path.is_absolute() {
            return Err(Error::Invalid("storage target path"));
        }
        let path = target
            .path
            .to_str()
            .ok_or(Error::Invalid("storage target path"))?;
        let _ = fs::create_dir_all(&target.path);
        let now = self.inner.now();
        let conn = lock(&self.inner.conn);
        conn.execute(
            "INSERT INTO storage_targets (id, kind, path, quota_bytes, quota_percent, \
                 reserve_bytes, reserve_percent, created_at_ms, updated_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8) \
             ON CONFLICT (id) DO UPDATE SET kind = excluded.kind, path = excluded.path, \
                 quota_bytes = excluded.quota_bytes, quota_percent = excluded.quota_percent, \
                 reserve_bytes = excluded.reserve_bytes, \
                 reserve_percent = excluded.reserve_percent, updated_at_ms = excluded.updated_at_ms",
            params![
                target.id,
                target.kind.as_str(),
                path,
                target.quota_bytes.map(to_i64).transpose()?,
                target.quota_percent,
                to_i64(target.reserve_bytes)?,
                target.reserve_percent,
                now
            ],
        )?;
        self.inner.reload(&conn)
    }

    /// Removes a target with its items and rules from the database. Files
    /// stay on disk. Returns false if there was no such target.
    pub fn remove_target(&self, id: &str) -> Result<bool, Error> {
        let conn = lock(&self.inner.conn);
        let n = conn.execute("DELETE FROM storage_targets WHERE id = ?1", [id])?;
        self.inner.reload(&conn)?;
        Ok(n > 0)
    }

    pub fn target(&self, id: &str) -> Result<Target, Error> {
        self.inner.target(id)
    }

    /// All targets, by id.
    pub fn targets(&self) -> Vec<Target> {
        let cache = self.inner.cache.read().unwrap_or_else(|e| e.into_inner());
        let mut targets: Vec<Target> = cache.targets.values().cloned().collect();
        targets.sort_by(|a, b| a.id.cmp(&b.id));
        targets
    }

    /// Space of every target now.
    pub fn usage(&self) -> Result<Vec<Usage>, Error> {
        let trash: HashMap<String, u64> = {
            let conn = lock(&self.inner.conn);
            let mut stmt = conn.prepare(
                "SELECT target_id, sum(size_bytes) FROM storage_items \
                 WHERE trashed_at_ms IS NOT NULL GROUP BY target_id",
            )?;
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
                .map(|r| r.map(|(id, n)| (id, to_u64(n))))
                .collect::<Result<_, _>>()?
        };
        let used = self
            .inner
            .cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .used
            .clone();
        Ok(self
            .targets()
            .into_iter()
            .map(|t| {
                let space = self.inner.fs.space(&t.path).ok();
                let reserve = space.map_or(t.reserve_bytes, |s| t.reserve(s.total));
                Usage {
                    total: space.map(|s| s.total),
                    available: space.map(|s| s.available),
                    reserve,
                    hard_reserve: self.inner.live.of(reserve),
                    quota: space.map_or(t.quota_bytes, |s| t.quota(s.total)),
                    used: used.get(&t.id).copied().unwrap_or(0),
                    trash: trash.get(&t.id).copied().unwrap_or(0),
                    pending: self.inner.pending(&t.id, None).0,
                    target_id: t.id,
                }
            })
            .collect())
    }

    // --- Admission and monitoring -----------------------------------------

    /// Does a write of `bytes` into `target_id` fit now?
    pub fn check_fit(&self, target_id: &str, bytes: u64) -> Fit {
        self.inner.fit(target_id, bytes, bytes, None)
    }

    /// The admission gate for [`JobQueue::set_gate`]: a job with
    /// `target_storage` starts only if its estimate (`bytes_total`, what is
    /// not written yet) fits; otherwise it waits with a [`wait`] reason.
    pub fn gate(&self) -> Arc<dyn ResourceGate> {
        Arc::new(StorageGate(Arc::clone(&self.inner)))
    }

    /// Enqueues a job, refusing it if it can never fit into its target
    /// (larger than the quota or the volume, unknown target). A job that
    /// does not fit now is queued and waits for space.
    pub fn enqueue(&self, queue: &JobQueue, job: &NewJob) -> Result<JobId, Error> {
        let fit = |target: &str| {
            if job.priority >= priority::LIVE {
                self.inner.fit_live(target)
            } else {
                self.check_fit(target, job.bytes_total.unwrap_or(0))
            }
        };
        if let Some(target) = &job.target_storage
            && let Fit::Never(reason) = fit(target)
        {
            return Err(Error::WontFit {
                target: target.clone(),
                reason,
            });
        }
        Ok(queue.enqueue(job)?)
    }

    /// One monitoring pass: forgets reservations of jobs that stopped,
    /// then, for every target whose free space is below the reserve, whose
    /// volume is gone, or whose kept items plus written bytes exceed the
    /// quota:
    ///
    /// 1. if a live recording (TWI-018) runs or waits there and free space
    ///    is short, empties the target's trash ([`purge_trash_of`]): a
    ///    broadcast that is not recorded now is lost, a trashed file was
    ///    already given up (the owner's decision, ADR-0062 / TWI-018);
    /// 2. if that was not enough, pauses ([`PauseReason::Resources`]) the
    ///    other running jobs of the target;
    /// 3. pauses live recordings only when free space reaches the hard
    ///    reserve ([`LiveReserve`]), the quota is exceeded by more than it,
    ///    or the volume is gone. The recorder records the gap.
    ///
    /// The queue moves paused jobs to `waiting_resources`; they continue
    /// once the gate admits them again.
    ///
    /// [`purge_trash_of`]: Self::purge_trash_of
    pub fn check(&self, queue: &JobQueue) -> Result<MonitorReport, Error> {
        let running = queue.list(&[JobState::Running])?;
        let waiting = queue.list(&[JobState::WaitingResources])?;
        {
            let mut reservations = lock(&self.inner.reservations);
            reservations.retain(|id, _| running.iter().any(|j| j.id == *id));
            for job in &running {
                if let Some(r) = reservations.get_mut(&job.id) {
                    r.full = r.full.max(job.bytes_done);
                    r.remaining = r.full - job.bytes_done;
                }
            }
        }
        let mut usage = self.usage()?;
        let mut report = MonitorReport::default();
        for u in &mut usage {
            let target = u.target_id.clone();
            let on_target = |j: &&Job| j.target_storage.as_deref() == Some(target.as_str());
            let jobs: Vec<&Job> = running.iter().filter(on_target).collect();
            let live_here =
                jobs.iter().any(|j| is_live(j)) || waiting.iter().filter(on_target).any(is_live);
            let written = jobs.iter().map(|j| j.bytes_done).sum::<u64>();
            let over_quota = |u: &Usage, margin: u64| {
                u.quota
                    .is_some_and(|q| u.used.saturating_add(written) > q.saturating_add(margin))
            };
            if !(u.below_reserve() || over_quota(u, 0)) {
                continue;
            }
            // 1. The trash goes first when a live recording needs the space
            //    (the trash takes physical space, not quota).
            if live_here && u.below_reserve() && u.available.is_some() && u.trash > 0 {
                let purged = self.purge_trash_of(&target)?;
                if !purged.purged.is_empty() {
                    report.purged.extend(&purged.purged);
                    report.purged_bytes = report.purged_bytes.saturating_add(purged.purged_bytes);
                    if let Some(fresh) = self.usage()?.into_iter().find(|f| f.target_id == target) {
                        *u = fresh;
                    }
                }
                if !(u.below_reserve() || over_quota(u, 0)) {
                    continue;
                }
            }
            // 2. Everything but live recordings stops at the reserve.
            for job in jobs
                .iter()
                .filter(|j| !is_live(j) && j.pause_reason.is_none())
            {
                // A job that finished meanwhile cannot be paused; skip it.
                if queue.pause_with(job.id, PauseReason::Resources).is_ok() {
                    report.paused.push(job.id);
                }
            }
            // 3. Live recordings stop only at the hard reserve.
            if !(u.below_hard_reserve() || over_quota(u, u.hard_reserve)) {
                continue;
            }
            for job in jobs
                .iter()
                .filter(|j| is_live(j) && j.pause_reason.is_none())
            {
                if queue.pause_with(job.id, PauseReason::Resources).is_ok() {
                    report.paused.push(job.id);
                    report.live_paused.push(job.id);
                }
            }
        }
        report.usage = usage;
        Ok(report)
    }

    /// Runs [`check`](Self::check) every `interval` on a thread and passes
    /// each report to `on_report` (notifications, dashboard).
    pub fn start_monitor(
        &self,
        queue: JobQueue,
        interval: Duration,
        on_report: impl Fn(&MonitorReport) + Send + 'static,
    ) -> MonitorDriver {
        let (tx, rx) = mpsc::channel::<()>();
        let manager = self.clone();
        let thread = thread::Builder::new()
            .name("pa-storage-monitor".into())
            .spawn(move || {
                loop {
                    if let Ok(report) = manager.check(&queue) {
                        on_report(&report);
                    }
                    match rx.recv_timeout(interval) {
                        Err(RecvTimeoutError::Timeout) => {}
                        _ => break,
                    }
                }
            })
            .ok();
        MonitorDriver {
            stop: Some(tx),
            thread,
        }
    }

    // --- Files ---------------------------------------------------------

    /// The temporary folder of `job` in a target (`.tmp/job-<id>/`),
    /// created. It survives restarts while the job exists.
    pub fn job_tmp_dir(&self, target_id: &str, job: JobId) -> Result<PathBuf, Error> {
        let dir = self
            .target(target_id)?
            .path
            .join(TMP_DIR)
            .join(format!("job-{job}"));
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Startup cleanup: deletes everything in the targets' `.tmp/` except
    /// the folders of jobs that are not finished. Returns how many entries
    /// were removed. Targets that are not available are skipped.
    pub fn clean_tmp(&self, queue: &JobQueue) -> Result<usize, Error> {
        let live: HashSet<JobId> = queue
            .list(&[
                JobState::Queued,
                JobState::WaitingResources,
                JobState::Running,
                JobState::Paused,
                JobState::Retrying,
            ])?
            .into_iter()
            .map(|j| j.id)
            .collect();
        self.clean_tmp_except(&live)
    }

    /// [`clean_tmp`](Self::clean_tmp) with an explicit set of jobs to keep.
    pub fn clean_tmp_except(&self, keep: &HashSet<JobId>) -> Result<usize, Error> {
        let mut removed = 0;
        for target in self.targets() {
            let Ok(entries) = fs::read_dir(target.path.join(TMP_DIR)) else {
                continue;
            };
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name();
                let kept = name
                    .to_str()
                    .and_then(|n| n.strip_prefix("job-"))
                    .and_then(|id| id.parse::<JobId>().ok())
                    .is_some_and(|id| keep.contains(&id));
                if !kept {
                    remove_any(&entry.path())?;
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    /// Moves a finished file (or folder) `src` into its target at
    /// `item.rel_path` atomically and records it. `job` releases that
    /// job's space reservation. Fails if the destination exists.
    pub fn commit_file(
        &self,
        src: &Path,
        item: &NewItem,
        job: Option<JobId>,
    ) -> Result<Item, Error> {
        let target = self.target(&item.target_id)?;
        let rel = check_rel_path(&item.rel_path)?;
        let dst = target.path.join(rel);
        if fs::symlink_metadata(&dst).is_ok() {
            return Err(Error::Exists(item.rel_path.clone()));
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        let size = dir_size(src)?;
        let now = self.inner.now();
        let mut conn = lock(&self.inner.conn);
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO storage_items (target_id, rel_path, group_key, size_bytes, profile_id, \
                 favorite, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                item.target_id,
                item.rel_path,
                item.group_key,
                to_i64(size)?,
                item.profile_id,
                item.favorite,
                now
            ],
        )?;
        let id = tx.last_insert_rowid();
        // The row is rolled back if the move fails.
        move_atomic(src, &dst, &target.path.join(TMP_DIR))?;
        tx.commit()?;
        self.inner.reload(&conn)?;
        drop(conn);
        if let Some(job) = job {
            lock(&self.inner.reservations).remove(&job);
        }
        self.item(id)
    }

    pub fn item(&self, id: i64) -> Result<Item, Error> {
        lock(&self.inner.conn)
            .query_row(
                &format!("SELECT {ITEM_COLUMNS} FROM storage_items WHERE id = ?1"),
                [id],
                item_from_row,
            )
            .optional()?
            .ok_or(Error::NotFound(id))
    }

    fn query_items(
        &self,
        filter: &str,
        args: &[&dyn pa_library::rusqlite::ToSql],
    ) -> Result<Vec<Item>, Error> {
        let conn = lock(&self.inner.conn);
        let mut stmt = conn.prepare(&format!(
            "SELECT {ITEM_COLUMNS} FROM storage_items WHERE {filter}"
        ))?;
        let items = stmt
            .query_map(args, item_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(items)
    }

    /// Kept items of a target, newest first.
    pub fn items(&self, target_id: &str) -> Result<Vec<Item>, Error> {
        self.query_items(
            "target_id = ?1 AND trashed_at_ms IS NULL ORDER BY created_at_ms DESC, id DESC",
            &[&target_id],
        )
    }

    /// Items in the trash of every target, most recently trashed first.
    pub fn trash(&self) -> Result<Vec<Item>, Error> {
        self.query_items(
            "trashed_at_ms IS NOT NULL ORDER BY trashed_at_ms DESC, id DESC",
            &[],
        )
    }

    pub fn set_favorite(&self, id: i64, favorite: bool) -> Result<(), Error> {
        let n = lock(&self.inner.conn).execute(
            "UPDATE storage_items SET favorite = ?1 WHERE id = ?2",
            params![favorite, id],
        )?;
        if n == 0 {
            Err(Error::NotFound(id))
        } else {
            Ok(())
        }
    }

    /// Marks an item watched at `at_ms` (or not watched).
    pub fn set_watched(&self, id: i64, at_ms: Option<i64>) -> Result<(), Error> {
        let n = lock(&self.inner.conn).execute(
            "UPDATE storage_items SET watched_at_ms = ?1 WHERE id = ?2",
            params![at_ms, id],
        )?;
        if n == 0 {
            Err(Error::NotFound(id))
        } else {
            Ok(())
        }
    }

    /// Moves an item to its target's trash (`.trash/<id>/`). A file that is
    /// already gone is trashed without moving.
    pub fn trash_item(&self, id: i64) -> Result<Item, Error> {
        let item = self.item(id)?;
        if item.trashed_at_ms.is_some() {
            return Ok(item);
        }
        let target = self.target(&item.target_id)?;
        let src = target.path.join(&item.rel_path);
        let name = Path::new(&item.rel_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("item");
        let trash_rel = format!("{TRASH_DIR}/{id}/{name}");
        let dst = target.path.join(&trash_rel);
        fs::create_dir_all(target.path.join(TRASH_DIR).join(id.to_string()))?;
        match move_atomic(&src, &dst, &target.path.join(TMP_DIR)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        let now = self.inner.now();
        let conn = lock(&self.inner.conn);
        conn.execute(
            "UPDATE storage_items SET trashed_at_ms = ?1, trash_path = ?2 WHERE id = ?3",
            params![now, trash_rel, id],
        )?;
        self.inner.reload(&conn)?;
        drop(conn);
        self.item(id)
    }

    /// Brings an item back from the trash to its path.
    pub fn restore(&self, id: i64) -> Result<Item, Error> {
        let item = self.item(id)?;
        if item.trashed_at_ms.is_none() {
            return Ok(item);
        }
        let target = self.target(&item.target_id)?;
        let trash_path: String = lock(&self.inner.conn).query_row(
            "SELECT trash_path FROM storage_items WHERE id = ?1",
            [id],
            |r| r.get(0),
        )?;
        let dst = target.path.join(&item.rel_path);
        if fs::symlink_metadata(&dst).is_ok() {
            return Err(Error::Exists(item.rel_path));
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut conn = lock(&self.inner.conn);
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE storage_items SET trashed_at_ms = NULL, trash_path = NULL WHERE id = ?1",
            [id],
        )?;
        move_atomic(
            &target.path.join(&trash_path),
            &dst,
            &target.path.join(TMP_DIR),
        )?;
        tx.commit()?;
        let _ = fs::remove_dir(target.path.join(TRASH_DIR).join(id.to_string()));
        self.inner.reload(&conn)?;
        drop(conn);
        self.item(id)
    }

    /// Deletes trashed items for good: all, or those trashed at or before
    /// `before_ms`. An item whose file cannot be deleted stays in the trash.
    pub fn purge_trash(&self, before_ms: Option<i64>) -> Result<RetentionReport, Error> {
        let items = match before_ms {
            Some(t) => {
                self.query_items("trashed_at_ms IS NOT NULL AND trashed_at_ms <= ?1", &[&t])?
            }
            None => self.query_items("trashed_at_ms IS NOT NULL", &[])?,
        };
        self.purge_items(items)
    }

    fn purge_items(&self, items: Vec<Item>) -> Result<RetentionReport, Error> {
        let mut report = RetentionReport::default();
        for item in items {
            let Ok(target) = self.target(&item.target_id) else {
                continue;
            };
            if remove_any(&target.path.join(TRASH_DIR).join(item.id.to_string())).is_err() {
                continue;
            }
            lock(&self.inner.conn).execute("DELETE FROM storage_items WHERE id = ?1", [item.id])?;
            report.purged.push(item.id);
            report.purged_bytes = report.purged_bytes.saturating_add(item.size_bytes);
        }
        Ok(report)
    }

    /// Deletes an item and its file now, without the trash (NET-019: the
    /// user deletes an offline copy, whose source still has the file).
    /// Returns the bytes freed. The row stays if the file cannot be deleted.
    pub fn delete_item(&self, id: i64) -> Result<u64, Error> {
        let item = self.item(id)?;
        let target = self.target(&item.target_id)?;
        if item.trashed_at_ms.is_some() {
            remove_any(&target.path.join(TRASH_DIR).join(id.to_string()))?;
        } else {
            remove_any(&target.path.join(check_rel_path(&item.rel_path)?))?;
        }
        let conn = lock(&self.inner.conn);
        conn.execute("DELETE FROM storage_items WHERE id = ?1", [id])?;
        self.inner.reload(&conn)?;
        Ok(item.size_bytes)
    }

    /// Deletes the whole trash of one target for good, now: what the
    /// monitor does before it would pause a live recording
    /// ([`check`](Self::check)).
    pub fn purge_trash_of(&self, target_id: &str) -> Result<RetentionReport, Error> {
        let items = self.query_items(
            "trashed_at_ms IS NOT NULL AND target_id = ?1",
            &[&target_id],
        )?;
        self.purge_items(items)
    }

    // --- Retention -----------------------------------------------------

    /// Adds or replaces the rule of (target, group).
    pub fn set_retention(&self, rule: &RetentionRule) -> Result<(), Error> {
        let now = self.inner.now();
        let mut conn = lock(&self.inner.conn);
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM storage_retention WHERE target_id = ?1 AND group_key IS ?2",
            params![rule.target_id, rule.group_key],
        )?;
        tx.execute(
            "INSERT INTO storage_retention (target_id, group_key, keep_last, max_age_days, \
                 watched_after_days, keep_favorites, updated_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                rule.target_id,
                rule.group_key,
                rule.keep_last,
                rule.max_age_days,
                rule.watched_after_days,
                rule.keep_favorites,
                now
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Removes the rule of (target, group); false if there was none.
    pub fn remove_retention(
        &self,
        target_id: &str,
        group_key: Option<&str>,
    ) -> Result<bool, Error> {
        let n = lock(&self.inner.conn).execute(
            "DELETE FROM storage_retention WHERE target_id = ?1 AND group_key IS ?2",
            params![target_id, group_key],
        )?;
        Ok(n > 0)
    }

    /// All rules, by target and group (the target default first).
    pub fn retention_rules(&self) -> Result<Vec<RetentionRule>, Error> {
        let conn = lock(&self.inner.conn);
        let mut stmt = conn.prepare(
            "SELECT target_id, group_key, keep_last, max_age_days, watched_after_days, \
                 keep_favorites FROM storage_retention ORDER BY target_id, group_key",
        )?;
        let rules = stmt
            .query_map([], |r| {
                Ok(RetentionRule {
                    target_id: r.get(0)?,
                    group_key: r.get(1)?,
                    keep_last: r.get(2)?,
                    max_age_days: r.get(3)?,
                    watched_after_days: r.get(4)?,
                    keep_favorites: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rules)
    }

    /// What the next cleanup moves to the trash ("will be deleted next").
    pub fn plan_retention(&self) -> Result<Vec<Expiry>, Error> {
        let now = self.inner.now();
        let rules: HashMap<(String, Option<String>), RetentionRule> = self
            .retention_rules()?
            .into_iter()
            .map(|r| ((r.target_id.clone(), r.group_key.clone()), r))
            .collect();
        let items = self.query_items(
            "trashed_at_ms IS NULL ORDER BY target_id, group_key, created_at_ms DESC, id DESC",
            &[],
        )?;
        let days = |d: u32| i64::from(d).saturating_mul(DAY_MS);
        let mut kept: HashMap<(String, Option<String>), u32> = HashMap::new();
        let mut out = Vec::new();
        for item in items {
            let key = (item.target_id.clone(), item.group_key.clone());
            let Some(rule) = rules
                .get(&key)
                .or_else(|| rules.get(&(item.target_id.clone(), None)))
            else {
                continue;
            };
            if rule.keep_favorites && item.favorite {
                continue;
            }
            let reason = if rule
                .max_age_days
                .is_some_and(|d| item.created_at_ms.saturating_add(days(d)) <= now)
            {
                ExpiryReason::MaxAge
            } else if let (Some(d), Some(at)) = (rule.watched_after_days, item.watched_at_ms)
                && at.saturating_add(days(d)) <= now
            {
                ExpiryReason::Watched
            } else {
                let n = kept.entry(key).or_default();
                if rule.keep_last.is_some_and(|k| *n >= k) {
                    ExpiryReason::KeepLast
                } else {
                    *n += 1;
                    continue;
                }
            };
            out.push(Expiry { item, reason });
        }
        Ok(out)
    }

    /// Applies the rules: expired items go to the trash, items trashed
    /// longer than the trash period (7 days) are deleted.
    pub fn apply_retention(&self) -> Result<RetentionReport, Error> {
        let mut trashed = Vec::new();
        for expiry in self.plan_retention()? {
            self.trash_item(expiry.item.id)?;
            trashed.push(expiry.item.id);
        }
        let before = self.inner.now().saturating_sub(self.inner.trash_ms);
        let mut report = self.purge_trash(Some(before))?;
        report.trashed = trashed;
        Ok(report)
    }

    /// Registers the `cleanup` job handler: [`apply_retention`](Self::apply_retention).
    pub fn register_cleanup(&self, queue: &JobQueue) {
        let manager = self.clone();
        queue.register(kinds::CLEANUP, move |_ctx: &JobContext| {
            match manager.apply_retention() {
                Ok(r) => RunOutcome::Done(Some(json!({
                    "trashed": r.trashed.len(),
                    "purged": r.purged.len(),
                    "purged_bytes": r.purged_bytes,
                }))),
                Err(e) => RunOutcome::Retryable(e.to_string()),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        assert!(check_rel_path("vod/a.mp4").is_ok());
        for bad in ["", "/abs", "../up", "a/../b", "./a", ".tmp/x", ".trash/1/a"] {
            assert!(check_rel_path(bad).is_err(), "{bad}");
        }
    }
}
