// SPDX-License-Identifier: MPL-2.0
//! The event stream of the core (CORE-009, ADR-0120).
//!
//! [`core_events`] is one stream for the whole app: library changes, scan
//! progress, jobs, sync and playback, from every open library of the
//! process. The UI subscribes once (for example in a provider at the top
//! of the widget tree) and refreshes what an event names; no polling.
//!
//! An event is a flat record: [`CoreEvent::domain`] says which payload
//! field is set. Calls with their own stream (`CoreLibrary.scan`,
//! `CoreSync.events`, `CoreOffline.watch`) keep it: the bus repeats the
//! same updates for whoever else listens (a badge, a notification), and
//! carries no secrets or remote-input text.

use flutter_rust_bridge::frb;

use crate::api::library::CoreScanUpdate;
use crate::api::sync::CoreSyncEventKind;
use crate::frb_generated::StreamSink;

/// What an event is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreEventDomain {
    /// Library contents changed: `library` is set.
    Library,
    /// Progress of a scan: `scan` is set.
    Scan,
    /// A background job changed: `job` is set.
    Job,
    /// The device family: `sync` is set.
    Sync,
    /// A playback position was saved: `playback` is set.
    Playback,
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
    Sync,
}

/// A library change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLibraryEvent {
    pub change: CoreLibraryChange,
    /// The library database the change is in.
    pub library_path: String,
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
    pub library: Option<CoreLibraryEvent>,
    pub scan: Option<CoreScanUpdate>,
    pub job: Option<CoreJobEvent>,
    pub sync: Option<CoreSyncEventKind>,
    pub playback: Option<CorePlaybackEvent>,
}

impl CoreEvent {
    #[frb(ignore)]
    fn of(domain: CoreEventDomain) -> Self {
        Self {
            domain,
            library: None,
            scan: None,
            job: None,
            sync: None,
            playback: None,
        }
    }

    #[frb(ignore)]
    pub(crate) fn library(event: CoreLibraryEvent) -> Self {
        Self {
            library: Some(event),
            ..Self::of(CoreEventDomain::Library)
        }
    }

    #[frb(ignore)]
    pub(crate) fn scan(update: CoreScanUpdate) -> Self {
        Self {
            scan: Some(update),
            ..Self::of(CoreEventDomain::Scan)
        }
    }

    #[frb(ignore)]
    pub(crate) fn job(event: CoreJobEvent) -> Self {
        Self {
            job: Some(event),
            ..Self::of(CoreEventDomain::Job)
        }
    }

    #[frb(ignore)]
    pub(crate) fn sync(kind: CoreSyncEventKind) -> Self {
        Self {
            sync: Some(kind),
            ..Self::of(CoreEventDomain::Sync)
        }
    }

    #[frb(ignore)]
    pub(crate) fn playback(event: CorePlaybackEvent) -> Self {
        Self {
            playback: Some(event),
            ..Self::of(CoreEventDomain::Playback)
        }
    }
}

/// All events of the core from now on. Returns at once; the stream stays
/// open until Dart cancels it. Any number of streams may be open.
pub fn core_events(sink: StreamSink<CoreEvent>) {
    crate::bus::subscribe(move |event| sink.add(event.clone()).is_ok());
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
        let e = CoreEvent::sync(CoreSyncEventKind::DevicesChanged);
        assert_eq!(e.domain, CoreEventDomain::Sync);
        assert!(e.library.is_none() && e.scan.is_none() && e.job.is_none());
        assert!(e.playback.is_none());
        let e = CoreEvent::playback(CorePlaybackEvent {
            item_id: "m1".into(),
            position_ms: 1,
            duration_ms: 2,
            watched: false,
        });
        assert_eq!(e.domain, CoreEventDomain::Playback);
        assert!(e.sync.is_none());
    }
}
