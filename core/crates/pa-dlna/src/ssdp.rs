// SPDX-License-Identifier: MPL-2.0
//! SSDP search (UPnP Device Architecture 1.1 §1.3): `M-SEARCH` for media
//! servers, answers with the `LOCATION` of their descriptions.

use std::collections::HashSet;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, Type};
use url::Url;

/// The SSDP multicast group and port.
pub const SSDP_ADDR: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900);

/// The search target of media servers.
pub const MEDIA_SERVER: &str = "urn:schemas-upnp-org:device:MediaServer:1";

/// How to search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOptions {
    /// How long to collect answers (devices answer within `MX` seconds).
    pub wait: Duration,
    /// Local IPv4 addresses to search from; empty: every up, non-loopback
    /// interface.
    pub interfaces: Vec<Ipv4Addr>,
    /// Where to send the search (default: the SSDP group; tests send it
    /// to one device).
    pub target: SocketAddr,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            wait: Duration::from_secs(3),
            interfaces: Vec::new(),
            target: SocketAddr::V4(SSDP_ADDR),
        }
    }
}

/// One answer to a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsdpAnswer {
    /// `LOCATION`: the device description.
    pub location: Url,
    /// `USN`.
    pub usn: String,
    /// `SERVER`, if given.
    pub server: Option<String>,
    /// Who answered.
    pub from: SocketAddr,
}

/// The `M-SEARCH` request for `st`.
#[must_use]
pub fn search_request(st: &str, mx: u64) -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: {mx}\r\nST: {st}\r\nUSER-AGENT: {} UPnP/1.1 Kinolune\r\n\r\n",
        std::env::consts::OS
    )
}

/// Reads an answer to a search. Only answers whose `LOCATION` is an
/// `http` URL on the address that sent the answer are kept: a device
/// cannot send the app to another host.
#[must_use]
pub fn parse_answer(datagram: &[u8], from: SocketAddr) -> Option<SsdpAnswer> {
    let text = std::str::from_utf8(datagram).ok()?;
    let mut lines = text.split("\r\n").flat_map(|l| l.split('\n'));
    let status = lines.next()?;
    if !status.starts_with("HTTP/1.") || !status.contains(" 200") {
        return None;
    }
    let (mut location, mut usn, mut server) = (None, String::new(), None);
    for line in lines {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k.trim().to_ascii_lowercase().as_str() {
            "location" => location = Url::parse(v).ok(),
            "usn" => usn = v.to_owned(),
            "server" => server = Some(v.to_owned()).filter(|s| !s.is_empty()),
            _ => {}
        }
    }
    let location = location?;
    if location.scheme() != "http" {
        return None;
    }
    let host: IpAddr = location.host_str()?.trim_matches(['[', ']']).parse().ok()?;
    if host != from.ip() {
        return None;
    }
    Some(SsdpAnswer {
        location,
        usn,
        server,
        from,
    })
}

/// The local IPv4 addresses to search from.
fn local_ipv4() -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.ip() {
            IpAddr::V4(v4) if !v4.is_link_local() => Some(v4),
            _ => None,
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn socket_on(iface: Ipv4Addr, multicast: bool) -> io::Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    if multicast {
        s.set_multicast_if_v4(&iface)?;
        s.set_multicast_ttl_v4(2)?;
    }
    s.bind(&SocketAddr::V4(SocketAddrV4::new(iface, 0)).into())?;
    Ok(s.into())
}

/// Searches for media servers and returns the answers (one per
/// `LOCATION`). Blocks for `opts.wait`.
///
/// # Errors
///
/// Only when no socket at all could be opened (no network).
pub fn search(opts: &SearchOptions) -> io::Result<Vec<SsdpAnswer>> {
    let multicast = opts.target.ip().is_multicast();
    let ifaces = if opts.interfaces.is_empty() {
        let mut l = local_ipv4();
        if l.is_empty() || !multicast {
            l.push(Ipv4Addr::UNSPECIFIED);
        }
        l
    } else {
        opts.interfaces.clone()
    };
    let mx = opts.wait.as_secs().clamp(1, 5);
    let request = search_request(MEDIA_SERVER, mx);
    let mut sockets = Vec::new();
    let mut last_err = None;
    for iface in ifaces {
        match socket_on(iface, multicast && !iface.is_unspecified()) {
            Ok(s) => {
                // UDP may drop one: send twice, as the spec suggests.
                let sent = s
                    .send_to(request.as_bytes(), opts.target)
                    .and_then(|_| s.send_to(request.as_bytes(), opts.target));
                match sent {
                    Ok(_) => sockets.push(s),
                    Err(e) => last_err = Some(e),
                }
            }
            Err(e) => last_err = Some(e),
        }
    }
    if sockets.is_empty() {
        return Err(last_err.unwrap_or_else(|| io::Error::other("no network interface")));
    }
    for s in &sockets {
        s.set_read_timeout(Some(Duration::from_millis(100)))?;
    }
    let deadline = Instant::now() + opts.wait;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut buf = [0u8; 2048];
    while Instant::now() < deadline {
        for s in &sockets {
            while let Ok((n, from)) = s.recv_from(&mut buf) {
                if let Some(a) = parse_answer(&buf[..n], from)
                    && seen.insert(a.location.as_str().to_owned())
                {
                    out.push(a);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_answers_from_their_own_host_only() {
        let from: SocketAddr = "192.168.1.5:1900".parse().unwrap();
        let ok = b"HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=1810\r\nST: urn:schemas-upnp-org:device:MediaServer:1\r\nUSN: uuid:4d69::urn:schemas-upnp-org:device:MediaServer:1\r\nEXT:\r\nSERVER: Linux DLNADOC/1.50 UPnP/1.0 MiniDLNA/1.3.3\r\nLocation: http://192.168.1.5:8200/rootDesc.xml\r\n\r\n";
        let a = parse_answer(ok, from).unwrap();
        assert_eq!(a.location.as_str(), "http://192.168.1.5:8200/rootDesc.xml");
        assert!(a.usn.starts_with("uuid:4d69"));
        assert!(a.server.unwrap().contains("MiniDLNA"));

        let elsewhere = b"HTTP/1.1 200 OK\r\nLOCATION: http://203.0.113.9/desc.xml\r\n\r\n";
        assert!(parse_answer(elsewhere, from).is_none());
        let named = b"HTTP/1.1 200 OK\r\nLOCATION: http://nas.local:8200/desc.xml\r\n\r\n";
        assert!(parse_answer(named, from).is_none());
        let notify = b"NOTIFY * HTTP/1.1\r\nLOCATION: http://192.168.1.5:8200/d.xml\r\n\r\n";
        assert!(parse_answer(notify, from).is_none());
        assert!(search_request(MEDIA_SERVER, 2).starts_with("M-SEARCH * HTTP/1.1\r\n"));
    }
}
