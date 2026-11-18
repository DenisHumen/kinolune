// SPDX-License-Identifier: MPL-2.0
//! State shared by the facades of one process: the secret store and the
//! job queue of a library database.
//!
//! The source manager ([`CoreSources`](crate::api::sources::CoreSources))
//! writes passwords and the library ([`CoreLibrary`](crate::api::library::CoreLibrary))
//! reads them to reach network sources (LIB-018). Both must use the same
//! store object: the encrypted fallback file caches its contents, so two
//! instances over one file would not see each other's writes.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, Weak};

use pa_catalog::Catalog;
use pa_catalog::pa_sync::lan::service::SyncService;

use pa_secrets::{OpenOptions, OpenedStore, SecretError};

static STORES: Mutex<Vec<(PathBuf, OpenedStore)>> = Mutex::new(Vec::new());

/// The secret store the app opened (`secrets_open_platform` or
/// `secrets_open_default`, CORE-015), or else the default store with its
/// fallback file in `data_dir`, opened once per process and directory.
pub(crate) fn secret_store(data_dir: &Path) -> Result<OpenedStore, SecretError> {
    if let Some(opened) = crate::api::secrets::opened_store() {
        return Ok(opened);
    }
    let mut stores = STORES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, opened)) = stores.iter().find(|(dir, _)| dir == data_dir) {
        return Ok(opened.clone());
    }
    let opened = pa_secrets::open(OpenOptions::new(Some(data_dir.to_path_buf())))?;
    stores.push((data_dir.to_path_buf(), opened.clone()));
    Ok(opened)
}

/// The job queue and the storage manager of one library database, shared
/// by every facade that runs jobs (ADR-0028: one queue per database and
/// process; ADR-0062: one gate per queue). Twitch downloads (TWI-017) and
/// offline copies (NET-019) register their handlers on it.
#[derive(Clone)]
pub(crate) struct Jobs {
    pub(crate) queue: pa_jobs::JobQueue,
    pub(crate) storage: pa_storage::StorageManager,
}

struct RunningJobs {
    jobs: Jobs,
    // Stop the scheduler and the monitor with the process.
    _driver: pa_jobs::QueueDriver,
    _monitor: pa_storage::MonitorDriver,
}

static JOBS: Mutex<Vec<(PathBuf, RunningJobs)>> = Mutex::new(Vec::new());

/// The running queue of the library database at `db`, started on first
/// use: the storage gate, the cleanup handler and the space monitor are
/// attached once.
pub(crate) fn jobs(db: &Path) -> Result<Jobs, String> {
    let mut all = JOBS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, running)) = all.iter().find(|(path, _)| path == db) {
        return Ok(running.jobs.clone());
    }
    let queue = pa_jobs::JobQueue::open(db, pa_jobs::QueueConfig::default())
        .map_err(|e| format!("jobs: {e}"))?;
    let storage = pa_storage::StorageManager::open(db, pa_storage::StorageConfig::default())
        .map_err(|e| format!("storage: {e}"))?;
    storage.attach(&queue);
    // CORE-009: job events go to `core_events` too.
    crate::bus::forward_jobs(&queue);
    let driver = queue.start();
    let monitor = storage.start_monitor(queue.clone(), std::time::Duration::from_secs(30), |_| {});
    let jobs = Jobs { queue, storage };
    all.push((
        db.to_path_buf(),
        RunningJobs {
            jobs: jobs.clone(),
            _driver: driver,
            _monitor: monitor,
        },
    ));
    Ok(jobs)
}

/// The catalog and sync service of a library database, for the source
/// list sync (SYNC-010): after the source manager
/// ([`CoreSources`](crate::api::sources::CoreSources)) changes a source,
/// the catalog records the ops and the service sends them.
struct SourceSync {
    db: PathBuf,
    catalog: Weak<Catalog>,
    service: Weak<SyncService>,
}

static SOURCE_SYNC: Mutex<Vec<SourceSync>> = Mutex::new(Vec::new());

fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

/// Connects the sync service of the library database at `db`.
pub(crate) fn register_source_sync(db: &Path, catalog: Weak<Catalog>, service: Weak<SyncService>) {
    let mut all = SOURCE_SYNC.lock().unwrap_or_else(|e| e.into_inner());
    all.retain(|s| s.catalog.strong_count() > 0 && !same_file(&s.db, db));
    all.push(SourceSync {
        db: db.to_path_buf(),
        catalog,
        service,
    });
}

/// The sources of the library database at `db` changed: queue sync ops for
/// synced sources and send them. Nothing happens while the device is not
/// paired. A failure leaves the edit in place; it goes out with the next
/// change or when sync starts.
pub(crate) fn sources_changed(db: &Path) {
    let targets: Vec<(Weak<Catalog>, Weak<SyncService>)> = SOURCE_SYNC
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|s| same_file(&s.db, db))
        .map(|s| (s.catalog.clone(), s.service.clone()))
        .collect();
    for (catalog, service) in targets {
        let Some(catalog) = catalog.upgrade() else {
            continue;
        };
        match catalog.sync_sources() {
            Ok(n) if n > 0 => {
                if let Some(service) = service.upgrade() {
                    service.flush();
                }
            }
            // Not queued now (a locked store, a busy database): the next
            // change or sync start reconciles it again.
            Ok(_) | Err(_) => {}
        }
    }
}
