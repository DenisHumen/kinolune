// SPDX-License-Identifier: MPL-2.0
//! The event bus of the Dart facade (CORE-009): one per process, read by
//! [`core_events`](crate::api::events::core_events).
//!
//! Facades publish here what they learn from the core crates: scan
//! updates, job events of the shared queues, sync service events, saved
//! positions. Publishing is cheap while nobody listens.

use std::path::Path;

use pa_runtime::EventBus;

use crate::api::events::{CoreEvent, CoreJobChange, CoreJobEvent, CoreLibraryChange, CoreLibraryEvent};
use crate::api::sync::CoreSyncEventKind;

static BUS: EventBus<CoreEvent> = EventBus::new();

/// Sends `event` to every open `core_events` stream.
pub(crate) fn publish(event: CoreEvent) {
    BUS.publish(&event);
}

/// Whether any stream is open: skip building events nobody reads.
pub(crate) fn listening() -> bool {
    BUS.has_subscribers()
}

pub(crate) fn subscribe(callback: impl Fn(&CoreEvent) -> bool + Send + Sync + 'static) {
    BUS.subscribe(callback);
}

/// The library at `db` changed.
pub(crate) fn library_changed(db: &Path, change: CoreLibraryChange, source_id: Option<i64>) {
    publish(CoreEvent::library(CoreLibraryEvent {
        change,
        library_path: db.to_string_lossy().into_owned(),
        source_id,
    }));
}

fn signed(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

impl From<pa_jobs::JobEvent> for CoreJobEvent {
    fn from(e: pa_jobs::JobEvent) -> Self {
        use pa_jobs::JobEvent as E;
        let base = |job_id, change| CoreJobEvent {
            job_id,
            change,
            kind: String::new(),
            state: None,
            progress: None,
            bytes_done: 0,
            bytes_total: None,
        };
        match e {
            E::Added { id, kind } => CoreJobEvent {
                kind,
                state: Some(pa_jobs::JobState::Queued.as_str().to_owned()),
                ..base(id, CoreJobChange::Added)
            },
            E::StateChanged { id, kind, to, .. } => CoreJobEvent {
                kind,
                state: Some(to.as_str().to_owned()),
                ..base(id, CoreJobChange::StateChanged)
            },
            E::Progress {
                id,
                progress,
                bytes_done,
                bytes_total,
            } => CoreJobEvent {
                progress,
                bytes_done: signed(bytes_done),
                bytes_total: bytes_total.map(signed),
                ..base(id, CoreJobChange::Progress)
            },
            E::Removed { id } => base(id, CoreJobChange::Removed),
        }
    }
}

/// Repeats the events of a job queue on the bus until the queue is gone.
pub(crate) fn forward_jobs(queue: &pa_jobs::JobQueue) {
    let rx = queue.subscribe();
    std::thread::Builder::new()
        .name("pa-jobs-events".into())
        .spawn(move || {
            for event in rx {
                if listening() {
                    publish(CoreEvent::job(event.into()));
                }
            }
        })
        .map(drop)
        .unwrap_or_default();
}

/// Repeats the kinds of sync service events on the bus until the service
/// is dropped; a `LibraryChanged` also becomes a library event of `db`.
/// Payloads (positions of other profiles, remote input) stay on
/// `CoreSync.events`.
pub(crate) fn forward_sync(
    rx: std::sync::mpsc::Receiver<pa_catalog::pa_sync::lan::service::ServiceEvent>,
    db: std::path::PathBuf,
) {
    std::thread::Builder::new()
        .name("pa-sync-bus".into())
        .spawn(move || {
            for event in rx {
                let kind = crate::api::sync::CoreSyncEvent::from(event).kind;
                if kind == CoreSyncEventKind::Input {
                    continue;
                }
                publish(CoreEvent::sync(kind));
                if kind == CoreSyncEventKind::LibraryChanged {
                    library_changed(&db, CoreLibraryChange::Sync, None);
                }
            }
        })
        .map(drop)
        .unwrap_or_default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use pa_jobs::{JobEvent, JobState};

    #[test]
    fn job_events_keep_state_and_progress() {
        let e: CoreJobEvent = JobEvent::StateChanged {
            id: 7,
            kind: "offline.copy".into(),
            from: JobState::Queued,
            to: JobState::Running,
        }
        .into();
        assert_eq!(e.change, CoreJobChange::StateChanged);
        assert_eq!((e.job_id, e.kind.as_str()), (7, "offline.copy"));
        assert_eq!(e.state.as_deref(), Some("running"));

        let e: CoreJobEvent = JobEvent::Progress {
            id: 7,
            progress: Some(0.5),
            bytes_done: 10,
            bytes_total: Some(u64::MAX),
        }
        .into();
        assert_eq!(e.change, CoreJobChange::Progress);
        assert_eq!(e.progress, Some(0.5));
        assert_eq!((e.bytes_done, e.bytes_total), (10, Some(i64::MAX)));
    }

    #[test]
    fn library_events_reach_subscribers() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = std::sync::Arc::clone(&seen);
        subscribe(move |e| {
            if let Some(l) = &e.library
                && l.library_path == "/tmp/bus-test.db"
            {
                s.lock().unwrap().push((l.change, l.source_id));
            }
            true
        });
        library_changed(Path::new("/tmp/bus-test.db"), CoreLibraryChange::Scan, Some(3));
        assert_eq!(*seen.lock().unwrap(), [(CoreLibraryChange::Scan, Some(3))]);
    }
}
