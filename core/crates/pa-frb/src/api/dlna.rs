// SPDX-License-Identifier: MPL-2.0
//! UPnP/DLNA media servers on the LAN (NET-018) for Dart.
//!
//! A DLNA server is a source like the others (`CoreSources`, kind `dlna`,
//! address = the URL of its device description, no login); an M3U
//! playlist is kind `playlist` (a link or a file). What only DLNA needs
//! is finding the servers: [`dlna_search`] sends an SSDP search on every
//! LAN interface and returns the media servers that answered, for the
//! "On this network" list of the add-source flow next to the mDNS shares.

use pa_sources as s;

use super::sources::SourcesError;

/// A media server found on the LAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDlnaServer {
    /// Its name (`friendlyName`).
    pub name: String,
    /// The URL of its device description: the address of the source.
    pub location: String,
    /// Its IP address, for the UI.
    pub host: String,
    /// Server software (`modelName`), if given.
    pub model: Option<String>,
}

/// Searches the LAN for UPnP/DLNA media servers for `wait_ms`
/// (100…10000 ms) and returns them sorted by name. Blocks for that long.
///
/// # Errors
///
/// `unreachable` when no search could be sent (no network).
pub fn dlna_search(wait_ms: u32) -> Result<Vec<CoreDlnaServer>, SourcesError> {
    let wait = std::time::Duration::from_millis(wait_ms.clamp(100, 10_000).into());
    Ok(s::dlna_discover(wait)?
        .into_iter()
        .map(|m| CoreDlnaServer {
            host: m.host().to_owned(),
            name: m.name,
            location: m.location.to_string(),
            model: m.model,
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
        let _ = dlna_search(100);
    }
}
