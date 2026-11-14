// SPDX-License-Identifier: MPL-2.0
//! The monitor: one task that owns the category tracker of TWI-008, the
//! notification gate, the EventSub session and the polling schedule.
//!
//! - EventSub (`stream.online`, `channel.update`, `stream.offline`) for the
//!   channels the budget covers; events reach the tracker as
//!   [`ChapterSource::EventSub`].
//! - `GET /streams` for all watched channels on the [`PollPolicy`]
//!   schedule: channels without EventSub get their "live" from here, the
//!   others get categories and ends refreshed. A poll right after an
//!   EventSub `stream.online` may not list the stream yet (Twitch's cache),
//!   so a fresh broadcast is not ended by its absence for
//!   [`MonitorConfig::offline_grace`].
//! - Every [`TimelineChange`] is stored ([`TimelineStore`]) and handed out;
//!   a started broadcast also goes through the [`NotificationGate`]. When a
//!   broadcast ends, its archive VOD gets our chapters.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use pa_twitch::Clock;
use pa_twitch::chapters::{ChapterSource, TimelineChange, TimelineTracker};
use pa_twitch_chat::WsConnector;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

use crate::api::{LiveApi, LiveError, unix_ms};
use crate::eventsub::{self, Coverage, EsMsg, EventSubSettings, EventSubState};
use crate::gate::{DEFAULT_ALERT_MAX_AGE, LiveAlert, NotificationGate};
use crate::plan::{DEFAULT_MAX_TOTAL_COST, SubscriptionPlan, WatchedChannel};
use crate::schedule::{PollPolicy, PowerState};
use crate::store::TimelineStore;

/// Poll this soon after an EventSub `stream.online`, for the title and
/// category of the new broadcast (chapters).
const POLL_AFTER_ONLINE: Duration = Duration::from_secs(20);

/// Monitor settings.
#[derive(Debug, Clone)]
pub struct MonitorConfig {
    /// EventSub connection.
    pub eventsub: EventSubSettings,
    /// Use EventSub at all (needs a user token and a WebSocket connector).
    pub use_eventsub: bool,
    /// Subscription budget of one session.
    pub max_total_cost: u32,
    /// Polling intervals.
    pub policy: PollPolicy,
    /// Broadcasts older than this when first seen are not announced.
    pub alert_max_age: Duration,
    /// A broadcast younger than this is not ended because a poll misses
    /// it.
    pub offline_grace: Duration,
    /// Events buffered for a slow consumer.
    pub buffer: usize,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            eventsub: EventSubSettings::default(),
            use_eventsub: true,
            max_total_cost: DEFAULT_MAX_TOTAL_COST,
            policy: PollPolicy::default(),
            alert_max_age: DEFAULT_ALERT_MAX_AGE,
            offline_grace: Duration::from_secs(180),
            buffer: 256,
        }
    }
}

/// Health of the monitor, for the settings screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorStatus {
    /// EventSub session.
    pub eventsub: EventSubState,
    /// Watched channels.
    pub channels: usize,
    /// Channels announced by EventSub `stream.online`.
    pub eventsub_channels: usize,
    /// Channels announced only by polling.
    pub polled_only: usize,
    /// Power state in use.
    pub power: PowerState,
    /// Time to the next poll.
    pub next_poll_in: Duration,
    /// Failed polls in a row.
    pub poll_errors: u32,
    /// Last poll error.
    pub last_error: Option<String>,
    /// Last successful poll (Unix ms).
    pub last_poll_ms: Option<i64>,
}

/// What the monitor reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitorEvent {
    /// Show an "X is live" notification.
    Alert(LiveAlert),
    /// A timeline changed (already stored).
    Timeline(TimelineChange),
    /// Status changed.
    Status(MonitorStatus),
}

enum Command {
    Channels(Vec<WatchedChannel>),
    Power(PowerState),
    PollNow,
}

/// Running monitor. Dropping it stops the task.
pub struct LiveMonitor {
    cmd: mpsc::UnboundedSender<Command>,
    events: Mutex<mpsc::Receiver<MonitorEvent>>,
    task: JoinHandle<()>,
}

impl std::fmt::Debug for LiveMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveMonitor").finish_non_exhaustive()
    }
}

/// What the monitor runs on.
pub struct MonitorDeps {
    /// Helix.
    pub api: Arc<dyn LiveApi>,
    /// EventSub WebSocket; `None` = polling only.
    pub connector: Option<Arc<dyn WsConnector>>,
    /// Time.
    pub clock: Arc<dyn Clock>,
    /// Timeline storage.
    pub store: Box<dyn TimelineStore>,
}

impl LiveMonitor {
    /// Starts the monitor inside the current Tokio runtime. The first poll
    /// runs at once.
    pub fn spawn(
        config: MonitorConfig,
        channels: Vec<WatchedChannel>,
        power: PowerState,
        deps: MonitorDeps,
    ) -> Self {
        let (cmd, cmd_rx) = mpsc::unbounded_channel();
        let (tx, rx) = mpsc::channel(config.buffer.max(1));
        let now = deps.clock.now();
        let driver = Driver {
            alert_gate: NotificationGate::new(config.alert_max_age),
            config,
            api: deps.api,
            connector: deps.connector,
            clock: deps.clock,
            store: std::sync::Mutex::new(deps.store),
            by_id: HashMap::new(),
            channels: Vec::new(),
            power,
            tracker: TimelineTracker::new(),
            es_task: None,
            es_rx: None,
            es_state: EventSubState::Off,
            coverage: Coverage::default(),
            plan: SubscriptionPlan::default(),
            errors: 0,
            retry_after: None,
            next_poll_at: now,
            last_error: None,
            last_poll_ms: None,
            tx,
        };
        Self {
            cmd,
            events: Mutex::new(rx),
            task: tokio::spawn(driver.run(channels, cmd_rx)),
        }
    }

    /// Replaces the watched channels (favourites changed, a toggle
    /// flipped, signed in again). Re-plans EventSub when needed.
    pub fn set_channels(&self, channels: Vec<WatchedChannel>) {
        let _ = self.cmd.send(Command::Channels(channels));
    }

    /// The device's power state changed.
    pub fn set_power(&self, power: PowerState) {
        let _ = self.cmd.send(Command::Power(power));
    }

    /// Polls now (the app came to the foreground).
    pub fn poll_now(&self) {
        let _ = self.cmd.send(Command::PollNow);
    }

    /// Next event; `None` once the monitor stopped.
    pub async fn next_event(&self) -> Option<MonitorEvent> {
        self.events.lock().await.recv().await
    }

    /// Stops the monitor.
    pub fn stop(&self) {
        self.task.abort();
    }
}

impl Drop for LiveMonitor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Driver {
    config: MonitorConfig,
    api: Arc<dyn LiveApi>,
    connector: Option<Arc<dyn WsConnector>>,
    clock: Arc<dyn Clock>,
    // Mutex: the driver future must be `Send`, a SQLite store is not `Sync`.
    store: std::sync::Mutex<Box<dyn TimelineStore>>,
    channels: Vec<WatchedChannel>,
    by_id: HashMap<String, WatchedChannel>,
    power: PowerState,
    tracker: TimelineTracker,
    alert_gate: NotificationGate,
    es_task: Option<JoinHandle<()>>,
    es_rx: Option<mpsc::Receiver<EsMsg>>,
    es_state: EventSubState,
    coverage: Coverage,
    plan: SubscriptionPlan,
    errors: u32,
    retry_after: Option<Duration>,
    next_poll_at: SystemTime,
    last_error: Option<String>,
    last_poll_ms: Option<i64>,
    tx: mpsc::Sender<MonitorEvent>,
}

async fn recv_es(rx: &mut Option<mpsc::Receiver<EsMsg>>) -> Option<EsMsg> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        if let Some(task) = self.es_task.take() {
            task.abort();
        }
    }
}

impl Driver {
    async fn emit(&self, event: MonitorEvent) -> bool {
        self.tx.send(event).await.is_ok()
    }

    fn now_ms(&self) -> i64 {
        unix_ms(&*self.clock)
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Box<dyn TimelineStore>> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn run(
        mut self,
        channels: Vec<WatchedChannel>,
        mut cmd: mpsc::UnboundedReceiver<Command>,
    ) {
        let open = self.store().open_timelines(self.now_ms());
        for timeline in open {
            // Announced by the process that stored it.
            self.alert_gate.remember(&timeline.stream_id);
            self.tracker.resume(timeline);
        }
        self.set_channels(channels);
        if !self.status().await {
            return;
        }
        loop {
            let wait = self
                .next_poll_at
                .duration_since(self.clock.now())
                .unwrap_or(Duration::ZERO);
            tokio::select! {
                biased;
                command = cmd.recv() => {
                    let Some(command) = command else { return };
                    match command {
                        Command::Channels(c) => self.set_channels(c),
                        Command::Power(p) => {
                            self.power = p;
                            self.reschedule();
                        }
                        Command::PollNow => self.next_poll_at = self.clock.now(),
                    }
                    if !self.status().await {
                        return;
                    }
                }
                msg = recv_es(&mut self.es_rx) => {
                    let ok = match msg {
                        None => {
                            self.es_rx = None;
                            true
                        }
                        Some(EsMsg::State(state)) => self.on_es_state(state).await,
                        Some(EsMsg::Event(event)) => {
                            let online = matches!(
                                event,
                                pa_twitch::chapters::TimelineEvent::Online { .. }
                            );
                            let changes = self.tracker.apply(event, ChapterSource::EventSub);
                            if online && !changes.is_empty() {
                                let soon = self.clock.now() + POLL_AFTER_ONLINE;
                                self.next_poll_at = self.next_poll_at.min(soon);
                            }
                            self.handle(changes, ChapterSource::EventSub).await
                        }
                    };
                    if !ok {
                        return;
                    }
                }
                () = self.clock.sleep(wait) => {
                    if !self.poll().await || !self.status().await {
                        return;
                    }
                }
            }
        }
    }

    fn set_channels(&mut self, channels: Vec<WatchedChannel>) {
        self.by_id = channels
            .iter()
            .map(|c| (c.user_id.clone(), c.clone()))
            .collect();
        let added = channels
            .iter()
            .any(|c| !self.channels.iter().any(|old| old.user_id == c.user_id));
        self.channels = channels;
        let plan = if self.config.use_eventsub && self.connector.is_some() {
            SubscriptionPlan::new(&self.channels, self.config.max_total_cost)
        } else {
            SubscriptionPlan::default()
        };
        let stopped = matches!(self.es_state, EventSubState::Stopped { .. });
        if plan != self.plan || stopped || (self.es_task.is_none() && !plan.subs.is_empty()) {
            self.plan = plan;
            self.restart_eventsub();
        }
        if added {
            self.next_poll_at = self.clock.now();
        }
    }

    fn restart_eventsub(&mut self) {
        if let Some(task) = self.es_task.take() {
            task.abort();
        }
        self.es_rx = None;
        self.coverage = Coverage::default();
        self.es_state = EventSubState::Off;
        let Some(connector) = self.connector.clone() else {
            return;
        };
        if self.plan.subs.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel(64);
        self.es_task = Some(eventsub::spawn(
            self.config.eventsub.clone(),
            self.plan.clone(),
            self.api.clone(),
            connector,
            self.clock.clone(),
            tx,
        ));
        self.es_rx = Some(rx);
        self.es_state = EventSubState::Connecting;
    }

    async fn on_es_state(&mut self, state: EventSubState) -> bool {
        let was_covered = self.covered();
        self.coverage = match &state {
            EventSubState::Connected(c) => c.clone(),
            _ => Coverage::default(),
        };
        self.es_state = state;
        if was_covered && !self.covered() {
            // EventSub went away: poll now rather than at the slow rate.
            self.next_poll_at = self.clock.now();
        } else {
            self.reschedule();
        }
        self.status().await
    }

    /// Every channel with notifications has a live `stream.online`.
    fn covered(&self) -> bool {
        matches!(self.es_state, EventSubState::Connected(_))
            && self
                .channels
                .iter()
                .filter(|c| c.notify)
                .all(|c| self.coverage.online.contains(&c.user_id))
    }

    fn reschedule(&mut self) {
        let delay = self.config.policy.next_delay(
            self.power,
            self.covered(),
            self.errors,
            self.retry_after,
        );
        let at = self
            .last_poll_ms
            .map(|ms| {
                SystemTime::UNIX_EPOCH + Duration::from_millis(u64::try_from(ms).unwrap_or(0))
            })
            .unwrap_or_else(|| self.clock.now())
            + delay;
        // Never later than already planned, so a switch to a slower rate
        // does not postpone a poll that is due.
        self.next_poll_at = self.next_poll_at.min(at).max(self.clock.now());
    }

    async fn poll(&mut self) -> bool {
        let ids: Vec<String> = self.channels.iter().map(|c| c.user_id.clone()).collect();
        if ids.is_empty() {
            self.schedule_after_poll();
            return true;
        }
        match self.api.live_streams(ids.clone()).await {
            Ok(streams) => {
                self.errors = 0;
                self.retry_after = None;
                self.last_error = None;
                let now_ms = self.now_ms();
                self.last_poll_ms = Some(now_ms);
                let listed: BTreeSet<&str> = streams.iter().map(|s| s.user_id.as_str()).collect();
                let eventsub_offline = matches!(self.es_state, EventSubState::Connected(_));
                let grace =
                    i64::try_from(self.config.offline_grace.as_millis()).unwrap_or(i64::MAX);
                let observed: Vec<&str> = ids
                    .iter()
                    .map(String::as_str)
                    .filter(|id| {
                        if listed.contains(id) {
                            return true;
                        }
                        let Some(live) = self.tracker.live(id) else {
                            return true;
                        };
                        let fresh = now_ms.saturating_sub(live.started_at_ms) < grace;
                        let told = eventsub_offline && self.coverage.offline.contains(*id);
                        !(fresh || told)
                    })
                    .collect();
                let changes = self.tracker.observe_poll(&observed, &streams, now_ms);
                let ok = self.handle(changes, ChapterSource::Polling).await;
                self.schedule_after_poll();
                ok
            }
            Err(e) => {
                self.errors = self.errors.saturating_add(1);
                self.retry_after = match &e {
                    LiveError::RateLimited(d) => Some(*d),
                    _ => None,
                };
                self.last_error = Some(e.to_string());
                self.schedule_after_poll();
                true
            }
        }
    }

    fn schedule_after_poll(&mut self) {
        let delay = self.config.policy.next_delay(
            self.power,
            self.covered(),
            self.errors,
            self.retry_after,
        );
        self.next_poll_at = self.clock.now() + delay;
    }

    async fn handle(&mut self, changes: Vec<TimelineChange>, source: ChapterSource) -> bool {
        for change in changes {
            let now_ms = self.now_ms();
            self.store().save(&change, now_ms);
            let channel = self.by_id.get(&change.timeline().user_id);
            if let Some(alert) = self.alert_gate.consider(&change, channel, now_ms, source)
                && !self.emit(MonitorEvent::Alert(alert)).await
            {
                return false;
            }
            if let TimelineChange::Ended(timeline) = &change
                && let Ok(Some(video)) = self
                    .api
                    .archive_for_stream(timeline.user_id.clone(), timeline.stream_id.clone())
                    .await
            {
                let now_ms = self.now_ms();
                self.store().save_vod(&video, timeline, now_ms);
            }
            if !self.emit(MonitorEvent::Timeline(change)).await {
                return false;
            }
        }
        true
    }

    async fn status(&self) -> bool {
        let eventsub_channels = self
            .channels
            .iter()
            .filter(|c| self.coverage.online.contains(&c.user_id))
            .count();
        let status = MonitorStatus {
            eventsub: self.es_state.clone(),
            channels: self.channels.len(),
            eventsub_channels,
            polled_only: self.channels.len() - eventsub_channels,
            power: self.power,
            next_poll_in: self
                .next_poll_at
                .duration_since(self.clock.now())
                .unwrap_or(Duration::ZERO),
            poll_errors: self.errors,
            last_error: self.last_error.clone(),
            last_poll_ms: self.last_poll_ms,
        };
        self.emit(MonitorEvent::Status(status)).await
    }
}
