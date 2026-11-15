// SPDX-License-Identifier: MPL-2.0
//! One request in flight per cache key (LIB-023).
//!
//! A metadata run asks several works at once; works of the same series
//! (seasons in separate folders, episodes recognised one by one) ask the
//! same search or the same details. The first caller sends the request; the
//! others wait for it and then read its answer from the response cache, so
//! a rate-limited provider (AniList: 30 a minute) sees each question once.
//! If the first request fails nothing is cached and the next caller asks
//! itself.

use std::collections::HashSet;
use std::sync::{Condvar, Mutex, PoisonError};

/// Keys being fetched right now.
#[derive(Debug, Default)]
pub(crate) struct InFlight {
    keys: Mutex<HashSet<String>>,
    done: Condvar,
}

/// Holds a key; dropping it lets the waiting callers go.
pub(crate) struct Ticket<'a> {
    flight: &'a InFlight,
    key: String,
}

impl InFlight {
    /// Waits until no other caller fetches `key`, then takes it. The
    /// caller should look into the cache again before asking the network.
    pub(crate) fn enter(&self, key: &str) -> Ticket<'_> {
        let mut keys = self.keys.lock().unwrap_or_else(PoisonError::into_inner);
        while keys.contains(key) {
            keys = self.done.wait(keys).unwrap_or_else(PoisonError::into_inner);
        }
        keys.insert(key.to_owned());
        Ticket {
            flight: self,
            key: key.to_owned(),
        }
    }
}

impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        self.flight
            .keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.key);
        self.flight.done.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn same_key_waits_other_keys_do_not() {
        let flight = Arc::new(InFlight::default());
        let inside = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..6)
            .map(|i| {
                let (flight, inside, max) = (flight.clone(), inside.clone(), max.clone());
                std::thread::spawn(move || {
                    let key = if i % 2 == 0 { "a" } else { "b" };
                    let _t = flight.enter(key);
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    max.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(30));
                    inside.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        // One holder per key at a time: at most two at once.
        assert_eq!(max.load(Ordering::SeqCst), 2);
    }
}
