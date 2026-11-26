// SPDX-License-Identifier: MPL-2.0
//! Live progress of library uploads on every device (LIB-028, ADR-NNNN),
//! sans-IO.
//!
//! A file dropped into the library on one device shows up at once on the
//! others as a grey card with a moving bar. The bar moves on ephemeral
//! `up` frames, like `pb.update` (SYNC-003) and `music` (MUS-015): relays
//! fan them out once per stamp and never store them; no outbox, journal
//! or HLC record in a database. What is durable — started, recognized,
//! done, cancelled — is the `upload.event` op ([`crate::OpBody::UploadEvent`]),
//! synced like any other op.
//!
//! - [`UploadFrame`] (`t: "up"`): progress of several uploads of one
//!   device in one frame ([`UploadTick`], a few dozen bytes each), a
//!   subscription ([`UploadFrame::watch`]) or a cancel request
//!   ([`UploadFrame::cancel`]).
//! - [`Pacer`], on the device that uploads: turns the app's reports (any
//!   rate) into frames — every [`Pace::watched_ms`] (100 ms on the LAN)
//!   while another device watches a screen with the card, else every
//!   [`Pace::idle_ms`] (1 s); only uploads that moved; a state change at
//!   once; speed as a moving average here, not on every receiver.
//! - [`Board`], on the devices that watch: the newest tick of every upload
//!   and whether its device went quiet ([`UploadState::Offline`]).
//!
//! ```
//! use pa_sync::hlc::Hlc;
//! use pa_sync::upload::{Board, Pace, Pacer, UploadFrame, UploadState};
//!
//! let mut pacer = Pacer::new(Pace::LAN);
//! pacer.watch("tv", 3_000, 0);
//! pacer.report("u1", UploadState::Copying, 0, 1_000_000, 0);
//! let first = pacer.poll(0).unwrap();
//! pacer.report("u1", UploadState::Copying, 8_500, 1_000_000, 100);
//! let ticks = pacer.poll(100).unwrap();
//! assert_eq!(ticks[0].done, 8_500);
//!
//! let hlc = Hlc { wall_ms: 100, counter: 0, device: "pc".into() };
//! let mut board = Board::default();
//! assert!(board.apply(&UploadFrame::ticks(hlc, ticks), 100));
//! assert_eq!(board.uploads(150)[0].done, 8_500);
//! # let _ = first;
//! ```

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::hlc::{Hlc, is_valid_device_id};

/// The rate of progress frames on one transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pace {
    /// Period while another device watches (a subscriber is live).
    pub watched_ms: u64,
    /// Period while nobody watches.
    pub idle_ms: u64,
}

impl Pace {
    /// The LAN (P2P node or the user's server hub): 10 Hz while watched,
    /// 1 Hz otherwise.
    pub const LAN: Self = Self {
        watched_ms: 100,
        idle_ms: 1_000,
    };

    /// The seam for the Kinolune cloud relay outside home (SUB-011,
    /// ECO-009, not built): the same frames, sealed with the family key
    /// ([`seal_frame`]), at most once a second whoever watches.
    pub const CLOUD: Self = Self {
        watched_ms: 1_000,
        idle_ms: 1_000,
    };

    fn period(self, watched: bool) -> u64 {
        if watched {
            self.watched_ms
        } else {
            self.idle_ms
        }
    }
}

/// A watcher's subscription lives this long; a screen with the card open
/// renews it every [`WATCH_RENEW_MS`].
pub const WATCH_TTL_MS: u64 = 3_000;

/// How often a device that shows upload cards renews its subscription.
pub const WATCH_RENEW_MS: u64 = 1_000;

/// An active upload is in a frame at least this often even if it did not
/// move (a stalled copy is still alive).
pub const KEEPALIVE_MS: u64 = 1_000;

/// An active upload with no frame for this long is shown as paused: its
/// device is offline («Пауза: <устройство> не в сети»).
pub const STALE_MS: u64 = 4_000;

/// A finished, cancelled or failed upload stays on the board this long
/// (the card turns normal or goes away; then it is forgotten).
pub const LINGER_MS: u64 = 10_000;

/// Progress of this share of the size goes out before its period ends
/// (when a frame goes anyway): 1/200 = 0.5 %.
pub const MIN_STEP_DIVISOR: u64 = 200;

/// Time constant of the speed's moving average.
pub const SPEED_TAU_MS: f64 = 2_000.0;

/// Longest upload id (`[0-9A-Za-z_-]`).
pub const MAX_ID_LEN: usize = 40;

/// Most uploads in one frame (the rest go in the next one).
pub const MAX_TICKS: usize = 64;

/// Most ids in one cancel request.
pub const MAX_CANCEL: usize = 64;

/// State of one upload, one letter on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UploadState {
    /// In the queue of its device.
    #[serde(rename = "w")]
    Waiting,
    /// Bytes are moving.
    #[serde(rename = "c")]
    Copying,
    /// The copy is complete: checked and renamed from `.part`.
    #[serde(rename = "v")]
    Verifying,
    /// The source cannot be reached; the device resumes by itself.
    #[serde(rename = "p")]
    Paused,
    /// In the library.
    #[serde(rename = "d")]
    Done,
    /// Cancelled: the card goes away everywhere.
    #[serde(rename = "x")]
    Cancelled,
    /// Failed for good (no space …), waits for «Повторить».
    #[serde(rename = "f")]
    Failed,
    /// Never sent: the receiver's view of an upload whose device went
    /// quiet for [`STALE_MS`].
    #[serde(rename = "o")]
    Offline,
    /// A state of a newer version; read as copying.
    #[serde(other)]
    Other,
}

impl UploadState {
    /// The upload has ended (done, cancelled, failed).
    pub fn is_final(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled | Self::Failed)
    }

    /// The upload is still on its way.
    pub fn is_active(self) -> bool {
        !self.is_final()
    }
}

/// Progress of one upload: `{"i":"…","s":"c","b":…,"n":…,"r":…}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadTick {
    /// Id of the upload (the `upload` of its `upload.event` ops).
    #[serde(rename = "i")]
    pub id: String,
    #[serde(rename = "s")]
    pub state: UploadState,
    /// Bytes written to the source.
    #[serde(rename = "b")]
    pub done: u64,
    /// Size of the file.
    #[serde(rename = "n")]
    pub size: u64,
    /// Speed, bytes per second, averaged by the sender.
    #[serde(rename = "r", default)]
    pub rate: u64,
}

impl UploadTick {
    /// Seconds left at the current speed, if it moves.
    pub fn eta_s(&self) -> Option<u64> {
        (self.rate > 0 && self.state == UploadState::Copying)
            .then(|| self.size.saturating_sub(self.done).div_ceil(self.rate))
    }
}

/// `up`: progress of a device's uploads, a subscription or a cancel
/// request. Ephemeral: relayed once per stamp, never stored or replayed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadFrame {
    /// When it was sent; `hlc.device` is the sender.
    pub hlc: Hlc,
    /// Progress of the sender's uploads that changed.
    #[serde(rename = "u", default, skip_serializing_if = "Vec::is_empty")]
    pub ticks: Vec<UploadTick>,
    /// The sender shows upload cards for this many ms: devices that upload
    /// go to [`Pace::watched_ms`] until it runs out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<u32>,
    /// Cancel these uploads of device [`to`](Self::to).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cancel: Vec<String>,
    /// The device a cancel request is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

fn check_id(id: &str) -> Result<(), Error> {
    let ok = !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then_some(()).ok_or(Error::Invalid("upload id"))
}

/// Whether `id` is a valid upload id.
pub fn is_valid_id(id: &str) -> bool {
    check_id(id).is_ok()
}

impl UploadFrame {
    /// A progress frame.
    pub fn ticks(hlc: Hlc, ticks: Vec<UploadTick>) -> Self {
        Self {
            hlc,
            ticks,
            watch: None,
            cancel: Vec::new(),
            to: None,
        }
    }

    /// A subscription for `ttl_ms`.
    pub fn watching(hlc: Hlc, ttl_ms: u32) -> Self {
        Self {
            watch: Some(ttl_ms),
            ..Self::ticks(hlc, Vec::new())
        }
    }

    /// A request to `to` to cancel `ids`.
    pub fn cancelling(hlc: Hlc, to: impl Into<String>, ids: Vec<String>) -> Self {
        Self {
            cancel: ids,
            to: Some(to.into()),
            ..Self::ticks(hlc, Vec::new())
        }
    }

    /// Checks ids and sizes.
    pub fn validate(&self) -> Result<(), Error> {
        if !is_valid_device_id(&self.hlc.device) {
            return Err(Error::Invalid("device id"));
        }
        if self.ticks.len() > MAX_TICKS || self.cancel.len() > MAX_CANCEL {
            return Err(Error::Invalid("upload frame"));
        }
        for t in &self.ticks {
            check_id(&t.id)?;
            if t.done > t.size || t.state == UploadState::Offline {
                return Err(Error::Invalid("upload tick"));
            }
        }
        for id in &self.cancel {
            check_id(id)?;
        }
        match (&self.to, self.cancel.is_empty()) {
            (Some(to), false) if is_valid_device_id(to) => Ok(()),
            (None, true) => Ok(()),
            _ => Err(Error::Invalid("upload cancel")),
        }
    }
}

/// The seal context of a sender's frames on the cloud relay (the seam of
/// ADR-NNNN §7; the relay itself is not built).
pub fn seal_context(device: &str) -> String {
    format!("up:{device}")
}

/// Seals a frame's JSON for a relay that must not read it (end to end,
/// family key). The LAN sends frames in clear inside its TLS links.
pub fn seal_frame(
    key: &crate::seal::SealKey,
    frame: &UploadFrame,
) -> Result<String, crate::seal::SealError> {
    let json = serde_json::to_string(frame).map_err(|_| crate::seal::SealError::Malformed)?;
    crate::seal::seal_up_to(
        key,
        &seal_context(&frame.hlc.device),
        json.as_bytes(),
        crate::proto::MAX_FRAME_LEN,
    )
}

/// Opens a frame sealed by [`seal_frame`] by device `from`.
pub fn open_frame(
    key: &crate::seal::SealKey,
    from: &str,
    sealed: &str,
) -> Result<UploadFrame, crate::seal::SealError> {
    let plain = crate::seal::open_up_to(
        key,
        &seal_context(from),
        sealed,
        crate::proto::MAX_FRAME_LEN,
    )?;
    let frame: UploadFrame =
        serde_json::from_slice(&plain).map_err(|_| crate::seal::SealError::Malformed)?;
    if frame.hlc.device != from || frame.validate().is_err() {
        return Err(crate::seal::SealError::Malformed);
    }
    Ok(frame)
}

/// `up` traffic of one node: frames and bytes of their JSON (without the
/// WebSocket and TLS framing, a few dozen bytes more per frame).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Traffic {
    pub recv_frames: u64,
    pub recv_bytes: u64,
    /// Per link: a frame sent to two links counts twice.
    pub sent_frames: u64,
    pub sent_bytes: u64,
}

// --- the sender -----------------------------------------------------------

/// Speed as an exponential moving average over [`SPEED_TAU_MS`].
#[derive(Debug, Clone, Default)]
struct Speed {
    last: Option<(u64, u64)>,
    rate: f64,
}

impl Speed {
    fn sample(&mut self, done: u64, now_ms: u64) {
        let Some((at, bytes)) = self.last else {
            self.last = Some((now_ms, done));
            return;
        };
        let dt = now_ms.saturating_sub(at);
        if done < bytes {
            // Restarted from a lower offset (a resume): start over.
            *self = Self::default();
            self.last = Some((now_ms, done));
            return;
        }
        // Samples closer than 50 ms are noise; keep collecting.
        if dt < 50 {
            return;
        }
        #[allow(clippy::cast_precision_loss)]
        let inst = (done - bytes) as f64 * 1000.0 / dt as f64;
        #[allow(clippy::cast_precision_loss)]
        let a = 1.0 - (-(dt as f64) / SPEED_TAU_MS).exp();
        self.rate = if self.rate == 0.0 {
            inst
        } else {
            self.rate + a * (inst - self.rate)
        };
        self.last = Some((now_ms, done));
    }

    fn get(&self) -> u64 {
        if self.rate.is_finite() && self.rate > 0.0 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let r = self.rate.round() as u64;
            r
        } else {
            0
        }
    }
}

#[derive(Debug, Clone)]
struct Local {
    state: UploadState,
    done: u64,
    size: u64,
    speed: Speed,
    /// What was sent last, and when.
    sent: Option<(UploadState, u64, u64)>,
}

/// The sender's side: reports in, frames out, at the pace of whoever
/// watches. Drive it with [`poll`](Self::poll) at [`next_poll_ms`](Self::next_poll_ms).
#[derive(Debug, Clone)]
pub struct Pacer {
    pace: Pace,
    uploads: BTreeMap<String, Local>,
    /// Watching devices and when their subscription runs out.
    watchers: HashMap<String, u64>,
    last_frame_ms: Option<u64>,
    /// A new watcher came: send everything at once.
    full: bool,
}

impl Pacer {
    /// A pacer for a transport of `pace`.
    pub fn new(pace: Pace) -> Self {
        Self {
            pace,
            uploads: BTreeMap::new(),
            watchers: HashMap::new(),
            last_frame_ms: None,
            full: false,
        }
    }

    /// The app reports an upload of this device (as often as it likes).
    pub fn report(&mut self, id: &str, state: UploadState, done: u64, size: u64, now_ms: u64) {
        if !is_valid_id(id) || state == UploadState::Offline {
            return;
        }
        let done = done.min(size);
        let local = self.uploads.entry(id.to_owned()).or_insert(Local {
            state,
            done,
            size,
            speed: Speed::default(),
            sent: None,
        });
        local.state = state;
        local.size = size;
        local.done = done;
        if state == UploadState::Copying {
            local.speed.sample(done, now_ms);
        } else {
            local.speed = Speed::default();
        }
    }

    /// The uploads this device reports, as ticks (for its own screen).
    pub fn local(&self) -> Vec<UploadTick> {
        self.uploads.iter().map(|(id, l)| tick(id, l)).collect()
    }

    /// Device `device` shows upload cards for `ttl_ms` from `now_ms`.
    pub fn watch(&mut self, device: &str, ttl_ms: u64, now_ms: u64) {
        let until = now_ms.saturating_add(ttl_ms.min(10 * WATCH_TTL_MS));
        let was = self.watched(now_ms);
        let known = self
            .watchers
            .insert(device.to_owned(), until)
            .is_some_and(|old| old > now_ms);
        if !was || !known {
            self.full = true;
        }
    }

    /// Somebody watches now.
    pub fn watched(&self, now_ms: u64) -> bool {
        self.watchers.values().any(|until| *until > now_ms)
    }

    /// The period now: [`Pace::watched_ms`] or [`Pace::idle_ms`].
    pub fn period(&self, now_ms: u64) -> u64 {
        self.pace.period(self.watched(now_ms))
    }

    /// Whether there is anything to send ever again.
    pub fn is_idle(&self) -> bool {
        self.uploads.is_empty()
    }

    /// When to call [`poll`](Self::poll) next (Unix ms), `None` when there
    /// is nothing to report.
    pub fn next_poll_ms(&self, now_ms: u64) -> Option<u64> {
        if self.uploads.is_empty() {
            return None;
        }
        let urgent = self.full
            || self
                .uploads
                .values()
                .any(|l| l.sent.is_none_or(|(s, _, _)| s != l.state));
        if urgent {
            return Some(now_ms);
        }
        Some(self.last_frame_ms.map_or(now_ms, |t| {
            t.saturating_add(self.period(now_ms)).max(now_ms)
        }))
    }

    /// The ticks to send now, if any. Batches every upload that is due:
    /// a new state at once; bytes that moved when the period is over, or
    /// moved by ≥ 0.5 % when a frame goes anyway; an active upload at
    /// least every [`KEEPALIVE_MS`]. Final uploads leave after they went.
    pub fn poll(&mut self, now_ms: u64) -> Option<Vec<UploadTick>> {
        self.watchers.retain(|_, until| *until > now_ms);
        let period = self.period(now_ms);
        let period_over = self
            .last_frame_ms
            .is_none_or(|t| now_ms.saturating_sub(t) >= period);
        let full = std::mem::take(&mut self.full);
        let state_changed = self
            .uploads
            .values()
            .any(|l| l.sent.is_none_or(|(s, _, _)| s != l.state));
        if !(period_over || state_changed || full) {
            return None;
        }
        let mut out = Vec::new();
        for (id, l) in &mut self.uploads {
            let due = match l.sent {
                _ if full => true,
                None => true,
                Some((s, _, _)) if s != l.state => true,
                Some((_, bytes, at)) => {
                    let moved = l.done != bytes;
                    let step =
                        l.done.abs_diff(bytes).saturating_mul(MIN_STEP_DIVISOR) >= l.size && moved;
                    let stale = now_ms.saturating_sub(at) >= KEEPALIVE_MS && l.state.is_active();
                    (period_over && (moved || stale)) || step
                }
            };
            if due && out.len() < MAX_TICKS {
                out.push(tick(id, l));
                l.sent = Some((l.state, l.done, now_ms));
            }
        }
        self.uploads
            .retain(|_, l| !(l.state.is_final() && l.sent.is_some_and(|(s, _, _)| s == l.state)));
        if out.is_empty() {
            return None;
        }
        self.last_frame_ms = Some(now_ms);
        Some(out)
    }
}

fn tick(id: &str, l: &Local) -> UploadTick {
    UploadTick {
        id: id.to_owned(),
        state: l.state,
        done: l.done,
        size: l.size,
        rate: if l.state == UploadState::Copying {
            l.speed.get()
        } else {
            0
        },
    }
}

// --- the receivers --------------------------------------------------------

/// An upload of another device as a receiver sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUpload {
    /// The device that uploads.
    pub device: String,
    /// Its last tick; [`UploadState::Offline`] when it went quiet.
    pub tick: UploadTick,
}

impl std::ops::Deref for RemoteUpload {
    type Target = UploadTick;
    fn deref(&self) -> &UploadTick {
        &self.tick
    }
}

#[derive(Debug, Clone)]
struct Seen {
    device: String,
    tick: UploadTick,
    stamp: Hlc,
    /// Local time of the last frame that carried it.
    at_ms: u64,
}

/// The receivers' side: the newest tick of every upload of the family.
#[derive(Debug, Clone, Default)]
pub struct Board {
    uploads: BTreeMap<String, Seen>,
    /// Devices that went away: their active uploads read as offline.
    gone: HashMap<String, u64>,
}

impl Board {
    /// Takes the ticks of `frame` received at `now_ms` (local clock).
    /// Returns whether anything a card shows changed. Older frames of an
    /// upload than the one it holds are ignored.
    pub fn apply(&mut self, frame: &UploadFrame, now_ms: u64) -> bool {
        let device = &frame.hlc.device;
        let mut changed = self.gone.remove(device).is_some() && !frame.ticks.is_empty();
        for t in &frame.ticks {
            match self.uploads.get_mut(&t.id) {
                Some(seen) if seen.stamp >= frame.hlc => {}
                Some(seen) => {
                    let was_offline = now_ms.saturating_sub(seen.at_ms) >= STALE_MS;
                    changed |= seen.tick != *t || was_offline;
                    seen.tick = t.clone();
                    seen.stamp = frame.hlc.clone();
                    seen.at_ms = now_ms;
                    seen.device.clone_from(device);
                }
                None => {
                    changed = true;
                    self.uploads.insert(
                        t.id.clone(),
                        Seen {
                            device: device.clone(),
                            tick: t.clone(),
                            stamp: frame.hlc.clone(),
                            at_ms: now_ms,
                        },
                    );
                }
            }
        }
        changed
    }

    /// Device `device` is gone (its link closed): its active uploads read
    /// as offline at once.
    pub fn device_gone(&mut self, device: &str, now_ms: u64) -> bool {
        let any = self
            .uploads
            .values()
            .any(|s| s.device == device && s.tick.state.is_active());
        if any {
            self.gone.insert(device.to_owned(), now_ms);
        }
        any
    }

    /// Forgets uploads that ended [`LINGER_MS`] ago. Returns whether a card
    /// changed since `since_ms` by time alone: an upload went quiet.
    pub fn expire(&mut self, since_ms: u64, now_ms: u64) -> bool {
        self.uploads.retain(|_, s| {
            !(s.tick.state.is_final() && now_ms.saturating_sub(s.at_ms) >= LINGER_MS)
        });
        self.uploads.values().any(|s| {
            s.tick.state.is_active()
                && s.at_ms.saturating_add(STALE_MS) > since_ms
                && s.at_ms.saturating_add(STALE_MS) <= now_ms
        })
    }

    /// Whether anything is on the board.
    pub fn is_empty(&self) -> bool {
        self.uploads.is_empty()
    }

    fn offline(&self, s: &Seen, now_ms: u64) -> bool {
        s.tick.state.is_active()
            && (now_ms.saturating_sub(s.at_ms) >= STALE_MS || self.gone.contains_key(&s.device))
    }

    /// Every upload on the board at `now_ms`, offline ones marked.
    pub fn uploads(&self, now_ms: u64) -> Vec<RemoteUpload> {
        self.uploads
            .values()
            .map(|s| {
                let mut tick = s.tick.clone();
                if self.offline(s, now_ms) {
                    tick.state = UploadState::Offline;
                    tick.rate = 0;
                }
                RemoteUpload {
                    device: s.device.clone(),
                    tick,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hlc(ms: u64, device: &str) -> Hlc {
        Hlc {
            wall_ms: ms,
            counter: 0,
            device: device.into(),
        }
    }

    /// Simulates `secs` of a copy at `bps` with the app reporting every
    /// `report_ms`, polling every 10 ms; returns the frames' times and
    /// ticks.
    fn run(
        pacer: &mut Pacer,
        ids: &[&str],
        size: u64,
        bps: u64,
        report_ms: u64,
        secs: u64,
    ) -> Vec<(u64, Vec<UploadTick>)> {
        let mut out = Vec::new();
        for t in (0..=secs * 1000).step_by(10) {
            if t % report_ms == 0 {
                for id in ids {
                    pacer.report(
                        id,
                        UploadState::Copying,
                        (bps * t / 1000).min(size),
                        size,
                        t,
                    );
                }
            }
            if let Some(ticks) = pacer.poll(t) {
                out.push((t, ticks));
            }
        }
        out
    }

    #[test]
    fn watched_uploads_go_at_ten_hertz_and_idle_ones_at_one() {
        let mut pacer = Pacer::new(Pace::LAN);
        pacer.watch("tv", 60_000, 0);
        let frames = run(&mut pacer, &["a"], 3_000_000_000, 85_000_000, 20, 10);
        // 10 s at 10 Hz: 100 frames (+ the first one).
        assert!((95..=102).contains(&frames.len()), "{}", frames.len());
        for w in frames.windows(2) {
            assert!(w[1].0 - w[0].0 >= 100, "never faster than 10 Hz");
        }

        let mut idle = Pacer::new(Pace::LAN);
        let frames = run(&mut idle, &["a"], 3_000_000_000, 85_000_000, 20, 10);
        assert!((10..=12).contains(&frames.len()), "{}", frames.len());
        for w in frames.windows(2) {
            assert!(w[1].0 - w[0].0 >= 1_000, "at most 1 Hz");
        }
    }

    #[test]
    fn several_uploads_share_one_frame() {
        let mut pacer = Pacer::new(Pace::LAN);
        pacer.watch("tv", 60_000, 0);
        let frames = run(
            &mut pacer,
            &["a", "b", "c", "d"],
            3_000_000_000,
            50_000_000,
            50,
            3,
        );
        assert!(frames.iter().skip(1).all(|(_, t)| t.len() == 4));
        assert!(frames.len() <= 32);
    }

    #[test]
    fn a_new_state_goes_at_once_and_a_final_one_leaves() {
        let mut pacer = Pacer::new(Pace::LAN);
        pacer.report("a", UploadState::Copying, 0, 100, 0);
        assert!(pacer.poll(0).is_some());
        pacer.report("a", UploadState::Copying, 10, 100, 10);
        assert!(pacer.poll(20).is_none(), "idle: waits for its second");
        pacer.report("a", UploadState::Cancelled, 10, 100, 30);
        let t = pacer.poll(30).expect("cancel goes at once");
        assert_eq!(t[0].state, UploadState::Cancelled);
        assert!(pacer.is_idle(), "gone after it was sent");
        assert!(pacer.poll(2_000).is_none());
    }

    #[test]
    fn half_a_percent_rides_along_with_a_state_change() {
        let mut pacer = Pacer::new(Pace::LAN);
        pacer.report("a", UploadState::Copying, 0, 1_000, 0);
        pacer.report("b", UploadState::Copying, 0, 1_000, 0);
        pacer.poll(0).unwrap();
        pacer.report("a", UploadState::Copying, 6, 1_000, 200);
        pacer.report("b", UploadState::Copying, 4, 1_000, 200);
        pacer.report("c", UploadState::Waiting, 0, 1_000, 200);
        let t = pacer.poll(200).unwrap();
        let ids: Vec<_> = t.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["a", "c"], "b moved < 0.5 %: waits for its period");
    }

    #[test]
    fn a_stalled_copy_still_sends_a_keepalive() {
        let mut pacer = Pacer::new(Pace::LAN);
        pacer.watch("tv", 60_000, 0);
        pacer.report("a", UploadState::Copying, 5, 100, 0);
        pacer.poll(0).unwrap();
        assert!(pacer.poll(500).is_none());
        assert!(pacer.poll(1_000).is_some());
    }

    #[test]
    fn a_new_watcher_gets_everything_at_once() {
        let mut pacer = Pacer::new(Pace::LAN);
        pacer.report("a", UploadState::Copying, 5, 100, 0);
        pacer.poll(0).unwrap();
        assert!(pacer.poll(10).is_none());
        pacer.watch("tv", 3_000, 20);
        assert_eq!(pacer.next_poll_ms(20), Some(20));
        assert!(pacer.poll(20).is_some());
        // The subscription ran out: back to 1 Hz.
        assert_eq!(pacer.period(3_100), 1_000);
    }

    #[test]
    fn speed_is_a_moving_average_and_gives_the_eta() {
        let mut pacer = Pacer::new(Pace::LAN);
        pacer.watch("tv", 60_000, 0);
        let frames = run(&mut pacer, &["a"], 2_900_000_000, 85_000_000, 100, 10);
        let last = &frames.last().unwrap().1[0];
        assert!(last.rate.abs_diff(85_000_000) < 1_000_000, "{}", last.rate);
        let eta = last.eta_s().unwrap();
        assert!((23..=25).contains(&eta), "{eta}");
    }

    #[test]
    fn the_board_keeps_the_newest_and_marks_quiet_devices() {
        let mut board = Board::default();
        let t = |b| UploadTick {
            id: "a".into(),
            state: UploadState::Copying,
            done: b,
            size: 100,
            rate: 10,
        };
        assert!(board.apply(&UploadFrame::ticks(hlc(10, "pc"), vec![t(10)]), 0));
        assert!(!board.apply(&UploadFrame::ticks(hlc(5, "pc"), vec![t(5)]), 10));
        assert_eq!(board.uploads(10)[0].done, 10);
        assert!(!board.expire(0, 3_000));
        assert!(board.expire(3_000, 4_000), "went quiet");
        assert_eq!(board.uploads(4_000)[0].state, UploadState::Offline);
        assert!(board.apply(&UploadFrame::ticks(hlc(20, "pc"), vec![t(20)]), 4_100));
        assert_eq!(board.uploads(4_100)[0].state, UploadState::Copying);
        assert!(board.device_gone("pc", 4_200));
        assert_eq!(board.uploads(4_200)[0].state, UploadState::Offline);
        let mut done = t(100);
        done.state = UploadState::Done;
        assert!(board.apply(&UploadFrame::ticks(hlc(30, "pc"), vec![done]), 5_000));
        board.expire(5_000, 15_000);
        assert!(board.is_empty());
    }

    #[test]
    fn frames_validate_and_stay_small() {
        let tick = UploadTick {
            id: "u0123456789ab".into(),
            state: UploadState::Copying,
            done: 1_288_490_188,
            size: 3_113_851_289,
            rate: 89_128_960,
        };
        let json = serde_json::to_string(&tick).unwrap();
        assert!(json.len() < 80, "{json}");
        let frame = UploadFrame::ticks(hlc(1, "pc"), vec![tick.clone()]);
        frame.validate().unwrap();
        let mut bad = frame.clone();
        bad.ticks[0].done = bad.ticks[0].size + 1;
        assert!(bad.validate().is_err());
        let mut bad = frame.clone();
        bad.ticks[0].id = "a b".into();
        assert!(bad.validate().is_err());
        let cancel = UploadFrame::cancelling(hlc(1, "tv"), "pc", vec!["u1".into()]);
        cancel.validate().unwrap();
        let mut bad = cancel;
        bad.to = None;
        assert!(bad.validate().is_err());
        UploadFrame::watching(hlc(1, "tv"), 3_000)
            .validate()
            .unwrap();
        // A state of a newer version reads as `Other`.
        let t: UploadTick = serde_json::from_str(r#"{"i":"a","s":"z","b":1,"n":2}"#).unwrap();
        assert_eq!(t.state, UploadState::Other);
    }

    #[test]
    fn the_cloud_seam_seals_frames_end_to_end() {
        let key = crate::seal::SealKey::new("fam", 1, &[3; 32]);
        let frame = UploadFrame::watching(hlc(1, "tv"), 3_000);
        let sealed = seal_frame(&key, &frame).unwrap();
        assert!(!sealed.contains("watch"));
        assert_eq!(open_frame(&key, "tv", &sealed).unwrap(), frame);
        assert!(
            open_frame(&key, "pc", &sealed).is_err(),
            "bound to its sender"
        );
        let mut cloud = Pacer::new(Pace::CLOUD);
        cloud.watch("tv", 60_000, 0);
        assert_eq!(cloud.period(0), 1_000);
    }
}
