// SPDX-License-Identifier: MPL-2.0
//! Where a link may point (ADR-0124).
//!
//! A playlist or a `.strm` file is data from somewhere else, and its links
//! are fetched by the app without the user looking at them. So a link may
//! only reach a network zone that is as public as, or more public than, the
//! place the list came from:
//!
//! | Origin | May reach |
//! |---|---|
//! | the user typed the address ([`LinkPolicy::typed`]) | everything below |
//! | [`Zone::Loopback`]: a server on this device | this device, the LAN, the Internet |
//! | [`Zone::Lan`]: a LAN server, a file on this device or on a share | the LAN, the Internet |
//! | [`Zone::Public`]: an Internet server | the Internet |
//!
//! Never: unspecified, multicast, broadcast, link-local (`169.254.0.0/16`
//! with the cloud metadata address, `fe80::/10`) and reserved addresses.
//!
//! The check runs on the addresses a connection really uses (a resolver
//! of the HTTP client, [`PolicyResolver`]), for the first request and for
//! every redirect, so a DNS name that resolves somewhere else later cannot
//! slip past it.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::NextTimeout;

/// How public a network address is. Ordered: `Loopback < Lan < Public`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Zone {
    /// This device (`127.0.0.0/8`, `::1`).
    Loopback,
    /// The local network: RFC 1918, carrier-grade NAT `100.64.0.0/10`,
    /// IPv6 unique local `fc00::/7` and site-local `fec0::/10`.
    Lan,
    /// Everything else that is routable.
    Public,
}

impl Zone {
    /// The zone of `ip`, or `None` for addresses a link may never reach
    /// (unspecified, multicast, broadcast, link-local, reserved).
    #[must_use]
    pub fn of(ip: IpAddr) -> Option<Self> {
        match ip {
            IpAddr::V4(v4) => zone_v4(v4),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => zone_v4(v4),
                None => zone_v6(v6),
            },
        }
    }

    /// The zone of a host as it resolves now: the most public zone among
    /// its addresses (a name that is both LAN and Internet is treated as
    /// Internet). `None` if it does not resolve or only to forbidden
    /// addresses.
    #[must_use]
    pub fn of_host(host: &str, port: u16) -> Option<Self> {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Self::of(ip);
        }
        (host, port)
            .to_socket_addrs()
            .ok()?
            .filter_map(|a| Self::of(a.ip()))
            .max()
    }
}

fn zone_v4(ip: Ipv4Addr) -> Option<Zone> {
    let [a, b, ..] = ip.octets();
    if ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_link_local()
        || a == 0
        || a >= 240
    {
        return None;
    }
    if ip.is_loopback() {
        return Some(Zone::Loopback);
    }
    if ip.is_private() || (a == 100 && (64..128).contains(&b)) {
        return Some(Zone::Lan);
    }
    Some(Zone::Public)
}

fn zone_v6(ip: Ipv6Addr) -> Option<Zone> {
    let first = ip.segments()[0];
    if ip.is_unspecified() || ip.is_multicast() || (first & 0xffc0) == 0xfe80 {
        return None;
    }
    if ip.is_loopback() {
        return Some(Zone::Loopback);
    }
    // Unique local fc00::/7, deprecated site-local fec0::/10.
    if (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfec0 {
        return Some(Zone::Lan);
    }
    Some(Zone::Public)
}

/// Which addresses links of one origin may reach (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LinkPolicy {
    lowest: Zone,
}

impl LinkPolicy {
    /// Links found in a list that came from `origin`: they may reach
    /// `origin` and more public zones.
    #[must_use]
    pub const fn from(origin: Zone) -> Self {
        Self { lowest: origin }
    }

    /// An address the user typed: every zone (still never link-local,
    /// multicast and the like).
    #[must_use]
    pub const fn typed() -> Self {
        Self::from(Zone::Loopback)
    }

    /// Links of a file on this device or on a network share (`.strm`, a
    /// playlist file): the LAN and the Internet, never this device.
    #[must_use]
    pub const fn local_file() -> Self {
        Self::from(Zone::Lan)
    }

    /// The least public zone these links may reach.
    #[must_use]
    pub const fn lowest(self) -> Zone {
        self.lowest
    }

    /// Whether a link may connect to `ip`.
    #[must_use]
    pub fn allows(self, ip: IpAddr) -> bool {
        Zone::of(ip).is_some_and(|z| z >= self.lowest)
    }
}

/// The marker error of a connection the policy refused; the client turns
/// it into [`ErrorKind::PermissionDenied`](pa_vfs::ErrorKind::PermissionDenied).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blocked;

impl fmt::Display for Blocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the link points where links from this list may not go")
    }
}

impl std::error::Error for Blocked {}

/// A `ureq` resolver that drops every address the [`LinkPolicy`] does not
/// allow and fails with [`Blocked`] when none is left.
#[derive(Debug)]
pub(crate) struct PolicyResolver {
    inner: DefaultResolver,
    policy: LinkPolicy,
}

impl PolicyResolver {
    pub(crate) fn new(policy: LinkPolicy) -> Self {
        Self {
            inner: DefaultResolver::default(),
            policy,
        }
    }
}

impl Resolver for PolicyResolver {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let all = self.inner.resolve(uri, config, timeout)?;
        let mut kept = self.empty();
        for addr in all.iter().copied().filter(|a: &SocketAddr| self.policy.allows(a.ip())) {
            kept.push(addr);
        }
        if kept.is_empty() {
            return Err(ureq::Error::Other(Box::new(Blocked)));
        }
        Ok(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(s: &str) -> Option<Zone> {
        Zone::of(s.parse().unwrap())
    }

    #[test]
    fn classifies_addresses() {
        assert_eq!(z("127.0.0.1"), Some(Zone::Loopback));
        assert_eq!(z("::1"), Some(Zone::Loopback));
        assert_eq!(z("::ffff:127.0.0.2"), Some(Zone::Loopback));
        for lan in ["10.1.2.3", "172.17.0.2", "192.168.1.10", "100.64.0.1", "fd00::1"] {
            assert_eq!(z(lan), Some(Zone::Lan), "{lan}");
        }
        for public in ["8.8.8.8", "93.184.216.34", "2001:4860:4860::8888", "100.128.0.1"] {
            assert_eq!(z(public), Some(Zone::Public), "{public}");
        }
        for never in [
            "0.0.0.0",
            "169.254.169.254",
            "224.0.0.1",
            "239.255.255.250",
            "255.255.255.255",
            "240.0.0.1",
            "0.1.2.3",
            "::",
            "fe80::1",
            "ff02::1",
            "::ffff:169.254.169.254",
        ] {
            assert_eq!(z(never), None, "{never}");
        }
    }

    #[test]
    fn links_never_reach_less_public_zones() {
        let lan: IpAddr = "192.168.1.2".parse().unwrap();
        let local: IpAddr = "127.0.0.1".parse().unwrap();
        let public: IpAddr = "1.1.1.1".parse().unwrap();
        let meta: IpAddr = "169.254.169.254".parse().unwrap();

        let p = LinkPolicy::from(Zone::Public);
        assert!(p.allows(public) && !p.allows(lan) && !p.allows(local) && !p.allows(meta));
        let l = LinkPolicy::local_file();
        assert!(l.allows(public) && l.allows(lan) && !l.allows(local) && !l.allows(meta));
        let t = LinkPolicy::typed();
        assert!(t.allows(public) && t.allows(lan) && t.allows(local) && !t.allows(meta));
    }

    #[test]
    fn hosts_take_the_most_public_zone() {
        assert_eq!(Zone::of_host("127.0.0.1", 80), Some(Zone::Loopback));
        assert_eq!(Zone::of_host("[::1]", 80), Some(Zone::Loopback));
        assert_eq!(Zone::of_host("10.0.0.1", 80), Some(Zone::Lan));
        assert_eq!(Zone::of_host("169.254.1.1", 80), None);
        assert_eq!(Zone::of_host("localhost", 80), Some(Zone::Loopback));
    }
}
