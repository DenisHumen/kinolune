// SPDX-License-Identifier: MPL-2.0
//! Automatic tuning of parallel prefetch (NET-021, ADR-0033).
//!
//! One TCP connection on a lossy link is slow: with 1 % random loss and
//! 30 ms RTT a Linux server's CUBIC gives one flow a few Mbit/s, and only
//! the server's congestion control could change that. The client can open
//! more connections and keep more blocks in flight, which is what the
//! [`Tuner`] does, with no setting for the user:
//!
//! - It measures how long readers wait for blocks, the time of every block
//!   (request to last byte) and the round trip of a tiny request (`stat`
//!   when a file is opened, then an `fstat` on a just-freed connection
//!   about every [`RTT_EVERY`] while streaming).
//! - Every epoch (at least [`EPOCH`] and one block per parallel slot) it
//!   decides. When readers were starved — waited for the network for at
//!   least [`STARVED`] of the epoch — and the path is not queueing, it
//!   doubles the number of parallel blocks (4, 8, 16, 32: the limit within
//!   seconds).
//! - The path is queueing when the smallest round trip of the epoch is
//!   above [`QUEUE_FACTOR`] × the smallest ever seen plus [`QUEUE_SLACK`]:
//!   the link is full and more connections only fill the router's buffer
//!   (a fast LAN, Wi-Fi at its rate). On a lossy link that is not full the
//!   round trip stays at its base, so the count keeps growing. A step that
//!   makes the path queue is undone (after one warm-up epoch, while new
//!   connections log on and leave slow start), with no new try for
//!   [`HOLD`].
//! - Throughput itself is not compared between levels: on a lossy link
//!   per-connection speed drifts a lot (slow start, TCP retransmission
//!   timeouts), and comparisons across epochs undid good steps.
//! - The read-ahead window follows the measured speed and block time: it
//!   holds the blocks in flight twice over and the data the reader needs
//!   while the slowest block of the epoch arrives, twice over; never
//!   below the configured read-ahead, never above [`MAX_AUTO_WINDOW`]
//!   (unless the configured read-ahead is larger). It only grows: a
//!   smaller window would throw away fetched blocks ahead of the reader
//!   and fetch them again (on the lossy profile that halved the speed).
//!
//! A player reading slower than the network is never starved, so on a
//! good link nothing changes: the defaults of NET-003 stay.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Shortest measuring epoch.
pub(crate) const EPOCH: Duration = Duration::from_secs(1);
/// Readers waiting at least this share of the epoch are network-bound.
pub(crate) const STARVED: f64 = 0.2;
/// Round trip above this multiple of the base (plus [`QUEUE_SLACK`])
/// means the path is queueing.
pub(crate) const QUEUE_FACTOR: u32 = 2;
/// Allowance for server and scheduling jitter in the queueing test.
pub(crate) const QUEUE_SLACK: Duration = Duration::from_millis(1);
/// How often streaming measures the round trip.
pub(crate) const RTT_EVERY: Duration = Duration::from_millis(200);
/// After a step that was undone, no new attempt for this long.
pub(crate) const HOLD: Duration = Duration::from_secs(30);
/// A pause without fetched blocks longer than this starts a new epoch
/// instead of counting the idle time.
const IDLE: Duration = Duration::from_secs(2);
/// Upper bound of the automatic read-ahead window.
pub(crate) const MAX_AUTO_WINDOW: usize = 128 * 1024 * 1024;

/// Measurements of the current epoch and the state of the search.
struct Ctl {
    epoch_start: Option<Instant>,
    last_block: Option<Instant>,
    bytes: u64,
    blocks: usize,
    stalled: Duration,
    slowest: Duration,
    /// Smallest round trip of the epoch.
    epoch_rtt: Option<Duration>,
    /// Smallest round trip ever seen: the base of the path.
    base_rtt: Option<Duration>,
    last_rtt_probe: Option<Instant>,
    /// The last step up is waiting for its verdict: the level before it.
    probe: Option<usize>,
    /// The level changed: the next epoch is a warm-up and is not judged.
    settling: bool,
    hold_until: Option<Instant>,
}

impl Ctl {
    fn reset_epoch(&mut self, now: Instant) {
        self.epoch_start = Some(now);
        self.bytes = 0;
        self.blocks = 0;
        self.stalled = Duration::ZERO;
        self.slowest = Duration::ZERO;
        self.epoch_rtt = None;
    }
}

/// What one epoch measured.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Epoch {
    /// Bytes per second fetched in the epoch.
    pub(crate) rate: f64,
    /// Readers waited at least [`STARVED`] of the epoch.
    pub(crate) starved: bool,
    /// The path queued (see [`QUEUE_FACTOR`]).
    pub(crate) queued: bool,
    /// Slowest block of the epoch (request to last byte).
    pub(crate) slowest: Duration,
}

/// Per-[`SmbFs`](crate::SmbFs) controller of parallel prefetch and the
/// read-ahead window, shared by all its open files.
pub(crate) struct Tuner {
    max: usize,
    chunk: usize,
    base_window: usize,
    max_window: usize,
    level: AtomicUsize,
    window: AtomicUsize,
    ctl: Mutex<Ctl>,
}

impl Tuner {
    /// `min..=max` parallel blocks (starting at `min`; `min` 0 means
    /// prefetch is off and stays off), blocks of `chunk` bytes, configured
    /// read-ahead `window` bytes.
    pub(crate) fn new(min: usize, max: usize, chunk: usize, window: usize) -> Self {
        let chunk = chunk.max(1);
        Self {
            max: if min == 0 { 0 } else { max.max(min) },
            chunk,
            base_window: window,
            max_window: window.max(MAX_AUTO_WINDOW),
            level: AtomicUsize::new(min),
            window: AtomicUsize::new(window),
            ctl: Mutex::new(Ctl {
                epoch_start: None,
                last_block: None,
                bytes: 0,
                blocks: 0,
                stalled: Duration::ZERO,
                slowest: Duration::ZERO,
                epoch_rtt: None,
                base_rtt: None,
                last_rtt_probe: None,
                probe: None,
                settling: true,
                hold_until: None,
            }),
        }
    }

    fn ctl(&self) -> std::sync::MutexGuard<'_, Ctl> {
        self.ctl.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Current number of blocks fetched in parallel.
    pub(crate) fn level(&self) -> usize {
        self.level.load(Ordering::Relaxed)
    }

    /// Current read-ahead window in blocks.
    pub(crate) fn window_chunks(&self) -> usize {
        self.window.load(Ordering::Relaxed) / self.chunk
    }

    /// A reader waited `waited` for a block that was not in the cache.
    pub(crate) fn on_stall(&self, waited: Duration) {
        let mut c = self.ctl();
        if c.epoch_start.is_some() {
            c.stalled += waited;
        }
    }

    /// `true` (once per [`RTT_EVERY`]) when the caller should time a tiny
    /// request and report it with [`on_rtt`](Self::on_rtt).
    pub(crate) fn wants_rtt(&self, now: Instant) -> bool {
        if self.max == 0 {
            return false;
        }
        let mut c = self.ctl();
        if c.last_rtt_probe
            .is_some_and(|t| now.duration_since(t) < RTT_EVERY)
        {
            return false;
        }
        c.last_rtt_probe = Some(now);
        true
    }

    /// A tiny request took `rtt` (request to reply).
    pub(crate) fn on_rtt(&self, rtt: Duration) {
        let mut c = self.ctl();
        c.base_rtt = Some(c.base_rtt.map_or(rtt, |b| b.min(rtt)));
        c.epoch_rtt = Some(c.epoch_rtt.map_or(rtt, |e| e.min(rtt)));
    }

    /// A block of `bytes` arrived `took` after it was requested. Returns
    /// the new level when it changed.
    pub(crate) fn on_block(&self, bytes: usize, took: Duration, now: Instant) -> Option<usize> {
        let mut c = self.ctl();
        let idle = c.last_block.is_none_or(|t| now.duration_since(t) > IDLE);
        c.last_block = Some(now);
        if idle || c.epoch_start.is_none() {
            // Start measuring from this block on.
            c.reset_epoch(now);
            return None;
        }
        c.bytes += bytes as u64;
        c.blocks += 1;
        c.slowest = c.slowest.max(took);
        let start = c.epoch_start.unwrap_or(now);
        let elapsed = now.duration_since(start);
        let level = self.level();
        if elapsed < EPOCH || c.blocks < level {
            return None;
        }
        let secs = elapsed.as_secs_f64();
        #[allow(clippy::cast_precision_loss)]
        let epoch = Epoch {
            rate: c.bytes as f64 / secs,
            starved: c.stalled.as_secs_f64() >= STARVED * secs,
            queued: match (c.epoch_rtt, c.base_rtt) {
                (Some(e), Some(b)) => e > b * QUEUE_FACTOR + QUEUE_SLACK,
                _ => false,
            },
            slowest: c.slowest,
        };
        c.reset_epoch(now);
        let next = self.decide(&mut c, level, epoch, now);
        drop(c);
        self.window
            .fetch_max(self.window_for(next, epoch), Ordering::Relaxed);
        if next == level {
            return None;
        }
        self.level.store(next, Ordering::Relaxed);
        Some(next)
    }

    /// The search step: double while starved readers see no queue, undo a
    /// step that made the path queue.
    fn decide(&self, c: &mut Ctl, level: usize, e: Epoch, now: Instant) -> usize {
        if std::mem::take(&mut c.settling) {
            return level;
        }
        if let Some(prev) = c.probe.take()
            && e.queued
        {
            c.hold_until = Some(now + HOLD);
            c.settling = true;
            return prev;
        }
        let held = c.hold_until.is_some_and(|t| now < t);
        if !e.starved || e.queued || held || level >= self.max {
            return level;
        }
        c.hold_until = None;
        c.probe = Some(level);
        c.settling = true;
        (2 * level).min(self.max)
    }

    /// Read-ahead bytes for `level` parallel blocks at the measured speed.
    fn window_for(&self, level: usize, e: Epoch) -> usize {
        let in_flight = (2 * level + 1).saturating_mul(self.chunk);
        // Truncation is fine: a window estimate in bytes.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let by_speed = (e.rate * 2.0 * e.slowest.as_secs_f64()) as usize;
        self.base_window
            .max(in_flight)
            .max(by_speed)
            .min(self.max_window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: usize = 1024 * 1024;
    const MBIT: f64 = 1e6 / 8.0;
    const BASE_RTT: Duration = Duration::from_millis(30);

    /// Drives the tuner with a model link: each parallel block gets
    /// `per_flow` bytes/s up to `cap` bytes/s in total; a full link
    /// triples the round trip (a router buffer); the reader is starved
    /// when the link is slower than `demand` bytes/s.
    fn run(t: &Tuner, per_flow: f64, cap: f64, demand: f64, secs: u64) -> Vec<usize> {
        let mut now = Instant::now();
        let mut levels = vec![t.level()];
        t.on_rtt(BASE_RTT);
        t.on_block(MIB, Duration::from_millis(100), now);
        // 10 ms slices; whole blocks of 1 MiB arrive at `rate`.
        let slice = Duration::from_millis(10);
        let mut pending = 0.0;
        #[allow(clippy::cast_precision_loss)]
        let block = MIB as f64;
        for _ in 0..secs * 100 {
            let level = t.level();
            #[allow(clippy::cast_precision_loss)]
            let wanted = per_flow * level as f64;
            let rate = wanted.min(cap);
            if rate < demand {
                t.on_stall(slice / 2);
            }
            if t.wants_rtt(now) {
                t.on_rtt(if wanted > cap { BASE_RTT * 3 } else { BASE_RTT });
            }
            pending += rate * slice.as_secs_f64();
            now += slice;
            while pending >= block {
                pending -= block;
                // Each of `level` flows gets an equal share.
                #[allow(clippy::cast_precision_loss)]
                let took = Duration::from_secs_f64(block * level as f64 / rate);
                if let Some(l) = t.on_block(MIB, took, now) {
                    levels.push(l);
                }
            }
        }
        levels
    }

    #[test]
    fn lossy_link_climbs_to_the_limit() {
        let t = Tuner::new(4, 48, MIB, 64 * MIB);
        let levels = run(&t, 5.0 * MBIT, 1000.0 * MBIT, f64::INFINITY, 60);
        assert_eq!(levels, vec![4, 8, 16, 32, 48], "{levels:?}");
        // 48 blocks in flight need a window of at least 97 blocks.
        assert!(t.window_chunks() >= 97, "{}", t.window_chunks());
        assert!(t.window_chunks() <= 128, "{}", t.window_chunks());
    }

    #[test]
    fn saturated_link_goes_back_and_holds() {
        // 4 flows already fill the link: one try, back to 4, and no new
        // try within HOLD.
        let t = Tuner::new(4, 48, MIB, 64 * MIB);
        let levels = run(&t, 100.0 * MBIT, 400.0 * MBIT, f64::INFINITY, 20);
        assert_eq!(levels, vec![4, 8, 4], "{levels:?}");
        // 8 blocks in flight fit the configured window.
        assert_eq!(t.window_chunks(), 64);
    }

    #[test]
    fn link_filling_midway_settles_below_the_limit() {
        // 12 flows fill 60 Mbit/s: 16 queue and are undone.
        let t = Tuner::new(4, 48, MIB, 64 * MIB);
        let levels = run(&t, 5.0 * MBIT, 60.0 * MBIT, f64::INFINITY, 30);
        assert_eq!(levels, vec![4, 8, 16, 8], "{levels:?}");
    }

    #[test]
    fn reader_slower_than_the_link_changes_nothing() {
        let t = Tuner::new(4, 48, MIB, 64 * MIB);
        let levels = run(&t, 5.0 * MBIT, 1000.0 * MBIT, 10.0 * MBIT, 30);
        assert_eq!(levels, vec![4]);
        assert_eq!(t.window_chunks(), 64);
    }

    #[test]
    fn fixed_range_never_moves() {
        let t = Tuner::new(8, 8, MIB, 64 * MIB);
        let levels = run(&t, 5.0 * MBIT, 1000.0 * MBIT, f64::INFINITY, 20);
        assert_eq!(levels, vec![8]);
    }

    #[test]
    fn prefetch_off_stays_off() {
        let t = Tuner::new(0, 32, MIB, 64 * MIB);
        assert!(!t.wants_rtt(Instant::now()));
        let levels = run(&t, 5.0 * MBIT, 1000.0 * MBIT, f64::INFINITY, 5);
        assert_eq!(levels, vec![0]);
    }

    #[test]
    fn idle_gap_starts_a_new_epoch() {
        let t = Tuner::new(4, 48, MIB, 64 * MIB);
        let now = Instant::now();
        t.on_block(MIB, Duration::from_millis(10), now);
        t.on_stall(Duration::from_secs(5));
        // Ten seconds later: the gap is not measured, nothing changes.
        let later = now + Duration::from_secs(10);
        assert_eq!(t.on_block(MIB, Duration::from_millis(10), later), None);
        for i in 1..=8u32 {
            let at = later + Duration::from_millis(200) * i;
            assert_eq!(t.on_block(MIB, Duration::from_millis(10), at), None);
        }
        assert_eq!(t.level(), 4);
    }
}
