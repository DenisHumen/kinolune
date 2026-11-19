// SPDX-License-Identifier: MPL-2.0
//! How often folder sync looks at the folder (SYNC-014, ADR-0137).
//!
//! - A title plays here (heartbeats of [`crate::api::sync::CoreSync`]):
//!   a full round every [`PLAYING_GAP`], so the latest position is in the
//!   folder within a second or two; pause and stop write at once.
//! - The app is in front: a cheap look ([`pa_catalog::Catalog::folder_probe`],
//!   one listing) every [`PROBE`], a round when another device's journal
//!   changed; slower after a long quiet spell ([`probe_gap`]).
//! - In the background, or nothing else asks: a round every [`ROUND`], as
//!   before.
//! - On a metered network the steps above are several times slower.
//!
//! No setting: the pace follows what the app does.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use pa_catalog::pa_sync::hlc::system_ms;

/// A heartbeat keeps the "playing" pace this long.
pub const PLAYING_FOR_MS: u64 = 4_000;
/// Rounds while a title plays.
pub const PLAYING_GAP: Duration = Duration::from_secs(1);
/// Rounds while a title plays, on a metered network.
pub const PLAYING_GAP_METERED: Duration = Duration::from_secs(5);
/// Looks at the folder while the app is in front.
pub const PROBE: Duration = Duration::from_millis(1_500);
/// A full round at least this often, whatever else happens.
pub const ROUND: Duration = Duration::from_secs(30);

/// The gap between looks in front after `quiet` without a change by
/// another device: 1.5 s, 5 s after two minutes, 10 s after ten; 10 s on
/// a metered network.
pub fn probe_gap(quiet: Duration, metered: bool) -> Duration {
    if metered || quiet >= Duration::from_secs(600) {
        Duration::from_secs(10)
    } else if quiet >= Duration::from_secs(120) {
        Duration::from_secs(5)
    } else {
        PROBE
    }
}

/// What the folder loop does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Write the own changes, read the others'.
    Round,
    /// List the journals; a round only if one changed.
    Probe,
    /// Nothing until the next wake-up.
    Wait,
}

/// Signals of the app for the folder loop.
#[derive(Debug)]
pub struct FolderPace {
    playing_until_ms: AtomicU64,
    foreground: AtomicBool,
    metered: AtomicBool,
    woken: Mutex<bool>,
    wake: Condvar,
}

impl Default for FolderPace {
    fn default() -> Self {
        Self {
            playing_until_ms: AtomicU64::new(0),
            foreground: AtomicBool::new(true),
            metered: AtomicBool::new(false),
            woken: Mutex::new(false),
            wake: Condvar::new(),
        }
    }
}

impl FolderPace {
    /// A heartbeat of the player: `playing` while it plays; a pause or a
    /// stop asks for a round now (the last position goes out).
    pub fn heartbeat(&self, playing: bool, now_ms: u64) {
        let was = self.is_playing(now_ms);
        if playing {
            self.playing_until_ms
                .store(now_ms.saturating_add(PLAYING_FOR_MS), Ordering::Release);
        } else {
            self.playing_until_ms.store(0, Ordering::Release);
        }
        if !playing || !was {
            self.wake_up();
        }
    }

    /// The app came to the front or went away, the network changed.
    pub fn set_app_state(&self, foreground: bool, metered: bool) {
        let was = self.foreground.swap(foreground, Ordering::AcqRel);
        let back = foreground && !was;
        self.metered.store(metered, Ordering::Release);
        if back {
            self.wake_up();
        }
    }

    pub fn is_playing(&self, now_ms: u64) -> bool {
        now_ms < self.playing_until_ms.load(Ordering::Acquire)
    }

    pub fn foreground(&self) -> bool {
        self.foreground.load(Ordering::Acquire)
    }

    pub fn metered(&self) -> bool {
        self.metered.load(Ordering::Acquire)
    }

    fn wake_up(&self) {
        *self.woken.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.wake.notify_all();
    }

    /// Sleeps `d` or until woken or `stop()`; `true` when woken.
    pub fn sleep(&self, d: Duration, stop: &dyn Fn() -> bool) -> bool {
        let deadline = std::time::Instant::now() + d;
        let mut woken = self.woken.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if *woken {
                *woken = false;
                return true;
            }
            if stop() {
                return false;
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return false;
            }
            // Short slices: `stop` is an atomic flag nobody notifies.
            let slice = left.min(Duration::from_millis(250));
            woken = self
                .wake
                .wait_timeout(woken, slice)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// The next step and how long to sleep after it. `since_round`: time
    /// since the last full round (`None`: none yet); `quiet`: time since
    /// another device's journal last changed; `woken`: a heartbeat or the
    /// app asked.
    pub fn plan(
        &self,
        since_round: Option<Duration>,
        quiet: Duration,
        woken: bool,
    ) -> (Step, Duration) {
        let metered = self.metered();
        let round_due = since_round.is_none_or(|d| d >= ROUND);
        if self.is_playing(system_ms()) {
            let gap = if metered {
                PLAYING_GAP_METERED
            } else {
                PLAYING_GAP
            };
            return (Step::Round, gap);
        }
        if woken || round_due {
            let next = if self.foreground() {
                probe_gap(quiet, metered)
            } else {
                ROUND
            };
            return (Step::Round, next);
        }
        if self.foreground() {
            return (Step::Probe, probe_gap(quiet, metered));
        }
        let left = ROUND.saturating_sub(since_round.unwrap_or_default());
        (Step::Wait, left.max(Duration::from_millis(250)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playing_rounds_every_second() {
        let p = FolderPace::default();
        let now = system_ms();
        p.heartbeat(true, now);
        let (step, gap) = p.plan(Some(Duration::from_millis(10)), Duration::ZERO, false);
        assert_eq!((step, gap), (Step::Round, PLAYING_GAP));
        p.set_app_state(true, true);
        assert_eq!(p.plan(None, Duration::ZERO, false).1, PLAYING_GAP_METERED);
        // A pause stops the playing pace and wakes the loop.
        p.heartbeat(false, now);
        assert!(!p.is_playing(now));
        assert!(p.sleep(Duration::from_secs(5), &|| false));
    }

    #[test]
    fn in_front_probes_and_backs_off() {
        let p = FolderPace::default();
        let recent = Some(Duration::from_secs(1));
        assert_eq!(p.plan(recent, Duration::ZERO, false), (Step::Probe, PROBE));
        assert_eq!(
            p.plan(recent, Duration::from_secs(180), false).1,
            Duration::from_secs(5)
        );
        assert_eq!(
            p.plan(recent, Duration::from_secs(900), false).1,
            Duration::from_secs(10)
        );
        // A round at least every 30 s, and at the first step.
        assert_eq!(p.plan(Some(ROUND), Duration::ZERO, false).0, Step::Round);
        assert_eq!(p.plan(None, Duration::ZERO, false).0, Step::Round);
        p.set_app_state(true, true);
        assert_eq!(
            p.plan(recent, Duration::ZERO, false).1,
            Duration::from_secs(10)
        );
    }

    #[test]
    fn background_waits_for_the_round() {
        let p = FolderPace::default();
        p.set_app_state(false, false);
        let (step, gap) = p.plan(Some(Duration::from_secs(10)), Duration::ZERO, false);
        assert_eq!((step, gap), (Step::Wait, Duration::from_secs(20)));
        // Coming back to the front wakes the loop.
        p.set_app_state(true, false);
        assert!(p.sleep(Duration::from_secs(5), &|| false));
        assert!(!p.sleep(Duration::from_millis(10), &|| false));
    }
}
