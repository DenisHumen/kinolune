// SPDX-License-Identifier: MPL-2.0
//! The `offline_copy` job: resumable copies with positional reads.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use pa_catalog::Catalog;
use pa_jobs::{JobContext, JobHandler, RunOutcome};
use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_storage::{NewItem, StorageManager};
use pa_vfs::{ByteSource, ErrorKind, Vfs, VfsError, VfsPath};
use serde_json::json;

use crate::plan::{CopyFile, CopyPlan};
use crate::{Error, TARGET};

/// Tuning of the copy.
#[derive(Debug, Clone)]
pub struct CopyConfig {
    /// Bytes per read.
    pub chunk: usize,
    /// How often the offset is synced to disk and saved in the job.
    pub checkpoint_every: Duration,
    /// How long a run keeps reconnecting to a source that went away before
    /// it gives the job back to the queue (retried with backoff).
    pub reconnect_window: Duration,
    /// Longest pause between two reconnects (they start at 1 s and double).
    pub reconnect_max_delay: Duration,
}

impl Default for CopyConfig {
    fn default() -> Self {
        Self {
            chunk: 1 << 20,
            checkpoint_every: Duration::from_secs(2),
            reconnect_window: Duration::from_secs(120),
            reconnect_max_delay: Duration::from_secs(15),
        }
    }
}

/// The handler of `offline_copy` jobs.
#[derive(Clone)]
pub struct CopyHandler {
    pub(crate) catalog: Arc<Catalog>,
    pub(crate) storage: StorageManager,
    pub(crate) db: Arc<Mutex<Connection>>,
    pub(crate) config: CopyConfig,
}

/// Why a run stopped early.
enum Stop {
    /// Pause, cancel or shutdown.
    Interrupted,
    /// Give the job back to the queue.
    Retry(String),
    /// The job cannot succeed.
    Fatal(String),
}

impl From<io::Error> for Stop {
    fn from(e: io::Error) -> Self {
        // Local disk errors (full, removed card): the storage monitor and
        // the gate decide when to go on.
        Stop::Retry(format!("local i/o: {e}"))
    }
}

impl From<Error> for Stop {
    fn from(e: Error) -> Self {
        Stop::Retry(e.to_string())
    }
}

/// A source error that does not go away by reconnecting.
fn permanent(e: &VfsError) -> bool {
    matches!(
        e.kind(),
        ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::AuthenticationFailed
            | ErrorKind::InvalidPath
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory
            | ErrorKind::Unsupported
    )
}

fn source_error(e: &VfsError) -> Stop {
    match e.kind() {
        ErrorKind::NotFound => Stop::Fatal(format!("source_missing: {e}")),
        ErrorKind::AuthenticationFailed => Stop::Fatal(format!("auth: {e}")),
        ErrorKind::PermissionDenied => Stop::Fatal(format!("denied: {e}")),
        _ => Stop::Fatal(format!("source: {e}")),
    }
}

/// The connection to the source of one run, reconnected after failures.
struct Session<'a> {
    handler: &'a CopyHandler,
    ctx: &'a JobContext,
    source_id: i64,
    vfs: Option<Arc<dyn Vfs>>,
}

impl Session<'_> {
    fn vfs(&mut self) -> Result<Arc<dyn Vfs>, VfsError> {
        if let Some(vfs) = &self.vfs {
            return Ok(Arc::clone(vfs));
        }
        let vfs = self
            .handler
            .catalog
            .connect(self.source_id)
            .map_err(|e| VfsError::new(ErrorKind::Unavailable, e.to_string()))?;
        self.vfs = Some(Arc::clone(&vfs));
        Ok(vfs)
    }

    /// Runs `op` until it succeeds, fails for good, or the source stays
    /// away longer than the reconnect window.
    fn retry<T>(
        &mut self,
        mut op: impl FnMut(&dyn Vfs) -> Result<T, VfsError>,
    ) -> Result<T, (Stop, Option<VfsError>)> {
        let started = Instant::now();
        let mut delay = Duration::from_secs(1);
        loop {
            let result = self.vfs().and_then(|vfs| op(vfs.as_ref()));
            match result {
                Ok(v) => return Ok(v),
                Err(e) if permanent(&e) => return Err((source_error(&e), Some(e))),
                Err(e) => {
                    // A new connection next time: the old one may be dead.
                    self.vfs = None;
                    if started.elapsed() >= self.handler.config.reconnect_window {
                        return Err((Stop::Retry(format!("source unavailable: {e}")), Some(e)));
                    }
                    if !self.sleep(delay) {
                        return Err((Stop::Interrupted, None));
                    }
                    delay = (delay * 2).min(self.handler.config.reconnect_max_delay);
                }
            }
        }
    }

    /// Sleeps, waking up for a stop request; false if stopped.
    fn sleep(&self, d: Duration) -> bool {
        let until = Instant::now() + d;
        while Instant::now() < until {
            if self.ctx.stop_requested() {
                return false;
            }
            thread::sleep(Duration::from_millis(50).min(until - Instant::now()));
        }
        !self.ctx.stop_requested()
    }
}

fn part_path(dst: &Path) -> PathBuf {
    let mut name = dst.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dst.with_file_name(name)
}

/// Where `path` of the source goes under `root`.
pub(crate) fn local_of(root: &Path, path: &str) -> PathBuf {
    path.trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .fold(root.to_path_buf(), |p, s| p.join(s))
}

impl JobHandler for CopyHandler {
    fn run(&self, ctx: &JobContext) -> RunOutcome {
        let plan: CopyPlan = match serde_json::from_value(ctx.params().clone()) {
            Ok(p) => p,
            Err(e) => return RunOutcome::Fatal(format!("bad_params: {e}")),
        };
        let job = ctx.job().id;
        let tmp = match self.storage.job_tmp_dir(TARGET, job) {
            Ok(t) => t,
            Err(e) => return RunOutcome::Retryable(format!("storage: {e}")),
        };
        match self.copy(ctx, &plan, &tmp) {
            Ok(result) => {
                let _ = fs::remove_dir_all(&tmp);
                RunOutcome::Done(Some(result))
            }
            Err(Stop::Interrupted) => {
                if ctx.is_cancelled() {
                    let _ = fs::remove_dir_all(&tmp);
                }
                RunOutcome::Interrupted
            }
            Err(Stop::Retry(m)) => RunOutcome::Retryable(m),
            Err(Stop::Fatal(m)) => {
                let _ = fs::remove_dir_all(&tmp);
                RunOutcome::Fatal(m)
            }
        }
    }
}

impl CopyHandler {
    fn copy(
        &self,
        ctx: &JobContext,
        plan: &CopyPlan,
        tmp: &Path,
    ) -> Result<serde_json::Value, Stop> {
        let job = ctx.job().id;
        let folder = plan.folder(job);
        // Committed by a run that was killed before it finished: only the
        // database rows are missing.
        if let Some(item) = self
            .storage
            .items(TARGET)
            .map_err(Error::from)?
            .into_iter()
            .find(|i| i.rel_path == folder)
        {
            return self.finish(ctx, plan, &item);
        }
        let out = tmp.join("out");
        let mut session = Session {
            handler: self,
            ctx,
            source_id: plan.source_id,
            vfs: None,
        };

        // Sizes first: the total for the progress bar and the gate. An
        // optional file (subtitle, art) that is gone is skipped.
        let mut sizes: Vec<Option<u64>> = Vec::with_capacity(plan.files.len());
        for f in &plan.files {
            let path = VfsPath::parse(&f.path)
                .map_err(|e| Stop::Fatal(format!("bad path {}: {e}", f.path)))?;
            match session.retry(|vfs| vfs.stat(&path)) {
                Ok(m) => sizes.push(Some(m.size)),
                Err((_, Some(e))) if e.kind() == ErrorKind::NotFound && !f.role.required() => {
                    sizes.push(None);
                }
                Err((stop, _)) => return Err(stop),
            }
        }
        let total: u64 = sizes.iter().flatten().sum();
        let mut done: u64 = 0;
        ctx.report_bytes(0, Some(total));

        for (index, (f, size)) in plan.files.iter().zip(&sizes).enumerate() {
            let Some(size) = *size else { continue };
            let dst = local_of(&out, &f.path);
            if fs::metadata(&dst).is_ok_and(|m| m.is_file() && m.len() == size) {
                done += size;
                continue;
            }
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            self.copy_file(&mut session, ctx, f, index, size, &dst, done, total)?;
            done += size;
        }
        ctx.report_bytes(total, Some(total));

        let mut new_item = NewItem::new(TARGET, &folder).group(plan.group.clone());
        if let Some(p) = ctx.job().owner_profile_id {
            new_item = new_item.profile(p);
        }
        fs::create_dir_all(&out)?;
        let item = self
            .storage
            .commit_file(&out, &new_item, Some(job))
            .map_err(|e| Stop::Retry(format!("commit: {e}")))?;
        self.finish(ctx, plan, &item)
    }

    #[allow(clippy::too_many_arguments)]
    fn copy_file(
        &self,
        session: &mut Session<'_>,
        ctx: &JobContext,
        f: &CopyFile,
        index: usize,
        size: u64,
        dst: &Path,
        done_before: u64,
        total: u64,
    ) -> Result<(), Stop> {
        let path = VfsPath::parse(&f.path)
            .map_err(|e| Stop::Fatal(format!("bad path {}: {e}", f.path)))?;
        let part = part_path(dst);
        let mut out: File = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&part)?;
        let mut offset = out.metadata()?.len();
        if offset > size {
            // The source got smaller: start over.
            offset = 0;
        }
        out.set_len(offset)?;
        out.seek(SeekFrom::Start(offset))?;
        let mut source: Arc<dyn ByteSource> = session
            .retry(|vfs| vfs.open(&path))
            .map(Arc::from)
            .map_err(|(s, _)| s)?;
        let mut buf = vec![0u8; self.config.chunk.max(4096)];
        let mut last_checkpoint = Instant::now();
        let checkpoint = |out: &File, offset: u64| -> Result<bool, Stop> {
            out.sync_data()?;
            ctx.save_checkpoint(&json!({ "file": index, "offset": offset }), done_before + offset)
                .map_err(|e| Stop::Retry(e.to_string()))
        };
        while offset < size {
            if ctx.stop_requested() {
                let _ = checkpoint(&out, offset);
                return Err(Stop::Interrupted);
            }
            let want = usize::try_from(size - offset)
                .unwrap_or(usize::MAX)
                .min(buf.len());
            let n = match source.read_at(offset, &mut buf[..want]) {
                Ok(n) => n,
                Err(e) if permanent(&e) => return Err(source_error(&e)),
                Err(_) => {
                    // The connection dropped: save where we are, reconnect
                    // and reopen the file, then go on from the offset.
                    let _ = checkpoint(&out, offset);
                    last_checkpoint = Instant::now();
                    match session.retry(|vfs| vfs.open(&path)) {
                        Ok(s) => {
                            source = Arc::from(s);
                            continue;
                        }
                        Err((stop, _)) => return Err(stop),
                    }
                }
            };
            if n == 0 {
                let _ = checkpoint(&out, offset);
                return Err(Stop::Retry(format!(
                    "{}: ended at {offset} of {size} bytes",
                    f.path
                )));
            }
            out.write_all(&buf[..n])?;
            offset += n as u64;
            ctx.report_bytes(done_before + offset, Some(total));
            if last_checkpoint.elapsed() >= self.config.checkpoint_every {
                if !checkpoint(&out, offset)? {
                    return Err(Stop::Interrupted);
                }
                last_checkpoint = Instant::now();
            }
        }
        out.sync_all()?;
        drop(out);
        fs::rename(&part, dst)?;
        Ok(())
    }

    /// Records the committed folder as a copy of the plan's item.
    fn finish(
        &self,
        ctx: &JobContext,
        plan: &CopyPlan,
        item: &pa_storage::Item,
    ) -> Result<serde_json::Value, Stop> {
        let target = self.storage.target(TARGET).map_err(Error::from)?;
        let root = target.path.join(&item.rel_path);
        let copy_id = record(
            &mut self.db.lock().unwrap_or_else(|e| e.into_inner()),
            plan,
            ctx.job().id,
            item,
            &root,
        )
        .map_err(|e| Stop::Retry(format!("record: {e}")))?;
        Ok(json!({
            "copy_id": copy_id,
            "storage_item_id": item.id,
            "folder": item.rel_path,
            "bytes": item.size_bytes,
        }))
    }
}

/// Inserts (or finds) the `offline_copies` row of storage item `item` and
/// the `offline_files` rows of the plan's files present under `root`.
pub(crate) fn record(
    conn: &mut Connection,
    plan: &CopyPlan,
    job: i64,
    item: &pa_storage::Item,
    root: &Path,
) -> Result<i64, Error> {
    let tx = conn.transaction()?;
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM offline_copies WHERE storage_item_id = ?1",
            [item.id],
            |r| r.get(0),
        )
        .optional()?;
    let copy_id = match existing {
        Some(id) => id,
        None => {
            tx.execute(
                "INSERT INTO offline_copies (storage_item_id, item_key, source_id, file_id, title, \
                     version_label, job_id, profile_id, created_at_ms) \
                 VALUES (?1, ?2, ?3, (SELECT id FROM files WHERE id = ?4), ?5, ?6, ?7, \
                     (SELECT id FROM profiles WHERE id = ?8), ?9)",
                params![
                    item.id,
                    plan.item,
                    plan.source_id,
                    plan.file_id,
                    plan.title,
                    plan.version,
                    job,
                    item.profile_id,
                    item.created_at_ms,
                ],
            )?;
            tx.last_insert_rowid()
        }
    };
    for f in &plan.files {
        let Ok(meta) = fs::metadata(local_of(root, &f.path)) else {
            continue;
        };
        tx.execute(
            "INSERT OR IGNORE INTO offline_files (copy_id, source_id, path, role, size_bytes) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                copy_id,
                plan.source_id,
                f.path,
                f.role.as_str(),
                i64::try_from(meta.len()).unwrap_or(i64::MAX)
            ],
        )?;
    }
    tx.commit()?;
    Ok(copy_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let root = Path::new("/t/x");
        assert_eq!(
            local_of(root, "/Movies/a b/c.mkv"),
            PathBuf::from("/t/x/Movies/a b/c.mkv")
        );
        assert_eq!(
            part_path(Path::new("/t/c.mkv")),
            PathBuf::from("/t/c.mkv.part")
        );
    }
}
