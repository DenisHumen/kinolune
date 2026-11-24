// SPDX-License-Identifier: MPL-2.0
//! Who may connect (owner decision 2026-11-25, ADR-NNNN): the server
//! listens on every interface, but by default accepts TCP connections only
//! from the home network, never from the internet. No UPnP, no port
//! forwarding: the server never opens itself to the WAN.
//!
//! [`Clients::Lan`] (default) admits loopback, the private IPv4 ranges of
//! RFC 1918, IPv4 link-local (169.254/16), IPv6 unique local (fc00::/7) and
//! link-local (fe80::/10). CGNAT (100.64/10) is not a home network: other
//! subscribers of the provider live there. [`Clients::Local`] admits the
//! machine itself only; [`Clients::Any`] everybody (explicit opt-in, e.g.
//! behind a VPN or a reverse proxy that filters on its own).
//!
//! Refused connections are closed right after `accept`, before TLS or HTTP.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use serde::Deserialize;
use tokio::net::{TcpListener, TcpStream};

/// Which client addresses the server accepts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Clients {
    /// This machine only (loopback).
    Local,
    /// The home network: loopback, private and link-local ranges.
    #[default]
    Lan,
    /// Any address, the internet included. Explicit opt-in.
    Any,
}

impl Clients {
    /// Parses `local`, `lan` or `any`.
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "local" => Some(Self::Local),
            "lan" => Some(Self::Lan),
            "any" => Some(Self::Any),
            _ => None,
        }
    }

    /// Whether a connection from `ip` is accepted.
    pub fn allows(self, ip: IpAddr) -> bool {
        match self {
            Self::Local => is_local(ip),
            Self::Lan => is_lan(ip),
            Self::Any => true,
        }
    }
}

/// The machine itself (also as an IPv4-mapped IPv6 address).
pub fn is_local(ip: IpAddr) -> bool {
    ip.to_canonical().is_loopback()
}

/// Loopback, private or link-local: an address of the home network.
pub fn is_lan(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => is_lan_v4(v4),
        IpAddr::V6(v6) => is_lan_v6(v6),
    }
}

fn is_lan_v4(ip: Ipv4Addr) -> bool {
    // 127/8, 10/8, 172.16/12, 192.168/16, 169.254/16.
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

fn is_lan_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    ip.is_loopback()
        // fc00::/7 unique local
        || (first & 0xfe00) == 0xfc00
        // fe80::/10 link-local
        || (first & 0xffc0) == 0xfe80
}

/// A plain TCP listener for `axum::serve` that closes connections the
/// [`Clients`] policy refuses.
pub struct FilteredListener {
    inner: TcpListener,
    allows: Box<dyn Fn(IpAddr) -> bool + Send + Sync>,
}

impl FilteredListener {
    /// Accepts what `clients` allows.
    pub fn new(inner: TcpListener, clients: Clients) -> Self {
        Self::with_filter(inner, move |ip| clients.allows(ip))
    }

    /// Accepts what `allows` says (tests).
    pub fn with_filter(
        inner: TcpListener,
        allows: impl Fn(IpAddr) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner,
            allows: Box::new(allows),
        }
    }
}

/// Logs a refused peer (debug only: a scan must not flood the log).
pub(crate) fn refused(peer: SocketAddr) {
    tracing::debug!(%peer, "connection refused: address outside the allowed clients");
}

impl axum::serve::Listener for FilteredListener {
    type Io = TcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.inner.accept().await {
                Ok((stream, peer)) if (self.allows)(peer.ip()) => return (stream, peer),
                Ok((stream, peer)) => {
                    refused(peer);
                    drop(stream);
                }
                Err(e) => {
                    // EMFILE and friends: back off instead of spinning.
                    tracing::warn!(error = %e, "accept failed");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::serve::Listener as _;
    use tokio::io::AsyncReadExt;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn lan_ranges() {
        for lan in [
            "127.0.0.1",
            "10.0.0.1",
            "10.255.255.255",
            "172.16.0.1",
            "172.31.255.254",
            "192.168.1.20",
            "169.254.10.1",
            "::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "::ffff:192.168.1.2",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_lan(ip(lan)), "{lan} is LAN");
            assert!(Clients::Lan.allows(ip(lan)));
        }
        for wan in [
            "8.8.8.8",
            "172.32.0.1",
            "172.15.255.255",
            "192.169.0.1",
            "100.64.0.1",
            "11.0.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "2001:db8::1",
            "2a00:1450::1",
            "fec0::1",
            "::ffff:8.8.8.8",
            "::",
        ] {
            assert!(!is_lan(ip(wan)), "{wan} is not LAN");
            assert!(!Clients::Lan.allows(ip(wan)));
            assert!(Clients::Any.allows(ip(wan)));
        }
    }

    #[test]
    fn local_and_any() {
        assert!(Clients::Local.allows(ip("127.0.0.1")));
        assert!(Clients::Local.allows(ip("::1")));
        assert!(Clients::Local.allows(ip("::ffff:127.0.0.1")));
        assert!(!Clients::Local.allows(ip("192.168.1.2")));
        assert!(!Clients::Local.allows(ip("fe80::1")));
        assert_eq!(Clients::default(), Clients::Lan);
        assert_eq!(Clients::parse("LAN"), Some(Clients::Lan));
        assert_eq!(Clients::parse("local"), Some(Clients::Local));
        assert_eq!(Clients::parse("any"), Some(Clients::Any));
        assert_eq!(Clients::parse("wan"), None);
    }

    /// A refused source is closed at once; an allowed one comes through.
    #[tokio::test]
    async fn refused_sources_are_closed() {
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = tcp.local_addr().unwrap();
        // Pretend loopback is the internet: refuse everything.
        let mut refusing = FilteredListener::with_filter(tcp, |_| false);
        let accepting = tokio::spawn(async move { refusing.accept().await });
        let mut client = TcpStream::connect(addr).await.unwrap();
        let mut buf = [0u8; 1];
        let read = tokio::time::timeout(Duration::from_secs(5), client.read(&mut buf))
            .await
            .expect("the server closes the connection");
        assert!(matches!(read, Ok(0) | Err(_)), "connection closed");
        assert!(!accepting.is_finished(), "nothing was handed to the server");
        accepting.abort();

        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = tcp.local_addr().unwrap();
        let mut lan = FilteredListener::new(tcp, Clients::Lan);
        let _client = TcpStream::connect(addr).await.unwrap();
        let (_, peer) = tokio::time::timeout(Duration::from_secs(5), lan.accept())
            .await
            .unwrap();
        assert!(peer.ip().is_loopback());
    }
}
