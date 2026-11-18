// SPDX-License-Identifier: MPL-2.0
//! LAN discovery beyond mDNS (NET-009) and SSDP (NET-018): NET-010,
//! ADR-0126.
//!
//! - **Windows shares:** WS-Discovery ([`wsd`]: `Probe` for computers,
//!   `Resolve`, metadata `Get` for the name and workgroup) and the NetBIOS
//!   name service ([`netbios`]: node status of the hosts of a small
//!   network and of the master browsers). A share is a host with a file
//!   server: the add flow then lists its shares over SMB as for any host.
//! - **Plex** over GDM and **Jellyfin / Emby** over their UDP
//!   auto-discovery ([`servers`]).
//!
//! [`discover`] runs every probe at once and returns one entry per
//! service and host. Everything that comes back is untrusted: datagrams
//! over 8 KiB are dropped, XML depth and size are capped, names are
//! cleaned and cut, and every address is built on the IP that answered.
//! Nothing here connects with credentials.
//!
//! iOS and tvOS need the multicast entitlement (PKG-012) before an app
//! may send multicast or broadcast; until then [`SUPPORTED`] is false
//! there and [`discover`] returns nothing.

pub mod netbios;
pub mod servers;
pub mod udp;
pub mod wsd;
mod xml;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Mutex;
use std::time::Duration;

use pa_vfs_http::{HttpClient, LinkPolicy, Timeouts, Zone};
use url::Url;

use crate::udp::{Answer, Iface, exchange, interfaces};

/// Whether this platform may probe the LAN (not iOS/tvOS before the
/// multicast entitlement, PKG-012).
pub const SUPPORTED: bool = !cfg!(any(target_os = "ios", target_os = "tvos"));

/// Longest name kept, in characters.
pub const MAX_NAME_CHARS: usize = 64;

/// Most hosts whose WS-Discovery metadata is read in one search.
const MAX_METADATA_HOSTS: usize = 32;

/// What was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ServiceKind {
    /// A Windows (or Samba) computer with a file server.
    Smb,
    /// A Jellyfin server.
    Jellyfin,
    /// An Emby server.
    Emby,
    /// A Plex Media Server.
    Plex,
}

/// One service found on the LAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanService {
    /// What it is.
    pub kind: ServiceKind,
    /// Its name, cleaned (the IP when it gave none).
    pub name: String,
    /// The IP that answered.
    pub host: Ipv4Addr,
    /// The address of the source: `smb://<ip>/` or `http(s)://<ip>:<port>`.
    pub url: String,
    /// The workgroup or domain of a computer, if known.
    pub workgroup: Option<String>,
    /// A stable id of a media server, if given.
    pub id: Option<String>,
}

/// How to search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// How long to collect the first answers. Names of computers that
    /// need a second question take longer (see [`discover`]).
    pub wait: Duration,
    /// Local IPv4 addresses to probe from; empty: every LAN interface.
    pub interfaces: Vec<Ipv4Addr>,
    /// Hosts that also get a unicast NetBIOS node status and a unicast
    /// Jellyfin/Emby/Plex question (tests: containers on a large bridge).
    pub hosts: Vec<Ipv4Addr>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            wait: Duration::from_millis(2500),
            interfaces: Vec::new(),
            hosts: Vec::new(),
        }
    }
}

/// A name from the network: control characters dropped, white space
/// collapsed, at most [`MAX_NAME_CHARS`] characters.
#[must_use]
pub fn clean_name(raw: &str) -> String {
    let words: Vec<&str> = raw
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect();
    words.join(" ").chars().take(MAX_NAME_CHARS).collect()
}

/// A fresh `urn:uuid:` message id (random enough to tell our answers from
/// others'; not a secret).
fn message_id() -> String {
    let rs = std::collections::hash_map::RandomState::new();
    let mut a = rs.build_hasher();
    a.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    let hi = a.finish();
    let mut b = rs.build_hasher();
    b.write_u64(hi ^ u64::from(std::process::id()));
    let lo = b.finish();
    let v = (u128::from(hi) << 64) | u128::from(lo);
    let h = format!("{v:032x}");
    format!(
        "urn:uuid:{}-{}-4{}-a{}-{}",
        &h[0..8],
        &h[8..12],
        &h[13..16],
        &h[17..20],
        &h[20..32]
    )
}

/// A 16-bit transaction id.
fn transaction_id() -> u16 {
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u32(std::process::id());
    (h.finish() & 0xffff) as u16
}

/// Sends `request` to the broadcast address of each interface, to the
/// multicast group `group` if given, and to each of `hosts`.
fn plan_to(
    request: &[u8],
    port: u16,
    group: Option<SocketAddrV4>,
    hosts: &[Ipv4Addr],
) -> impl Fn(&Iface) -> Vec<(SocketAddrV4, Vec<u8>)> {
    move |iface| {
        let mut v = Vec::new();
        // UDP may drop one: twice to the group and the broadcast address.
        for _ in 0..2 {
            if let Some(g) = group {
                v.push((g, request.to_vec()));
            }
            v.push((SocketAddrV4::new(iface.broadcast, port), request.to_vec()));
        }
        for h in hosts {
            v.push((SocketAddrV4::new(*h, port), request.to_vec()));
        }
        v
    }
}

#[derive(Default)]
struct Computer {
    netbios: Option<netbios::FileServer>,
    wsd: Option<wsd::WsdMatch>,
    info: Option<wsd::ComputerInfo>,
}

/// Searches the LAN for Windows shares, Plex, Jellyfin and Emby servers.
/// Blocks for about `opts.wait`, plus a second when a computer answered
/// only WS-Discovery or a master browser query, plus up to 3.5 s when one
/// still has no NetBIOS name (its WS-Discovery metadata over HTTP). Sorted by kind, then name; one entry per kind and host.
///
/// # Errors
///
/// Only when no probe at all could be sent (no network).
pub fn discover(opts: &Options) -> io::Result<Vec<LanService>> {
    if !SUPPORTED {
        return Ok(Vec::new());
    }
    let ifaces = interfaces(&opts.interfaces);
    let wsd_id = message_id();
    let nb_status = transaction_id();
    let nb_browse = nb_status.wrapping_add(1);
    let hosts = &opts.hosts;

    let wsd_probe = wsd::probe_message(&wsd_id).into_bytes();
    let wsd_plan = |_: &Iface| {
        vec![
            (wsd::WSD_ADDR, wsd_probe.clone()),
            (wsd::WSD_ADDR, wsd_probe.clone()),
        ]
    };
    let status = netbios::node_status_request(nb_status);
    let browse = netbios::master_browser_query(nb_browse);
    let nb_plan = |iface: &Iface| {
        let mut v = Vec::new();
        for _ in 0..2 {
            v.push((netbios::nbns(iface.broadcast), status.clone()));
            v.push((netbios::nbns(iface.broadcast), browse.clone()));
        }
        // Windows does not answer a broadcast node status: ask each host
        // of a small network (at most 254 datagrams of 50 bytes).
        for h in iface.hosts().into_iter().chain(hosts.iter().copied()) {
            v.push((netbios::nbns(h), status.clone()));
        }
        v
    };
    let gdm_plan = plan_to(servers::GDM_REQUEST, servers::GDM_ADDR.port(), Some(servers::GDM_ADDR), hosts);
    let jf_plan = plan_to(servers::JELLYFIN_REQUEST, servers::AUTODISCOVERY_PORT, None, hosts);
    let emby_plan = plan_to(servers::EMBY_REQUEST, servers::AUTODISCOVERY_PORT, None, hosts);

    let wait = opts.wait;
    let (wsd_r, nb_r, gdm_r, jf_r, emby_r) = std::thread::scope(|s| {
        let w = s.spawn(|| exchange(&ifaces, wsd_plan, wait));
        let n = s.spawn(|| exchange(&ifaces, nb_plan, wait));
        let g = s.spawn(|| exchange(&ifaces, &gdm_plan, wait));
        let j = s.spawn(|| exchange(&ifaces, &jf_plan, wait));
        let e = s.spawn(|| exchange(&ifaces, &emby_plan, wait));
        let join = |h: std::thread::ScopedJoinHandle<'_, io::Result<Vec<Answer>>>| {
            h.join()
                .unwrap_or_else(|_| Err(io::Error::other("probe thread panicked")))
        };
        (join(w), join(n), join(g), join(j), join(e))
    });
    if [&wsd_r, &nb_r, &gdm_r, &jf_r, &emby_r]
        .iter()
        .all(|r| r.is_err())
    {
        return Err(wsd_r.err().unwrap_or_else(|| io::Error::other("no network")));
    }

    let mut out = Vec::new();
    out.extend(media_servers(ServiceKind::Plex, gdm_r.unwrap_or_default(), servers::parse_gdm));
    let jellyfin = media_servers(
        ServiceKind::Jellyfin,
        jf_r.unwrap_or_default(),
        servers::parse_autodiscovery,
    );
    let jellyfin_ids: HashSet<Option<String>> = jellyfin.iter().map(|s| s.id.clone()).collect();
    let jellyfin_hosts: HashSet<String> = jellyfin.iter().map(|s| s.url.clone()).collect();
    out.extend(jellyfin);
    // A server that answers both questions is Jellyfin (it began as a
    // fork of Emby).
    out.extend(
        media_servers(ServiceKind::Emby, emby_r.unwrap_or_default(), servers::parse_autodiscovery)
            .into_iter()
            .filter(|s| {
                !jellyfin_hosts.contains(&s.url) && (s.id.is_none() || !jellyfin_ids.contains(&s.id))
            }),
    );
    out.extend(computers(
        &ifaces,
        &wsd_id,
        nb_status,
        nb_browse,
        wsd_r.unwrap_or_default(),
        nb_r.unwrap_or_default(),
    ));
    out.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.host.cmp(&b.host))
    });
    Ok(out)
}

fn media_servers(
    kind: ServiceKind,
    answers: Vec<Answer>,
    parse: fn(&[u8], Ipv4Addr) -> Option<servers::ServerAnswer>,
) -> Vec<LanService> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for a in answers {
        let host = *a.from.ip();
        let Some(s) = parse(&a.data, host) else {
            continue;
        };
        let url = s.url.as_str().trim_end_matches('/').to_owned();
        if !seen.insert(url.clone()) {
            continue;
        }
        let name = clean_name(&s.name);
        out.push(LanService {
            kind,
            name: if name.is_empty() { host.to_string() } else { name },
            host,
            url,
            workgroup: None,
            id: s.id.map(|i| i.chars().take(64).collect()),
        });
    }
    out
}

/// Windows shares from the WS-Discovery and NetBIOS answers, with a second
/// round for computers that still have no name.
fn computers(
    ifaces: &[Iface],
    wsd_id: &str,
    nb_status: u16,
    nb_browse: u16,
    wsd_answers: Vec<Answer>,
    nb_answers: Vec<Answer>,
) -> Vec<LanService> {
    let mut hosts: BTreeMap<Ipv4Addr, Computer> = BTreeMap::new();
    let mut browsers = Vec::new();
    for a in &nb_answers {
        let ip = *a.from.ip();
        if let Some(names) = netbios::parse_node_status(&a.data, nb_status) {
            if let Some(fs) = netbios::file_server(&names) {
                hosts.entry(ip).or_default().netbios = Some(fs);
            }
        } else if netbios::is_master_browser_answer(&a.data, nb_browse) {
            browsers.push(ip);
        }
    }
    for a in &wsd_answers {
        let ip = *a.from.ip();
        if let Some(m) = wsd::parse_matches(&a.data, ip, wsd_id).into_iter().next() {
            hosts.entry(ip).or_default().wsd = Some(m);
        }
    }

    // Round two: a node status of master browsers and of computers that
    // only answered WS-Discovery; a Resolve of computers that sent no
    // XAddrs.
    let mut ask_status: Vec<Ipv4Addr> = browsers
        .into_iter()
        .chain(
            hosts
                .iter()
                .filter(|(_, c)| c.netbios.is_none())
                .map(|(ip, _)| *ip),
        )
        .filter(|ip| hosts.get(ip).is_none_or(|c| c.netbios.is_none()))
        .collect();
    ask_status.sort_unstable();
    ask_status.dedup();
    let resolve_id = message_id();
    let resolves: Vec<Vec<u8>> = hosts
        .values()
        .filter_map(|c| c.wsd.as_ref())
        .filter(|m| m.xaddrs.is_empty())
        .take(MAX_METADATA_HOSTS)
        .map(|m| wsd::resolve_message(&resolve_id, &m.endpoint).into_bytes())
        .collect();
    let status = netbios::node_status_request(nb_status);
    let round_two = Duration::from_secs(1);
    let (st, rs) = std::thread::scope(|s| {
        let st = (!ask_status.is_empty()).then(|| {
            s.spawn(|| {
                exchange(
                    ifaces,
                    |_| {
                        ask_status
                            .iter()
                            .map(|h| (netbios::nbns(*h), status.clone()))
                            .collect()
                    },
                    round_two,
                )
            })
        });
        let rs = (!resolves.is_empty()).then(|| {
            s.spawn(|| {
                exchange(
                    ifaces,
                    |_| resolves.iter().map(|r| (wsd::WSD_ADDR, r.clone())).collect(),
                    round_two,
                )
            })
        });
        (
            st.and_then(|h| h.join().ok()).and_then(Result::ok),
            rs.and_then(|h| h.join().ok()).and_then(Result::ok),
        )
    });
    for a in st.unwrap_or_default() {
        let ip = *a.from.ip();
        if let Some(fs) = netbios::parse_node_status(&a.data, nb_status)
            .as_deref()
            .and_then(netbios::file_server)
        {
            hosts.entry(ip).or_default().netbios = Some(fs);
        }
    }
    for a in rs.unwrap_or_default() {
        let ip = *a.from.ip();
        for m in wsd::parse_matches(&a.data, ip, &resolve_id) {
            if let Some(c) = hosts.get_mut(&ip)
                && let Some(w) = c.wsd.as_mut()
                && w.endpoint == m.endpoint
                && w.xaddrs.is_empty()
            {
                w.xaddrs = m.xaddrs;
            }
        }
    }

    // The metadata of WS-Discovery computers that NetBIOS did not name, in
    // parallel: the device is on the LAN and its XAddrs on its own IP;
    // never this device itself.
    let client = HttpClient::with_timeouts(
        LinkPolicy::from(Zone::Lan),
        Timeouts {
            connect: Duration::from_millis(1500),
            read: Duration::from_secs(2),
        },
    );
    let jobs: Vec<(Ipv4Addr, Url, String)> = hosts
        .iter()
        .filter(|(_, c)| c.netbios.is_none())
        .filter_map(|(ip, c)| {
            let m = c.wsd.as_ref()?;
            Some((*ip, m.xaddrs.first()?.clone(), m.endpoint.clone()))
        })
        .take(MAX_METADATA_HOSTS)
        .collect();
    let infos = Mutex::new(HashMap::new());
    std::thread::scope(|s| {
        for (ip, url, endpoint) in &jobs {
            let (client, infos) = (&client, &infos);
            s.spawn(move || {
                let body = wsd::get_message(&message_id(), endpoint);
                let Ok((200, text)) = client.post(
                    url,
                    "application/soap+xml; charset=utf-8",
                    None,
                    &body,
                    wsd::MAX_METADATA_BYTES,
                ) else {
                    return;
                };
                if let Some(info) = wsd::parse_metadata(&text) {
                    infos
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(*ip, info);
                }
            });
        }
    });
    for (ip, info) in infos
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        if let Some(c) = hosts.get_mut(&ip) {
            c.info = Some(info);
        }
    }

    hosts
        .into_iter()
        .map(|(ip, c)| {
            let (name, workgroup) = match (c.info, c.netbios) {
                (Some(i), nb) => (i.name, i.workgroup.or(nb.and_then(|n| n.workgroup))),
                (None, Some(n)) => (n.name, n.workgroup),
                (None, None) => (String::new(), None),
            };
            let name = clean_name(&name);
            LanService {
                kind: ServiceKind::Smb,
                name: if name.is_empty() { ip.to_string() } else { name },
                host: ip,
                url: format!("smb://{ip}/"),
                workgroup: workgroup.map(|w| clean_name(&w)).filter(|w| !w.is_empty()),
                id: c.wsd.map(|m| m.endpoint),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned_and_cut() {
        assert_eq!(clean_name("  Living\u{0}  room\n NAS "), "Living room NAS");
        assert_eq!(clean_name(&"x".repeat(200)).chars().count(), MAX_NAME_CHARS);
        assert_eq!(clean_name("\u{7}\u{1b}"), "");
    }

    #[test]
    fn message_ids_look_like_uuids_and_differ() {
        let a = message_id();
        let b = message_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), "urn:uuid:".len() + 36);
        assert!(a.starts_with("urn:uuid:"));
    }

    #[test]
    fn a_short_search_returns() {
        // The LAN of the test machine is unknown: the call must return (or
        // fail cleanly without a network) and give well-formed entries.
        let opts = Options {
            wait: Duration::from_millis(200),
            ..Options::default()
        };
        if let Ok(found) = discover(&opts) {
            for s in found {
                assert!(s.url.starts_with("smb://") || s.url.starts_with("http"));
                assert!(!s.name.is_empty());
            }
        }
    }
}
