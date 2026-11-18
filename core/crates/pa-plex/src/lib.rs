// SPDX-License-Identifier: MPL-2.0
//! Plex client of the Kinolune core (NET-012, ADR-0131).
//!
//! A small hand-written client of the two Plex APIs, blocking like the rest
//! of the source layer (ADR-0013), on `ureq` with rustls:
//!
//! - **plex.tv** ([`PlexTv`]): the PIN sign-in of TVs and consoles — the
//!   app shows a four-character code, the user enters it at
//!   <https://plex.tv/link> on any device where they are signed in, and the
//!   app receives an access token ([`PlexTv::create_pin`],
//!   [`PlexTv::check_pin`]); the account ([`PlexTv::account`]), the users
//!   of a **Plex Home** and switching to one of them with their PIN
//!   ([`PlexTv::home_users`], [`PlexTv::switch_user`]); the servers the
//!   user can reach with their connections ([`PlexTv::servers`]).
//! - **Plex Media Server** ([`Server`]): one server reached over several
//!   connections — local (on the LAN), remote (the public address) and
//!   relay (Plex's own relay, slow) — tried in that order
//!   ([`order_connections`]); [`probe_connections`] checks them all at once
//!   and tells which ones answer. Libraries ([`Server::sections`]), their
//!   items with external ids, artwork and files ([`Server::items`]), music
//!   playlists ([`Server::playlists`]).
//! - [`PlexFs`]: the libraries of a server as a read-only `pa_vfs::Vfs`
//!   tree, laid out like local folders (`Movie (Year)/Movie (Year).mkv`,
//!   `movie.nfo` with the TMDB/IMDb/TVDB ids, `poster.jpg`), so the
//!   scanner, recognition, the library, the stream proxy and offline
//!   copies treat a Plex server like a folder of files.
//! - Playback (NET-013, ADR-0134): timeline reports ([`Server::timeline`]),
//!   watched marks ([`Server::scrobble`], [`Server::unscrobble`]),
//!   Continue Watching ([`Server::remove_from_continue_watching`]) and the
//!   transcoder on request ([`Server::transcode_url`]).
//!
//! No Plex code is used: the client follows the public Plex API
//! documentation; tests run against synthetic answers in the shape the
//! API documents (`tests/fixtures/`).
//!
//! Secrets: an access token is a [`Token`] whose `Debug` hides it; error
//! texts never contain URLs or tokens.

mod fs;
mod http;
mod model;
mod playback;
mod server;
mod tv;

pub use fs::{DEFAULT_READ_AHEAD, MediaRef, PlexFs};
pub use http::{ClientInfo, PLEX_TV, Token, normalize_address};
pub use model::{
    Account, Connection, HomeUser, Identity, Item, Media, Part, Pin, Playlist, Resource, Section,
    Tag,
};
pub use playback::{
    LIBRARY_IDENTIFIER, PlayState, Timeline, TranscodeRequest, is_lan_address, needs_transcode,
    new_session_id,
};
pub use server::{ConnectionKind, Server, order_connections, probe_connections};
pub use tv::PlexTv;

/// Error of a Plex call. No variant carries a token or a URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A bad argument (address, id).
    #[error("invalid: {0}")]
    Invalid(String),
    /// No connection of the server could be reached.
    #[error("server unreachable: {0}")]
    Unreachable(String),
    /// The server did not answer in time.
    #[error("timed out")]
    TimedOut,
    /// Not signed in, a wrong PIN, an expired or revoked token.
    #[error("authentication failed")]
    Unauthorized,
    /// The user may not do this.
    #[error("permission denied")]
    Forbidden,
    /// No such item, PIN or endpoint.
    #[error("not found")]
    NotFound,
    /// Another HTTP error.
    #[error("HTTP {status}: {message}")]
    Status {
        /// HTTP status.
        status: u16,
        /// Start of the body.
        message: String,
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
    use pa_vfs::ErrorKind;
    let kind = match e {
        Error::Invalid(_) => ErrorKind::InvalidPath,
        Error::Unreachable(_) => ErrorKind::Unavailable,
        Error::TimedOut => ErrorKind::TimedOut,
        Error::Unauthorized => ErrorKind::AuthenticationFailed,
        Error::Forbidden => ErrorKind::PermissionDenied,
        Error::NotFound => ErrorKind::NotFound,
        _ => ErrorKind::Other,
    };
    pa_vfs::VfsError::new(kind, e.to_string())
}
