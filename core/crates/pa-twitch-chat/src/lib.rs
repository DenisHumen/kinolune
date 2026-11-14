// SPDX-License-Identifier: MPL-2.0
//! Twitch live chat (TWI-010): the [`ChatSource`] abstraction and its two
//! implementations (research 06 §3.1, §3.3).
//!
//! - [`IrcChatSource`]: anonymous IRC over WebSocket
//!   (`wss://irc-ws.chat.twitch.tv`, nick `justinfan<random>`, IRCv3 tags),
//!   read-only, works without sign-in and for many channels at once.
//! - [`EventSubChatSource`]: signed-in. Reads through the EventSub
//!   WebSocket (`channel.chat.message`, `channel.chat.notification` and the
//!   moderation feeds) and sends through Helix `POST /chat/messages`
//!   ([`ChatApi`], implemented for `pa_twitch::HelixClient` and
//!   `pa_twitch::AuthorizedHelix`).
//!
//! Both normalise into one model ([`ChatMessage`] inside [`ChatEvent`]):
//! user, colour, raw badge ids, emote ranges, reply/thread, bits, system
//! notices, plus moderation events (delete, ban, timeout, clear, room
//! modes). The chat UI is TWI-012.
//!
//! # Emotes and badges (TWI-011)
//!
//! - [`AssetStore`] fetches Twitch global and channel emotes and badges
//!   through Helix ([`EmoteApi`]) and keeps one immutable [`ChannelAssets`]
//!   per channel for an hour ([`EMOTE_MAP_TTL`]).
//! - [`ChannelAssets::render`] turns a [`ChatMessage`] into
//!   [`RenderedMessage`]: badges plus text and emote [`Fragment`]s. Emotes
//!   that are in no catalog still render from the Twitch CDN by id, so the
//!   anonymous chat ([`ChannelAssets::cdn_only`]) shows them too.
//! - Each emote and badge is one shared asset with a stable key
//!   ([`EmoteAsset::key`]); [`Animation`] tells the renderer whether to
//!   decode an animation, once per key.
//! - [`ChatImages`] stores the images through `pa-imagecache` in a
//!   separate cache limited to 24 hours (Twitch Developer Agreement).
//! - 7TV, BTTV and FFZ are not implemented: their API terms are not
//!   confirmed (docs/legal/providers.md §2.4).
//!
//! # Design
//!
//! - **Sans-IO parsers** ([`irc`], [`eventsub`]) are pure functions, tested
//!   on recorded frames.
//! - **Injectable transport and time**: sockets go through [`WsConnector`]
//!   (tokio-tungstenite + rustls/ring + webpki-roots behind the default `ws`
//!   feature; no OpenSSL) and sleeps through `pa_twitch::Clock`, so the
//!   reconnect loops run on scripted connections in tests ([`testing`]).
//! - **Reconnect with backoff** ([`Backoff`]): exponential with jitter on
//!   network loss; immediate on IRC `RECONNECT`; EventSub
//!   `session_reconnect` moves to the new URL without resubscribing. Idle
//!   and keepalive timeouts detect dead sockets.
//! - **No client id here.** The EventSub source uses whatever Helix client
//!   the app built with its configured client id; nothing third-party is
//!   hard-coded (ADR-0011 §5).

mod assets;
pub mod eventsub;
mod eventsub_source;
mod images;
pub mod irc;
mod irc_source;
mod model;
mod render;
mod source;
#[cfg(feature = "testing")]
pub mod testing;
mod ws;

pub use assets::{
    Animation, AssetStore, BadgeAsset, ChannelAssets, EMOTE_MAP_TTL, EmoteApi, EmoteAsset,
    EmoteProvider, ImagePrefs, MAX_CACHED_CHANNELS, Scale, TWITCH_EMOTE_TEMPLATE, Theme,
};
pub use images::{ChatImages, DEFAULT_CHAT_CACHE_BYTES, ImageRequest, image_requests};
pub use render::{Fragment, RenderedBadge, RenderedMessage};
pub use eventsub_source::{DEFAULT_EVENTSUB_URL, EventSubChatSource, EventSubConfig};
pub use irc_source::{
    DEFAULT_IRC_URL, IrcChatSource, IrcConfig, MAX_IRC_CHANNELS, anonymous_nick, normalise_login,
};
pub use model::*;
pub use pa_twitch::SentChatMessage;
pub use source::{Backoff, ChatApi, ChatError, ChatSource, SourceKind};
#[cfg(feature = "ws")]
pub use ws::TungsteniteConnector;
pub use ws::{WsConnection, WsConnector};
