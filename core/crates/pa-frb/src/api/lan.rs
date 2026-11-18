// SPDX-License-Identifier: MPL-2.0
//! LAN discovery of Windows shares, Plex, Jellyfin and Emby (NET-010,
//! ADR-0126) for Dart.
//!
//! mDNS (NET-009) finds what announces itself over Bonjour, SSDP (NET-018)
//! the DLNA servers; [`lan_search`] adds Windows computers with shared
//! folders (WS-Discovery, NetBIOS), Plex Media Servers (GDM) and Jellyfin
//! and Emby servers (their UDP auto-discovery). The add-source flow shows
//! them in «В этой сети»; tapping one prefills its form. Nothing connects
//! or signs in on its own.

use pa_sources as s;
use s::pa_lan_probe::ServiceKind;

use super::sources::SourcesError;

/// What a LAN service is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreLanKind {
    /// A Windows (or Samba) computer with shared folders.
    Smb,
    /// A Jellyfin server.
    Jellyfin,
    /// An Emby server.
    Emby,
    /// A Plex Media Server (sign-in: NET-012).
    Plex,
}

/// A service found on the LAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLanService {
    /// What it is.
    pub kind: CoreLanKind,
    /// Its name (the IP when it gave none).
    pub name: String,
    /// The IP that answered.
    pub host: String,
    /// The address for the form: `smb://<ip>/` or `http(s)://<ip>:<port>`.
    pub url: String,
    /// The workgroup or domain of a computer, if known.
    pub workgroup: Option<String>,
}

/// Searches the LAN for about `wait_ms` (100…10000 ms; a computer that
/// needs a second question for its name takes up to 4.5 s more) and
/// returns what answered, sorted by kind and name. Blocks for that long.
///
/// # Errors
///
/// `unreachable` when no probe could be sent (no network).
pub fn lan_search(wait_ms: u32) -> Result<Vec<CoreLanService>, SourcesError> {
    let wait = std::time::Duration::from_millis(wait_ms.clamp(100, 10_000).into());
    Ok(s::lan_discover(wait)?
        .into_iter()
        .map(|l| CoreLanService {
            kind: match l.kind {
                ServiceKind::Smb => CoreLanKind::Smb,
                ServiceKind::Jellyfin => CoreLanKind::Jellyfin,
                ServiceKind::Emby => CoreLanKind::Emby,
                ServiceKind::Plex => CoreLanKind::Plex,
            },
            name: l.name,
            host: l.host.to_string(),
            url: l.url,
            workgroup: l.workgroup,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_search_returns() {
        // No assertion on what is found: the LAN of the test machine is
        // unknown. The call must return (or fail cleanly without network).
        if let Ok(found) = lan_search(100) {
            assert!(found.iter().all(|s| !s.name.is_empty()));
        }
    }
}
