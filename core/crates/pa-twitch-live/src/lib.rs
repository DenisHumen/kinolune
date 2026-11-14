// SPDX-License-Identifier: MPL-2.0
//! "X is live" notifications without any cloud (TWI-009).
//!
//! The project runs no servers (ADR-0004), so nobody can receive Twitch
//! webhooks for our users. Everything happens on the user's device or the
//! user's own media server, with the user's own token:
//!
//! - **EventSub WebSocket** ([`LiveMonitor`] with a [`WsConnector`]) while
//!   signed in: `stream.online` within seconds. A user-token session may
//!   spend `max_total_cost` (10) on subscriptions to broadcasters that did
//!   not authorise us, one per subscription, so [`SubscriptionPlan`] gives
//!   `stream.online` to about ten channels — notifications on first — and
//!   spends what is left on `channel.update` and `stream.offline` for
//!   chapter precision.
//! - **Polling** `GET /streams` for all watched channels on a
//!   battery-aware schedule with backoff ([`PollPolicy`]): 60 s on mains,
//!   90 s on battery, 5 min in battery saver. It announces the channels
//!   EventSub does not cover and keeps categories and ends current.
//! - **Background checks** ([`check_once`]) for platforms that do not keep
//!   the app running (Android WorkManager, at best every 15 minutes).
//! - **Gate** ([`NotificationGate`]): one notification per broadcast, only
//!   for channels with the toggle on, never for a broadcast that was
//!   already running when we started looking.
//! - **Chapters** (TWI-008): every event goes through
//!   `pa_twitch::chapters::TimelineTracker`; timelines are stored
//!   ([`TimelineStore`], `library` feature) and, when a broadcast ends,
//!   mapped onto its archive VOD.
//!
//! [`demo::FakeTwitch`] is a made-up Twitch (Helix and an EventSub
//! WebSocket server) for the tests and the app's demo mode.
//!
//! [`WsConnector`]: pa_twitch_chat::WsConnector

mod api;
mod check;
pub mod demo;
mod eventsub;
mod gate;
mod monitor;
mod plan;
mod schedule;
mod store;

pub use api::{LiveApi, LiveError, TokioClock};
pub use check::{CheckReport, check_once};
pub use eventsub::{Coverage, DEFAULT_EVENTSUB_URL, EventSubSettings, EventSubState};
pub use gate::{DEFAULT_ALERT_MAX_AGE, LiveAlert, NotificationGate};
pub use monitor::{LiveMonitor, MonitorConfig, MonitorDeps, MonitorEvent, MonitorStatus};
pub use plan::{
    DEFAULT_MAX_TOTAL_COST, PlannedSub, SUB_OFFLINE, SUB_ONLINE, SUB_UPDATE, SubscriptionPlan,
    WatchedChannel,
};
pub use schedule::{PollPolicy, PowerState};
#[cfg(feature = "library")]
pub use store::{LibraryTimelineStore, RESUME_MAX_AGE_MS};
pub use store::{NoStore, TimelineStore};
