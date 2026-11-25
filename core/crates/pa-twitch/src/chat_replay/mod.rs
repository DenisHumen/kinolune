// SPDX-License-Identifier: MPL-2.0
//! VOD chat replay (TWI-016, research 06 §2.5, §3.4), full builds only.
//!
//! - **Format** ([`td`]): the chat of a VOD is kept in the JSON of
//!   TwitchDownloader (`streamer`, `video`, `comments[]` with
//!   `content_offset_seconds`, `commenter`, `message.fragments`,
//!   `user_badges`, `user_color`; `embeddedData` kept as is). The same
//!   parser reads a `.json` / `.json.gz` saved next to a downloaded VOD
//!   (TwitchDownloader, our downloader TWI-017, our live recorder TWI-018),
//!   so the replay works offline.
//! - **Online** ([`OnlineReplay`]): GQL `VideoCommentsByOffsetOrCursor`
//!   only when the signed remote config turns `twitch.chat_replay` on
//!   (with `twitch.gql_playback`, no kill-switch) and carries the
//!   persisted-query hash of [`COMMENTS_OPERATION`]; never a compiled-in
//!   hash or client-id ([`ChatReplayGql`]). Pages are asked by offset
//!   around the playhead (the offset mode is the stable one, 06 §2.5),
//!   continued by offset or, when a page does not move forward, by
//!   cursor; every page is cached for the life of the replay, and the
//!   caller prefetches the next one ahead of the playhead.
//! - **Sync** ([`ReplayTimeline`]): comments sorted by their offset, a
//!   binary search finds the last one at or before the playhead; after a
//!   seek the window of the last N comments before the new position is
//!   drawn again.
//!
//! Comments deleted by moderators are dropped ([`td::Comment::is_deleted`]).
//! Nothing here is compiled into store builds (`cfg(pa_twitch_native)`,
//! ADR-0041).

mod gql;
mod online;
pub mod td;
mod timeline;

use pa_remote_config::{ClientContext, RemoteConfig};

pub use online::{Coverage, OnlineReplay, PageStats};
pub use td::{ChatReplayFile, Comment, ReplayFileError};
pub use timeline::{ReplayTimeline, TimedComment};

use crate::playback::Flavor;

/// Name of the GQL operation whose persisted-query hash the remote config
/// must carry in `twitch.persisted_queries` for the online replay.
pub const COMMENTS_OPERATION: &str = "VideoCommentsByOffsetOrCursor";

/// GQL values from the remote config needed for the replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatReplaySettings {
    /// Public `Client-ID` header for `gql.twitch.tv`.
    pub client_id: String,
    /// Persisted-query hash of [`COMMENTS_OPERATION`].
    pub comments_hash: String,
}

/// Why the online replay is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatReplayOff {
    /// Store build (never reached: the module is full-only; kept for the
    /// diagnostics of [`ChatReplayGql::resolve_for`]).
    StoreBuild,
    /// `twitch.chat_replay` (or `twitch.gql_playback`) is off or killed for
    /// this app version, or the config has no GQL client-id.
    FlagOff,
    /// The config has no persisted-query hash of [`COMMENTS_OPERATION`].
    MissingQueryHash,
}

impl ChatReplayOff {
    /// Stable code for logs and the UI.
    pub fn code(self) -> &'static str {
        match self {
            ChatReplayOff::StoreBuild => "store_build",
            ChatReplayOff::FlagOff => "flag_off",
            ChatReplayOff::MissingQueryHash => "missing_query_hash",
        }
    }
}

/// Whether this client may ask GQL for the chat of a VOD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatReplayGql {
    /// No GQL request is ever made.
    Off(ChatReplayOff),
    /// GQL with these settings.
    On(ChatReplaySettings),
}

impl ChatReplayGql {
    /// For this build ([`Flavor::CURRENT`]).
    pub fn resolve(config: &RemoteConfig, ctx: &ClientContext) -> Self {
        Self::resolve_for(config, ctx, Flavor::CURRENT)
    }

    /// For a given flavor (tests and diagnostics).
    pub fn resolve_for(config: &RemoteConfig, ctx: &ClientContext, flavor: Flavor) -> Self {
        if flavor == Flavor::Store {
            return ChatReplayGql::Off(ChatReplayOff::StoreBuild);
        }
        let twitch = config.twitch_flags(ctx);
        if !twitch.chat_replay {
            return ChatReplayGql::Off(ChatReplayOff::FlagOff);
        }
        match (
            twitch.gql_client_id,
            twitch.persisted_queries.get(COMMENTS_OPERATION),
        ) {
            (Some(client_id), Some(hash)) if !client_id.is_empty() && !hash.is_empty() => {
                ChatReplayGql::On(ChatReplaySettings {
                    client_id,
                    comments_hash: hash.clone(),
                })
            }
            (None, _) => ChatReplayGql::Off(ChatReplayOff::FlagOff),
            _ => ChatReplayGql::Off(ChatReplayOff::MissingQueryHash),
        }
    }

    /// GQL is allowed by the config.
    pub fn is_on(&self) -> bool {
        matches!(self, ChatReplayGql::On(_))
    }
}

/// Replay failures. GQL codes match the monitoring names of 06 §2.7 and
/// the chapter errors (`chapters::ChaptersError`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChatReplayError {
    /// The config does not allow the online replay.
    #[error("VOD chat replay is off: {}", .0.code())]
    Off(ChatReplayOff),
    /// No response from GQL.
    #[error("network error: {0}")]
    Transport(String),
    /// GQL does not know the persisted-query hash: the config is stale.
    #[error("Twitch GQL rejected the persisted-query hash")]
    GqlHashInvalid,
    /// GQL wants a client-integrity token, which we do not produce.
    #[error("Twitch GQL requires a client-integrity token")]
    IntegrityRequired,
    /// The VOD is unknown, deleted or not public.
    #[error("the VOD is not available")]
    NotFound,
    /// Any other GQL failure.
    #[error("Twitch GQL error {status}: {message}")]
    Gql {
        /// HTTP status.
        status: u16,
        /// Error message (short).
        message: String,
    },
    /// The answer or a file is not what we expect.
    #[error("unexpected chat replay data: {0}")]
    Decode(String),
}

impl ChatReplayError {
    /// Stable code for logs, diagnostics and the UI.
    pub fn code(&self) -> &'static str {
        match self {
            ChatReplayError::Off(off) => off.code(),
            ChatReplayError::Transport(_) => "network",
            ChatReplayError::GqlHashInvalid => "gql_hash_invalid",
            ChatReplayError::IntegrityRequired => "integrity_required",
            ChatReplayError::NotFound => "not_found",
            ChatReplayError::Gql { .. } => "gql_error",
            ChatReplayError::Decode(_) => "decode",
        }
    }
}

impl From<ReplayFileError> for ChatReplayError {
    fn from(e: ReplayFileError) -> Self {
        ChatReplayError::Decode(e.to_string())
    }
}
