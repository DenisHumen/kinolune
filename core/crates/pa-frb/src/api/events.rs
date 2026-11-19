// SPDX-License-Identifier: MPL-2.0
//! The event stream of the core (CORE-009, ADR-0121).
//!
//! [`core_events`] is one stream for the whole app: library changes, scan
//! progress, jobs, sync and playback, from every open library of the
//! process. The UI subscribes once (for example in a provider at the top
//! of the widget tree) and refreshes what an event names; no polling.
//!
//! An event is a flat record: [`CoreEvent::domain`] says which payload
//! field is set (the field is named like the domain: `scan_progress` for
//! `ScanProgress`). Calls with their own stream (`CoreLibrary.scan`,
//! `CoreSync.events`, `CoreOffline.watch`) keep it: the bus repeats the
//! same updates for whoever else listens (a badge, a notification), and
//! carries no secrets or remote-input text.

use std::path::Path;

use flutter_rust_bridge::frb;

use crate::api::library::CoreScanUpdate;
use crate::api::sync::CoreSyncEventKind;
use crate::frb_generated::StreamSink;

/// What an event is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreEventDomain {
    /// Library contents changed: `library_changed` is set.
    LibraryChanged,
    /// Progress of a scan: `scan_progress` is set.
    ScanProgress,
    /// A background job changed: `job_changed` is set.
    JobChanged,
    /// The device family: `sync_changed` is set.
    SyncChanged,
    /// A playback position was saved: `playback_saved` is set.
    PlaybackSaved,
}

/// Why the library changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreLibraryChange {
    /// A scan or a deferred probe finished.
    Scan,
    /// The change watcher rescanned folders (LIB-005).
    Watcher,
    /// Provider metadata arrived.
    Metadata,
    /// Changes from another device of the family arrived.
    OtherDevice,
}

/// A library change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLibraryEvent {
    pub change: CoreLibraryChange,
    /// The source it is about, if one.
    pub source_id: Option<i64>,
}

/// What happened to a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreJobChange {
    Added,
    /// `state` is the new state.
    StateChanged,
    /// `progress`, `bytes_done`, `bytes_total` are set.
    Progress,
    Removed,
}

/// A change of a job of the persistent queue (CORE-011).
#[derive(Debug, Clone, PartialEq)]
pub struct CoreJobEvent {
    pub job_id: i64,
    pub change: CoreJobChange,
    /// Job kind (`twitch.vod_download`, `offline.copy`…); empty for
    /// `Progress` and `Removed`.
    pub kind: String,
    /// `queued`, `waiting_resources`, `running`, `paused`, `retrying`,
    /// `done`, `failed` or `cancelled`, for `Added` and `StateChanged`.
    pub state: Option<String>,
    /// 0.0…1.0 if known.
    pub progress: Option<f64>,
    pub bytes_done: i64,
    pub bytes_total: Option<i64>,
}

/// A playback position saved through `CoreLibrary.save_progress`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePlaybackEvent {
    pub item_id: String,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub watched: bool,
}

/// One event of the core; exactly the payload named by `domain` is set.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreEvent {
    pub domain: CoreEventDomain,
    /// The library database (`CoreLibrary.open` path) the event is about:
    /// an app with several libraries (or tests in one process) tells them
    /// apart by it.
    pub library_path: String,
    pub library_changed: Option<CoreLibraryEvent>,
    pub scan_progress: Option<CoreScanUpdate>,
    pub job_changed: Option<CoreJobEvent>,
    /// The kind of the sync service event; its data stays on
    /// `CoreSync.events`.
    pub sync_changed: Option<CoreSyncEventKind>,
    pub playback_saved: Option<CorePlaybackEvent>,
}

impl CoreEvent {
    #[frb(ignore)]
    fn of(domain: CoreEventDomain, db: &Path) -> Self {
        Self {
            domain,
            library_path: db.to_string_lossy().into_owned(),
            library_changed: None,
            scan_progress: None,
            job_changed: None,
            sync_changed: None,
            playback_saved: None,
        }
    }

    #[frb(ignore)]
    pub(crate) fn library(db: &Path, event: CoreLibraryEvent) -> Self {
        Self {
            library_changed: Some(event),
            ..Self::of(CoreEventDomain::LibraryChanged, db)
        }
    }

    #[frb(ignore)]
    pub(crate) fn scan(db: &Path, update: CoreScanUpdate) -> Self {
        Self {
            scan_progress: Some(update),
            ..Self::of(CoreEventDomain::ScanProgress, db)
        }
    }

    #[frb(ignore)]
    pub(crate) fn job(db: &Path, event: CoreJobEvent) -> Self {
        Self {
            job_changed: Some(event),
            ..Self::of(CoreEventDomain::JobChanged, db)
        }
    }

    #[frb(ignore)]
    pub(crate) fn sync(db: &Path, kind: CoreSyncEventKind) -> Self {
        Self {
            sync_changed: Some(kind),
            ..Self::of(CoreEventDomain::SyncChanged, db)
        }
    }

    #[frb(ignore)]
    pub(crate) fn playback(db: &Path, event: CorePlaybackEvent) -> Self {
        Self {
            playback_saved: Some(event),
            ..Self::of(CoreEventDomain::PlaybackSaved, db)
        }
    }
}

// The Dart `coreEvents()` (hand-written in pa_core) opens and closes a
// stream with the three calls below. They are sync on purpose (BUG-007):
// the subscription exists before `coreEvents().listen` returns, so no
// event of a later call is lost, and a cancel ends the stream at once
// instead of at its next event.

/// The id for [`core_events_listen`]. In Dart use `coreEvents()`.
#[frb(sync)]
pub fn core_events_next_id() -> i64 {
    crate::bus::next_stream_id()
}

/// All events of the core from now on, until [`core_events_close`] with
/// the same `id`. Any number of streams may be open. In Dart use
/// `coreEvents()`.
#[frb(sync)]
pub fn core_events_listen(id: i64, sink: StreamSink<CoreEvent>) {
    crate::bus::open_stream(id, move |event| sink.add(event.clone()).is_ok());
}

/// Ends the stream `id` now: Dart gets the end of the stream right away.
/// In Dart use `coreEvents()`, whose `cancel()` calls this.
#[frb(sync)]
pub fn core_events_close(id: i64) {
    crate::bus::close_stream(id);
}

/// Emits `count` ticks `0, 1, …, count - 1` into `sink`, then closes the stream.
///
/// A self-test of Rust → Dart streams; stops early if Dart has cancelled
/// the subscription.
pub async fn tick_stream(count: u32, sink: StreamSink<u32>) {
    for tick in ticks(count) {
        if sink.add(tick).is_err() {
            break;
        }
    }
}

fn ticks(count: u32) -> impl Iterator<Item = u32> {
    0..count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_count_up_from_zero() {
        assert_eq!(ticks(3).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert_eq!(ticks(0).count(), 0);
    }

    #[test]
    fn constructors_set_only_their_payload() {
        let db = Path::new("/db");
        let e = CoreEvent::sync(db, CoreSyncEventKind::DevicesChanged);
        assert_eq!(e.library_path, "/db");
        assert_eq!(e.domain, CoreEventDomain::SyncChanged);
        assert!(e.library_changed.is_none() && e.scan_progress.is_none());
        assert!(e.job_changed.is_none() && e.playback_saved.is_none());
        let e = CoreEvent::playback(
            db,
            CorePlaybackEvent {
                item_id: "m1".into(),
                position_ms: 1,
                duration_ms: 2,
                watched: false,
            },
        );
        assert_eq!(e.domain, CoreEventDomain::PlaybackSaved);
        assert!(e.sync_changed.is_none());
    }
}
