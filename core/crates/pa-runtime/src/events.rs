// SPDX-License-Identifier: MPL-2.0
//! Event bus of the core (CORE-009, ADR-0120).
//!
//! One [`EventBus`] per facade and process carries what the UI must hear
//! about without asking: library changes, scan progress, jobs, sync and
//! playback. Subsystems [`publish`](EventBus::publish); the facade
//! subscribes one callback per UI stream (a flutter_rust_bridge
//! `StreamSink`, a UniFFI callback interface).
//!
//! Delivery is synchronous on the publishing thread and lossless: there is
//! no buffer that could fill up. A subscriber must therefore return fast
//! (posting to a Dart port or a Swift queue is fast) and must not block on
//! the publisher. Callbacks run outside the bus lock, so a callback may
//! subscribe, unsubscribe or publish itself. A callback returning `false`
//! is removed (the stream on the other side was closed).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

type Callback<E> = Arc<dyn Fn(&E) -> bool + Send + Sync>;

/// Handle of one subscription, for [`EventBus::unsubscribe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriptionId(u64);

/// A publish/subscribe bus for events of type `E`.
///
/// `const`-constructible, so a facade keeps it in a `static`.
pub struct EventBus<E> {
    subscribers: Mutex<Vec<(SubscriptionId, Callback<E>)>>,
    next_id: AtomicU64,
}

impl<E> Default for EventBus<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> std::fmt::Debug for EventBus<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBus")
            .field("subscribers", &self.subscriber_count())
            .finish()
    }
}

impl<E> EventBus<E> {
    /// An empty bus.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(SubscriptionId, Callback<E>)>> {
        self.subscribers.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Calls `callback` with every event published from now on, until it
    /// returns `false` or [`unsubscribe`](Self::unsubscribe) is called.
    pub fn subscribe(
        &self,
        callback: impl Fn(&E) -> bool + Send + Sync + 'static,
    ) -> SubscriptionId {
        let id = SubscriptionId(self.next_id.fetch_add(1, Ordering::Relaxed));
        self.lock().push((id, Arc::new(callback)));
        id
    }

    /// Removes a subscription; returns whether it was there.
    pub fn unsubscribe(&self, id: SubscriptionId) -> bool {
        let mut subs = self.lock();
        let before = subs.len();
        subs.retain(|(sid, _)| *sid != id);
        subs.len() != before
    }

    /// Delivers `event` to every subscriber, in subscription order. Cheap
    /// when nobody listens.
    pub fn publish(&self, event: &E) {
        let targets: Vec<(SubscriptionId, Callback<E>)> = {
            let subs = self.lock();
            if subs.is_empty() {
                return;
            }
            subs.clone()
        };
        let closed: Vec<SubscriptionId> = targets
            .into_iter()
            .filter(|(_, callback)| !callback(event))
            .map(|(id, _)| id)
            .collect();
        if !closed.is_empty() {
            self.lock().retain(|(id, _)| !closed.contains(id));
        }
    }

    /// Whether anyone listens: a publisher may skip building costly events.
    #[must_use]
    pub fn has_subscribers(&self) -> bool {
        !self.lock().is_empty()
    }

    /// Number of live subscriptions.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn delivers_to_every_subscriber_in_order() {
        let bus = EventBus::<u32>::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        for tag in ["a", "b"] {
            let seen = Arc::clone(&seen);
            bus.subscribe(move |e| {
                seen.lock().unwrap().push(format!("{tag}{e}"));
                true
            });
        }
        bus.publish(&1);
        bus.publish(&2);
        assert_eq!(*seen.lock().unwrap(), ["a1", "b1", "a2", "b2"]);
    }

    #[test]
    fn closed_subscriber_is_dropped() {
        let bus = EventBus::<u32>::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        bus.subscribe(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            false
        });
        bus.publish(&1);
        bus.publish(&2);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!bus.has_subscribers());
    }

    #[test]
    fn unsubscribe_stops_delivery() {
        let bus = EventBus::<u32>::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        let id = bus.subscribe(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            true
        });
        bus.publish(&1);
        assert!(bus.unsubscribe(id));
        assert!(!bus.unsubscribe(id));
        bus.publish(&2);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn callback_may_use_the_bus() {
        static BUS: EventBus<u32> = EventBus::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = Arc::clone(&seen);
        BUS.subscribe(move |e| {
            s.lock().unwrap().push(*e);
            if *e == 1 {
                // Re-entrant publish and subscribe must not deadlock.
                BUS.subscribe(|_| true);
                BUS.publish(&2);
            }
            true
        });
        BUS.publish(&1);
        assert_eq!(*seen.lock().unwrap(), [1, 2]);
        assert_eq!(BUS.subscriber_count(), 2);
    }

    #[test]
    fn publish_from_many_threads() {
        let bus = Arc::new(EventBus::<u32>::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        bus.subscribe(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            true
        });
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let bus = Arc::clone(&bus);
                std::thread::spawn(move || (0..100).for_each(|i| bus.publish(&i)))
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        assert_eq!(calls.load(Ordering::SeqCst), 400);
    }
}
