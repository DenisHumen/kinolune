// SPDX-License-Identifier: MPL-2.0
//! Hub election, sans-IO: every device computes the same hub from what it
//! sees in the LAN, so no coordination round is needed.
//!
//! The hub is the best live candidate by `(class rank, device_id)`: server,
//! then desktop, TV, phone (ROADMAP SYNC-006), ties broken by the smaller
//! `device_id`. A peer whose connection failed is skipped for a cooldown,
//! so a powered-off hub is replaced as soon as its link times out, even if
//! its mDNS record is still cached.
//!
//! A dead peer that would beat the current hub is retried in the background
//! with capped exponential backoff ([`Election::retry_due`]); a handshake
//! with it, either way, revives it at once ([`Election::revive`]). So a hub
//! that restarts at the same address (its cached announcement does not
//! change) is back within one retry, not after the whole cooldown.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use pa_discovery::DeviceClass;

/// Rank of a device class in the election; lower wins.
pub const fn class_rank(class: DeviceClass) -> u8 {
    match class {
        DeviceClass::Server => 0,
        DeviceClass::Desktop => 1,
        DeviceClass::Tv => 2,
        DeviceClass::Mobile => 3,
        DeviceClass::Other => 4,
    }
}

/// Election key of a device: lower is a better hub.
pub fn hub_key(class: DeviceClass, device_id: &str) -> (u8, &str) {
    (class_rank(class), device_id)
}

/// A device seen in the LAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub class: DeviceClass,
    pub addr: SocketAddr,
    /// Skipped by the election until then (its connection failed).
    pub dead_until: Option<Instant>,
    /// Failed connections in a row at this address.
    pub failures: u32,
    /// When to retry it in the background while it is dead.
    pub retry_at: Option<Instant>,
    /// Since when it is known to be up again after a failure. A peer that
    /// stays up for the longest backoff delay starts over from the first
    /// one on its next failure; one that keeps failing right after a
    /// handshake keeps backing off.
    pub alive_since: Option<Instant>,
}

/// First and longest delay between background retries of a dead peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub first: Duration,
    pub max: Duration,
}

impl Backoff {
    /// Delay after `failures` (at least 1) failed connections in a row:
    /// `first`, doubled per failure, at most `max`.
    pub fn delay(self, failures: u32) -> Duration {
        let doublings = failures.saturating_sub(1).min(31);
        self.first
            .saturating_mul(1 << doublings)
            .min(self.max.max(self.first))
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            first: Duration::from_secs(1),
            max: Duration::from_secs(20),
        }
    }
}

/// Who should be the hub now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hub {
    /// This device.
    Me,
    /// Another device: connect to it.
    Peer { device_id: String, addr: SocketAddr },
}

/// The peers this device knows and the election over them.
#[derive(Debug, Clone)]
pub struct Election {
    me: String,
    class: DeviceClass,
    peers: BTreeMap<String, Peer>,
    backoff: Backoff,
}

impl Election {
    /// Election for this device (`me`, of `class`) with no peers yet.
    pub fn new(me: impl Into<String>, class: DeviceClass) -> Self {
        Self {
            me: me.into(),
            class,
            peers: BTreeMap::new(),
            backoff: Backoff::default(),
        }
    }

    /// The same, retrying dead peers with `backoff`.
    #[must_use]
    pub fn with_backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    /// A peer appeared or changed its address. A new address clears the
    /// dead mark: the device came back.
    pub fn upsert(&mut self, device_id: &str, class: DeviceClass, addr: SocketAddr) {
        if device_id == self.me {
            return;
        }
        let peer = self
            .peers
            .get(device_id)
            .filter(|p| p.addr == addr)
            .map_or(
                Peer {
                    class,
                    addr,
                    dead_until: None,
                    failures: 0,
                    retry_at: None,
                    alive_since: None,
                },
                |p| Peer { class, ..p.clone() },
            );
        self.peers.insert(device_id.to_owned(), peer);
    }

    /// A peer went away.
    pub fn remove(&mut self, device_id: &str) {
        self.peers.remove(device_id);
    }

    /// The link to `device_id` at `addr` failed at `now`: skip it until
    /// `until` and retry it in the background with backoff meanwhile. A
    /// failure at an address the peer has since left (it restarted or
    /// moved while the link was dying) says nothing about the new one and
    /// is ignored.
    pub fn mark_dead(&mut self, device_id: &str, addr: SocketAddr, now: Instant, until: Instant) {
        let backoff = self.backoff;
        if let Some(p) = self.peers.get_mut(device_id)
            && p.addr == addr
        {
            if p.alive_since.take().is_some_and(|t| now >= t + backoff.max) {
                p.failures = 0;
            }
            p.dead_until = Some(until);
            p.failures = p.failures.saturating_add(1);
            p.retry_at = Some(now + backoff.delay(p.failures));
        }
    }

    /// A background retry of `device_id` at `addr` failed at `now`: the
    /// next one waits twice as long (up to the backoff maximum). Ignored
    /// if the peer is no longer dead there.
    pub fn retry_failed(&mut self, device_id: &str, addr: SocketAddr, now: Instant) {
        let backoff = self.backoff;
        if let Some(p) = self.peers.get_mut(device_id)
            && p.addr == addr
            && p.retry_at.is_some()
        {
            p.failures = p.failures.saturating_add(1);
            p.retry_at = Some(now + backoff.delay(p.failures));
        }
    }

    /// `device_id` completed a handshake with this device at `now` (it
    /// dialed us, or we reached it): it is alive, whatever its dead mark
    /// said.
    pub fn revive(&mut self, device_id: &str, now: Instant) {
        if let Some(p) = self.peers.get_mut(device_id)
            && p.dead_until.is_some()
        {
            p.dead_until = None;
            p.retry_at = None;
            p.alive_since = Some(now);
        }
    }

    /// Whether `device_id` is skipped as dead at `now`.
    pub fn is_dead(&self, device_id: &str, now: Instant) -> bool {
        self.peers
            .get(device_id)
            .is_some_and(|p| p.dead_until.is_some_and(|t| t > now))
    }

    /// A dead peer to retry in the background now: the best one whose
    /// retry is due and that would beat the current hub if it were alive.
    /// Dead peers that would not be elected anyway are left alone.
    pub fn retry_due(&self, now: Instant) -> Option<(String, SocketAddr)> {
        let hub = self.hub(now);
        let bar = match &hub {
            Hub::Me => hub_key(self.class, &self.me),
            Hub::Peer { device_id, .. } => hub_key(self.peers[device_id].class, device_id),
        };
        self.peers
            .iter()
            .filter(|(_, p)| p.dead_until.is_some_and(|t| t > now))
            .filter(|(_, p)| p.retry_at.is_some_and(|t| t <= now))
            .filter(|(id, p)| hub_key(p.class, id) < bar)
            .min_by(|(a, pa), (b, pb)| hub_key(pa.class, a).cmp(&hub_key(pb.class, b)))
            .map(|(id, p)| (id.clone(), p.addr))
    }

    /// Known peers.
    pub fn peers(&self) -> &BTreeMap<String, Peer> {
        &self.peers
    }

    /// The hub at `now`.
    pub fn hub(&self, now: Instant) -> Hub {
        let best = self
            .peers
            .iter()
            .filter(|(_, p)| p.dead_until.is_none_or(|t| t <= now))
            .min_by(|(a, pa), (b, pb)| hub_key(pa.class, a).cmp(&hub_key(pb.class, b)));
        match best {
            Some((id, p)) if hub_key(p.class, id) < hub_key(self.class, &self.me) => Hub::Peer {
                device_id: id.clone(),
                addr: p.addr,
            },
            _ => Hub::Me,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    #[test]
    fn server_desktop_tv_phone_order() {
        let now = Instant::now();
        let mut e = Election::new("phone", DeviceClass::Mobile);
        assert_eq!(e.hub(now), Hub::Me);
        e.upsert("tv", DeviceClass::Tv, addr(1));
        e.upsert("pc", DeviceClass::Desktop, addr(2));
        e.upsert("weird", DeviceClass::Other, addr(3));
        assert_eq!(
            e.hub(now),
            Hub::Peer {
                device_id: "pc".into(),
                addr: addr(2)
            }
        );
        e.upsert("nas", DeviceClass::Server, addr(4));
        assert!(matches!(e.hub(now), Hub::Peer { device_id, .. } if device_id == "nas"));
        // The desktop itself sees the server too, and nobody picks the phone.
        let mut pc = Election::new("pc", DeviceClass::Desktop);
        pc.upsert("phone", DeviceClass::Mobile, addr(5));
        assert_eq!(pc.hub(now), Hub::Me);
    }

    #[test]
    fn ties_go_to_the_smaller_id_on_every_device() {
        let now = Instant::now();
        let mut a = Election::new("tv-a", DeviceClass::Tv);
        a.upsert("tv-b", DeviceClass::Tv, addr(1));
        let mut b = Election::new("tv-b", DeviceClass::Tv);
        b.upsert("tv-a", DeviceClass::Tv, addr(2));
        assert_eq!(a.hub(now), Hub::Me);
        assert!(matches!(b.hub(now), Hub::Peer { device_id, .. } if device_id == "tv-a"));
    }

    #[test]
    fn dead_hub_is_skipped_until_cooldown_or_new_address() {
        let now = Instant::now();
        let mut e = Election::new("phone", DeviceClass::Mobile);
        e.upsert("pc", DeviceClass::Desktop, addr(1));
        e.upsert("tv", DeviceClass::Tv, addr(2));
        e.mark_dead("pc", addr(1), now, now + Duration::from_secs(20));
        assert!(matches!(e.hub(now), Hub::Peer { device_id, .. } if device_id == "tv"));
        // The same announcement again does not revive it...
        e.upsert("pc", DeviceClass::Desktop, addr(1));
        assert!(matches!(e.hub(now), Hub::Peer { device_id, .. } if device_id == "tv"));
        // ...the cooldown does, and so does a new address (restart).
        assert!(
            matches!(e.hub(now + Duration::from_secs(21)), Hub::Peer { device_id, .. } if device_id == "pc")
        );
        e.upsert("pc", DeviceClass::Desktop, addr(9));
        assert!(matches!(e.hub(now), Hub::Peer { device_id, .. } if device_id == "pc"));
        // A failure noticed only after the restart is about the old
        // address: the new one stays electable.
        e.mark_dead("pc", addr(1), now, now + Duration::from_secs(20));
        assert!(matches!(e.hub(now), Hub::Peer { device_id, .. } if device_id == "pc"));
        e.remove("pc");
        e.remove("tv");
        assert_eq!(e.hub(now), Hub::Me);
        // Our own announcement is ignored.
        e.upsert("phone", DeviceClass::Server, addr(3));
        assert_eq!(e.hub(now), Hub::Me);
    }

    #[test]
    fn backoff_doubles_up_to_the_max() {
        let b = Backoff {
            first: Duration::from_secs(1),
            max: Duration::from_secs(20),
        };
        let delays: Vec<u64> = (1..=7).map(|n| b.delay(n).as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 20, 20]);
        assert_eq!(b.delay(u32::MAX), Duration::from_secs(20));
    }

    #[test]
    fn dead_hub_is_retried_with_backoff_and_revived_by_a_handshake() {
        let now = Instant::now();
        let s = Duration::from_secs;
        let mut e = Election::new("phone", DeviceClass::Mobile).with_backoff(Backoff {
            first: s(1),
            max: s(8),
        });
        e.upsert("pc", DeviceClass::Desktop, addr(1));
        e.upsert("tv", DeviceClass::Tv, addr(2));
        e.mark_dead("pc", addr(1), now, now + s(20));
        assert!(e.is_dead("pc", now));
        // Not before the first delay, then the dead hub is due: it beats tv.
        assert_eq!(e.retry_due(now), None);
        assert_eq!(e.retry_due(now + s(1)), Some(("pc".into(), addr(1))));
        // Each failed retry doubles the wait, up to the max.
        e.retry_failed("pc", addr(1), now + s(1));
        assert_eq!(e.retry_due(now + s(2)), None);
        assert_eq!(e.retry_due(now + s(3)), Some(("pc".into(), addr(1))));
        e.retry_failed("pc", addr(1), now + s(3));
        e.retry_failed("pc", addr(1), now + s(3));
        e.retry_failed("pc", addr(1), now + s(3));
        assert_eq!(e.retry_due(now + s(10)), None);
        assert_eq!(e.retry_due(now + s(11)), Some(("pc".into(), addr(1))));
        // A failure at an old address does not touch the schedule.
        e.retry_failed("pc", addr(7), now + s(11));
        assert_eq!(e.retry_due(now + s(11)), Some(("pc".into(), addr(1))));
        // A handshake with it (inbound or a retry) revives it at once,
        // although its announcement and the cooldown did not change.
        e.revive("pc", now + s(11));
        assert!(!e.is_dead("pc", now + s(11)));
        assert!(matches!(e.hub(now), Hub::Peer { device_id, .. } if device_id == "pc"));
        assert_eq!(e.retry_due(now + s(11)), None);
        // Failing again right after that keeps backing off (no flapping)...
        e.mark_dead("pc", addr(1), now + s(12), now + s(32));
        assert_eq!(e.retry_due(now + s(19)), None);
        assert_eq!(e.retry_due(now + s(20)), Some(("pc".into(), addr(1))));
        // ...while a failure after it stayed up for the longest delay
        // starts over from the first one.
        e.revive("pc", now + s(20));
        e.mark_dead("pc", addr(1), now + s(28), now + s(48));
        assert_eq!(e.retry_due(now + s(28)), None);
        assert_eq!(e.retry_due(now + s(29)), Some(("pc".into(), addr(1))));
        e.revive("pc", now + s(29));
        e.mark_dead("pc", addr(1), now, now + s(20));
        // A dead peer that would not beat the hub is not retried.
        e.mark_dead("tv", addr(2), now, now + s(20));
        e.upsert("nas", DeviceClass::Server, addr(3));
        assert_eq!(e.retry_due(now + s(5)), None);
        let mut nas = Election::new("nas", DeviceClass::Server);
        nas.upsert("pc", DeviceClass::Desktop, addr(1));
        nas.mark_dead("pc", addr(1), now, now + s(20));
        assert_eq!(nas.retry_due(now + s(5)), None);
    }
}
