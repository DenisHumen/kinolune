// SPDX-License-Identifier: MPL-2.0
//! Availability of servers and sources (NET-024, ADR-0157 §8).
//!
//! One monitor for everything the app talks to — SMB, NFS, WebDAV, SFTP,
//! FTP, Jellyfin/Emby/Plex, the own Kinolune server, the sync hub and
//! peers on the LAN, the sync folder, clouds — **no matter how it was
//! added**: by hand, from the LAN list, by QR, arrived by sync from
//! another device or picked automatically as the sync folder. The monitor
//! only sees [`Target`]s; where they came from is a label
//! ([`Origin`]), never a reason to treat one differently.
//!
//! Two kinds of signals change a status:
//!
//! - **active probes**: a cheap protocol check ([`Prober`]) every
//!   [`Policy::interval`] (30 s) while the app is in front; when a target
//!   is down the interval doubles up to [`Policy::max_backoff`] (5 min).
//!   A network change and coming back to the front probe everything at
//!   once. A phone in the background does not probe at all (battery); a
//!   desktop, TV or server in the background probes every
//!   [`Policy::background_interval`];
//! - **passive signals**: the result of any real operation (a scan, a
//!   playback, a sync round) reported with [`Monitor::report`] or by the
//!   [`TargetSource`]; a success postpones the next active probe, so a
//!   source in use costs no extra requests.
//!
//! Statuses ([`Status`]): available, slow (round trip over
//! [`Policy::slow_rtt_ms`]), no access (password, token, rights — never
//! confused with "down"), unavailable since a time, no network on the
//! device. Every change is a [`Transition`], kept in a short history and
//! published on the monitor's [`EventBus`].
//!
//! The state machine ([`Tracker`]) is pure: it takes the time as an
//! argument, so tests drive it with a fake clock. [`Monitor`] adds the
//! prober, the clock and a thread; [`Monitor::tick`] runs one step
//! synchronously for tests.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::events::EventBus;

/// What a target is: decides the probe and the icon, nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TargetKind {
    /// A folder on this device (an external drive can go away too).
    Local,
    /// SMB2/3 share.
    Smb,
    /// NFS export.
    Nfs,
    /// WebDAV collection.
    WebDav,
    /// SFTP server.
    Sftp,
    /// FTP or FTPS server.
    Ftp,
    /// Jellyfin server.
    Jellyfin,
    /// Emby server.
    Emby,
    /// Plex Media Server.
    Plex,
    /// Subsonic music server.
    Subsonic,
    /// UPnP/DLNA media server.
    Dlna,
    /// M3U playlist link.
    Playlist,
    /// Cloud storage (MEGA, Google Drive, OneDrive, Dropbox).
    Cloud,
    /// The own Kinolune server (SRV-001), probed at `/health`.
    KinoluneServer,
    /// A sync hub on the LAN.
    SyncHub,
    /// Another device of the family (LAN sync, SYNC-006) or a member of
    /// the server cluster (SYNC-019).
    SyncPeer,
}

impl TargetKind {
    /// Stable name for the FFI and logs.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Smb => "smb",
            Self::Nfs => "nfs",
            Self::WebDav => "webdav",
            Self::Sftp => "sftp",
            Self::Ftp => "ftp",
            Self::Jellyfin => "jellyfin",
            Self::Emby => "emby",
            Self::Plex => "plex",
            Self::Subsonic => "subsonic",
            Self::Dlna => "dlna",
            Self::Playlist => "playlist",
            Self::Cloud => "cloud",
            Self::KinoluneServer => "kinolune",
            Self::SyncHub => "sync-hub",
            Self::SyncPeer => "sync-peer",
        }
    }

    /// From [`as_str`](Self::as_str).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "local" => Self::Local,
            "smb" => Self::Smb,
            "nfs" => Self::Nfs,
            "webdav" => Self::WebDav,
            "sftp" => Self::Sftp,
            "ftp" => Self::Ftp,
            "jellyfin" => Self::Jellyfin,
            "emby" => Self::Emby,
            "plex" => Self::Plex,
            "subsonic" => Self::Subsonic,
            "dlna" => Self::Dlna,
            "playlist" => Self::Playlist,
            "cloud" => Self::Cloud,
            "kinolune" => Self::KinoluneServer,
            "sync-hub" => Self::SyncHub,
            "sync-peer" => Self::SyncPeer,
            _ => return None,
        })
    }
}

/// How a target got into the app. A label for the UI: the monitor checks
/// every origin the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Origin {
    /// Added on this device (by hand, from the LAN list or by QR).
    #[default]
    Added,
    /// Arrived by sync from another device (SYNC-010).
    Synced,
    /// Picked by the core itself (the automatic sync folder, a peer seen
    /// on the LAN).
    Auto,
}

impl Origin {
    /// Stable name for the FFI.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Synced => "synced",
            Self::Auto => "auto",
        }
    }
}

/// Something the monitor watches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Unique id: `source:<id>`, `peer:<device id>`, `endpoint:<id>`…
    pub id: String,
    /// What it is.
    pub kind: TargetKind,
    /// Display name.
    pub name: String,
    /// How it got here.
    pub origin: Origin,
    /// Sync goes through it (the sync folder's source, the hub, a peer):
    /// the UI shows "sync paused" while it is down (UI-053).
    pub sync_channel: bool,
    /// Whether the monitor probes it actively. `false`: only passive
    /// signals change its status (peers that announce themselves).
    pub active: bool,
    /// Never probe more often than this, even in front (clouds: their
    /// API quotas). `None`: [`Policy::interval`].
    pub min_interval: Option<Duration>,
}

impl Target {
    /// An actively probed target added on this device.
    #[must_use]
    pub fn new(id: impl Into<String>, kind: TargetKind, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind,
            name: name.into(),
            origin: Origin::Added,
            sync_channel: false,
            active: true,
            min_interval: None,
        }
    }
}

/// Result of one probe or of a real operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// It answered, in `rtt_ms`.
    Ok {
        /// Round trip of the probe (0 when unknown, e.g. passive).
        rtt_ms: u32,
    },
    /// It answered but refused us: wrong password, expired token, no
    /// rights. Never counted as "down".
    Denied(String),
    /// No answer: connection refused, timeout, host not found.
    Unreachable(String),
}

/// Status of a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Status {
    /// Not checked yet.
    #[default]
    Unknown,
    /// Answers.
    Available,
    /// Answers, slowly (round trip over [`Policy::slow_rtt_ms`]).
    Slow,
    /// Answers but refuses us: password, token or rights.
    NoAccess,
    /// Does not answer (see [`TargetState::since_ms`]).
    Unavailable,
    /// This device has no network: nothing is probed.
    NoNetwork,
}

impl Status {
    /// Stable name for the FFI.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Available => "available",
            Self::Slow => "slow",
            Self::NoAccess => "no-access",
            Self::Unavailable => "unavailable",
            Self::NoNetwork => "no-network",
        }
    }

    /// Whether the target can be used (available or slow).
    #[must_use]
    pub fn is_up(self) -> bool {
        matches!(self, Self::Available | Self::Slow)
    }
}

/// Current state of one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetState {
    /// The target.
    pub target: Target,
    /// Its status.
    pub status: Status,
    /// Since when it has this status (Unix ms): "unavailable since 14:05".
    pub since_ms: Option<i64>,
    /// Last time it answered (Unix ms).
    pub last_ok_ms: Option<i64>,
    /// Last probe or operation result (Unix ms).
    pub checked_at_ms: Option<i64>,
    /// Round trip of the last answer.
    pub rtt_ms: Option<u32>,
    /// Why it is not available (no secrets: probers scrub them).
    pub reason: Option<String>,
    /// Failed probes in a row.
    pub failures: u32,
    /// When the next active probe is due (Unix ms); `None` while probes
    /// are paused (no network, phone in the background, passive target).
    pub next_probe_ms: Option<i64>,
}

/// A status change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    /// Target id.
    pub target_id: String,
    /// Its display name at the time.
    pub name: String,
    /// Old status.
    pub from: Status,
    /// New status.
    pub to: Status,
    /// When (Unix ms).
    pub at_ms: i64,
    /// Why (for `NoAccess`, `Unavailable`).
    pub reason: Option<String>,
    /// Caused by a real operation, not by a probe.
    pub passive: bool,
}

/// What the monitor publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvailabilityEvent {
    /// A target changed its status.
    Changed(Transition),
    /// Targets were added or removed.
    TargetsChanged,
}

/// Where the app is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppState {
    /// On screen.
    #[default]
    Foreground,
    /// Not on screen.
    Background,
}

/// Timing of the monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Probe interval in front while a target answers (30 s).
    pub interval: Duration,
    /// Longest interval while a target is down (5 min).
    pub max_backoff: Duration,
    /// Interval in the background on a device that may probe there
    /// (desktop, TV, server): 5 min.
    pub background_interval: Duration,
    /// A phone or tablet: no probes at all in the background (battery).
    pub mobile: bool,
    /// Round trip from which a target counts as slow.
    pub slow_rtt_ms: u32,
    /// Transitions kept in the history.
    pub history_len: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            max_backoff: Duration::from_secs(300),
            background_interval: Duration::from_secs(300),
            mobile: false,
            slow_rtt_ms: 1500,
            history_len: 100,
        }
    }
}

fn ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

/// The state machine of the monitor: pure, the time is an argument.
#[derive(Debug, Clone)]
pub struct Tracker {
    policy: Policy,
    app: AppState,
    network: bool,
    targets: BTreeMap<String, TargetState>,
    history: VecDeque<Transition>,
}

impl Tracker {
    /// No targets, in front, with network.
    #[must_use]
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            app: AppState::Foreground,
            network: true,
            targets: BTreeMap::new(),
            history: VecDeque::new(),
        }
    }

    /// The policy.
    #[must_use]
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Whether active probes run now at all.
    #[must_use]
    pub fn probing(&self) -> bool {
        self.network && !(self.app == AppState::Background && self.policy.mobile)
    }

    /// The interval while a target answers, in the current app state.
    fn base_interval(&self, target: &Target) -> i64 {
        let base = match self.app {
            AppState::Foreground => self.policy.interval,
            AppState::Background => self.policy.background_interval,
        };
        ms(target.min_interval.map_or(base, |m| m.max(base)))
    }

    /// The interval after `failures` failed probes in a row: 30 s, 60 s,
    /// 120 s, 240 s, then 5 min.
    #[must_use]
    pub fn backoff(&self, failures: u32) -> Duration {
        let steps = failures.saturating_sub(1).min(16);
        self.policy
            .interval
            .saturating_mul(1 << steps)
            .min(self.policy.max_backoff)
    }

    /// Replaces the set of targets. New ones are probed at once (their
    /// status `Unknown`, or `NoNetwork`); known ones keep their state and
    /// take the new name, flags and origin; missing ones are dropped.
    /// Returns whether the set changed.
    pub fn set_targets(&mut self, targets: Vec<Target>, now_ms: i64) -> bool {
        let mut changed = false;
        let mut next = BTreeMap::new();
        for t in targets {
            let state = match self.targets.remove(&t.id) {
                Some(mut s) => {
                    if s.target != t {
                        changed = true;
                        s.target = t.clone();
                        if !t.active {
                            s.next_probe_ms = None;
                        } else if s.next_probe_ms.is_none() && self.probing() {
                            s.next_probe_ms = Some(now_ms);
                        }
                    }
                    s
                }
                None => {
                    changed = true;
                    TargetState {
                        status: if self.network {
                            Status::Unknown
                        } else {
                            Status::NoNetwork
                        },
                        since_ms: Some(now_ms),
                        last_ok_ms: None,
                        checked_at_ms: None,
                        rtt_ms: None,
                        reason: None,
                        failures: 0,
                        next_probe_ms: (t.active && self.probing()).then_some(now_ms),
                        target: t.clone(),
                    }
                }
            };
            next.insert(t.id, state);
        }
        changed |= !self.targets.is_empty();
        self.targets = next;
        changed
    }

    /// Ids of the targets whose active probe is due at `now_ms`.
    #[must_use]
    pub fn due(&self, now_ms: i64) -> Vec<Target> {
        if !self.probing() {
            return Vec::new();
        }
        self.targets
            .values()
            .filter(|s| s.target.active && s.next_probe_ms.is_some_and(|t| t <= now_ms))
            .map(|s| s.target.clone())
            .collect()
    }

    /// When the next probe is due; `None`: nothing to probe.
    #[must_use]
    pub fn next_wakeup(&self) -> Option<i64> {
        if !self.probing() {
            return None;
        }
        self.targets
            .values()
            .filter(|s| s.target.active)
            .filter_map(|s| s.next_probe_ms)
            .min()
    }

    /// Records the result of a probe (`passive = false`) or of a real
    /// operation (`passive = true`) for target `id`. Returns the
    /// transition, if the status changed.
    pub fn record(
        &mut self,
        id: &str,
        outcome: &ProbeOutcome,
        now_ms: i64,
        passive: bool,
    ) -> Option<Transition> {
        let probing = self.probing();
        let network = self.network;
        let policy = self.policy;
        let base = self
            .targets
            .get(id)
            .map(|s| self.base_interval(&s.target))?;
        let failures_after = self.targets.get(id).map_or(0, |s| s.failures + 1);
        let backoff = ms(self.backoff(failures_after)).max(base.min(ms(policy.max_backoff)));
        let state = self.targets.get_mut(id)?;
        // A failed operation while the device is offline says nothing
        // about the target.
        if !network && !matches!(outcome, ProbeOutcome::Ok { .. }) {
            return None;
        }
        state.checked_at_ms = Some(now_ms);
        let (status, reason) = match outcome {
            ProbeOutcome::Ok { rtt_ms } => {
                state.failures = 0;
                state.last_ok_ms = Some(now_ms);
                if *rtt_ms > 0 || !passive {
                    state.rtt_ms = Some(*rtt_ms);
                }
                state.next_probe_ms = (state.target.active && probing).then_some(now_ms + base);
                let slow = !passive && *rtt_ms >= policy.slow_rtt_ms;
                (
                    if slow {
                        Status::Slow
                    } else if passive && state.status == Status::Slow {
                        // A passive success does not measure speed.
                        Status::Slow
                    } else {
                        Status::Available
                    },
                    None,
                )
            }
            ProbeOutcome::Denied(why) => {
                state.failures = 0;
                state.next_probe_ms = (state.target.active && probing).then_some(now_ms + base);
                (Status::NoAccess, Some(why.clone()))
            }
            ProbeOutcome::Unreachable(why) => {
                state.failures = failures_after;
                state.next_probe_ms = (state.target.active && probing).then_some(now_ms + backoff);
                (Status::Unavailable, Some(why.clone()))
            }
        };
        state.reason = reason.clone();
        self.set_status(id, status, reason, now_ms, passive)
    }

    fn set_status(
        &mut self,
        id: &str,
        to: Status,
        reason: Option<String>,
        now_ms: i64,
        passive: bool,
    ) -> Option<Transition> {
        let state = self.targets.get_mut(id)?;
        if state.status == to {
            return None;
        }
        let t = Transition {
            target_id: id.to_owned(),
            name: state.target.name.clone(),
            from: state.status,
            to,
            at_ms: now_ms,
            reason,
            passive,
        };
        state.status = to;
        state.since_ms = Some(now_ms);
        self.history.push_back(t.clone());
        while self.history.len() > self.policy.history_len.max(1) {
            self.history.pop_front();
        }
        Some(t)
    }

    /// The device lost or got its network. Offline: every target becomes
    /// `NoNetwork` and nothing is probed. Online (or another network):
    /// everything is probed at once, backoffs forgotten.
    pub fn network_changed(&mut self, online: bool, now_ms: i64) -> Vec<Transition> {
        self.network = online;
        let probing = self.probing();
        let ids: Vec<String> = self.targets.keys().cloned().collect();
        let mut out = Vec::new();
        for id in ids {
            if online {
                if let Some(s) = self.targets.get_mut(&id) {
                    s.failures = 0;
                    s.next_probe_ms = (s.target.active && probing).then_some(now_ms);
                }
            } else {
                if let Some(s) = self.targets.get_mut(&id) {
                    s.next_probe_ms = None;
                }
                out.extend(self.set_status(&id, Status::NoNetwork, None, now_ms, false));
            }
        }
        out
    }

    /// The app came to the front or left it. Coming back probes
    /// everything at once (the network may have changed while away).
    pub fn set_app_state(&mut self, app: AppState, now_ms: i64) {
        let was = self.app;
        self.app = app;
        if was == app {
            return;
        }
        let probing = self.probing();
        let back = app == AppState::Foreground;
        let ids: Vec<String> = self.targets.keys().cloned().collect();
        for id in ids {
            let base = self.targets.get(&id).map(|s| self.base_interval(&s.target));
            if let (Some(s), Some(base)) = (self.targets.get_mut(&id), base) {
                s.next_probe_ms = if !s.target.active || !probing {
                    None
                } else if back {
                    Some(now_ms)
                } else {
                    // Background: no sooner than the background interval.
                    Some(s.next_probe_ms.unwrap_or(now_ms).max(now_ms + base))
                };
            }
        }
    }

    /// Probe `id` (or every target when `None`) at once.
    pub fn probe_now(&mut self, id: Option<&str>, now_ms: i64) {
        let probing = self.probing();
        for s in self.targets.values_mut() {
            if id.is_none_or(|id| id == s.target.id) && s.target.active && probing {
                s.next_probe_ms = Some(now_ms);
            }
        }
    }

    /// State of every target, ordered by id.
    #[must_use]
    pub fn snapshot(&self) -> Vec<TargetState> {
        self.targets.values().cloned().collect()
    }

    /// State of one target.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&TargetState> {
        self.targets.get(id)
    }

    /// Recent transitions, oldest first.
    #[must_use]
    pub fn history(&self) -> Vec<Transition> {
        self.history.iter().cloned().collect()
    }

    /// Whether the device has a network, as last reported.
    #[must_use]
    pub fn network(&self) -> bool {
        self.network
    }
}

/// Probes one target: a cheap protocol check (TCP connect plus the
/// protocol's hello). Must return within a few seconds (its own timeout)
/// and must not put secrets into the reasons.
pub trait Prober: Send + Sync {
    /// Probes `target`.
    fn probe(&self, target: &Target) -> ProbeOutcome;
}

/// Wall clock in Unix ms; tests use a fake one.
pub trait Clock: Send + Sync {
    /// Now, Unix ms.
    fn now_ms(&self) -> i64;
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, ms)
    }
}

/// Where the targets come from (sources, sync, endpoints), read at every
/// step of the monitor, and the passive signals they recorded since.
pub trait TargetSource: Send + Sync {
    /// Every target that exists now.
    fn targets(&self) -> Vec<Target>;

    /// Results of real operations since the last call (a scan that
    /// reached a source, a sync round that failed…).
    fn passive(&self) -> Vec<(String, ProbeOutcome)> {
        Vec::new()
    }
}

/// A fixed list of targets (tests, tools).
impl TargetSource for Mutex<Vec<Target>> {
    fn targets(&self) -> Vec<Target> {
        self.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

struct Shared {
    tracker: Mutex<Tracker>,
    wake: Condvar,
    stop: AtomicBool,
    bus: EventBus<AvailabilityEvent>,
    prober: Arc<dyn Prober>,
    source: Arc<dyn TargetSource>,
    clock: Arc<dyn Clock>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Tracker> {
        self.tracker.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, transitions: Vec<Transition>) {
        for t in transitions {
            self.bus.publish(&AvailabilityEvent::Changed(t));
        }
    }
}

/// The availability monitor: a [`Tracker`] driven by a [`Prober`], a
/// [`TargetSource`] and a [`Clock`].
///
/// [`start`](Self::start) runs it on its own thread until the last handle
/// is dropped or [`stop`](Self::stop); tests call [`tick`](Self::tick).
#[derive(Clone)]
pub struct Monitor {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Monitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Monitor").finish_non_exhaustive()
    }
}

/// How often the thread looks for new targets and passive signals when
/// no probe is due.
const REFRESH: Duration = Duration::from_secs(5);

impl Monitor {
    /// A monitor that does not run yet.
    pub fn new(
        policy: Policy,
        prober: Arc<dyn Prober>,
        source: Arc<dyn TargetSource>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                tracker: Mutex::new(Tracker::new(policy)),
                wake: Condvar::new(),
                stop: AtomicBool::new(false),
                bus: EventBus::new(),
                prober,
                source,
                clock,
            }),
        }
    }

    /// Starts the monitor thread. It stops with [`stop`](Self::stop).
    ///
    /// # Errors
    ///
    /// The thread could not be spawned.
    pub fn start(&self) -> std::io::Result<()> {
        let shared = Arc::downgrade(&self.shared);
        std::thread::Builder::new()
            .name("pa-availability".into())
            .spawn(move || {
                loop {
                    let Some(s) = shared.upgrade() else { return };
                    if s.stop.load(Ordering::Acquire) {
                        return;
                    }
                    let m = Self { shared: s };
                    m.tick();
                    let s = &m.shared;
                    let now = s.clock.now_ms();
                    let guard = s.lock();
                    let wait = guard
                        .next_wakeup()
                        .map_or(ms(REFRESH), |t| (t - now).clamp(50, ms(REFRESH)));
                    let wait = Duration::from_millis(u64::try_from(wait).unwrap_or(0));
                    drop(s.wake.wait_timeout(guard, wait));
                }
            })
            .map(drop)
    }

    /// Stops the thread (within a moment).
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.wake.notify_all();
    }

    /// The events: every status change and target set change.
    #[must_use]
    pub fn events(&self) -> &EventBus<AvailabilityEvent> {
        &self.shared.bus
    }

    /// One step: reads the targets and passive signals, then probes every
    /// due target (in parallel) and records the results.
    pub fn tick(&self) {
        let s = &self.shared;
        let targets = s.source.targets();
        let passive = s.source.passive();
        let now = s.clock.now_ms();
        let (changed, mut transitions, due) = {
            let mut t = s.lock();
            let changed = t.set_targets(targets, now);
            let transitions: Vec<Transition> = passive
                .iter()
                .filter_map(|(id, o)| t.record(id, o, now, true))
                .collect();
            (changed, transitions, t.due(now))
        };
        if changed {
            s.bus.publish(&AvailabilityEvent::TargetsChanged);
        }
        let results: Vec<(String, ProbeOutcome)> = std::thread::scope(|scope| {
            let handles: Vec<_> = due
                .iter()
                .map(|target| {
                    let prober = Arc::clone(&s.prober);
                    scope.spawn(move || (target.id.clone(), prober.probe(target)))
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        let now = s.clock.now_ms();
        {
            let mut t = s.lock();
            transitions.extend(
                results
                    .iter()
                    .filter_map(|(id, o)| t.record(id, o, now, false)),
            );
        }
        s.publish(transitions);
    }

    /// A real operation on target `id` succeeded or failed (a scan, a
    /// playback, a sync round): updates the status without a probe.
    pub fn report(&self, id: &str, outcome: &ProbeOutcome) {
        let now = self.shared.clock.now_ms();
        let t = self.shared.lock().record(id, outcome, now, true);
        self.shared.publish(t.into_iter().collect());
        self.shared.wake.notify_all();
    }

    /// The device lost its network (`false`) or got one (`true`, also
    /// after switching networks): see [`Tracker::network_changed`].
    pub fn network_changed(&self, online: bool) {
        let now = self.shared.clock.now_ms();
        let t = self.shared.lock().network_changed(online, now);
        self.shared.publish(t);
        self.shared.wake.notify_all();
    }

    /// The app came to the front or left it.
    pub fn set_app_state(&self, app: AppState) {
        let now = self.shared.clock.now_ms();
        self.shared.lock().set_app_state(app, now);
        self.shared.wake.notify_all();
    }

    /// Probe `id` (or everything) as soon as possible.
    pub fn probe_now(&self, id: Option<&str>) {
        let now = self.shared.clock.now_ms();
        self.shared.lock().probe_now(id, now);
        self.shared.wake.notify_all();
    }

    /// State of every target.
    #[must_use]
    pub fn snapshot(&self) -> Vec<TargetState> {
        self.shared.lock().snapshot()
    }

    /// Recent transitions, oldest first.
    #[must_use]
    pub fn history(&self) -> Vec<Transition> {
        self.shared.lock().history()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicI64;

    const S: i64 = 1000;

    fn target(id: &str) -> Target {
        Target::new(id, TargetKind::Smb, id)
    }

    fn ok() -> ProbeOutcome {
        ProbeOutcome::Ok { rtt_ms: 5 }
    }

    fn down() -> ProbeOutcome {
        ProbeOutcome::Unreachable("connection refused".into())
    }

    #[test]
    fn new_targets_are_probed_at_once() {
        let mut t = Tracker::new(Policy::default());
        assert!(t.set_targets(vec![target("a"), target("b")], 0));
        assert_eq!(t.due(0).len(), 2);
        assert!(!t.set_targets(vec![target("a"), target("b")], 0));
    }

    #[test]
    fn up_target_is_probed_every_30_s() {
        let mut t = Tracker::new(Policy::default());
        t.set_targets(vec![target("a")], 0);
        let tr = t.record("a", &ok(), 0, false).unwrap();
        assert_eq!((tr.from, tr.to), (Status::Unknown, Status::Available));
        assert!(t.due(29 * S).is_empty());
        assert_eq!(t.due(30 * S).len(), 1);
        assert_eq!(t.next_wakeup(), Some(30 * S));
    }

    #[test]
    fn one_failed_probe_marks_unavailable_within_35_s() {
        // Probed at 0, container stops at 1 s: the probe at 30 s fails
        // (well within 35 s with a 5 s probe timeout).
        let mut t = Tracker::new(Policy::default());
        t.set_targets(vec![target("a")], 0);
        t.record("a", &ok(), 0, false);
        let tr = t.record("a", &down(), 30 * S, false).unwrap();
        assert_eq!(tr.to, Status::Unavailable);
        assert_eq!(t.get("a").unwrap().since_ms, Some(30 * S));
        assert_eq!(tr.reason.as_deref(), Some("connection refused"));
    }

    #[test]
    fn backoff_doubles_up_to_5_min_and_resets() {
        let mut t = Tracker::new(Policy::default());
        t.set_targets(vec![target("a")], 0);
        let mut now = 0;
        let mut gaps = Vec::new();
        for _ in 0..7 {
            t.record("a", &down(), now, false);
            let next = t.get("a").unwrap().next_probe_ms.unwrap();
            gaps.push((next - now) / S);
            now = next;
        }
        assert_eq!(gaps, vec![30, 60, 120, 240, 300, 300, 300]);
        // "Unavailable since" stays at the first failure.
        assert_eq!(t.get("a").unwrap().since_ms, Some(0));
        t.record("a", &ok(), now, false);
        assert_eq!(t.get("a").unwrap().failures, 0);
        assert_eq!(t.get("a").unwrap().next_probe_ms, Some(now + 30 * S));
    }

    #[test]
    fn wrong_password_is_no_access_not_unavailable() {
        let mut t = Tracker::new(Policy::default());
        t.set_targets(vec![target("a")], 0);
        let tr = t
            .record(
                "a",
                &ProbeOutcome::Denied("authentication failed".into()),
                0,
                false,
            )
            .unwrap();
        assert_eq!(tr.to, Status::NoAccess);
        // No backoff: the user may fix the password any moment.
        assert_eq!(t.get("a").unwrap().next_probe_ms, Some(30 * S));
        assert_eq!(t.get("a").unwrap().failures, 0);
    }

    #[test]
    fn slow_round_trip_is_slow() {
        let mut t = Tracker::new(Policy::default());
        t.set_targets(vec![target("a")], 0);
        let tr = t
            .record("a", &ProbeOutcome::Ok { rtt_ms: 2000 }, 0, false)
            .unwrap();
        assert_eq!(tr.to, Status::Slow);
        // A passive success keeps "slow": it measures nothing.
        assert!(
            t.record("a", &ProbeOutcome::Ok { rtt_ms: 0 }, S, true)
                .is_none()
        );
        let tr = t.record("a", &ok(), 30 * S, false).unwrap();
        assert_eq!(tr.to, Status::Available);
    }

    #[test]
    fn network_change_probes_everything_at_once() {
        let mut t = Tracker::new(Policy::default());
        t.set_targets(vec![target("a"), target("b")], 0);
        for _ in 0..4 {
            t.record("a", &down(), 0, false);
        }
        t.record("b", &ok(), 0, false);
        let off = t.network_changed(false, 10 * S);
        assert_eq!(off.len(), 2);
        assert!(off.iter().all(|x| x.to == Status::NoNetwork));
        assert!(t.due(1_000 * S).is_empty());
        assert_eq!(t.next_wakeup(), None);
        // Failed operations while offline say nothing about the target.
        assert!(t.record("b", &down(), 11 * S, true).is_none());
        t.network_changed(true, 20 * S);
        assert_eq!(t.due(20 * S).len(), 2);
        assert_eq!(t.get("a").unwrap().failures, 0);
    }

    #[test]
    fn container_back_is_seen_within_10_s_of_a_network_event() {
        let mut t = Tracker::new(Policy::default());
        t.set_targets(vec![target("a")], 0);
        for i in 0..5 {
            t.record("a", &down(), i * 300 * S, false);
        }
        // Next probe by backoff is 5 min away; a network event probes now.
        t.network_changed(true, 1_300 * S);
        assert_eq!(t.due(1_300 * S).len(), 1);
        let tr = t.record("a", &ok(), 1_301 * S, false).unwrap();
        assert_eq!(tr.to, Status::Available);
    }

    #[test]
    fn phone_in_background_does_not_probe() {
        let mut t = Tracker::new(Policy {
            mobile: true,
            ..Policy::default()
        });
        t.set_targets(vec![target("a")], 0);
        t.record("a", &ok(), 0, false);
        t.set_app_state(AppState::Background, S);
        assert!(!t.probing());
        assert!(t.due(10_000 * S).is_empty());
        assert_eq!(t.next_wakeup(), None);
        // New targets arriving by sync in the background wait too.
        t.set_targets(vec![target("a"), target("b")], 2 * S);
        assert!(t.due(10_000 * S).is_empty());
        // Back in front: everything at once.
        t.set_app_state(AppState::Foreground, 100 * S);
        assert_eq!(t.due(100 * S).len(), 2);
    }

    #[test]
    fn desktop_in_background_probes_every_5_min() {
        let mut t = Tracker::new(Policy::default());
        t.set_targets(vec![target("a")], 0);
        t.record("a", &ok(), 0, false);
        t.set_app_state(AppState::Background, S);
        assert!(t.due(30 * S).is_empty());
        assert_eq!(t.next_wakeup(), Some(301 * S));
        t.record("a", &ok(), 301 * S, false);
        assert_eq!(t.get("a").unwrap().next_probe_ms, Some(601 * S));
    }

    #[test]
    fn passive_success_postpones_the_probe() {
        let mut t = Tracker::new(Policy::default());
        t.set_targets(vec![target("a")], 0);
        t.record("a", &ok(), 0, false);
        t.record("a", &ProbeOutcome::Ok { rtt_ms: 0 }, 25 * S, true);
        assert!(t.due(30 * S).is_empty());
        assert_eq!(t.due(55 * S).len(), 1);
        // A failed scan marks it down at once.
        let tr = t.record("a", &down(), 40 * S, true).unwrap();
        assert!(tr.passive);
        assert_eq!(tr.to, Status::Unavailable);
    }

    #[test]
    fn passive_targets_are_never_probed() {
        let mut t = Tracker::new(Policy::default());
        let mut peer = Target::new("peer:x", TargetKind::SyncPeer, "TV");
        peer.active = false;
        t.set_targets(vec![peer], 0);
        assert!(t.due(0).is_empty());
        let tr = t.record("peer:x", &ok(), 0, true).unwrap();
        assert_eq!(tr.to, Status::Available);
        assert!(t.due(1_000 * S).is_empty());
    }

    #[test]
    fn min_interval_slows_clouds_down() {
        let mut t = Tracker::new(Policy::default());
        let mut cloud = Target::new("source:9", TargetKind::Cloud, "Drive");
        cloud.min_interval = Some(Duration::from_secs(300));
        t.set_targets(vec![cloud], 0);
        t.record("source:9", &ok(), 0, false);
        assert_eq!(t.next_wakeup(), Some(300 * S));
    }

    #[test]
    fn history_keeps_the_last_transitions() {
        let mut t = Tracker::new(Policy {
            history_len: 3,
            ..Policy::default()
        });
        t.set_targets(vec![target("a")], 0);
        for i in 0..6 {
            let o = if i % 2 == 0 { ok() } else { down() };
            t.record("a", &o, i * S, false);
        }
        let h = t.history();
        assert_eq!(h.len(), 3);
        assert_eq!(h.last().unwrap().at_ms, 5 * S);
    }

    #[test]
    fn removed_targets_are_dropped_and_known_ones_keep_state() {
        let mut t = Tracker::new(Policy::default());
        t.set_targets(vec![target("a"), target("b")], 0);
        t.record("a", &down(), 0, false);
        let mut renamed = target("a");
        renamed.name = "NAS".into();
        assert!(t.set_targets(vec![renamed], S));
        assert_eq!(t.snapshot().len(), 1);
        let a = t.get("a").unwrap();
        assert_eq!(
            (a.status, a.target.name.as_str()),
            (Status::Unavailable, "NAS")
        );
        assert!(t.record("b", &ok(), S, false).is_none());
    }

    // --- the driver, with a fake prober and clock ---

    struct FakeClock(AtomicI64);
    impl Clock for FakeClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    #[derive(Default)]
    struct FakeProber {
        answers: Mutex<HashMap<String, ProbeOutcome>>,
        calls: Mutex<Vec<String>>,
    }
    impl Prober for FakeProber {
        fn probe(&self, target: &Target) -> ProbeOutcome {
            self.calls.lock().unwrap().push(target.id.clone());
            self.answers
                .lock()
                .unwrap()
                .get(&target.id)
                .cloned()
                .unwrap_or_else(ok)
        }
    }

    struct Fixture {
        monitor: Monitor,
        clock: Arc<FakeClock>,
        prober: Arc<FakeProber>,
        targets: Arc<Mutex<Vec<Target>>>,
        events: Arc<Mutex<Vec<AvailabilityEvent>>>,
    }

    fn fixture(targets: Vec<Target>) -> Fixture {
        let clock = Arc::new(FakeClock(AtomicI64::new(0)));
        let prober = Arc::new(FakeProber::default());
        let targets = Arc::new(Mutex::new(targets));
        let monitor = Monitor::new(
            Policy::default(),
            Arc::clone(&prober) as Arc<dyn Prober>,
            Arc::clone(&targets) as Arc<dyn TargetSource>,
            Arc::clone(&clock) as Arc<dyn Clock>,
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        monitor.events().subscribe(move |e| {
            sink.lock().unwrap().push(e.clone());
            true
        });
        Fixture {
            monitor,
            clock,
            prober,
            targets,
            events,
        }
    }

    #[test]
    fn monitor_probes_and_publishes_changes() {
        let f = fixture(vec![target("source:1"), target("source:2")]);
        f.prober.answers.lock().unwrap().insert(
            "source:2".into(),
            ProbeOutcome::Denied("bad password".into()),
        );
        f.monitor.tick();
        let states: Vec<Status> = f.monitor.snapshot().iter().map(|s| s.status).collect();
        assert_eq!(states, vec![Status::Available, Status::NoAccess]);
        let events = f.events.lock().unwrap();
        assert_eq!(events[0], AvailabilityEvent::TargetsChanged);
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn monitor_does_not_probe_before_due() {
        let f = fixture(vec![target("source:1")]);
        f.monitor.tick();
        f.clock.0.store(10 * S, Ordering::SeqCst);
        f.monitor.tick();
        assert_eq!(f.prober.calls.lock().unwrap().len(), 1);
        f.clock.0.store(30 * S, Ordering::SeqCst);
        f.monitor.tick();
        assert_eq!(f.prober.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn source_arriving_by_sync_is_checked_like_any_other() {
        let f = fixture(vec![target("source:1")]);
        f.monitor.tick();
        let mut synced = target("source:7");
        synced.origin = Origin::Synced;
        let mut folder = target("source:8");
        folder.origin = Origin::Auto;
        folder.sync_channel = true;
        f.targets.lock().unwrap().extend([synced, folder]);
        f.clock.0.store(S, Ordering::SeqCst);
        f.monitor.tick();
        let calls = f.prober.calls.lock().unwrap().clone();
        assert_eq!(calls.iter().filter(|c| *c == "source:7").count(), 1);
        assert_eq!(calls.iter().filter(|c| *c == "source:8").count(), 1);
        assert!(
            f.monitor
                .snapshot()
                .iter()
                .all(|s| s.status == Status::Available)
        );
    }

    #[test]
    fn monitor_report_and_network_events() {
        let f = fixture(vec![target("source:1")]);
        f.monitor.tick();
        f.monitor.report("source:1", &down());
        assert_eq!(f.monitor.snapshot()[0].status, Status::Unavailable);
        f.monitor.network_changed(false);
        assert_eq!(f.monitor.snapshot()[0].status, Status::NoNetwork);
        f.clock.0.store(1_000 * S, Ordering::SeqCst);
        f.monitor.tick();
        assert_eq!(f.prober.calls.lock().unwrap().len(), 1);
        f.monitor.network_changed(true);
        f.monitor.tick();
        assert_eq!(f.monitor.snapshot()[0].status, Status::Available);
        assert_eq!(f.monitor.history().len(), 4);
    }

    #[test]
    fn monitor_thread_runs_and_stops() {
        let f = fixture(vec![target("source:1")]);
        f.monitor.start().unwrap();
        for _ in 0..100 {
            if !f.prober.calls.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        f.monitor.stop();
        assert!(!f.prober.calls.lock().unwrap().is_empty());
    }
}
