// SPDX-License-Identifier: MPL-2.0
//! Polling schedule of `GET /streams`: battery-aware intervals, exponential
//! backoff on errors.
//!
//! | Power | Interval | Worst-case delay of a notification |
//! |---|---|---|
//! | mains (desktop, server) | 60 s | ≤ 2 min |
//! | battery | 90 s | ≤ 2 min |
//! | battery saver / low battery | 300 s | ≤ 5 min (saver wins over the promise) |
//!
//! "Worst case" adds Twitch's own delay before a new stream appears in
//! `GET /streams` (usually well under 30 s). A channel covered by EventSub
//! `stream.online` is notified within seconds whatever the interval; when
//! every channel with notifications is covered, polling only refreshes
//! categories and ends of broadcasts and slows to 120 s (or the saver
//! interval). Errors double the interval up to 15 minutes; a rate limit
//! waits at least until the bucket refills.

use std::time::Duration;

/// Power state of the device, reported by the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PowerState {
    /// Mains power (desktop, server, charging phone).
    #[default]
    Plugged,
    /// On battery.
    Battery,
    /// Battery saver on or battery low.
    Saver,
}

/// Polling intervals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollPolicy {
    /// On mains.
    pub plugged: Duration,
    /// On battery.
    pub battery: Duration,
    /// Battery saver.
    pub saver: Duration,
    /// When EventSub covers every channel with notifications.
    pub covered: Duration,
    /// Longest delay after errors.
    pub max_backoff: Duration,
    /// Spread polls by up to −10 % so many devices do not poll in step.
    pub jitter: bool,
}

impl Default for PollPolicy {
    fn default() -> Self {
        Self {
            plugged: Duration::from_secs(60),
            battery: Duration::from_secs(90),
            saver: Duration::from_secs(300),
            covered: Duration::from_secs(120),
            max_backoff: Duration::from_secs(15 * 60),
            jitter: true,
        }
    }
}

impl PollPolicy {
    /// Interval without errors.
    pub fn interval(&self, power: PowerState, covered_by_eventsub: bool) -> Duration {
        let base = match power {
            PowerState::Plugged => self.plugged,
            PowerState::Battery => self.battery,
            PowerState::Saver => self.saver,
        };
        if covered_by_eventsub {
            base.max(self.covered)
        } else {
            base
        }
    }

    /// Delay before the next poll after `errors` failed polls in a row;
    /// `retry_after` is the rate-limit reset of the last failure.
    pub fn next_delay(
        &self,
        power: PowerState,
        covered_by_eventsub: bool,
        errors: u32,
        retry_after: Option<Duration>,
    ) -> Duration {
        let base = self.interval(power, covered_by_eventsub);
        let delay = if errors == 0 {
            base
        } else {
            base.saturating_mul(1u32 << errors.min(16))
                .min(self.max_backoff.max(base))
        };
        let delay = retry_after.map_or(delay, |r| delay.max(r));
        if self.jitter && errors == 0 {
            jitter(delay)
        } else {
            delay
        }
    }
}

/// 90 % … 100 % of `d`.
fn jitter(d: Duration) -> Duration {
    let mut b = [0u8; 2];
    if getrandom::fill(&mut b).is_err() {
        return d;
    }
    let permille = 900 + u64::from(u16::from_le_bytes(b)) * 100 / u64::from(u16::MAX);
    Duration::from_millis(
        u64::try_from(d.as_millis())
            .unwrap_or(u64::MAX)
            .saturating_mul(permille)
            / 1000,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> PollPolicy {
        PollPolicy {
            jitter: false,
            ..PollPolicy::default()
        }
    }

    #[test]
    fn intervals_follow_power() {
        let p = policy();
        assert_eq!(p.interval(PowerState::Plugged, false).as_secs(), 60);
        assert_eq!(p.interval(PowerState::Battery, false).as_secs(), 90);
        assert_eq!(p.interval(PowerState::Saver, false).as_secs(), 300);
        assert_eq!(p.interval(PowerState::Plugged, true).as_secs(), 120);
        assert_eq!(p.interval(PowerState::Saver, true).as_secs(), 300);
    }

    #[test]
    fn promise_holds_without_saver() {
        let p = PollPolicy::default();
        for power in [PowerState::Plugged, PowerState::Battery] {
            assert!(p.next_delay(power, false, 0, None) <= Duration::from_secs(90));
        }
    }

    #[test]
    fn errors_back_off_exponentially_and_cap() {
        let p = policy();
        let d = |e| p.next_delay(PowerState::Plugged, false, e, None).as_secs();
        assert_eq!(d(0), 60);
        assert_eq!(d(1), 120);
        assert_eq!(d(2), 240);
        assert_eq!(d(3), 480);
        assert_eq!(d(4), 900);
        assert_eq!(d(30), 900);
    }

    #[test]
    fn rate_limit_waits_for_reset() {
        let p = policy();
        let d = p.next_delay(
            PowerState::Plugged,
            false,
            1,
            Some(Duration::from_secs(1000)),
        );
        assert_eq!(d.as_secs(), 1000);
    }

    #[test]
    fn jitter_stays_within_ten_percent() {
        let p = PollPolicy::default();
        for _ in 0..200 {
            let d = p
                .next_delay(PowerState::Plugged, false, 0, None)
                .as_millis();
            assert!((54_000..=60_000).contains(&d), "{d}");
        }
    }
}
