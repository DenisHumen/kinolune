// SPDX-License-Identifier: MPL-2.0
//! Jellyfin and Emby client of the Kinolune core (NET-014, NET-015).
//!
//! A small hand-written client of the Jellyfin REST API (the 10.10 OpenAPI
//! schema; tested against 10.10.7, works with 10.9+), blocking like the
//! rest of the source layer (ADR-0013), on `ureq` with rustls. The same
//! client talks to **Emby Server** (4.8+, tested against 4.10.2) with
//! [`Client::with_kind`]`(`[`ServerKind::Emby`]`)`: the two share the
//! API of their common ancestor, and the differences (headers, user-scoped
//! paths, no Quick Connect, chapter markers in place of media segments,
//! BIF in place of trickplay sheets) are switched inside the client
//! (ADR-NNNN).
//!
//! What it covers:
//!
//! - [`Client`]: one server reached at several addresses (LAN and public),
//!   with failover to the next address that answers;
//! - sign-in with a password or **Quick Connect**
//!   ([`Client::quick_connect_start`]), sign-out, users
//!   ([`Client::me`], [`Client::public_users`]);
//! - libraries and items ([`Client::libraries`], [`Client::items`]);
//! - **Continue Watching** ([`Client::resume`]) and removal from it
//!   ([`Client::remove_from_resume`]), **favorites**, watched state;
//! - **playback**: [`Client::playback_plan`] picks DirectPlay, DirectStream
//!   (remux to HLS) or Transcode (HLS) from [`DeviceCapabilities`];
//!   playback reports ([`Client::report_start`], `…_progress`,
//!   `…_stopped`);
//! - **trickplay** sheets and **media segments** of the server;
//! - [`JellyfinFs`]: the user's libraries as a read-only `pa_vfs::Vfs`, so
//!   the source manager, the scanner, the stream proxy, trickplay and the
//!   skip-marker import treat a Jellyfin server like a folder of files.
//!
//! Why hand-written and not generated from the OpenAPI document: the
//! generated client is tens of thousands of lines for ~30 endpoints we use,
//! needs an async runtime or a template set of its own, and every server
//! release changes it; the DTOs here read only the fields we need, ignore
//! the rest and treat `null` as a default, which keeps old and new servers
//! working (ADR-NNNN).
//!
//! Secrets: the access token is a [`Token`] whose `Debug` hides it; error
//! texts never contain URLs (a URL may carry `api_key`).

mod api;
mod fs;
mod http;
mod model;
mod playback;

pub use api::{EMBY_ITEM_FIELDS, ITEM_FIELDS, ItemQuery, Session};
pub use fs::{DEFAULT_READ_AHEAD, JellyfinFs, MediaRef};
pub use http::{Client, DeviceInfo, ServerKind, Token, normalize_address};
pub use model::{
    Chapter, Item, ItemPage, MediaSource, MediaStream, PublicSystemInfo, QuickConnectState,
    Segment, SegmentKind, TrickplayInfo, User, UserData, ms_to_ticks, parse_date_ms,
    segments_from_chapters, segments_json, ticks_to_ms,
};
pub use playback::{
    DeviceCapabilities, PlayMethod, PlaybackPlan, PlaybackReport, PlaybackRequest, classify,
    static_stream_path,
};

/// Error of a Jellyfin call. No variant carries a token or a URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A bad argument (address, id).
    #[error("invalid: {0}")]
    Invalid(String),
    /// No address of the server could be reached.
    #[error("server unreachable: {0}")]
    Unreachable(String),
    /// The server did not answer in time.
    #[error("timed out")]
    TimedOut,
    /// Not signed in, a wrong password, an expired or revoked token.
    #[error("authentication failed")]
    Unauthorized,
    /// The user may not do this.
    #[error("permission denied")]
    Forbidden,
    /// No such item or endpoint.
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
    /// This kind of server has no such feature (Quick Connect on Emby).
    #[error("not supported: {0}")]
    Unsupported(String),
}

/// Shorthand for results of this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Maps an error to the `pa-vfs` kind the source layer reports.
#[must_use]
pub fn vfs_error(e: &Error) -> pa_vfs::VfsError {
    fs::vfs_error(e)
}
