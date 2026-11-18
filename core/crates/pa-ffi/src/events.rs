// SPDX-License-Identifier: MPL-2.0
//! Event stream of the core for Kotlin and Swift (CORE-009, ADR-0120).
//!
//! The app registers one [`CoreEventListener`] with
//! [`core_events_subscribe`] and keeps the returned
//! [`CoreEventSubscription`]; dropping it (or calling
//! [`CoreEventSubscription::cancel`]) stops the events. Events come for
//! what this facade exports: the lifecycle and the settings. Domains that
//! are exported to UniFFI later (library, scan, jobs, sync, playback) add
//! their variants to [`CoreEvent`] and publish into the same bus, as the
//! Dart facade does already.

use std::sync::Arc;

use pa_runtime::{EventBus, SubscriptionId};

use crate::CoreLifecycle;

/// An event of the core.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreEvent {
    /// The lifecycle state changed (also once after `core_init` and once
    /// with `ShutDown`).
    Lifecycle {
        /// The new state.
        state: CoreLifecycle,
    },
    /// Settings were written through this facade: re-read these keys.
    /// An import lists every key of the document.
    SettingsChanged {
        /// Changed or removed keys.
        keys: Vec<String>,
    },
}

/// Receiver of core events, implemented by the app.
///
/// Called synchronously on a core thread: return fast (hand the event to
/// your own queue) and do not call core functions from inside.
#[uniffi::export(with_foreign)]
pub trait CoreEventListener: Send + Sync {
    /// One event.
    fn on_event(&self, event: CoreEvent);
}

static BUS: EventBus<CoreEvent> = EventBus::new();

/// Sends `event` to every listener.
pub(crate) fn publish(event: CoreEvent) {
    BUS.publish(&event);
}

/// A live subscription. Dropping it unsubscribes.
#[derive(Debug, uniffi::Object)]
pub struct CoreEventSubscription {
    id: SubscriptionId,
}

#[uniffi::export]
impl CoreEventSubscription {
    /// Stops the events now; returns whether the subscription was live.
    pub fn cancel(&self) -> bool {
        BUS.unsubscribe(self.id)
    }
}

impl Drop for CoreEventSubscription {
    fn drop(&mut self) {
        BUS.unsubscribe(self.id);
    }
}

/// Delivers every core event from now on to `listener`.
#[uniffi::export]
pub fn core_events_subscribe(listener: Arc<dyn CoreEventListener>) -> Arc<CoreEventSubscription> {
    let id = BUS.subscribe(move |event| {
        listener.on_event(event.clone());
        true
    });
    Arc::new(CoreEventSubscription { id })
}
