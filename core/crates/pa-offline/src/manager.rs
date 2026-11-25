// SPDX-License-Identifier: MPL-2.0
//! [`Offline`]: what the app calls to download, list and delete copies.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use pa_catalog::{Catalog, Item, ItemKind, ItemRef};
use pa_jobs::{Job, JobId, JobQueue, JobState, NetworkPolicy, NewJob, priority};
use pa_library::rusqlite::{Connection, Row, params};
use pa_storage::{RetentionReport, RetentionRule, StorageManager, Target, TargetKind, Usage};

use crate::copy::{CopyConfig, CopyHandler};
use crate::plan::{CopyPlan, plan_in_group};
use crate::{Error, KIND, TARGET};

/// What to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// A movie, an episode or a track; a show means all its episodes.
    Item(ItemRef),
    /// One season of a show.
    Season { show: ItemRef, season: i64 },
    /// Every track of a music album (`music_albums.id`).
    Album(i64),
    /// A video playlist (LIB-010).
    Playlist(i64),
    /// An audio playlist (MUS-004).
    MusicPlaylist(i64),
}

/// How to download.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadOptions {
    /// The version (`files.id`) of a single movie or episode; the default
    /// version otherwise.
    pub version_file_id: Option<i64>,
}

/// State of a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DownloadState {
    Queued,
    /// Waits for space, the network or Wi-Fi.
    Waiting,
    Running,
    Paused,
    /// Waits to retry after an error.
    Retrying,
    Failed,
    Cancelled,
    /// The copy is on the device.
    Done,
}

impl DownloadState {
    pub const fn as_str(self) -> &'static str {
        match self {
            DownloadState::Queued => "queued",
            DownloadState::Waiting => "waiting",
            DownloadState::Running => "running",
            DownloadState::Paused => "paused",
            DownloadState::Retrying => "retrying",
            DownloadState::Failed => "failed",
            DownloadState::Cancelled => "cancelled",
            DownloadState::Done => "done",
        }
    }

    fn of(state: JobState) -> Self {
        match state {
            JobState::Queued => DownloadState::Queued,
            JobState::WaitingResources => DownloadState::Waiting,
            JobState::Running => DownloadState::Running,
            JobState::Paused => DownloadState::Paused,
            JobState::Retrying => DownloadState::Retrying,
            JobState::Failed => DownloadState::Failed,
            JobState::Cancelled => DownloadState::Cancelled,
            JobState::Done => DownloadState::Done,
        }
    }

    /// Not finished: the item must not be queued again.
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            DownloadState::Queued
                | DownloadState::Waiting
                | DownloadState::Running
                | DownloadState::Paused
                | DownloadState::Retrying
        )
    }
}

/// A copy on the device.
#[derive(Debug, Clone, PartialEq)]
pub struct OfflineCopy {
    pub id: i64,
    pub storage_item_id: i64,
    /// Item id (`m12`, `e40`, `t7`).
    pub item: String,
    pub title: String,
    pub version: Option<String>,
    pub size_bytes: u64,
    pub watched_at_ms: Option<i64>,
    pub created_at_ms: i64,
    /// Folder of the copy.
    pub folder: PathBuf,
}

/// One row of the downloads screen: a job in the queue or a finished copy.
#[derive(Debug, Clone, PartialEq)]
pub struct Download {
    pub job_id: Option<JobId>,
    pub copy_id: Option<i64>,
    pub item: String,
    pub title: String,
    pub version: Option<String>,
    pub state: DownloadState,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    /// Why it waits (`disk_reserve`, `quota`…) or is paused (`user`,
    /// `network`, `resources`).
    pub waiting_for: Option<String>,
    pub error: Option<String>,
    pub watched: bool,
    pub created_at_ms: i64,
}

/// Settings of offline copies.
#[derive(Debug, Clone, PartialEq)]
pub struct OfflineSettings {
    /// The folder copies go to; `None` until set.
    pub folder: Option<PathBuf>,
    pub wifi_only: bool,
    /// Delete watched copies this many days after they were watched
    /// (0: at the next cleanup); `None`: keep them.
    pub delete_watched_after_days: Option<u32>,
}

/// Offline copies of one library database.
#[derive(Clone)]
pub struct Offline {
    inner: Arc<Inner>,
}

struct Inner {
    catalog: Arc<Catalog>,
    queue: JobQueue,
    storage: StorageManager,
    db: Arc<Mutex<Connection>>,
    wifi_only: AtomicBool,
    config: CopyConfig,
}

const COPY_COLUMNS: &str = "c.id, c.storage_item_id, c.item_key, c.title, c.version_label, \
     si.size_bytes, si.watched_at_ms, si.created_at_ms, t.path, si.rel_path";

const COPY_FROM: &str = "FROM offline_copies c \
     JOIN storage_items si ON si.id = c.storage_item_id AND si.trashed_at_ms IS NULL \
     JOIN storage_targets t ON t.id = si.target_id";

fn copy_from_row(r: &Row<'_>) -> pa_library::rusqlite::Result<OfflineCopy> {
    let target: String = r.get(8)?;
    let rel: String = r.get(9)?;
    Ok(OfflineCopy {
        id: r.get(0)?,
        storage_item_id: r.get(1)?,
        item: r.get(2)?,
        title: r.get(3)?,
        version: r.get(4)?,
        size_bytes: u64::try_from(r.get::<_, i64>(5)?).unwrap_or(0),
        watched_at_ms: r.get(6)?,
        created_at_ms: r.get(7)?,
        folder: pa_vfs::host::join_relative(Path::new(&target), &rel),
    })
}

impl Offline {
    /// Offline copies over `catalog`'s database, run by `queue` behind the
    /// gate of `storage` (both on the same database; `storage` attached to
    /// `queue`). Call [`register`](Self::register) to run the copies and
    /// [`set_folder`](Self::set_folder) before the first download.
    pub fn new(
        catalog: Arc<Catalog>,
        queue: JobQueue,
        storage: StorageManager,
    ) -> Result<Self, Error> {
        Self::with_config(catalog, queue, storage, CopyConfig::default())
    }

    pub fn with_config(
        catalog: Arc<Catalog>,
        queue: JobQueue,
        storage: StorageManager,
        config: CopyConfig,
    ) -> Result<Self, Error> {
        let conn = pa_library::open(catalog.path())?;
        let wifi_only = conn.query_row(
            "SELECT count(*) > 0 AND count(*) = sum(network = 'unmetered') FROM jobs \
             WHERE type = ?1 AND state NOT IN ('done', 'failed', 'cancelled')",
            [KIND],
            |r| r.get::<_, Option<bool>>(0),
        )?;
        Ok(Self {
            inner: Arc::new(Inner {
                catalog,
                queue,
                storage,
                db: Arc::new(Mutex::new(conn)),
                wifi_only: AtomicBool::new(wifi_only.unwrap_or(false)),
                config,
            }),
        })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.inner.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The job handler of this manager.
    pub fn handler(&self) -> CopyHandler {
        CopyHandler {
            catalog: Arc::clone(&self.inner.catalog),
            storage: self.inner.storage.clone(),
            db: Arc::clone(&self.inner.db),
            config: self.inner.config.clone(),
        }
    }

    /// Registers the `offline_copy` handler on the queue.
    pub fn register(&self) {
        self.inner.queue.register(KIND, self.handler());
    }

    // --- Settings ------------------------------------------------------

    /// Sets the folder copies go to (target `offline`, 5 % of the volume
    /// kept free). It cannot move while copies or downloads use the old
    /// folder.
    pub fn set_folder(&self, dir: &Path) -> Result<(), Error> {
        // BUG-022: the app may pass `C:\Users\me\Kinolune/downloads/offline`.
        let dir = &pa_vfs::host::native(dir.to_path_buf());
        if let Ok(current) = self.inner.storage.target(TARGET) {
            if current.path == dir {
                return Ok(());
            }
            if !self.copies()?.is_empty() {
                return Err(Error::Busy("copies"));
            }
            if self
                .jobs()?
                .iter()
                .any(|j| DownloadState::of(j.state).is_active())
            {
                return Err(Error::Busy("downloads"));
            }
        }
        let target = Target::new(TARGET, TargetKind::Downloads, dir).reserve_percent(5.0);
        self.inner.storage.set_target(&target)?;
        Ok(())
    }

    /// The folder copies go to, with native separators (a folder saved
    /// before BUG-022 may mix `\` and `/` on Windows).
    pub fn folder(&self) -> Option<PathBuf> {
        self.inner
            .storage
            .target(TARGET)
            .ok()
            .map(|t| pa_vfs::host::native(t.path))
    }

    /// Wi-Fi (unmetered) only for new downloads and those not finished;
    /// returns how many queued downloads changed.
    pub fn set_wifi_only(&self, on: bool) -> Result<usize, Error> {
        self.inner.wifi_only.store(on, Ordering::SeqCst);
        let policy = if on {
            NetworkPolicy::Unmetered
        } else {
            NetworkPolicy::Any
        };
        let n = self.db().execute(
            "UPDATE jobs SET network = ?1 WHERE type = ?2 \
             AND state IN ('queued', 'waiting_resources', 'running', 'paused', 'retrying')",
            params![policy.as_str(), KIND],
        )?;
        Ok(n)
    }

    pub fn wifi_only(&self) -> bool {
        self.inner.wifi_only.load(Ordering::SeqCst)
    }

    /// "Delete watched copies": a retention rule of the target for every
    /// group (`None` removes it). Favourites are kept.
    pub fn set_delete_watched(&self, after_days: Option<u32>) -> Result<(), Error> {
        let storage = &self.inner.storage;
        match after_days {
            Some(days) => {
                let mut rule = storage
                    .retention_rules()?
                    .into_iter()
                    .find(|r| r.target_id == TARGET && r.group_key.is_none())
                    .unwrap_or_else(|| RetentionRule::new(TARGET, None));
                rule.watched_after_days = Some(days);
                rule.keep_favorites = true;
                storage.set_retention(&rule)?;
            }
            None => {
                storage.remove_retention(TARGET, None)?;
            }
        }
        Ok(())
    }

    pub fn settings(&self) -> Result<OfflineSettings, Error> {
        let rule = self
            .inner
            .storage
            .retention_rules()?
            .into_iter()
            .find(|r| r.target_id == TARGET && r.group_key.is_none());
        Ok(OfflineSettings {
            folder: self.folder(),
            wifi_only: self.wifi_only(),
            delete_watched_after_days: rule.and_then(|r| r.watched_after_days),
        })
    }

    /// Space of the offline folder, if it is set.
    pub fn usage(&self) -> Result<Option<Usage>, Error> {
        Ok(self
            .inner
            .storage
            .usage()?
            .into_iter()
            .find(|u| u.target_id == TARGET))
    }

    // --- Downloads -----------------------------------------------------

    fn items_of(&self, selection: &Selection) -> Result<Vec<(Item, Option<String>)>, Error> {
        let catalog = &self.inner.catalog;
        let plain = |items: Vec<Item>| items.into_iter().map(|i| (i, None)).collect();
        Ok(match selection {
            Selection::Item(ItemRef::Show(id)) => plain(catalog.episodes(ItemRef::Show(*id))?),
            Selection::Item(r) => match catalog.item(*r)? {
                Some(item) => vec![(item, None)],
                None => return Err(Error::NotFound(r.to_string())),
            },
            Selection::Season { show, season } => plain(
                catalog
                    .episodes(*show)?
                    .into_iter()
                    .filter(|e| e.season == Some(*season))
                    .collect(),
            ),
            Selection::Album(id) => catalog
                .music_tracks()?
                .into_iter()
                .filter(|t| t.album_id == Some(*id))
                .map(|t| (t.item, Some(format!("album:{id}"))))
                .collect(),
            Selection::Playlist(id) => plain(catalog.playlist_items(*id)?),
            Selection::MusicPlaylist(id) => plain(catalog.music_playlist_items(*id)?),
        })
    }

    /// Queues copies of what `selection` names; returns the new jobs.
    /// Items already on the device or being downloaded are skipped, and so
    /// are items of local folders and unplayable files when the selection
    /// is more than one item. Fails with `WontFit` if a copy can never fit.
    pub fn download(
        &self,
        selection: &Selection,
        options: &DownloadOptions,
    ) -> Result<Vec<JobId>, Error> {
        if self.folder().is_none() {
            return Err(Error::Invalid("no offline folder".into()));
        }
        let single = matches!(selection, Selection::Item(r) if !matches!(r, ItemRef::Show(_)));
        let items = self.items_of(selection)?;
        let busy: HashSet<String> = self
            .downloads()?
            .into_iter()
            .filter(|d| d.state.is_active() || d.state == DownloadState::Done)
            .map(|d| d.item)
            .collect();
        let profile = self.inner.catalog.active_profile().ok().map(|p| p.id);
        let network = if self.wifi_only() {
            NetworkPolicy::Unmetered
        } else {
            NetworkPolicy::Any
        };
        let mut jobs = Vec::new();
        let mut seen = HashSet::new();
        for (item, group) in items {
            let key = item.id.to_string();
            if busy.contains(&key) || !seen.insert(key) {
                continue;
            }
            let version = if single {
                options.version_file_id
            } else {
                None
            };
            let plan = match plan_in_group(&self.inner.catalog, item.id, version, group) {
                Ok(p) => p,
                Err(Error::Local(_) | Error::Unplayable(_)) if !single => continue,
                Err(e) => return Err(e),
            };
            let mut job = NewJob::new(KIND, serde_json::to_value(&plan)?)
                .priority(priority::USER_WAITING)
                .network(network)
                .max_attempts(20)
                .target_storage(TARGET);
            let bytes = plan.known_bytes();
            if bytes > 0 {
                job = job.bytes_total(bytes);
            }
            if let Some(p) = profile {
                job = job.owner(p);
            }
            jobs.push(self.inner.storage.enqueue(&self.inner.queue, &job)?);
        }
        Ok(jobs)
    }

    fn jobs(&self) -> Result<Vec<Job>, Error> {
        let mut jobs = self.inner.queue.list(&[])?;
        jobs.retain(|j| j.kind == KIND);
        Ok(jobs)
    }

    /// Copies on the device, newest first.
    pub fn copies(&self) -> Result<Vec<OfflineCopy>, Error> {
        let conn = self.db();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COPY_COLUMNS} {COPY_FROM} ORDER BY c.created_at_ms DESC, c.id DESC"
        ))?;
        let copies = stmt
            .query_map([], copy_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(copies)
    }

    /// Downloads in the queue (not finished, failed) and copies on the
    /// device, newest first. A finished job shows as its copy.
    pub fn downloads(&self) -> Result<Vec<Download>, Error> {
        let copies = self.copies()?;
        let copied_jobs: HashMap<i64, ()> = {
            let conn = self.db();
            let mut stmt = conn.prepare(
                "SELECT job_id FROM offline_copies c \
                 JOIN storage_items si ON si.id = c.storage_item_id \
                 WHERE si.trashed_at_ms IS NULL AND job_id IS NOT NULL",
            )?;
            stmt.query_map([], |r| r.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(|j| (j, ()))
                .collect()
        };
        let mut out: Vec<Download> = Vec::new();
        for j in self.jobs()? {
            let state = DownloadState::of(j.state);
            if state == DownloadState::Done || copied_jobs.contains_key(&j.id) {
                continue;
            }
            let plan: Option<CopyPlan> = serde_json::from_value(j.params.clone()).ok();
            out.push(Download {
                job_id: Some(j.id),
                copy_id: None,
                item: plan.as_ref().map(|p| p.item.clone()).unwrap_or_default(),
                title: plan.as_ref().map(|p| p.title.clone()).unwrap_or_default(),
                version: plan.and_then(|p| p.version),
                state,
                bytes_done: j.bytes_done,
                bytes_total: j.bytes_total,
                waiting_for: j
                    .wait_reason
                    .clone()
                    .or_else(|| j.pause_reason.map(|r| r.to_string())),
                error: matches!(state, DownloadState::Failed | DownloadState::Retrying)
                    .then(|| j.error.clone())
                    .flatten(),
                watched: false,
                created_at_ms: j.created_at_ms,
            });
        }
        for c in copies {
            out.push(Download {
                job_id: None,
                copy_id: Some(c.id),
                item: c.item,
                title: c.title,
                version: c.version,
                state: DownloadState::Done,
                bytes_done: c.size_bytes,
                bytes_total: Some(c.size_bytes),
                waiting_for: None,
                error: None,
                watched: c.watched_at_ms.is_some(),
                created_at_ms: c.created_at_ms,
            });
        }
        out.sort_by_key(|d| std::cmp::Reverse(d.created_at_ms));
        Ok(out)
    }

    /// The download or copy of one item, if any: a copy on the device
    /// first, else the newest job.
    pub fn state(&self, item: &str) -> Result<Option<Download>, Error> {
        let all = self.downloads()?;
        let mine: Vec<Download> = all.into_iter().filter(|d| d.item == item).collect();
        Ok(mine
            .iter()
            .find(|d| d.state == DownloadState::Done)
            .or_else(|| mine.first())
            .cloned())
    }

    /// `pause`, `resume`, `cancel` (also removes the job), `retry` or
    /// `remove` (a finished job).
    pub fn control(&self, job_id: JobId, action: &str) -> Result<(), Error> {
        let queue = &self.inner.queue;
        let job = queue.get(job_id)?;
        if job.kind != KIND {
            return Err(Error::Invalid("not an offline copy".into()));
        }
        match action {
            "pause" => queue.pause(job_id).map(|_| ())?,
            "resume" => queue.resume(job_id).map(|_| ())?,
            "retry" => queue.retry(job_id).map(|_| ())?,
            "cancel" => {
                queue.cancel(job_id)?;
                // The handler, if running, deletes its tmp folder.
                let _ = queue.remove(job_id);
            }
            "remove" => queue.remove(job_id)?,
            _ => return Err(Error::Invalid(format!("action {action:?}"))),
        }
        Ok(())
    }

    /// Deletes a copy now (the source still has the files); returns the
    /// bytes freed.
    pub fn delete_copy(&self, copy_id: i64) -> Result<u64, Error> {
        let storage_item: i64 = self
            .db()
            .query_row(
                "SELECT storage_item_id FROM offline_copies WHERE id = ?1",
                [copy_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::NotFound(format!("copy {copy_id}")))?;
        Ok(self.inner.storage.delete_item(storage_item)?)
    }

    /// Deletes the copies of an item and cancels its downloads.
    pub fn delete_item(&self, item: &str) -> Result<u64, Error> {
        let mut freed = 0;
        for d in self.downloads()?.into_iter().filter(|d| d.item == item) {
            if let Some(copy) = d.copy_id {
                freed += self.delete_copy(copy)?;
            } else if let Some(job) = d.job_id {
                if d.state.is_active() {
                    self.control(job, "cancel")?;
                } else {
                    let _ = self.inner.queue.remove(job);
                }
            }
        }
        Ok(freed)
    }

    // --- Retention -----------------------------------------------------

    /// Copies the active profile watched get their storage item marked
    /// watched (and unmarked when progress was reset), so the "delete
    /// watched" rule sees them. Returns how many marks changed.
    pub fn sync_watched(&self) -> Result<usize, Error> {
        let mut changed = 0;
        for copy in self.copies()? {
            let Ok(item) = copy.item.parse::<ItemRef>() else {
                continue;
            };
            if matches!(item, ItemRef::Track(_)) {
                continue;
            }
            let progress = self.inner.catalog.progress(item)?;
            let watched_at = progress.filter(|p| p.watched).map(|p| p.updated_at_ms);
            if watched_at.is_some() != copy.watched_at_ms.is_some() {
                self.inner
                    .storage
                    .set_watched(copy.storage_item_id, watched_at)?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// [`sync_watched`](Self::sync_watched), then the retention rules and
    /// the trash of every target.
    pub fn cleanup(&self) -> Result<RetentionReport, Error> {
        self.sync_watched()?;
        Ok(self.inner.storage.apply_retention()?)
    }

    /// Whether the item kind can be downloaded at all.
    pub fn downloadable(kind: ItemKind) -> bool {
        kind != ItemKind::Show
    }
}
