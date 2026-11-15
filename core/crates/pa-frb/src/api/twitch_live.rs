// SPDX-License-Identifier: MPL-2.0
//! "X is live" notifications for Dart (TWI-009): [`CoreTwitchLive`] runs
//! the `pa-twitch-live` monitor (EventSub WebSocket within the cost budget,
//! polling `GET /streams` on a battery-aware schedule) on its own thread
//! while the app runs, and streams alerts and status to Dart, which shows
//! the local notifications. Background checks without a running app
//! (Android WorkManager) use [`super::twitch::CoreTwitch::check_live_once`].
//!
//! - [`super::twitch::CoreTwitch::live_monitor`]: the real one; needs a
//!   Twitch sign-in (Helix answers only with a token).
//! - [`CoreTwitchLive::demo`]: a made-up Twitch in process (demo mode,
//!   emulator checks); never touches the network.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use flutter_rust_bridge::frb;
use pa_twitch::{Clock, SystemClock};
use pa_twitch_live as live;

use crate::frb_generated::StreamSink;

/// A watched channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLiveChannel {
    /// Broadcaster id.
    pub user_id: String,
    /// Login.
    pub login: String,
    /// Display name.
    pub display_name: String,
    /// "X is live" notifications on.
    pub notify: bool,
}

impl From<CoreLiveChannel> for live::WatchedChannel {
    fn from(c: CoreLiveChannel) -> Self {
        Self {
            user_id: c.user_id,
            login: c.login,
            display_name: c.display_name,
            notify: c.notify,
        }
    }
}

/// Power state of the device; sets the polling interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreLivePower {
    /// Mains power: 60 s.
    Plugged,
    /// Battery: 90 s.
    Battery,
    /// Battery saver or low battery: 5 min.
    Saver,
}

impl From<CoreLivePower> for live::PowerState {
    fn from(p: CoreLivePower) -> Self {
        match p {
            CoreLivePower::Plugged => live::PowerState::Plugged,
            CoreLivePower::Battery => live::PowerState::Battery,
            CoreLivePower::Saver => live::PowerState::Saver,
        }
    }
}

/// An "X is live" notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLiveAlert {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    /// One notification per broadcast: use it as the notification id.
    pub stream_id: String,
    pub title: Option<String>,
    pub game_name: Option<String>,
    pub started_at_ms: i64,
    pub detected_at_ms: i64,
    /// `eventsub` or `polling`.
    pub source: String,
}

impl From<live::LiveAlert> for CoreLiveAlert {
    fn from(a: live::LiveAlert) -> Self {
        Self {
            user_id: a.user_id,
            login: a.login,
            display_name: a.display_name,
            stream_id: a.stream_id,
            title: a.title,
            game_name: a.game_name,
            started_at_ms: a.started_at_ms,
            detected_at_ms: a.detected_at_ms,
            source: a.source.as_str().to_owned(),
        }
    }
}

/// Health of the monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLiveStatus {
    /// `off`, `connecting`, `connected`, `reconnecting`, `stopped`.
    pub eventsub: String,
    /// Why EventSub stopped or reconnects.
    pub eventsub_reason: Option<String>,
    /// Channels announced by EventSub.
    pub eventsub_channels: u32,
    /// Channels announced by polling only.
    pub polled_only: u32,
    /// Twitch refused subscriptions for cost.
    pub budget_hit: bool,
    /// Seconds to the next poll.
    pub next_poll_secs: u32,
    /// Failed polls in a row.
    pub poll_errors: u32,
    pub last_error: Option<String>,
    pub last_poll_ms: Option<i64>,
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl From<live::MonitorStatus> for CoreLiveStatus {
    fn from(s: live::MonitorStatus) -> Self {
        let (eventsub, reason, budget_hit) = match &s.eventsub {
            live::EventSubState::Off => ("off", None, false),
            live::EventSubState::Connecting => ("connecting", None, false),
            live::EventSubState::Connected(c) => ("connected", None, c.budget_hit),
            live::EventSubState::Reconnecting { reason, .. } => {
                ("reconnecting", Some(reason.clone()), false)
            }
            live::EventSubState::Stopped { reason } => ("stopped", Some(reason.clone()), false),
        };
        Self {
            eventsub: eventsub.to_owned(),
            eventsub_reason: reason,
            eventsub_channels: count(s.eventsub_channels),
            polled_only: count(s.polled_only),
            budget_hit,
            next_poll_secs: u32::try_from(s.next_poll_in.as_secs()).unwrap_or(u32::MAX),
            poll_errors: s.poll_errors,
            last_error: s.last_error,
            last_poll_ms: s.last_poll_ms,
        }
    }
}

/// What a [`CoreLiveEvent`] carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreLiveEventKind {
    /// `alert` is set.
    Alert,
    /// `status` is set.
    Status,
    /// A broadcast of `user_id` started, changed or ended (`live` tells
    /// which); chapters are stored already.
    Timeline,
}

/// One event of the monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLiveEvent {
    pub kind: CoreLiveEventKind,
    pub alert: Option<CoreLiveAlert>,
    pub status: Option<CoreLiveStatus>,
    /// Timeline: the channel.
    pub user_id: Option<String>,
    /// Timeline: the broadcast is on air after this change.
    pub live: bool,
}

impl From<live::MonitorEvent> for CoreLiveEvent {
    fn from(e: live::MonitorEvent) -> Self {
        let base = |kind| CoreLiveEvent {
            kind,
            alert: None,
            status: None,
            user_id: None,
            live: false,
        };
        match e {
            live::MonitorEvent::Alert(a) => CoreLiveEvent {
                alert: Some(a.into()),
                ..base(CoreLiveEventKind::Alert)
            },
            live::MonitorEvent::Status(s) => CoreLiveEvent {
                status: Some(s.into()),
                ..base(CoreLiveEventKind::Status)
            },
            live::MonitorEvent::Timeline(change) => CoreLiveEvent {
                user_id: Some(change.timeline().user_id.clone()),
                live: !matches!(change, pa_twitch::chapters::TimelineChange::Ended(_)),
                ..base(CoreLiveEventKind::Timeline)
            },
        }
    }
}

/// Error of the live monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLiveError {
    /// `signed_out`, `io`, `network`, `twitch`.
    pub code: String,
    pub message: String,
}

impl CoreLiveError {
    pub(crate) fn new(code: &str, message: impl ToString) -> Self {
        Self {
            code: code.into(),
            message: message.to_string(),
        }
    }
}

impl From<live::LiveError> for CoreLiveError {
    fn from(e: live::LiveError) -> Self {
        let code = match &e {
            live::LiveError::Unauthorized(_) => "signed_out",
            live::LiveError::Network(_) => "network",
            _ => "twitch",
        };
        Self::new(code, e)
    }
}

pub(crate) enum Backend {
    /// Helix with the user's token, timelines in the library at `db_path`.
    Twitch {
        api: Arc<dyn live::LiveApi>,
        db_path: Option<String>,
    },
    /// The made-up Twitch; the first watched channel goes live after this.
    Demo(Duration),
}

struct Running {
    monitor: Arc<live::LiveMonitor>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.monitor.stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The live monitor of a running app: [`events`](Self::events) starts it
/// on a background thread (no tray icon, no service: it lives as long as
/// the app process), [`stop`](Self::stop) ends it.
#[frb(opaque)]
pub struct CoreTwitchLive {
    backend: Backend,
    running: Mutex<Option<Running>>,
    /// Rust-side listeners of every event (TWI-019: AutoRecord), called on
    /// the monitor thread before the event goes to Dart. They get every
    /// event, whatever profile is active (ADR-NNNN).
    observers: Arc<Mutex<Vec<Observer>>>,
    /// Whether an "X is live" alert may go to Dart now (the notify
    /// consumer); `None` lets every alert through (demo).
    alert_gate: Option<AlertGate>,
}

/// A Rust-side listener of monitor events.
pub(crate) type Observer = Box<dyn Fn(&live::MonitorEvent) + Send + Sync>;

/// Asked for every alert: `false` drops it (a restricted profile is
/// active, ADR-NNNN).
pub(crate) type AlertGate = Arc<dyn Fn() -> bool + Send + Sync>;

/// One monitor event: every observer (the record consumer) sees it; the
/// result is what goes to Dart (the notify consumer), `None` for an alert
/// the gate drops.
fn dispatch(
    event: live::MonitorEvent,
    observers: &Mutex<Vec<Observer>>,
    alert_gate: Option<&AlertGate>,
) -> Option<CoreLiveEvent> {
    for o in observers.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        o(&event);
    }
    if matches!(event, live::MonitorEvent::Alert(_)) && alert_gate.is_some_and(|allowed| !allowed())
    {
        return None;
    }
    Some(event.into())
}

impl CoreTwitchLive {
    pub(crate) fn with(backend: Backend) -> Self {
        Self {
            backend,
            running: Mutex::new(None),
            observers: Arc::new(Mutex::new(Vec::new())),
            alert_gate: None,
        }
    }

    /// Alerts reach Dart only while `gate` says so.
    #[frb(ignore)]
    pub(crate) fn with_alert_gate(mut self, gate: AlertGate) -> Self {
        self.alert_gate = Some(gate);
        self
    }

    /// Adds a Rust-side listener; it stays for the life of this object,
    /// across [`events`](Self::events) restarts.
    #[frb(ignore)]
    pub(crate) fn add_observer(&self, observer: Observer) {
        self.observers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(observer);
    }

    /// A made-up Twitch: the first watched channel goes live after
    /// `first_after_secs`, the second two minutes later. No network.
    #[frb(sync)]
    pub fn demo(first_after_secs: u32) -> CoreTwitchLive {
        Self::with(Backend::Demo(Duration::from_secs(u64::from(
            first_after_secs,
        ))))
    }

    /// Starts watching `channels` and streams events. A second call
    /// replaces the first monitor.
    pub fn events(
        &self,
        channels: Vec<CoreLiveChannel>,
        power: CoreLivePower,
        sink: StreamSink<CoreLiveEvent>,
    ) -> Result<(), CoreLiveError> {
        let channels: Vec<live::WatchedChannel> = channels.into_iter().map(Into::into).collect();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| CoreLiveError::new("io", e))?;
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let mut config = live::MonitorConfig::default();
        let guard = rt.enter();
        let (deps, script) = match &self.backend {
            Backend::Twitch { api, db_path } => {
                let store: Box<dyn live::TimelineStore> =
                    match db_path.as_deref().map(live::LibraryTimelineStore::open) {
                        Some(Ok(s)) => Box::new(s),
                        _ => Box::new(live::NoStore),
                    };
                let connector = pa_twitch_chat::TungsteniteConnector::new()
                    .ok()
                    .map(|c| Arc::new(c) as Arc<dyn pa_twitch_chat::WsConnector>);
                (
                    live::MonitorDeps {
                        api: api.clone(),
                        connector,
                        clock,
                        store,
                    },
                    None,
                )
            }
            Backend::Demo(first_after) => {
                let fake = live::demo::FakeTwitch::new(clock.clone());
                fake.ensure_channels(&channels);
                config.eventsub.url = live::demo::FAKE_EVENTSUB_URL.to_owned();
                let ids = channels
                    .iter()
                    .filter(|c| c.notify)
                    .map(|c| c.user_id.clone())
                    .collect();
                let script = live::demo::spawn_demo_script(fake.clone(), ids, *first_after);
                (
                    live::MonitorDeps {
                        api: Arc::new(fake.clone()),
                        connector: Some(Arc::new(fake)),
                        clock,
                        store: Box::new(live::NoStore),
                    },
                    Some(script),
                )
            }
        };
        let monitor = Arc::new(live::LiveMonitor::spawn(
            config,
            channels,
            power.into(),
            deps,
        ));
        drop(guard);
        let pump = monitor.clone();
        let observers = Arc::clone(&self.observers);
        let alert_gate = self.alert_gate.clone();
        let thread = std::thread::Builder::new()
            .name("twitch-live".to_owned())
            .spawn(move || {
                rt.block_on(async move {
                    while let Some(event) = pump.next_event().await {
                        let Some(out) = dispatch(event, &observers, alert_gate.as_ref()) else {
                            continue;
                        };
                        if sink.add(out).is_err() {
                            break;
                        }
                    }
                    if let Some(script) = script {
                        script.abort();
                    }
                });
            })
            .map_err(|e| CoreLiveError::new("io", e))?;
        let mut slot = self.running.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(Running {
            monitor,
            thread: Some(thread),
        });
        Ok(())
    }

    fn with_monitor(&self, f: impl FnOnce(&live::LiveMonitor)) {
        if let Some(r) = self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            f(&r.monitor);
        }
    }

    /// Favourites or a toggle changed.
    #[frb(sync)]
    pub fn set_channels(&self, channels: Vec<CoreLiveChannel>) {
        self.with_monitor(|m| m.set_channels(channels.into_iter().map(Into::into).collect()));
    }

    /// The device's power state changed.
    #[frb(sync)]
    pub fn set_power(&self, power: CoreLivePower) {
        self.with_monitor(|m| m.set_power(power.into()));
    }

    /// Poll now (the app came to the foreground).
    #[frb(sync)]
    pub fn poll_now(&self) {
        self.with_monitor(live::LiveMonitor::poll_now);
    }

    /// Stops the monitor; the stream ends.
    #[frb(sync)]
    pub fn stop(&self) {
        let running = self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        drop(running);
    }
}

/// Runs a single check (`check_once`) on a fresh runtime.
pub(crate) fn check_once_blocking(
    api: &dyn live::LiveApi,
    db_path: Option<&str>,
    channels: Vec<CoreLiveChannel>,
    max_age_secs: u32,
) -> Result<Vec<CoreLiveAlert>, CoreLiveError> {
    let channels: Vec<live::WatchedChannel> = channels.into_iter().map(Into::into).collect();
    let mut store: Box<dyn live::TimelineStore> =
        match db_path.map(live::LibraryTimelineStore::open) {
            Some(Ok(s)) => Box::new(s),
            Some(Err(e)) => return Err(CoreLiveError::new("io", e)),
            None => Box::new(live::NoStore),
        };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CoreLiveError::new("io", e))?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    let report = rt.block_on(live::check_once(
        api,
        &channels,
        store.as_mut(),
        Duration::from_secs(u64::from(max_age_secs)),
        now_ms,
    ))?;
    Ok(report.alerts.into_iter().map(Into::into).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_monitor_starts_and_stops() {
        let m = CoreTwitchLive::demo(1);
        // Without a Dart sink we can only check the plumbing that does not
        // need one.
        m.set_power(CoreLivePower::Battery);
        m.poll_now();
        m.stop();
    }

    /// ADR-NNNN: a kid profile drops the alert for Dart, never the event
    /// for the record consumer.
    #[test]
    fn gated_alerts_still_reach_the_record_consumer() {
        use pa_twitch::chapters::{ChapterSource, LiveTimeline, TimelineChange};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let seen = Arc::new(AtomicUsize::new(0));
        let observers: Mutex<Vec<Observer>> = Mutex::new(Vec::new());
        {
            let seen = Arc::clone(&seen);
            observers.lock().unwrap().push(Box::new(move |_| {
                seen.fetch_add(1, Ordering::SeqCst);
            }));
        }
        let allowed = Arc::new(AtomicBool::new(true));
        let gate: AlertGate = {
            let allowed = Arc::clone(&allowed);
            Arc::new(move || allowed.load(Ordering::SeqCst))
        };
        let alert = || {
            live::MonitorEvent::Alert(live::LiveAlert {
                user_id: "81000001".into(),
                login: "made_up".into(),
                display_name: "Made Up".into(),
                stream_id: "5001".into(),
                title: None,
                game_name: None,
                started_at_ms: 1,
                detected_at_ms: 2,
                source: ChapterSource::EventSub,
            })
        };
        let started = || {
            live::MonitorEvent::Timeline(TimelineChange::Started(LiveTimeline::new(
                "5001", "81000001", 1,
            )))
        };

        // A parent profile: both go to Dart.
        assert!(dispatch(alert(), &observers, Some(&gate)).is_some());
        assert!(dispatch(started(), &observers, Some(&gate)).is_some());
        // A kid profile: no alert for Dart, the timeline still goes.
        allowed.store(false, Ordering::SeqCst);
        assert!(dispatch(alert(), &observers, Some(&gate)).is_none());
        let out = dispatch(started(), &observers, Some(&gate)).expect("timeline");
        assert!(matches!(out.kind, CoreLiveEventKind::Timeline));
        assert!(out.live);
        // The record consumer saw all four, gated or not.
        assert_eq!(seen.load(Ordering::SeqCst), 4);
        // Without a gate (demo) every alert goes.
        assert!(dispatch(alert(), &observers, None).is_some());
    }

    #[test]
    fn statuses_convert() {
        let s: CoreLiveStatus = live::MonitorStatus {
            eventsub: live::EventSubState::Stopped {
                reason: "made-up".into(),
            },
            channels: 3,
            eventsub_channels: 1,
            polled_only: 2,
            power: live::PowerState::Plugged,
            next_poll_in: Duration::from_secs(60),
            poll_errors: 0,
            last_error: None,
            last_poll_ms: Some(1),
        }
        .into();
        assert_eq!(s.eventsub, "stopped");
        assert_eq!(s.eventsub_reason.as_deref(), Some("made-up"));
        assert_eq!(s.polled_only, 2);
        assert_eq!(s.next_poll_secs, 60);
    }

    #[test]
    fn demo_check_once_without_store() {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let fake = live::demo::FakeTwitch::demo(clock);
        let channels: Vec<CoreLiveChannel> = live::demo::FakeTwitch::demo_channels()
            .into_iter()
            .map(|c| CoreLiveChannel {
                user_id: c.user_id,
                login: c.login,
                display_name: c.display_name,
                notify: true,
            })
            .collect();
        fake.go_live(&channels[0].user_id, "Made-up check");
        let alerts = check_once_blocking(&fake, None, channels, 600).expect("check");
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].display_name, "Moonlit Pancake");
        assert_eq!(alerts[0].source, "polling");
    }
}
