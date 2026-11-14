// SPDX-License-Identifier: MPL-2.0
//! Live latency and catch-up policy (TWI-006, 06 §6.4), full builds only.
//!
//! Latency is the distance between Twitch's program date-time of the frame
//! on screen and the wall clock: `now − PDT(playhead)`. The player knows how
//! many seconds it holds ahead of the playhead (`demuxer-cache-duration` in
//! mpv); the loopback proxy knows the program date-time of the last byte it
//! handed to the player. Together: `PDT(playhead) = PDT(end of delivered
//! media) − cache ahead`.
//!
//! When the player falls behind the target, it plays slightly faster
//! ([`CatchUpPolicy::rate`], at most 1.1×, mpv keeps the pitch) until it is
//! back. During an ad break the rate stays 1.0×: ads are played as is.

use super::playlist::LiveMode;

/// Catch-up settings for a [`LiveMode`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CatchUpPolicy {
    /// Latency the player aims for, seconds.
    pub target: f64,
    /// Catch-up starts when the latency exceeds `target + start_margin`.
    pub start_margin: f64,
    /// Above `target + fast_margin` the faster rate is used.
    pub fast_margin: f64,
    /// Rate when slightly behind.
    pub slow_rate: f64,
    /// Rate when far behind.
    pub fast_rate: f64,
    /// Beyond `target + jump_margin` speeding up takes too long: the UI
    /// offers (or does) "go to live" instead.
    pub jump_margin: f64,
}

impl CatchUpPolicy {
    /// The policy of `mode`.
    pub const fn for_mode(mode: LiveMode) -> Self {
        match mode {
            LiveMode::LowLatency => Self {
                target: 3.0,
                start_margin: 1.0,
                fast_margin: 3.0,
                slow_rate: 1.05,
                fast_rate: 1.1,
                jump_margin: 20.0,
            },
            LiveMode::Stable => Self {
                target: 8.0,
                start_margin: 3.0,
                fast_margin: 6.0,
                slow_rate: 1.05,
                fast_rate: 1.1,
                jump_margin: 30.0,
            },
        }
    }

    /// Playback rate for a `latency` (seconds) given the `current` rate.
    ///
    /// Hysteresis: catch-up starts above `target + start_margin` and, once
    /// running (`current > 1`), continues until the latency is back at the
    /// target. Always 1.0 during an ad, when the latency is unknown or when
    /// the user chose another rate (`current` not one of ours).
    pub fn rate(&self, latency: Option<f64>, in_ad: bool, current: f64) -> f64 {
        let ours = [1.0, self.slow_rate, self.fast_rate]
            .iter()
            .any(|r| (r - current).abs() < 1e-3);
        if !ours {
            return current;
        }
        let Some(latency) = latency else { return 1.0 };
        if in_ad {
            return 1.0;
        }
        let behind = latency - self.target;
        let catching_up = current > 1.0 + 1e-3;
        if behind > self.fast_margin {
            self.fast_rate
        } else if behind > self.start_margin || (catching_up && behind > 0.0) {
            self.slow_rate
        } else {
            1.0
        }
    }

    /// Whether the player is so far behind that it should jump to live.
    pub fn should_jump(&self, latency: Option<f64>) -> bool {
        latency.is_some_and(|l| l > self.target + self.jump_margin)
    }
}

/// `now − PDT(playhead)` in seconds, from the program date-time of the end
/// of the media delivered to the player and the seconds the player holds
/// ahead of its playhead. `None` without a date-time.
pub fn latency_seconds(
    now_ms: i64,
    delivered_end_ms: Option<i64>,
    cache_ahead: f64,
) -> Option<f64> {
    let end = delivered_end_ms?;
    let playhead = end as f64 - cache_ahead.max(0.0) * 1000.0;
    Some(((now_ms as f64 - playhead) / 1000.0).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_from_delivered_media_and_cache() {
        assert_eq!(latency_seconds(10_000, Some(9_000), 1.5), Some(2.5));
        assert_eq!(latency_seconds(10_000, None, 1.5), None);
        // Clock skew never yields a negative latency.
        assert_eq!(latency_seconds(10_000, Some(12_000), 0.0), Some(0.0));
    }

    #[test]
    fn catch_up_with_hysteresis() {
        let p = CatchUpPolicy::for_mode(LiveMode::LowLatency);
        assert_eq!(p.rate(Some(3.5), false, 1.0), 1.0);
        assert_eq!(p.rate(Some(4.5), false, 1.0), 1.05);
        assert_eq!(p.rate(Some(7.0), false, 1.0), 1.1);
        // Keeps catching up until back at the target.
        assert_eq!(p.rate(Some(3.5), false, 1.05), 1.05);
        assert_eq!(p.rate(Some(2.9), false, 1.05), 1.0);
        // Unknown latency.
        assert_eq!(p.rate(None, false, 1.05), 1.0);
    }

    #[test]
    fn ads_play_at_normal_speed() {
        let p = CatchUpPolicy::for_mode(LiveMode::LowLatency);
        assert_eq!(p.rate(Some(10.0), true, 1.0), 1.0);
        assert_eq!(p.rate(Some(10.0), true, 1.1), 1.0);
    }

    #[test]
    fn user_rate_is_left_alone() {
        let p = CatchUpPolicy::for_mode(LiveMode::Stable);
        assert_eq!(p.rate(Some(30.0), false, 1.5), 1.5);
        assert_eq!(p.rate(Some(30.0), false, 0.5), 0.5);
    }

    #[test]
    fn far_behind_means_jump() {
        let p = CatchUpPolicy::for_mode(LiveMode::LowLatency);
        assert!(!p.should_jump(Some(10.0)));
        assert!(p.should_jump(Some(30.0)));
        assert!(!p.should_jump(None));
    }
}
