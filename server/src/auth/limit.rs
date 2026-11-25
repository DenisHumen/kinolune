// SPDX-License-Identifier: MPL-2.0
//! Brute-force protection of the login endpoints: failures are counted per
//! account and per client address; past a free allowance the key is locked
//! with an exponential backoff.
//!
//! The state lives in memory only: a restart forgets it, which costs an
//! attacker a server restart he cannot trigger. A key without failures for
//! [`FORGET_AFTER`] is forgotten.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A key without new failures for this long starts from zero again.
pub const FORGET_AFTER: Duration = Duration::from_secs(60 * 60);

/// Bound of remembered keys (memory guard against address spraying).
const MAX_KEYS: usize = 50_000;

/// How many failures a key gets for free and how long it is locked after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Failures before the first lock.
    pub free: u32,
    /// First lock; each further failure doubles it…
    pub base: Duration,
    /// …up to this.
    pub max: Duration,
}

/// Per account (login name or user id): 5 tries, then 30 s, 1 min, 2 min…
/// up to 15 min. A 4-digit PIN would take decades to guess.
pub const ACCOUNT: Policy = Policy {
    free: 5,
    base: Duration::from_secs(30),
    max: Duration::from_secs(15 * 60),
};

/// Per client address: more room (a family behind one NAT, typos across
/// accounts), the same backoff.
pub const ADDRESS: Policy = Policy {
    free: 20,
    base: Duration::from_secs(30),
    max: Duration::from_secs(15 * 60),
};

impl Policy {
    /// Lock after `failures` failures; zero within the free allowance.
    pub fn lock_for(&self, failures: u32) -> Duration {
        if failures < self.free {
            return Duration::ZERO;
        }
        let doublings = (failures - self.free).min(20);
        self.base
            .checked_mul(1 << doublings)
            .unwrap_or(self.max)
            .min(self.max)
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    failures: u32,
    locked_until: Instant,
    last: Instant,
}

/// Failure counters by key.
#[derive(Debug, Default)]
pub struct Limiter {
    entries: Mutex<HashMap<String, Entry>>,
}

impl Limiter {
    /// An empty limiter.
    pub fn new() -> Self {
        Self::default()
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `Err(wait)` when any of `keys` is locked at `now`: the longest wait.
    pub fn check(&self, keys: &[&str], now: Instant) -> Result<(), Duration> {
        let entries = self.entries();
        let wait = keys
            .iter()
            .filter_map(|k| entries.get(*k))
            .map(|e| e.locked_until.saturating_duration_since(now))
            .max()
            .unwrap_or(Duration::ZERO);
        if wait.is_zero() { Ok(()) } else { Err(wait) }
    }

    /// Counts a failure of `key`; returns the lock it now has.
    pub fn fail(&self, key: &str, policy: &Policy, now: Instant) -> Duration {
        let mut entries = self.entries();
        if entries.len() >= MAX_KEYS {
            entries.retain(|_, e| {
                e.locked_until > now || now.saturating_duration_since(e.last) < FORGET_AFTER
            });
        }
        let entry = entries.entry(key.to_owned()).or_insert(Entry {
            failures: 0,
            locked_until: now,
            last: now,
        });
        if now.saturating_duration_since(entry.last) >= FORGET_AFTER && entry.locked_until <= now {
            entry.failures = 0;
        }
        entry.failures = entry.failures.saturating_add(1);
        entry.last = now;
        let lock = policy.lock_for(entry.failures);
        entry.locked_until = entry.locked_until.max(now + lock);
        lock
    }

    /// A success of `key` clears its failures.
    pub fn succeed(&self, key: &str) {
        self.entries().remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_the_cap() {
        let p = ACCOUNT;
        assert_eq!(p.lock_for(4), Duration::ZERO);
        assert_eq!(p.lock_for(5), Duration::from_secs(30));
        assert_eq!(p.lock_for(6), Duration::from_secs(60));
        assert_eq!(p.lock_for(7), Duration::from_secs(120));
        assert_eq!(p.lock_for(50), Duration::from_secs(15 * 60));
        assert_eq!(p.lock_for(u32::MAX), Duration::from_secs(15 * 60));
    }

    #[test]
    fn locks_after_the_allowance_and_unlocks_later() {
        let l = Limiter::new();
        let t0 = Instant::now();
        for _ in 0..4 {
            assert_eq!(l.fail("user:a", &ACCOUNT, t0), Duration::ZERO);
            assert!(l.check(&["user:a"], t0).is_ok());
        }
        assert_eq!(l.fail("user:a", &ACCOUNT, t0), Duration::from_secs(30));
        assert_eq!(
            l.check(&["user:a", "ip:x"], t0),
            Err(Duration::from_secs(30))
        );
        // Other keys are not affected.
        assert!(l.check(&["user:b", "ip:x"], t0).is_ok());
        let later = t0 + Duration::from_secs(31);
        assert!(l.check(&["user:a"], later).is_ok());
        // The next failure locks twice as long.
        assert_eq!(l.fail("user:a", &ACCOUNT, later), Duration::from_secs(60));
        // A success clears the key.
        l.succeed("user:a");
        assert!(l.check(&["user:a"], later).is_ok());
    }

    #[test]
    fn old_failures_are_forgotten() {
        let l = Limiter::new();
        let t0 = Instant::now();
        for _ in 0..4 {
            l.fail("k", &ACCOUNT, t0);
        }
        let much_later = t0 + FORGET_AFTER + Duration::from_secs(1);
        assert_eq!(l.fail("k", &ACCOUNT, much_later), Duration::ZERO);
    }
}
