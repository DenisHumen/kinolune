// SPDX-License-Identifier: MPL-2.0
//! Media behind plain HTTP links (NET-018, ADR-0124).
//!
//! - [`HttpFile`]: a file behind an `http(s)` link as a
//!   [`ByteSource`](pa_vfs::ByteSource), read with byte ranges. The DLNA
//!   backend (`pa-dlna`), playlists and `.strm` files use it.
//! - [`LinkPolicy`]: where links from a list may point. A playlist from
//!   the Internet cannot reach into the LAN or this device; only
//!   `http(s)` links without credentials are fetched; the check runs on
//!   the addresses each connection really uses.
//! - [`PlaylistFs`]: an M3U/M3U8 playlist (a link or a file) as a
//!   read-only [`Vfs`](pa_vfs::Vfs): its files (by `#EXTINF` title) in
//!   folders by `group-title`. Live channels are left out.
//! - [`parse_strm`] / [`open_strm`]: a `.strm` file (Kodi, Jellyfin) found
//!   in any source plays the link it holds.
//!
//! Error texts name the host of a link, never the link: links often carry
//! tokens.

mod client;
mod file;
mod m3u;
mod names;
mod playlist;
mod policy;

pub use client::{HttpClient, Probe, Timeouts, USER_AGENT, check_link};
pub use file::{CHUNK, HttpFile};
pub use m3u::{Playlist, PlaylistEntry, is_strm, parse_m3u, parse_strm};
pub use names::{
    MEDIA_EXTENSIONS, UniqueNames, extension_of_mime, extension_of_url, file_name, is_media_mime,
    sanitize, stem_of_url,
};
pub use playlist::{MAX_ENTRIES, MAX_PLAYLIST_BYTES, PlaylistFs, PlaylistLocation};
pub use policy::{Blocked, LinkPolicy, Zone};

use std::sync::Arc;

use pa_vfs::{ByteSource, ErrorKind, VfsError};

/// Largest `.strm` file read.
const MAX_STRM_BYTES: u64 = 64 * 1024;

/// Opens the link of the `.strm` file `strm` (already opened from its
/// source): reads the link, checks it under [`LinkPolicy::local_file`]
/// (the LAN and the Internet, not this device), asks the server for the
/// size and returns the remote file.
///
/// # Errors
///
/// The file holds no playable link, the link is refused, the server
/// cannot be reached or does not give a size (a live stream).
pub fn open_strm(strm: &dyn ByteSource) -> pa_vfs::Result<HttpFile> {
    open_strm_with(strm, LinkPolicy::local_file())
}

/// [`open_strm`] under another policy (tests: a server on this device).
///
/// # Errors
///
/// As [`open_strm`].
pub fn open_strm_with(strm: &dyn ByteSource, policy: LinkPolicy) -> pa_vfs::Result<HttpFile> {
    let len = strm.size().min(MAX_STRM_BYTES);
    let mut buf = vec![0; usize::try_from(len).unwrap_or(0)];
    strm.read_exact_at(0, &mut buf)?;
    let url = parse_strm(&String::from_utf8_lossy(&buf))?;
    let client = Arc::new(HttpClient::new(policy));
    let size = client.probe(&url)?.size.ok_or_else(|| {
        VfsError::new(
            ErrorKind::Unsupported,
            "the .strm link is a live stream without a size",
        )
    })?;
    client.open(url, size)
}
