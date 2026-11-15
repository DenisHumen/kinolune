// SPDX-License-Identifier: MPL-2.0
//! Subsonic and OpenSubsonic client of the Kinolune core (NET-016).
//!
//! A small hand-written client of the Subsonic REST API (1.16.1) and its
//! OpenSubsonic extensions, blocking like the rest of the source layer
//! (ADR-0013), on `ureq` with rustls. Tested against **Navidrome**; the
//! same calls work with Gonic, Airsonic-Advanced, Ampache's Subsonic API
//! and the original Subsonic, which all speak the same API.
//!
//! What it covers:
//!
//! - [`Client`]: authentication with a user name and password as the
//!   salted **token** (`t = md5(password + salt)`, `s = salt`), with a
//!   fall back to the hex-encoded password for servers that cannot check
//!   tokens (error 41, LDAP users), or with an OpenSubsonic **API key**
//!   (`apiKey`) when the server offers the `apiKeyAuthentication`
//!   extension ([`Credentials`]);
//! - [`Client::ping`] and [`Client::extensions`]: what the server is;
//! - **artists, albums, tracks**: [`Client::artists`], [`Client::artist`],
//!   [`Client::album`], [`Client::album_list`], [`Client::song`],
//!   [`Client::search`];
//! - **streaming**: [`Client::stream_path`] (the original file,
//!   `format=raw`, read with byte ranges) and cover art;
//! - **scrobbles**: [`Client::scrobble`] ("now playing" and played);
//! - [`SubsonicFs`]: the user's music as a read-only `pa_vfs::Vfs`
//!   (`Artist/Album/NN - Title.ext` with `cover.jpg`), so the source
//!   manager, the scanner (tags of the original files), the stream proxy
//!   and the music player treat a Subsonic server like a folder of music.
//!
//! Secrets: the password or API key is a [`Secret`] whose `Debug` hides
//! it; error texts never contain URLs (every request URL carries the
//! credentials).

mod api;
mod fs;
mod http;
mod model;

pub use api::{AlbumListType, SearchQuery};
pub use fs::{DEFAULT_READ_AHEAD, SongRef, SubsonicFs};
pub use http::{
    API_VERSION, AuthMode, CLIENT_NAME, Client, Credentials, Secret, normalize_address,
    salted_token,
};
pub use model::{Album, Artist, Extension, ServerInfo, Song, parse_date_ms};

/// Error of a Subsonic call. No variant carries a credential or a URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A bad argument (address, id).
    #[error("invalid: {0}")]
    Invalid(String),
    /// The server could not be reached.
    #[error("server unreachable: {0}")]
    Unreachable(String),
    /// The server did not answer in time.
    #[error("timed out")]
    TimedOut,
    /// Wrong user name, password or API key (Subsonic errors 40, 44).
    #[error("authentication failed")]
    Unauthorized,
    /// The user may not do this (error 50).
    #[error("permission denied")]
    Forbidden,
    /// No such artist, album or song (error 70).
    #[error("not found")]
    NotFound,
    /// The server cannot use these credentials: no token auth for this
    /// user and no password to fall back to, or no API key support
    /// (errors 41, 42, 43), or an API version it does not speak (20, 30).
    #[error("not supported: {0}")]
    Unsupported(String),
    /// Another error the server reported (`code`, `message`).
    #[error("server error {code}: {message}")]
    Server {
        /// Subsonic error code.
        code: i64,
        /// Its message.
        message: String,
    },
    /// An HTTP error without a Subsonic answer.
    #[error("HTTP {status}")]
    Status {
        /// HTTP status.
        status: u16,
    },
    /// The answer was not what the API describes.
    #[error("unexpected answer: {0}")]
    Protocol(String),
}

/// Shorthand for results of this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Maps an error to the `pa-vfs` kind the source layer reports.
#[must_use]
pub fn vfs_error(e: &Error) -> pa_vfs::VfsError {
    fs::vfs_error(e)
}
