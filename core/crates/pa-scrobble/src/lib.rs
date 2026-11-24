// SPDX-License-Identifier: MPL-2.0
//! Music scrobbling to Last.fm and ListenBrainz (MUS-015, ADR-NNNN).
//!
//! - [`rules`]: when a play counts (sans-IO): «now playing» when a track
//!   starts, a scrobble once it has been *listened to* for half its length
//!   or 4 minutes, whichever comes first; tracks under 30 s never count.
//!   Seeks forward do not count as listening; a repeat of the same track
//!   is a new play.
//! - [`queue`]: the offline queue of each service, kept in a JSON file:
//!   scrobbles wait there while the network or the service is down and go
//!   out in order, in batches, with a growing pause between attempts.
//! - [`lastfm`], [`listenbrainz`]: requests and answers of both APIs
//!   (sans-IO), the Last.fm desktop sign-in and the ListenBrainz token
//!   check.
//! - [`Scrobbler`]: ties them together over a [`Transport`].
//!
//! The user's accounts are their own: the Last.fm session key and the
//! ListenBrainz token are kept by the app in the system keychain and only
//! passed in here. No server of the project is involved.

pub mod http;
pub mod key;
pub mod lastfm;
pub mod listenbrainz;
pub mod queue;
pub mod rules;
mod scrobbler;

use serde::{Deserialize, Serialize};

pub use http::{Request, Response, Transport, TransportError};
pub use scrobbler::{Config, LastFmConfig, Scrobbler, Status};

/// A track as the services see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Track {
    pub artist: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_artist: Option<String>,
    /// Length, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_number: Option<u32>,
    /// MusicBrainz recording id, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mbid: Option<String>,
}

impl Track {
    /// Both services need an artist and a title.
    pub fn is_complete(&self) -> bool {
        !self.artist.trim().is_empty() && !self.title.trim().is_empty()
    }
}

/// A play that counts: the track and when it started (Unix seconds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listen {
    pub track: Track,
    pub started_at: u64,
}

/// Where scrobbles go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    LastFm,
    ListenBrainz,
}

impl Service {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LastFm => "lastfm",
            Self::ListenBrainz => "listenbrainz",
        }
    }
}

/// What an answer of a service means for the scrobbles sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Accepted (or filtered by the service: never sent again).
    Done,
    /// Try again later (network, service down, rate limit); `after_ms`
    /// when the service says how long to wait.
    Retry { after_ms: Option<u64> },
    /// The account must be connected again (revoked session, bad token,
    /// bad API key): the queue waits.
    Auth,
    /// The service refuses this data for good: drop it.
    Drop,
}

/// An error of a sign-in step or a token check.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    /// No API key or secret of Last.fm.
    #[error("no Last.fm API key")]
    NoKey,
    /// The network or the service failed.
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// The service refused: its code and message.
    #[error("refused ({code}): {message}")]
    Refused { code: i64, message: String },
    /// An answer that does not parse.
    #[error("unexpected answer")]
    Malformed,
}

/// `User-Agent` of every request (ListenBrainz asks for a name, a version
/// and a contact URL).
pub fn user_agent() -> String {
    format!(
        "Kinolune/{} ( https://github.com/DenisHumen/kinolune )",
        env!("CARGO_PKG_VERSION")
    )
}
