// SPDX-License-Identifier: MPL-2.0
//! UPnP/DLNA media servers (NET-018, ADR-0124).
//!
//! - [`search`] / [`discover`]: SSDP `M-SEARCH` for
//!   `urn:schemas-upnp-org:device:MediaServer:1` on every LAN interface,
//!   then the device descriptions ([`MediaServer`]: name, UDN, the
//!   ContentDirectory control URL).
//! - [`browse_all`]: ContentDirectory `Browse(BrowseDirectChildren)`, page
//!   by page, DIDL-Lite read into folders and video/audio items.
//! - [`DlnaFs`]: the server as a read-only `pa_vfs::Vfs`, so the source
//!   manager, the scanner and the stream proxy treat it like a folder.
//!   Files stream from the server's own `http-get` URLs with byte ranges
//!   (`pa_vfs_http::HttpFile`).
//!
//! Tested against minidlna in Docker (`tests/minidlna/run.sh`) and a
//! mock server.

mod browse;
mod device;
mod fs;
mod ssdp;
mod xml;

pub use browse::{
    DidlObject, ItemKind, MAX_CHILDREN, PAGE, Page, browse_all, browse_request, parse_browse,
    parse_didl,
};
pub use device::{MediaServer, fetch_server, parse_description};
pub use fs::{DlnaFs, FOLDER_VIEWS};
pub use ssdp::{
    MEDIA_SERVER, SSDP_ADDR, SearchOptions, SsdpAnswer, parse_answer, search, search_request,
};

use std::collections::HashSet;
use std::sync::Mutex;

use pa_vfs_http::{HttpClient, LinkPolicy, Timeouts};

/// Searches the LAN for media servers and reads their descriptions (in
/// parallel). Servers are returned once each (by UDN), sorted by name.
/// Devices that are not media servers or do not answer are left out.
///
/// # Errors
///
/// Only when the search cannot be sent at all (no network).
pub fn discover(opts: &SearchOptions) -> std::io::Result<Vec<MediaServer>> {
    let answers = search(opts)?;
    // A device answered from its own address: reading its description is
    // like the user typing it.
    let client = HttpClient::with_timeouts(
        LinkPolicy::typed(),
        Timeouts {
            connect: std::time::Duration::from_secs(3),
            read: std::time::Duration::from_secs(5),
        },
    );
    let found = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for a in &answers {
            let (client, found) = (&client, &found);
            s.spawn(move || {
                if let Ok(server) = fetch_server(client, &a.location) {
                    found
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(server);
                }
            });
        }
    });
    let mut servers = found
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    servers.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.location.as_str().cmp(b.location.as_str()))
    });
    let mut seen = HashSet::new();
    servers.retain(|s| s.udn.is_empty() || seen.insert(s.udn.clone()));
    Ok(servers)
}
