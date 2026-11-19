// SPDX-License-Identifier: MPL-2.0
//! The event bus of the Dart facade (CORE-009): one per process, read by
//! [`core_events_listen`](crate::api::events::core_events_listen).
//!
//! Facades publish here what they learn from the core crates: scan
//! updates, job events of the shared queues, sync service events, saved
//! positions. Publishing is cheap while nobody listens.
//!
//! Dart closes its streams by id ([`close_stream`], BUG-007): a
//! flutter_rust_bridge stream notices a Dart cancel only at its next event,
//! and on a quiet bus that never comes. Until then `cancel()` does not
//! complete and the open port keeps the isolate alive.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

use pa_runtime::{EventBus, SubscriptionId};

use crate::api::events::{
    CoreEvent, CoreJobChange, CoreJobEvent, CoreLibraryChange, CoreLibraryEvent,
};
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

pub(crate) fn subscribe(
    callback: impl Fn(&CoreEvent) -> bool + Send + Sync + 'static,
) -> SubscriptionId {
    BUS.subscribe(callback)
}

/// Open Dart streams: the id Dart closes them by, and their subscription.
static STREAMS: Mutex<Vec<(i64, SubscriptionId)>> = Mutex::new(Vec::new());
static NEXT_STREAM: AtomicI64 = AtomicI64::new(1);

/// A new stream id, unique in the process (every isolate shares the bus).
pub(crate) fn next_stream_id() -> i64 {
    NEXT_STREAM.fetch_add(1, Ordering::Relaxed)
}

/// Subscribes `callback` as the stream `id` until [`close_stream`].
pub(crate) fn open_stream(id: i64, callback: impl Fn(&CoreEvent) -> bool + Send + Sync + 'static) {
    let sub = subscribe(callback);
    // A stream whose port closed without a close (a hot restart) is
    // dropped by the bus but keeps its entry here; ids are never reused,
    // so the entry is only a few bytes.
    STREAMS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((id, sub));
}

/// Ends the stream `id`: the bus drops its callback and with it the
/// `StreamSink`, which tells Dart that the stream is closed. Returns
/// whether the stream was open.
pub(crate) fn close_stream(id: i64) -> bool {
    let sub = {
        let mut streams = STREAMS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(i) = streams.iter().position(|(sid, _)| *sid == id) else {
            return false;
        };
        streams.swap_remove(i).1
    };
    BUS.unsubscribe(sub)
}

/// The library at `db` changed.
pub(crate) fn library_changed(db: &Path, change: CoreLibraryChange, source_id: Option<i64>) {
    publish(CoreEvent::library(
        db,
        CoreLibraryEvent { change, source_id },
    ));
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

/// Repeats the events of the job queue of the library at `db` on the bus
/// until the queue is gone.
pub(crate) fn forward_jobs(queue: &pa_jobs::JobQueue, db: std::path::PathBuf) {
    let rx = queue.subscribe();
    std::thread::Builder::new()
        .name("pa-jobs-events".into())
        .spawn(move || {
            for event in rx {
                if listening() {
                    publish(CoreEvent::job(&db, event.into()));
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
                publish(CoreEvent::sync(&db, kind));
                if kind == CoreSyncEventKind::LibraryChanged {
                    library_changed(&db, CoreLibraryChange::OtherDevice, None);
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
            if let Some(l) = &e.library_changed
                && e.library_path == "/tmp/bus-test.db"
            {
                s.lock().unwrap().push((l.change, l.source_id));
            }
            true
        });
        library_changed(
            Path::new("/tmp/bus-test.db"),
            CoreLibraryChange::Scan,
            Some(3),
        );
        assert_eq!(*seen.lock().unwrap(), [(CoreLibraryChange::Scan, Some(3))]);
    }

    /// BUG-007: closing a stream drops its callback (and so the Dart sink)
    /// at once, without waiting for another event.
    #[test]
    fn closing_a_stream_drops_its_callback() {
        let db = "/tmp/bus-close-test.db";
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (a, b) = (next_stream_id(), next_stream_id());
        assert_ne!(a, b);
        let s = std::sync::Arc::clone(&seen);
        open_stream(a, move |e| {
            if e.library_path == db {
                s.fetch_add(1, Ordering::SeqCst);
            }
            true
        });
        library_changed(Path::new(db), CoreLibraryChange::Scan, None);
        assert_eq!(seen.load(Ordering::SeqCst), 1);
        assert_eq!(std::sync::Arc::strong_count(&seen), 2);

        assert!(close_stream(a));
        assert_eq!(
            std::sync::Arc::strong_count(&seen),
            1,
            "the callback is dropped"
        );
        library_changed(Path::new(db), CoreLibraryChange::Scan, None);
        assert_eq!(seen.load(Ordering::SeqCst), 1);
        assert!(!close_stream(a), "closed once");
        assert!(!close_stream(b), "never opened");
    }
}
