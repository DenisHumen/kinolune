// SPDX-License-Identifier: MPL-2.0
//! Client-side rate limiting and retry backoff.
//!
//! [`RateLimiter`] is a token bucket shared by all threads of one client:
//! it keeps us under the provider's published limit before the provider has
//! to say no. When it says no anyway (HTTP 429), [`RetryPolicy`] decides how
//! long to wait, and the limiter is paused for everyone until then.

use std::sync::Mutex;
use std::time::Duration;

use crate::clock::Clock;

/// Token bucket parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimit {
    /// Requests that may go out back to back.
    pub burst: u32,
    /// Sustained requests per second.
    pub per_second: f64,
}

impl RateLimit {
    /// A bucket that never lets more than `per_minute` requests through in
    /// any 60-second window, `burst` of them back to back.
    ///
    /// A token bucket admits at most `burst + per_second × window` requests
    /// in a window, so the refill rate is `(per_minute − burst) / 60`. A
    /// small burst also keeps short windows low: with `burst = 3` and 90/min
    /// at most 4 requests fall into any one second.
    pub fn per_minute(per_minute: u32, burst: u32) -> Self {
        let burst = burst.clamp(1, per_minute.max(2) - 1);
        Self {
            burst,
            per_second: f64::from(per_minute.saturating_sub(burst).max(1)) / 60.0,
        }
    }

    fn sanitized(self) -> Self {
        Self {
            burst: self.burst.max(1),
            per_second: if self.per_second > 0.0 {
                self.per_second
            } else {
                1.0
            },
        }
    }
}

/// Shared token bucket.
#[derive(Debug)]
pub struct RateLimiter {
    state: Mutex<BucketState>,
}

#[derive(Debug)]
struct BucketState {
    limit: RateLimit,
    tokens: f64,
    last: Option<Duration>,
    paused_until: Duration,
}

impl RateLimiter {
    /// A full bucket.
    pub fn new(limit: RateLimit) -> Self {
        let limit = limit.sanitized();
        Self {
            state: Mutex::new(BucketState {
                limit,
                tokens: f64::from(limit.burst),
                last: None,
                paused_until: Duration::ZERO,
            }),
        }
    }

    /// Takes one token, sleeping on `clock` until one is available and any
    /// pause has passed.
    ///
    /// The token is reserved under the lock and the wait happens outside
    /// it: callers queue in order of arrival (the bucket goes negative by
    /// one token per waiting caller), and [`pause_for`](Self::pause_for)
    /// and [`waiting`](Self::waiting) never block behind a sleeping caller
    /// (LIB-023). A pause that starts while a caller waits holds it too.
    pub fn acquire(&self, clock: &dyn Clock) {
        let (pause, queued) = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let limit = s.limit;
            let now = clock.monotonic();
            // Tokens refill from the later of the last take and the pause.
            let start = now.max(s.paused_until);
            if let Some(last) = s.last {
                let refill = start.saturating_sub(last).as_secs_f64() * limit.per_second;
                s.tokens = (s.tokens + refill).min(f64::from(limit.burst));
            }
            s.last = Some(start);
            s.tokens -= 1.0;
            let queued = if s.tokens < 0.0 {
                Duration::from_secs_f64(-s.tokens / limit.per_second)
            } else {
                Duration::ZERO
            };
            (start - now, queued)
        };
        // The provider's pause first, then our place in the queue.
        for wait in [pause, queued] {
            if !wait.is_zero() {
                clock.sleep(wait);
            }
        }
        // A provider asked everyone to wait while this caller queued.
        loop {
            let pause = {
                let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                s.paused_until.saturating_sub(clock.monotonic())
            };
            if pause.is_zero() {
                return;
            }
            clock.sleep(pause);
        }
    }

    /// How long a request taken now would wait, and whether that is a pause
    /// the provider asked for (HTTP 429, quota used up) rather than our own
    /// pacing. `None` when it would go out at once.
    pub fn waiting(&self, clock: &dyn Clock) -> Option<Wait> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = clock.monotonic();
        let pause = s.paused_until.saturating_sub(now);
        let start = now.max(s.paused_until);
        let mut tokens = s.tokens;
        if let Some(last) = s.last {
            tokens = (tokens + start.saturating_sub(last).as_secs_f64() * s.limit.per_second)
                .min(f64::from(s.limit.burst));
        }
        let queued = if tokens < 1.0 {
            Duration::from_secs_f64((1.0 - tokens) / s.limit.per_second)
        } else {
            Duration::ZERO
        };
        let total = pause + queued;
        (!total.is_zero()).then_some(Wait {
            duration: total,
            provider_asked: !pause.is_zero(),
        })
    }

    /// The limit in force.
    pub fn limit(&self) -> RateLimit {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).limit
    }

    /// Changes the limit for the following requests (a provider announced a
    /// different quota). Tokens already in the bucket are kept, capped at the
    /// new burst, so a lower limit takes effect at once.
    pub fn set_limit(&self, limit: RateLimit) {
        let limit = limit.sanitized();
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.limit = limit;
        s.tokens = s.tokens.min(f64::from(limit.burst));
    }

    /// Holds every caller until `delay` from now (after a 429).
    pub fn pause_for(&self, clock: &dyn Clock, delay: Duration) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let until = clock.monotonic() + delay;
        if until > s.paused_until {
            s.paused_until = until;
        }
        // Restart slowly after the pause instead of with a full burst;
        // callers already queued keep their places after it.
        s.tokens = s.tokens.min(0.0);
        s.last = Some(s.paused_until);
    }
}

/// A wait before the next request ([`RateLimiter::waiting`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wait {
    /// Time until a request taken now goes out.
    pub duration: Duration,
    /// The provider asked to wait (429 with `Retry-After`, quota used up);
    /// otherwise it is the client's own pacing under the published limit.
    pub provider_asked: bool,
}

/// Retries for HTTP 429 and transient 5xx responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Retries after the first attempt.
    pub max_retries: u32,
    /// Delay before the first retry; doubles each time.
    pub base_delay: Duration,
    /// Upper bound for one delay, also for a provider's `Retry-After`.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 4,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// Delay before retry number `retry` (0-based): the provider's
    /// `Retry-After` if given, else exponential; both capped by `max_delay`.
    pub fn delay(&self, retry: u32, retry_after: Option<Duration>) -> Duration {
        let exponential = self
            .base_delay
            .saturating_mul(2u32.saturating_pow(retry.min(16)));
        retry_after.unwrap_or(exponential).min(self.max_delay)
    }
}

/// Parses `Retry-After` in its delta-seconds form. The HTTP-date form is
/// ignored (exponential backoff is used instead).
pub fn parse_retry_after(value: Option<&str>) -> Option<Duration> {
    value?.trim().parse::<u64>().ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;

    #[test]
    fn burst_then_steady_rate() {
        let clock = ManualClock::default();
        let limiter = RateLimiter::new(RateLimit {
            burst: 3,
            per_second: 10.0,
        });
        for _ in 0..3 {
            limiter.acquire(&clock);
        }
        assert!(clock.sleeps().is_empty(), "burst goes out without waiting");
        limiter.acquire(&clock);
        limiter.acquire(&clock);
        let sleeps = clock.sleeps();
        assert_eq!(sleeps.len(), 2);
        for s in sleeps {
            assert!((s.as_secs_f64() - 0.1).abs() < 1e-6, "{s:?}");
        }
    }

    #[test]
    fn refills_over_time() {
        let clock = ManualClock::default();
        let limiter = RateLimiter::new(RateLimit {
            burst: 1,
            per_second: 2.0,
        });
        limiter.acquire(&clock);
        clock.advance(Duration::from_secs(1));
        limiter.acquire(&clock);
        assert!(clock.sleeps().is_empty());
    }

    #[test]
    fn pause_holds_next_caller() {
        let clock = ManualClock::default();
        let limiter = RateLimiter::new(RateLimit {
            burst: 10,
            per_second: 10.0,
        });
        limiter.pause_for(&clock, Duration::from_secs(5));
        let wait = limiter.waiting(&clock).unwrap();
        assert!(wait.provider_asked);
        limiter.acquire(&clock);
        assert_eq!(clock.sleeps()[0], Duration::from_secs(5));
        // Then a slow restart: one token at the steady rate.
        let slept: Duration = clock.sleeps().iter().sum();
        assert_eq!(wait.duration, slept);
    }

    #[test]
    fn queued_callers_wait_in_turn() {
        let clock = ManualClock::default();
        let limiter = RateLimiter::new(RateLimit {
            burst: 1,
            per_second: 2.0,
        });
        assert_eq!(limiter.waiting(&clock), None);
        limiter.acquire(&clock);
        let wait = limiter.waiting(&clock).unwrap();
        assert!(!wait.provider_asked, "own pacing, not the provider's");
        assert_eq!(wait.duration, Duration::from_millis(500));
    }

    /// LIB-023: a caller waiting for its token does not hold the lock, so
    /// a 429 seen by another thread pauses at once and the wait is visible.
    #[test]
    fn waiting_caller_does_not_block_pause_or_status() {
        use crate::clock::SystemClock;
        use std::sync::Arc;
        use std::time::Instant;
        let clock = Arc::new(SystemClock::default());
        let limiter = Arc::new(RateLimiter::new(RateLimit {
            burst: 1,
            per_second: 0.5,
        }));
        limiter.acquire(clock.as_ref());
        let waiter = {
            let (limiter, clock) = (limiter.clone(), clock.clone());
            std::thread::spawn(move || limiter.acquire(clock.as_ref()))
        };
        std::thread::sleep(Duration::from_millis(100));
        let t = Instant::now();
        limiter.pause_for(clock.as_ref(), Duration::from_millis(300));
        let wait = limiter.waiting(clock.as_ref()).unwrap();
        assert!(
            t.elapsed() < Duration::from_millis(100),
            "blocked behind the sleeper"
        );
        assert!(wait.provider_asked);
        waiter.join().unwrap();
    }

    #[test]
    fn per_minute_never_exceeds_the_window() {
        let l = RateLimit::per_minute(30, 2);
        assert_eq!(l.burst, 2);
        assert!((l.per_second - 28.0 / 60.0).abs() < 1e-9);
        let l = RateLimit::per_minute(1, 5);
        assert_eq!(l.burst, 1);
        assert!(l.per_second > 0.0);
    }

    #[test]
    fn set_limit_caps_tokens() {
        let clock = ManualClock::default();
        let limiter = RateLimiter::new(RateLimit {
            burst: 10,
            per_second: 10.0,
        });
        limiter.set_limit(RateLimit {
            burst: 1,
            per_second: 1.0,
        });
        limiter.acquire(&clock);
        limiter.acquire(&clock);
        assert_eq!(clock.sleeps(), vec![Duration::from_secs(1)]);
        assert_eq!(limiter.limit().burst, 1);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let p = RetryPolicy::default();
        assert_eq!(p.delay(0, None), Duration::from_secs(1));
        assert_eq!(p.delay(3, None), Duration::from_secs(8));
        assert_eq!(p.delay(10, None), Duration::from_secs(30));
        assert_eq!(
            p.delay(0, Some(Duration::from_secs(7))),
            Duration::from_secs(7)
        );
        assert_eq!(
            p.delay(0, Some(Duration::from_secs(600))),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn retry_after_seconds_only() {
        assert_eq!(parse_retry_after(Some(" 3 ")), Some(Duration::from_secs(3)));
        assert_eq!(
            parse_retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT")),
            None
        );
        assert_eq!(parse_retry_after(None), None);
    }
}
