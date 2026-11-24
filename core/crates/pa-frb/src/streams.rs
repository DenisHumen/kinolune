// SPDX-License-Identifier: MPL-2.0
//! Long streams that Dart closes by id (BUG-007, BUG-008, ADR-0121).
//!
//! A flutter_rust_bridge stream notices a Dart cancel only at its next
//! event: on a quiet stream `cancel()` does not complete, the open port
//! keeps the isolate alive, and the core keeps its subscription. So every
//! long stream (the event bus, `CoreSync.events`, `CoreDiscovery.watch`,
//! `CoreOffline.watch`) takes an id from [`next_id`] and registers how to
//! end it; `core_stream_close(id)` ends it at once: its `StreamSink` is
//! dropped, which tells Dart that the stream is closed.
//!
//! A stream fed by a thread holds a [`Closable`] sink: after the close the
//! thread sees it closed within [`POLL`] (or at its next event) and drops
//! its subscription in the core.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

type Closer = Box<dyn FnOnce() + Send>;

/// Open streams: their id and how to end them.
static STREAMS: Mutex<Vec<(i64, Closer)>> = Mutex::new(Vec::new());
static NEXT: AtomicI64 = AtomicI64::new(1);

/// How often a thread blocked on a quiet subscription checks for a close.
pub(crate) const POLL: Duration = Duration::from_millis(250);

/// A new stream id, unique in the process (every isolate shares it).
pub(crate) fn next_id() -> i64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Registers how to end the stream `id`. A stream whose port closed
/// without a close (a hot restart) keeps its entry until its feeder ends;
/// ids are never reused.
pub(crate) fn register(id: i64, closer: impl FnOnce() + Send + 'static) {
    STREAMS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push((id, Box::new(closer)));
}

/// Forgets the stream `id` without running its closer: it ended by itself.
fn forget(id: i64) {
    let mut streams = STREAMS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(i) = streams.iter().position(|(sid, _)| *sid == id) {
        drop(streams.swap_remove(i));
    }
}

/// Ends the stream `id` now. Returns whether it was open.
pub(crate) fn close(id: i64) -> bool {
    let closer = {
        let mut streams = STREAMS.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(i) = streams.iter().position(|(sid, _)| *sid == id) else {
            return false;
        };
        streams.swap_remove(i).1
    };
    // Outside the lock: the closer drops a sink, which may take a while.
    closer();
    true
}

/// A sink (`StreamSink`) of the stream `id` that [`close`] can take away
/// from the thread that feeds it. Dropping it forgets the stream.
pub(crate) struct Closable<S: Send + 'static> {
    id: i64,
    slot: Arc<Mutex<Option<S>>>,
}

impl<S: Send + 'static> Closable<S> {
    /// Registers `sink` as the stream `id`.
    pub(crate) fn open(id: i64, sink: S) -> Self {
        let slot = Arc::new(Mutex::new(Some(sink)));
        let closer = Arc::clone(&slot);
        register(id, move || {
            let sink = closer.lock().unwrap_or_else(PoisonError::into_inner).take();
            drop(sink);
        });
        Self { id, slot }
    }

    /// Hands the sink to `send` while the stream is open; `send` says
    /// whether Dart still listens. Returns `false` once the stream is
    /// closed: the feeder stops.
    pub(crate) fn send(&self, send: impl FnOnce(&S) -> bool) -> bool {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        match slot.as_ref() {
            Some(sink) if send(sink) => true,
            Some(_) => {
                // Dart is gone (its port closed): drop the sink too.
                *slot = None;
                false
            }
            None => false,
        }
    }

    /// Whether the stream is still open.
    pub(crate) fn is_open(&self) -> bool {
        self.slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// The next event of `rx`, or `None` once the stream is closed or the
    /// subscription ended. A close is noticed within [`POLL`].
    pub(crate) fn recv<E>(&self, rx: &Receiver<E>) -> Option<E> {
        loop {
            if !self.is_open() {
                return None;
            }
            match rx.recv_timeout(POLL) {
                Ok(e) => return Some(e),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
    }
}

impl<S: Send + 'static> Drop for Closable<S> {
    fn drop(&mut self) {
        forget(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Instant;

    /// A stand-in sink: counts what it got, and its drop is the end of
    /// the Dart stream.
    struct Sink(Arc<Mutex<Vec<u32>>>);
    impl Sink {
        fn add(&self, v: u32) -> bool {
            self.0.lock().unwrap().push(v);
            true
        }
    }

    #[test]
    fn ids_are_unique() {
        let (a, b) = (next_id(), next_id());
        assert_ne!(a, b);
    }

    /// BUG-008: a close on a quiet stream drops its sink at once and the
    /// feeder lets go of its subscription without another event.
    #[test]
    fn a_close_ends_a_quiet_stream_at_once() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let id = next_id();
        let sink = Closable::open(id, Sink(Arc::clone(&got)));
        let (tx, rx) = mpsc::channel::<u32>();
        let feeder = std::thread::spawn(move || {
            while let Some(v) = sink.recv(&rx) {
                if !sink.send(|s| s.add(v)) {
                    break;
                }
            }
        });
        tx.send(1).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while got.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "the event arrives");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(Arc::strong_count(&got), 2);

        // Nothing more is sent: the close alone ends the stream.
        assert!(close(id));
        assert_eq!(Arc::strong_count(&got), 1, "the sink is dropped at once");
        feeder.join().unwrap();
        assert!(tx.send(2).is_err(), "the subscription is gone");
        assert!(!close(id), "closed once");
    }

    #[test]
    fn a_stream_that_ends_by_itself_is_forgotten() {
        let id = next_id();
        let sink = Closable::open(id, Sink(Arc::new(Mutex::new(Vec::new()))));
        let (tx, rx) = mpsc::channel::<u32>();
        drop(tx);
        assert!(sink.recv(&rx).is_none());
        drop(sink);
        assert!(!close(id), "nothing left to close");
    }

    #[test]
    fn a_gone_listener_stops_the_feeder() {
        let id = next_id();
        let sink = Closable::open(id, Sink(Arc::new(Mutex::new(Vec::new()))));
        assert!(!sink.send(|_| false));
        assert!(!sink.is_open());
        drop(sink);
        assert!(!close(id));
    }
}
