// SPDX-License-Identifier: MPL-2.0
//! One round of UDP probes: a socket per local interface, the probes sent,
//! the answers collected until a deadline. Answers are untrusted: a
//! datagram larger than [`MAX_DATAGRAM`] is dropped (not read truncated)
//! and at most [`MAX_ANSWERS`] are kept per round.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, Type};

/// Largest answer read; bigger datagrams are dropped.
pub const MAX_DATAGRAM: usize = 8 * 1024;

/// Most answers kept in one round (a busy or hostile LAN cannot make the
/// app collect without end).
pub const MAX_ANSWERS: usize = 512;

/// A local IPv4 interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Iface {
    /// Its address (`0.0.0.0` when no interface is known).
    pub ip: Ipv4Addr,
    /// Its directed broadcast address (`192.168.1.255`).
    pub broadcast: Ipv4Addr,
    /// The network prefix length (`24`).
    pub prefix: u8,
}

impl Iface {
    /// The addresses of the other hosts of a small network (prefix 24 or
    /// longer: at most 254 hosts), for unicast probes; empty for larger
    /// networks and the unspecified interface.
    #[must_use]
    pub fn hosts(&self) -> Vec<Ipv4Addr> {
        if self.ip.is_unspecified() || self.prefix < 24 || self.prefix > 30 {
            return Vec::new();
        }
        let mask = u32::MAX << (32 - u32::from(self.prefix));
        let net = u32::from(self.ip) & mask;
        let last = net | !mask;
        ((net + 1)..last)
            .map(Ipv4Addr::from)
            .filter(|h| *h != self.ip)
            .collect()
    }
}

/// The local IPv4 interfaces to probe from: up, not loopback, not
/// link-local. `only` keeps the listed addresses (tests: the Docker
/// bridge). Without any, the unspecified interface with the limited
/// broadcast address.
#[must_use]
pub fn interfaces(only: &[Ipv4Addr]) -> Vec<Iface> {
    let mut out: Vec<Iface> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback() && !i.is_p2p)
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) if !v4.ip.is_link_local() => {
                let prefix = v4.prefixlen.min(32);
                let broadcast = v4.broadcast.unwrap_or_else(|| {
                    let mask = u32::from(v4.netmask);
                    Ipv4Addr::from(u32::from(v4.ip) | !mask)
                });
                Some(Iface {
                    ip: v4.ip,
                    broadcast,
                    prefix,
                })
            }
            _ => None,
        })
        .filter(|i| only.is_empty() || only.contains(&i.ip))
        .collect();
    out.sort_by_key(|i| i.ip);
    out.dedup_by_key(|i| i.ip);
    if out.is_empty() {
        out.push(Iface {
            ip: Ipv4Addr::UNSPECIFIED,
            broadcast: Ipv4Addr::BROADCAST,
            prefix: 0,
        });
    }
    out
}

fn socket_on(iface: Ipv4Addr) -> io::Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_broadcast(true)?;
    if !iface.is_unspecified() {
        // Best effort: a probe without multicast still sends broadcasts.
        let _ = s.set_multicast_if_v4(&iface);
    }
    let _ = s.set_multicast_ttl_v4(1);
    s.bind(&SocketAddr::V4(SocketAddrV4::new(iface, 0)).into())?;
    let s: UdpSocket = s.into();
    s.set_read_timeout(Some(Duration::from_millis(50)))?;
    Ok(s)
}

/// An answer: the datagram and who sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The datagram (at most [`MAX_DATAGRAM`] bytes).
    pub data: Vec<u8>,
    /// Who sent it.
    pub from: SocketAddrV4,
}

/// Sends what `plan` gives for each interface (destination, datagram) and
/// collects the answers for `wait`. A server on this device answers too
/// (it is a source like any other).
///
/// # Errors
///
/// Only when no socket could be opened or nothing could be sent at all.
pub fn exchange(
    ifaces: &[Iface],
    plan: impl Fn(&Iface) -> Vec<(SocketAddrV4, Vec<u8>)>,
    wait: Duration,
) -> io::Result<Vec<Answer>> {
    let mut sockets = Vec::new();
    let mut last_err = None;
    for iface in ifaces {
        let s = match socket_on(iface.ip) {
            Ok(s) => s,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        let mut sent = false;
        for (to, data) in plan(iface) {
            // One host that is unreachable must not stop the others.
            match s.send_to(&data, to) {
                Ok(_) => sent = true,
                Err(e) => last_err = Some(e),
            }
        }
        if sent {
            sockets.push(s);
        }
    }
    if sockets.is_empty() {
        return Err(last_err.unwrap_or_else(|| io::Error::other("no network interface")));
    }
    let deadline = Instant::now() + wait;
    let mut out = Vec::new();
    // One byte more than accepted: a datagram that fills it was larger.
    let mut buf = vec![0u8; MAX_DATAGRAM + 1];
    while Instant::now() < deadline && out.len() < MAX_ANSWERS {
        for s in &sockets {
            while let Ok((n, from)) = s.recv_from(&mut buf) {
                let SocketAddr::V4(from) = from else { continue };
                if n > MAX_DATAGRAM {
                    continue;
                }
                out.push(Answer {
                    data: buf[..n].to_vec(),
                    from,
                });
                if out.len() >= MAX_ANSWERS {
                    break;
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
    fn hosts_of_small_networks_only() {
        let i = Iface {
            ip: Ipv4Addr::new(192, 168, 1, 10),
            broadcast: Ipv4Addr::new(192, 168, 1, 255),
            prefix: 24,
        };
        let h = i.hosts();
        assert_eq!(h.len(), 253);
        assert_eq!(h[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(*h.last().unwrap(), Ipv4Addr::new(192, 168, 1, 254));
        assert!(!h.contains(&i.ip));
        assert!(Iface { prefix: 16, ..i }.hosts().is_empty());
        assert!(Iface { prefix: 31, ..i }.hosts().is_empty());
        assert_eq!(
            Iface {
                ip: Ipv4Addr::new(10, 0, 0, 5),
                prefix: 30,
                ..i
            }
            .hosts(),
            [Ipv4Addr::new(10, 0, 0, 6)]
        );
    }

    #[test]
    fn exchange_collects_answers_and_drops_oversized_ones() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = match server.local_addr().unwrap() {
            SocketAddr::V4(a) => a,
            SocketAddr::V6(_) => unreachable!(),
        };
        let t = std::thread::spawn(move || {
            let mut buf = [0u8; 64];
            let (n, from) = server.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"ping");
            server.send_to(&vec![b'x'; MAX_DATAGRAM + 10], from).unwrap();
            server.send_to(b"pong", from).unwrap();
        });
        let iface = Iface {
            ip: Ipv4Addr::UNSPECIFIED,
            broadcast: Ipv4Addr::BROADCAST,
            prefix: 0,
        };
        let answers = exchange(
            &[iface],
            |_| vec![(addr, b"ping".to_vec())],
            Duration::from_millis(500),
        )
        .unwrap();
        t.join().unwrap();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].data, b"pong");
        assert_eq!(answers[0].from, addr);
    }
}
