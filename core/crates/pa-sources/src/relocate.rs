// SPDX-License-Identifier: MPL-2.0
//! Finding a LAN server again after its address changed (BUG-004).
//!
//! A source picked in «В этой сети» is saved by the IP that answered
//! (`smb://192.168.1.20/media`): connecting by IP is fast and works where
//! `.local` names do not resolve (Android). Its stable name is kept next to
//! it: [`SourceOptions::lan_name`](crate::SourceOptions::lan_name) (the
//! mDNS host `nas.local`, the NetBIOS or WS-Discovery computer name, or the
//! UDN of a DLNA server), or the server id of a Jellyfin, Emby or Plex
//! source. When the IP stops answering (DHCP gave the server another one),
//! [`SourceManager::connect`] asks a [`LanResolver`] for the server by that
//! name, saves the new address and connects there, without the user.
//!
//! Sources saved by IP before BUG-004 keep working; a check
//! ([`SourceManager::check`]) that reaches one asks the network for the
//! name of the computer at its IP and keeps it for next time.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::sync::mpsc;
use std::time::Duration;

use pa_vfs::{Vfs, VfsPath};

use crate::{Source, SourceKind};

/// How long one search of the LAN waits for answers.
const SEARCH_WAIT: Duration = Duration::from_millis(2500);

/// How long the system resolver may take for a name.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);

/// Finds LAN servers by their stable name. The default,
/// [`NetworkResolver`], asks the network; tests use their own.
pub trait LanResolver: Send + Sync {
    /// The current address of the computer called `name`: an mDNS host
    /// (`nas.local`) or a NetBIOS / WS-Discovery name (`NAS`).
    fn host(&self, name: &str) -> Option<IpAddr>;

    /// The current address (`http(s)://ip:port`) of the Jellyfin, Emby or
    /// Plex server with this id.
    fn media_server(&self, kind: SourceKind, id: &str) -> Option<String>;

    /// The current description URL of the DLNA server with this UDN.
    fn dlna(&self, udn: &str) -> Option<String>;

    /// The stable name of the server behind the address of `source` (a
    /// source saved by IP before BUG-004): the computer name at its IP, or
    /// the UDN of a DLNA server.
    fn name_of(&self, source: &Source, ip: IpAddr) -> Option<String>;
}

/// Asks the network: the system resolver (mDNS `.local` names on Apple
/// systems, Windows, Android 12+ and Linux with nss-mdns; NetBIOS names on
/// Windows), then a NetBIOS / WS-Discovery search of the LAN (Samba and
/// Windows on every system), the UDP auto-discovery of Jellyfin, Emby and
/// Plex, and SSDP for DLNA. Every call blocks for a few seconds at most.
#[derive(Debug, Clone, Copy, Default)]
pub struct NetworkResolver;

impl NetworkResolver {
    fn lan(hosts: Vec<Ipv4Addr>) -> Vec<pa_lan_probe::LanService> {
        pa_lan_probe::discover(&pa_lan_probe::Options {
            wait: SEARCH_WAIT,
            hosts,
            ..pa_lan_probe::Options::default()
        })
        .unwrap_or_default()
    }

    fn dlna_servers() -> Vec<pa_dlna::MediaServer> {
        pa_dlna::discover(&pa_dlna::SearchOptions {
            wait: SEARCH_WAIT,
            ..pa_dlna::SearchOptions::default()
        })
        .unwrap_or_default()
    }
}

/// The system resolver, given up after [`RESOLVE_TIMEOUT`] (a lookup that
/// hangs keeps its thread, not the caller).
fn system_resolve(name: &str) -> Option<IpAddr> {
    let (tx, rx) = mpsc::channel();
    let name = name.to_owned();
    std::thread::Builder::new()
        .name("pa-sources-resolve".into())
        .spawn(move || {
            let found = (name.as_str(), 0)
                .to_socket_addrs()
                .ok()
                .and_then(|mut a| a.next())
                .map(|a| a.ip());
            let _ = tx.send(found);
        })
        .ok()?;
    rx.recv_timeout(RESOLVE_TIMEOUT).ok().flatten()
}

/// Whether a computer name from the LAN is `name` (`NAS` for `nas.local`,
/// case-insensitive).
pub(crate) fn same_computer(found: &str, name: &str) -> bool {
    let short = |n: &str| {
        let n = n.trim_end_matches('.');
        n.strip_suffix(".local").unwrap_or(n).to_lowercase()
    };
    !found.is_empty() && short(found) == short(name)
}

impl LanResolver for NetworkResolver {
    fn host(&self, name: &str) -> Option<IpAddr> {
        if let Some(ip) = system_resolve(name) {
            return Some(ip);
        }
        Self::lan(Vec::new())
            .into_iter()
            .find(|s| s.kind == pa_lan_probe::ServiceKind::Smb && same_computer(&s.name, name))
            .map(|s| IpAddr::V4(s.host))
    }

    fn media_server(&self, kind: SourceKind, id: &str) -> Option<String> {
        let want = match kind {
            SourceKind::Jellyfin => pa_lan_probe::ServiceKind::Jellyfin,
            SourceKind::Emby => pa_lan_probe::ServiceKind::Emby,
            SourceKind::Plex => pa_lan_probe::ServiceKind::Plex,
            _ => return None,
        };
        Self::lan(Vec::new())
            .into_iter()
            .find(|s| s.kind == want && s.id.as_deref() == Some(id))
            .map(|s| s.url)
    }

    fn dlna(&self, udn: &str) -> Option<String> {
        Self::dlna_servers()
            .into_iter()
            .find(|s| s.udn.eq_ignore_ascii_case(udn))
            .map(|s| s.location.to_string())
    }

    fn name_of(&self, source: &Source, ip: IpAddr) -> Option<String> {
        match source.kind {
            SourceKind::Dlna => Self::dlna_servers()
                .into_iter()
                .find(|s| s.location.as_str() == source.uri)
                .map(|s| s.udn),
            _ => {
                let IpAddr::V4(v4) = ip else { return None };
                Self::lan(vec![v4])
                    .into_iter()
                    .find(|s| {
                        s.kind == pa_lan_probe::ServiceKind::Smb
                            && s.host == v4
                            // A computer without a name is listed by its IP.
                            && s.name.parse::<IpAddr>().is_err()
                    })
                    .map(|s| s.name)
            }
        }
    }
}

/// How a source can be found again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Identity {
    /// A computer name (SMB, WebDAV, SFTP, FTP).
    Host(String),
    /// The UDN of a DLNA server.
    Dlna(String),
    /// The id of a Jellyfin, Emby or Plex server.
    MediaServer(String),
}

/// The stable name of a source, if it has one.
pub(crate) fn identity(s: &Source) -> Option<Identity> {
    let name = s.options.lan_name.clone().filter(|n| !n.trim().is_empty());
    match s.kind {
        SourceKind::Smb | SourceKind::WebDav | SourceKind::Sftp | SourceKind::Ftp => {
            name.map(Identity::Host)
        }
        SourceKind::Dlna => name.map(Identity::Dlna),
        SourceKind::Jellyfin | SourceKind::Emby => s
            .options
            .jellyfin
            .as_ref()
            .map(|j| j.server_id.clone())
            .filter(|id| !id.is_empty())
            .map(Identity::MediaServer),
        SourceKind::Plex => s
            .options
            .plex
            .as_ref()
            .map(|p| p.server_id.clone())
            .filter(|id| !id.is_empty())
            .map(Identity::MediaServer),
        _ => None,
    }
}

/// The IP in the address of a source, if it is saved by IP.
pub(crate) fn ip_of(uri: &str) -> Option<IpAddr> {
    // `smb://` is not a special scheme: its host is opaque text to `url`.
    let url = url::Url::parse(uri).ok()?;
    let host = url.host_str()?;
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok()
}

/// `uri` with its host replaced by `ip` (port, path and the rest kept).
pub(crate) fn with_host(uri: &str, ip: IpAddr) -> Option<String> {
    let mut url = url::Url::parse(uri).ok()?;
    url.set_ip_host(ip).ok()?;
    Some(url.to_string())
}

/// Whether a probe failure means the server is not at this address (rather
/// than a refused login or a missing folder).
pub(crate) fn is_unreachable(e: &pa_vfs::VfsError) -> bool {
    e.kind().is_transient()
}

/// One cheap request that needs the server: the root of a file share, the
/// library list of a media server.
pub(crate) fn probe(kind: SourceKind, vfs: &dyn Vfs) -> pa_vfs::Result<()> {
    if matches!(
        kind,
        SourceKind::Jellyfin | SourceKind::Emby | SourceKind::Plex
    ) {
        vfs.list(&VfsPath::root()).map(|_| ())
    } else {
        vfs.stat(&VfsPath::root()).map(|_| ())
    }
}

/// Sources whose name was already looked for in this run (one search per
/// source and run, so a NAS without a name does not slow every check).
#[derive(Debug, Default)]
pub(crate) struct Asked(HashSet<i64>);

impl Asked {
    /// `true` the first time for `id`.
    pub(crate) fn first(&mut self, id: i64) -> bool {
        self.0.insert(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_without_case_and_local() {
        assert!(same_computer("NAS", "nas.local"));
        assert!(same_computer("nas", "NAS"));
        assert!(same_computer("Nas.local.", "nas"));
        assert!(!same_computer("NAS2", "nas.local"));
        assert!(!same_computer("", "nas"));
    }

    #[test]
    fn host_is_replaced_and_port_kept() {
        let ip: IpAddr = "192.168.1.30".parse().unwrap();
        assert_eq!(
            with_host("smb://192.168.1.20/media/Films", ip).unwrap(),
            "smb://192.168.1.30/media/Films"
        );
        assert_eq!(
            with_host("http://192.168.1.20:8080/dav", ip).unwrap(),
            "http://192.168.1.30:8080/dav"
        );
        let v6: IpAddr = "fd00::5".parse().unwrap();
        assert_eq!(
            with_host("smb://nas.local:1445/media", v6).unwrap(),
            "smb://[fd00::5]:1445/media"
        );
        assert_eq!(
            ip_of("smb://192.168.1.20/media"),
            "192.168.1.20".parse().ok()
        );
        assert_eq!(ip_of("smb://nas.local/media"), None);
    }
}
