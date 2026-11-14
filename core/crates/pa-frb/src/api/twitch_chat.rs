// SPDX-License-Identifier: MPL-2.0
//! Twitch live chat for Dart (TWI-012): [`CoreTwitchChat`] streams rendered
//! messages (badges, text and emote fragments), moderation events and
//! "image ready" notices in batches every ~150 ms.
//!
//! - [`CoreTwitchChat::anonymous`]: read-only chat of a channel over
//!   anonymous IRC, no sign-in and no client id. Without Helix there are
//!   no badge images; badges still arrive by set and version and the UI
//!   draws its own marks. Emotes come from the Twitch CDN by id.
//! - [`CoreTwitchChat::demo`]: a made-up busy chat with local images
//!   (demo mode, CPU measurements); never touches the network.
//! - [`CoreTwitchChat::signed_in`] (TWI-024): the signed-in chat over
//!   EventSub with the channel's emotes and badges from Helix; sends
//!   through Helix `POST /chat/messages` when the sign-in has
//!   `user:write:chat` ([`CoreTwitchChat::can_send`]).
//!
//! Viewer filters (block list, word filter, mentions, hide bots) are
//! applied by the core before rendering ([`CoreTwitchChat::set_filters`]);
//! room modes arrive merged, as a full state.
//!
//! Images are downloaded once per key by the core into the 24-hour chat
//! cache (`pa_twitch_chat::ChatImages`) and handed over as local paths.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flutter_rust_bridge::frb;
use pa_imagecache::UreqFetcher;
use pa_twitch_chat as tc;

use crate::frb_generated::StreamSink;

use super::twitch::CoreTwitch;

/// `User-Agent` of chat image downloads.
const USER_AGENT: &str = concat!(
    "Kinolune/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/DenisHumen/project_alpha)"
);

/// Error of a chat call; Dart receives it as an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreChatError {
    /// `invalid` (channel name), `io` (cache directory, thread), `closed`.
    pub code: String,
    /// Human-readable message.
    pub message: String,
}

impl CoreChatError {
    fn new(code: &str, message: impl ToString) -> Self {
        Self {
            code: code.into(),
            message: message.to_string(),
        }
    }
}

/// How the chat wants its images and batches.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreChatOptions {
    /// App cache directory; the chat uses `<cache_dir>/twitch-chat`.
    pub cache_dir: String,
    /// Device pixel ratio (picks 1×, 2× or 4× images).
    pub device_pixel_ratio: f64,
    /// Dark chat background (Twitch has per-theme emote variants).
    pub dark: bool,
    /// Download animated variants. `false`: still images only.
    pub animate: bool,
    /// Batch interval in milliseconds (0: the default 150 ms).
    pub flush_ms: u32,
}

/// What a [`CoreChatEvent`] carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreChatEventKind {
    /// `message` is set.
    Message,
    /// A moderator deleted the message `target_id`.
    Deleted,
    /// All messages of user `target_id` (`target_login`) were removed:
    /// `banned`, or a timeout of `timeout_secs`, or a plain clear.
    UserCleared,
    /// The whole chat was cleared.
    ChatCleared,
    /// Server notice in `text`.
    Notice,
    /// Connection state in `status` (`text`: reason).
    Status,
    /// `image` is on disk.
    Image,
    /// The image `target_id` (a key) could not be fetched.
    ImageFailed,
    /// Room modes changed; `room` has the full current state.
    RoomModes,
}

/// Chat room modes, full state (the core merges partial updates).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoreChatRoomModes {
    pub emote_only: bool,
    pub subscribers_only: bool,
    pub unique_chat: bool,
    /// Slow mode: seconds between messages, 0 when off.
    pub slow_secs: u32,
    /// Followers-only: minimum follow age in minutes (0: any follower);
    /// `None` when off.
    pub followers_only_mins: Option<u32>,
}

/// A word-filter rule (TWI-024).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreChatWordRule {
    /// A word or phrase, or a regular expression when `regex`.
    pub pattern: String,
    pub regex: bool,
    /// Hide the whole message; `false`: mask the match with `*`.
    pub hide_message: bool,
}

/// Viewer filters of the active profile (TWI-024).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoreChatFilters {
    /// Blocked logins (or user ids).
    pub blocked: Vec<String>,
    pub words: Vec<CoreChatWordRule>,
    /// Login of the signed-in user (mentions); `None` when signed out.
    pub own_login: Option<String>,
    /// Extra words that highlight a message.
    pub highlight_keywords: Vec<String>,
    /// Hide the known bots and `extra_bots`.
    pub hide_bots: bool,
    pub extra_bots: Vec<String>,
}

impl CoreChatFilters {
    fn settings(&self) -> tc::FilterSettings {
        tc::FilterSettings {
            blocked: self.blocked.clone(),
            words: self.words.iter().map(word_rule).collect(),
            own_login: self.own_login.clone(),
            highlight_keywords: self.highlight_keywords.clone(),
            hide_bots: self.hide_bots,
            extra_bots: self.extra_bots.clone(),
        }
    }
}

fn word_rule(r: &CoreChatWordRule) -> tc::WordRule {
    tc::WordRule {
        pattern: r.pattern.clone(),
        regex: r.regex,
        action: if r.hide_message {
            tc::WordAction::Hide
        } else {
            tc::WordAction::Mask
        },
    }
}

/// Why a word rule is invalid, or `None` when it compiles (the settings
/// screen checks the field while the user types).
#[frb(sync)]
pub fn check_chat_word_rule(rule: CoreChatWordRule) -> Option<String> {
    tc::check_rule(&word_rule(&rule)).err()
}

/// The bots "hide bots" knows (lower-case logins).
#[frb(sync)]
pub fn known_chat_bots() -> Vec<String> {
    tc::KNOWN_BOTS.iter().map(|b| (*b).to_owned()).collect()
}

/// An emote the user can type: autocomplete of `:code` and Tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreChatEmote {
    /// Code as typed (`Kappa`).
    pub code: String,
    /// Image key (same as in fragments).
    pub key: String,
    /// From the channel's own set (listed first); else global.
    pub channel: bool,
}

/// Answer of [`CoreTwitchChat::send`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreChatSent {
    /// Twitch accepted the message.
    pub sent: bool,
    /// Id of the new message (empty when dropped).
    pub message_id: String,
    /// Why Twitch dropped it (`msg_duplicate`, `msg_slowmode`,
    /// `msg_subsonly`, ...).
    pub drop_code: Option<String>,
    /// Twitch's text for the drop.
    pub drop_message: Option<String>,
}

/// Helix and the account behind a signed-in chat (built by
/// `CoreTwitch::chat_access`).
#[frb(ignore)]
pub struct ChatAccess {
    pub chat: Arc<dyn tc::ChatApi>,
    pub emotes: Arc<dyn tc::EmoteApi>,
    pub user_id: String,
    pub login: String,
    pub can_send: bool,
}

/// Connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreChatStatus {
    Connected,
    Joined,
    Reconnecting,
    Stopped,
}

/// A badge in front of a name. `key` is empty when the core has no image
/// for it (anonymous chat): the UI draws a mark for well-known sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreChatBadge {
    /// Image key (see [`CoreChatImage::key`]) or empty.
    pub key: String,
    /// Set, e.g. `moderator`, `subscriber`.
    pub set_id: String,
    /// Version in the set.
    pub version: String,
    /// Title for the tooltip (empty without Helix).
    pub title: String,
    /// Extra info, e.g. exact subscription months.
    pub info: Option<String>,
}

/// A piece of a message body: text, or an emote with its image key and
/// the code as typed (shown until the image is ready).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreChatFragment {
    /// Text, or the emote code.
    pub text: String,
    /// Image key of an emote; `None` for text.
    pub emote_key: Option<String>,
}

/// A rendered chat message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreChatMessage {
    pub id: String,
    /// Server time, Unix milliseconds.
    pub ts_ms: i64,
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    /// `#RRGGBB` picked by the user; the UI corrects its contrast.
    pub color: Option<String>,
    pub badges: Vec<CoreChatBadge>,
    pub fragments: Vec<CoreChatFragment>,
    /// `/me` message.
    pub is_action: bool,
    pub bits: Option<u32>,
    /// Display name of the author this message replies to.
    pub reply_to: Option<String>,
    /// System text of a sub, raid or announcement notice.
    pub notice_text: Option<String>,
    /// Highlighted with channel points or an announcement.
    pub highlighted: bool,
    /// The user's first message in the channel.
    pub first_message: bool,
    /// Mentions the signed-in user or a highlight keyword (TWI-024).
    pub mention: bool,
}

/// An image on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreChatImage {
    /// Key shared by every message that shows the image: decode it once.
    pub key: String,
    /// Local file.
    pub path: String,
    /// The file is an animation.
    pub animated: bool,
}

/// One item of a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreChatEvent {
    pub kind: CoreChatEventKind,
    pub message: Option<CoreChatMessage>,
    /// Message id (`Deleted`), user id (`UserCleared`), image key
    /// (`ImageFailed`).
    pub target_id: Option<String>,
    /// User login (`UserCleared`, sometimes `Deleted`).
    pub target_login: Option<String>,
    /// Timeout length (`UserCleared`).
    pub timeout_secs: Option<u32>,
    /// Permanent ban (`UserCleared`).
    pub banned: bool,
    /// Notice text or status reason.
    pub text: Option<String>,
    pub status: Option<CoreChatStatus>,
    pub image: Option<CoreChatImage>,
    /// Full room modes (`RoomModes`).
    pub room: Option<CoreChatRoomModes>,
}

impl CoreChatEvent {
    fn of(kind: CoreChatEventKind) -> Self {
        Self {
            kind,
            message: None,
            target_id: None,
            target_login: None,
            timeout_secs: None,
            banned: false,
            text: None,
            status: None,
            image: None,
            room: None,
        }
    }
}

enum Mode {
    Anonymous(String),
    Demo(u32),
    Signed(Box<Signed>),
}

struct Signed {
    login: String,
    broadcaster_id: String,
    access: ChatAccess,
    /// Transport and URL overrides (tests).
    connector: Option<Arc<dyn tc::WsConnector>>,
    url: Option<String>,
}

/// A live chat view. Create it, listen to [`events`](Self::events), and
/// [`close`](Self::close) it when the view goes away.
#[frb(opaque)]
pub struct CoreTwitchChat {
    mode: Mode,
    options: CoreChatOptions,
    session: Mutex<Option<tc::ChatSession>>,
    filters: Mutex<Arc<tc::ChatFilters>>,
    assets: Mutex<Option<Arc<tc::ChannelAssets>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl CoreTwitchChat {
    /// Read-only chat of `channel` (a login) without sign-in.
    #[frb(sync)]
    pub fn anonymous(
        channel: String,
        options: CoreChatOptions,
    ) -> Result<CoreTwitchChat, CoreChatError> {
        let login = tc::normalise_login(&channel).map_err(|e| CoreChatError::new("invalid", e))?;
        Ok(Self::with(Mode::Anonymous(login), options))
    }

    /// Made-up chat at `rate_per_second` messages with local images.
    #[frb(sync)]
    pub fn demo(rate_per_second: u32, options: CoreChatOptions) -> CoreTwitchChat {
        Self::with(Mode::Demo(rate_per_second.max(1)), options)
    }

    /// The signed-in chat of `channel` (login) / `broadcaster_id` over
    /// EventSub. Sending works when [`Self::can_send`].
    #[frb(sync)]
    pub fn signed_in(
        twitch: &CoreTwitch,
        channel: String,
        broadcaster_id: String,
        options: CoreChatOptions,
    ) -> Result<CoreTwitchChat, CoreChatError> {
        let access = twitch
            .chat_access()
            .map_err(|e| CoreChatError::new("signed_out", e.message))?;
        Self::signed_with(access, channel, broadcaster_id, options, None, None)
    }

    /// [`Self::signed_in`] with injected Helix and transport (tests).
    #[frb(ignore)]
    pub fn signed_with(
        access: ChatAccess,
        channel: String,
        broadcaster_id: String,
        options: CoreChatOptions,
        connector: Option<Arc<dyn tc::WsConnector>>,
        url: Option<String>,
    ) -> Result<CoreTwitchChat, CoreChatError> {
        let login = tc::normalise_login(&channel).map_err(|e| CoreChatError::new("invalid", e))?;
        if broadcaster_id.is_empty() || !broadcaster_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(CoreChatError::new("invalid", "broadcaster id"));
        }
        Ok(Self::with(
            Mode::Signed(Box::new(Signed {
                login,
                broadcaster_id,
                access,
                connector,
                url,
            })),
            options,
        ))
    }

    fn with(mode: Mode, options: CoreChatOptions) -> Self {
        Self {
            mode,
            options,
            session: Mutex::new(None),
            filters: Mutex::new(Arc::new(tc::ChatFilters::default())),
            assets: Mutex::new(None),
        }
    }

    /// Messages can be sent: signed in with `user:write:chat`.
    #[frb(sync)]
    pub fn can_send(&self) -> bool {
        matches!(&self.mode, Mode::Signed(s) if s.access.can_send)
    }

    /// Login of the signed-in user (`None` for the anonymous and demo
    /// chats).
    #[frb(sync)]
    pub fn own_login(&self) -> Option<String> {
        match &self.mode {
            Mode::Signed(s) => Some(s.access.login.clone()),
            _ => None,
        }
    }

    /// Replaces the viewer filters. Returns the problems of word rules
    /// that were skipped, as `"<index>: <reason>"`.
    #[frb(sync)]
    pub fn set_filters(&self, filters: CoreChatFilters) -> Vec<String> {
        let (compiled, errors) = tc::ChatFilters::new(&filters.settings());
        let compiled = Arc::new(compiled);
        *lock(&self.filters) = Arc::clone(&compiled);
        if let Some(session) = lock(&self.session).as_ref() {
            session.set_filters(compiled);
        }
        errors
            .into_iter()
            .map(|e| format!("{}: {}", e.index, e.message))
            .collect()
    }

    /// Emotes of the channel and the global set, for autocomplete
    /// (empty until [`Self::events`] loaded them, and for the anonymous
    /// chat, which has no Helix). Channel emotes first, then by code.
    #[frb(sync)]
    pub fn emotes(&self) -> Vec<CoreChatEmote> {
        let Some(assets) = lock(&self.assets).clone() else {
            return Vec::new();
        };
        emote_list(&assets, &self.prefs())
    }

    /// Sends `text` (1-500 characters), optionally as a reply. A message
    /// Twitch dropped comes back with `sent == false` and the reason.
    pub fn send(
        &self,
        text: String,
        reply_to: Option<String>,
    ) -> Result<CoreChatSent, CoreChatError> {
        let Mode::Signed(signed) = &self.mode else {
            return Err(CoreChatError::new(
                "read_only",
                "anonymous chat is read-only",
            ));
        };
        if !signed.access.can_send {
            return Err(CoreChatError::new(
                "read_only",
                "the sign-in does not allow sending messages",
            ));
        }
        let result = {
            let slot = lock(&self.session);
            let Some(session) = slot.as_ref() else {
                return Err(CoreChatError::new("closed", "the chat is not running"));
            };
            session.send(&signed.broadcaster_id, &text, reply_to.as_deref())
        };
        match result {
            Ok(m) => Ok(CoreChatSent {
                sent: m.is_sent,
                message_id: m.message_id,
                drop_code: m.drop_reason.as_ref().map(|d| d.code.clone()),
                drop_message: m.drop_reason.map(|d| d.message),
            }),
            Err(e) => Err(send_error(e)),
        }
    }

    fn prefs(&self) -> tc::ImagePrefs {
        tc::ImagePrefs {
            scale: tc::Scale::for_ratio(self.options.device_pixel_ratio as f32),
            theme: if self.options.dark {
                tc::Theme::Dark
            } else {
                tc::Theme::Light
            },
            animate: self.options.animate,
        }
    }

    /// Channel login.
    #[frb(sync)]
    pub fn channel(&self) -> String {
        match &self.mode {
            Mode::Anonymous(login) => login.clone(),
            Mode::Demo(_) => tc::DEMO_CHANNEL.to_owned(),
            Mode::Signed(s) => s.login.clone(),
        }
    }

    /// Starts the chat and streams its batches. A second call replaces the
    /// first stream.
    pub fn events(&self, sink: StreamSink<Vec<CoreChatEvent>>) -> Result<(), CoreChatError> {
        let prefs = self.prefs();
        let config = tc::SessionConfig {
            prefs,
            flush_interval: if self.options.flush_ms == 0 {
                tc::SessionConfig::default().flush_interval
            } else {
                Duration::from_millis(u64::from(self.options.flush_ms))
            },
            ..tc::SessionConfig::default()
        };
        let cache = PathBuf::from(&self.options.cache_dir);
        let io = |e: std::io::Error| CoreChatError::new("io", e);
        let images = tc::ChatImages::open(
            cache.join("twitch-chat"),
            tc::DEFAULT_CHAT_CACHE_BYTES,
            Arc::new(UreqFetcher::new(USER_AGENT)),
        )
        .map_err(|e| CoreChatError::new("io", e))?;
        let mut room = tc::RoomModes::default();
        let emit = move |batch: Vec<tc::SessionEvent>| {
            let out: Vec<CoreChatEvent> = batch
                .into_iter()
                .filter_map(|e| match e {
                    tc::SessionEvent::Event(tc::ChatEvent::RoomModes { modes, .. }) => {
                        room.merge(&modes);
                        Some(CoreChatEvent {
                            room: Some(room_modes(&room)),
                            ..CoreChatEvent::of(CoreChatEventKind::RoomModes)
                        })
                    }
                    other => convert(other, &prefs),
                })
                .collect();
            out.is_empty() || sink.add(out).is_ok()
        };
        let filters = Arc::clone(&lock(&self.filters));
        let session = match &self.mode {
            Mode::Anonymous(login) => {
                let login = login.clone();
                tc::ChatSession::start_filtered(
                    move || {
                        let s = tc::IrcChatSource::connect(tc::IrcConfig::new([login]))?;
                        Ok(Box::new(s) as Box<dyn tc::ChatSource>)
                    },
                    Arc::new(tc::ChannelAssets::cdn_only()),
                    Some(images),
                    config,
                    filters,
                    emit,
                )
                .map_err(io)?
            }
            Mode::Demo(rate) => {
                let rate = *rate;
                let assets = tc::demo_assets(&cache.join("twitch-chat-demo")).map_err(io)?;
                let assets = Arc::new(assets);
                *lock(&self.assets) = Some(Arc::clone(&assets));
                tc::ChatSession::start_filtered(
                    move || {
                        Ok(Box::new(tc::SyntheticChatSource::new(rate, 0x5EED))
                            as Box<dyn tc::ChatSource>)
                    },
                    assets,
                    Some(images),
                    config,
                    filters,
                    emit,
                )
                .map_err(io)?
            }
            Mode::Signed(signed) => {
                let assets = channel_assets(signed);
                *lock(&self.assets) = Some(Arc::clone(&assets));
                let mut es = tc::EventSubConfig::new(
                    signed.access.user_id.clone(),
                    [signed.broadcaster_id.clone()],
                );
                if let Some(url) = &signed.url {
                    es.url.clone_from(url);
                }
                let api = Arc::clone(&signed.access.chat);
                let connector = signed.connector.clone();
                tc::ChatSession::start_filtered(
                    move || {
                        let s = match connector {
                            Some(c) => tc::EventSubChatSource::spawn(
                                es,
                                api,
                                c,
                                Arc::new(pa_twitch::SystemClock),
                            )?,
                            None => tc::EventSubChatSource::connect(es, api)?,
                        };
                        Ok(Box::new(s) as Box<dyn tc::ChatSource>)
                    },
                    assets,
                    Some(images),
                    config,
                    filters,
                    emit,
                )
                .map_err(io)?
            }
        };
        let mut slot = self.session.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(session);
        Ok(())
    }

    /// Stops the chat; the stream ends.
    #[frb(sync)]
    pub fn close(&self) {
        let mut slot = self.session.lock().unwrap_or_else(|e| e.into_inner());
        slot.take();
    }
}

/// Emotes and badges of a signed-in channel from Helix; on failure the
/// chat still runs with emotes from the CDN by id.
fn channel_assets(signed: &Signed) -> Arc<tc::ChannelAssets> {
    let store = tc::AssetStore::new(Arc::clone(&signed.access.emotes));
    let fetched = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()
        .and_then(|rt| rt.block_on(store.channel(&signed.broadcaster_id)).ok());
    fetched.unwrap_or_else(|| Arc::new(tc::ChannelAssets::cdn_only()))
}

fn emote_list(assets: &tc::ChannelAssets, prefs: &tc::ImagePrefs) -> Vec<CoreChatEmote> {
    let channel = assets.channel_emote_ids();
    let mut out: Vec<CoreChatEmote> = assets
        .emotes()
        .iter()
        .filter(|e| !e.code.is_empty())
        .map(|e| CoreChatEmote {
            code: e.code.clone(),
            key: tc::ImageRequest::emote(e, prefs).key,
            channel: channel.contains(&e.id),
        })
        .collect();
    out.sort_by(|a, b| {
        b.channel
            .cmp(&a.channel)
            .then_with(|| a.code.to_lowercase().cmp(&b.code.to_lowercase()))
    });
    out.dedup_by(|a, b| a.code == b.code);
    out
}

fn room_modes(m: &tc::RoomModes) -> CoreChatRoomModes {
    let secs = |d: Duration| u32::try_from(d.as_secs()).unwrap_or(u32::MAX);
    CoreChatRoomModes {
        emote_only: m.emote_only.unwrap_or(false),
        subscribers_only: m.subscribers_only.unwrap_or(false),
        unique_chat: m.unique_chat.unwrap_or(false),
        slow_secs: m.slow.flatten().map_or(0, secs),
        followers_only_mins: m.followers_only.flatten().map(|d| secs(d) / 60),
    }
}

fn send_error(e: tc::ChatError) -> CoreChatError {
    let code = match &e {
        tc::ChatError::ReadOnly => "read_only",
        tc::ChatError::Unauthorized => "signed_out",
        tc::ChatError::InvalidArgument(_) => "invalid",
        tc::ChatError::Closed => "closed",
        _ => "network",
    };
    CoreChatError::new(code, e)
}

fn convert(event: tc::SessionEvent, prefs: &tc::ImagePrefs) -> Option<CoreChatEvent> {
    use CoreChatEventKind as K;
    Some(match event {
        tc::SessionEvent::Line(line) => CoreChatEvent {
            message: Some(message(line, prefs)),
            ..CoreChatEvent::of(K::Message)
        },
        tc::SessionEvent::Image(i) => CoreChatEvent {
            image: Some(CoreChatImage {
                key: i.key,
                path: i.path.to_string_lossy().into_owned(),
                animated: i.animated,
            }),
            ..CoreChatEvent::of(K::Image)
        },
        tc::SessionEvent::ImageFailed { key } => CoreChatEvent {
            target_id: Some(key),
            ..CoreChatEvent::of(K::ImageFailed)
        },
        tc::SessionEvent::Event(e) => match e {
            tc::ChatEvent::Message(_) | tc::ChatEvent::RoomModes { .. } => return None,
            tc::ChatEvent::MessageDeleted {
                message_id,
                user_login,
                ..
            } => CoreChatEvent {
                target_id: Some(message_id),
                target_login: user_login,
                ..CoreChatEvent::of(K::Deleted)
            },
            tc::ChatEvent::UserCleared {
                user_id,
                user_login,
                action,
                ..
            } => CoreChatEvent {
                target_id: Some(user_id),
                target_login: Some(user_login),
                banned: action == tc::ClearAction::Ban,
                timeout_secs: match action {
                    tc::ClearAction::Timeout(d) => {
                        Some(u32::try_from(d.as_secs()).unwrap_or(u32::MAX))
                    }
                    _ => None,
                },
                ..CoreChatEvent::of(K::UserCleared)
            },
            tc::ChatEvent::ChatCleared { .. } => CoreChatEvent::of(K::ChatCleared),
            tc::ChatEvent::Notice { text, .. } => CoreChatEvent {
                text: Some(text),
                ..CoreChatEvent::of(K::Notice)
            },
            tc::ChatEvent::Status(s) => {
                let (status, text) = match s {
                    tc::SourceStatus::Connected => (CoreChatStatus::Connected, None),
                    tc::SourceStatus::Joined(_) => (CoreChatStatus::Joined, None),
                    tc::SourceStatus::Reconnecting { reason, .. } => {
                        (CoreChatStatus::Reconnecting, Some(reason))
                    }
                    tc::SourceStatus::Stopped { reason } => (CoreChatStatus::Stopped, Some(reason)),
                };
                CoreChatEvent {
                    status: Some(status),
                    text,
                    ..CoreChatEvent::of(K::Status)
                }
            }
        },
    })
}

fn message(line: tc::ChatLine, prefs: &tc::ImagePrefs) -> CoreChatMessage {
    let tc::ChatLine {
        message: m,
        rendered,
        mention,
    } = line;
    let m = *m;
    let badges = m
        .user
        .badges
        .iter()
        .map(|b| {
            let found = rendered
                .badges
                .iter()
                .find(|r| r.asset.set_id == b.set_id && r.asset.version == b.version);
            let (key, title) = match found {
                Some(r) => {
                    let req = tc::ImageRequest::badge(&r.asset, prefs);
                    let key = if req.url.is_empty() {
                        String::new()
                    } else {
                        req.key
                    };
                    (key, r.asset.title.clone())
                }
                None => (String::new(), String::new()),
            };
            CoreChatBadge {
                key,
                set_id: b.set_id.clone(),
                version: b.version.clone(),
                title,
                info: b.info.clone(),
            }
        })
        .collect();
    let fragments = rendered
        .fragments
        .into_iter()
        .map(|f| match f {
            tc::Fragment::Text(text) => CoreChatFragment {
                text,
                emote_key: None,
            },
            tc::Fragment::Emote { asset, code } => CoreChatFragment {
                emote_key: Some(tc::ImageRequest::emote(&asset, prefs).key),
                text: code,
            },
        })
        .collect();
    let highlighted = m.is_highlighted
        || matches!(
            m.notice.as_ref().map(|n| &n.kind),
            Some(tc::NoticeKind::Announcement { .. })
        );
    CoreChatMessage {
        id: m.id,
        ts_ms: m.ts_ms,
        user_id: m.user.id,
        login: m.user.login,
        display_name: m.user.display_name,
        color: m.user.color,
        badges,
        fragments,
        is_action: m.is_action,
        bits: m.bits,
        reply_to: m.reply.map(|r| r.parent_display_name),
        notice_text: m.notice.map(|n| n.system_text),
        highlighted,
        first_message: m.is_first_message,
        mention,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymous_validates_the_login() {
        let o = || CoreChatOptions {
            cache_dir: String::new(),
            device_pixel_ratio: 2.0,
            dark: true,
            animate: true,
            flush_ms: 0,
        };
        assert_eq!(
            CoreTwitchChat::anonymous("Some_Streamer".into(), o())
                .unwrap()
                .channel(),
            "some_streamer"
        );
        assert_eq!(
            CoreTwitchChat::anonymous("no spaces!".into(), o())
                .err()
                .map(|e| e.code),
            Some("invalid".into())
        );
    }

    #[test]
    fn lines_keep_raw_badges_and_emote_keys() {
        let assets = tc::ChannelAssets::cdn_only();
        let m = tc::ChatMessage {
            id: "x".into(),
            text: "hi Kappa".into(),
            emotes: vec![tc::EmoteRange {
                id: "25".into(),
                start: 3,
                end: 8,
            }],
            user: tc::ChatUser {
                badges: vec![tc::Badge {
                    set_id: "moderator".into(),
                    version: "1".into(),
                    info: None,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let rendered = assets.render(&m);
        let prefs = tc::ImagePrefs::default();
        let out = message(
            tc::ChatLine {
                message: Box::new(m),
                rendered,
                mention: false,
            },
            &prefs,
        );
        assert_eq!(out.badges.len(), 1);
        assert_eq!(out.badges[0].set_id, "moderator");
        assert!(out.badges[0].key.is_empty());
        assert_eq!(out.fragments.len(), 2);
        assert_eq!(out.fragments[1].text, "Kappa");
        assert_eq!(
            out.fragments[1].emote_key.as_deref(),
            Some("twitch:emote:25")
        );
    }
}
