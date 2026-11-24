// SPDX-License-Identifier: MPL-2.0
//! VOD chat replay for Dart (TWI-016, full builds only).
//!
//! [`CoreChatReplay`] holds the chat of one VOD and hands Dart rendered
//! lines with their offsets ([`CoreReplayChunk`]); the Dart player keeps
//! them in step with the playhead (binary search, seeks, the ±N s delay).
//!
//! - [`CoreChatReplay::online`]: GQL `VideoCommentsByOffsetOrCursor` when
//!   the signed remote config allows it (`twitch.chat_replay`, hash,
//!   kill-switches; `pa_twitch::chat_replay::ChatReplayGql`). Each
//!   [`CoreChatReplay::chunk_at`] fetches the pages around a position that
//!   are not cached yet; Dart asks for the next stretch ahead of the
//!   playhead (prefetch).
//! - [`CoreChatReplay::open_file`]: a TwitchDownloader chat file (`.json`
//!   or `.json.gz`) next to a downloaded VOD ([`find_chat_replay_file`]):
//!   the replay without network.
//!
//! Lines are rendered like the live chat (TWI-012): badges, Twitch emotes,
//! 7TV/BetterTTV/FFZ emotes (TWI-023), the viewer's filters (TWI-024).
//! Images go through the same 24-hour chat cache and arrive on
//! [`CoreChatReplay::images`]. Comments deleted by moderators never reach
//! Dart.
//!
//! The API is the same in every flavor so the generated bindings are too;
//! store builds have no replay code (`cfg(pa_twitch_native)`, ADR-0041) and
//! every call there fails with `store_build`.

use flutter_rust_bridge::frb;

use super::remote_config::CoreRemoteConfig;
use super::twitch::CoreTwitch;
use super::twitch_chat::{
    CoreChatError, CoreChatFilters, CoreChatImage, CoreChatMessage, CoreChatOptions,
};
use crate::frb_generated::StreamSink;

/// One line of the replay.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreReplayLine {
    /// When it is shown, ms from the VOD start.
    pub offset_ms: i64,
    /// The rendered message.
    pub message: CoreChatMessage,
}

/// Lines of a stretch of the VOD: every comment shown in
/// `[from_ms, to_ms)` is in `lines`, in order (`to_ms` `None`: up to the
/// end of the VOD).
#[derive(Debug, Clone, PartialEq)]
pub struct CoreReplayChunk {
    pub from_ms: i64,
    pub to_ms: Option<i64>,
    pub lines: Vec<CoreReplayLine>,
}

/// Whether the online replay is allowed: full build, the config turns
/// `twitch.chat_replay` on and has the query hash. No network.
#[frb(sync)]
pub fn chat_replay_online_allowed(config: &CoreRemoteConfig) -> bool {
    #[cfg(pa_twitch_native)]
    {
        let (cfg, ctx) = config.config_and_context();
        pa_twitch::chat_replay::ChatReplayGql::resolve(&cfg, &ctx).is_on()
    }
    #[cfg(not(pa_twitch_native))]
    {
        let _ = config;
        false
    }
}

/// A chat file next to the media file `media_path` (our `.chat.json.gz`
/// sidecar, TwitchDownloader's `.json`), if there is one.
#[frb(sync)]
pub fn find_chat_replay_file(media_path: String) -> Option<String> {
    #[cfg(pa_twitch_native)]
    {
        pa_twitch::chat_replay::td::sidecar_candidates(std::path::Path::new(&media_path))
            .into_iter()
            .find(|p| p.is_file())
            .map(|p| p.to_string_lossy().into_owned())
    }
    #[cfg(not(pa_twitch_native))]
    {
        let _ = media_path;
        None
    }
}

/// The chat of one VOD. [`Self::close`] it when the player goes away.
#[frb(opaque)]
pub struct CoreChatReplay {
    #[cfg(pa_twitch_native)]
    inner: std::sync::Arc<native::Replay>,
}

#[cfg(not(pa_twitch_native))]
fn store() -> CoreChatError {
    CoreChatError::new("store_build", "VOD chat replay is not in this build")
}

impl CoreChatReplay {
    /// The online replay of `video_id` on `channel` (login, for the
    /// header) with broadcaster `channel_id` (third-party emotes; learned
    /// from the answers when `None`). Fails with `flag_off`,
    /// `missing_query_hash`, `invalid` or `store_build`. No network yet.
    #[frb(sync)]
    pub fn online(
        config: &CoreRemoteConfig,
        video_id: String,
        channel: String,
        channel_id: Option<String>,
        options: CoreChatOptions,
    ) -> Result<CoreChatReplay, CoreChatError> {
        #[cfg(pa_twitch_native)]
        {
            let (cfg, ctx) = config.config_and_context();
            let gql = pa_twitch::chat_replay::ChatReplayGql::resolve(&cfg, &ctx);
            let inner = native::Replay::online(&gql, &video_id, channel, channel_id, options)?;
            Ok(Self {
                inner: std::sync::Arc::new(inner),
            })
        }
        #[cfg(not(pa_twitch_native))]
        {
            let _ = (config, video_id, channel, channel_id, options);
            Err(store())
        }
    }

    /// The replay of a TwitchDownloader chat file (`.json`, `.json.gz`).
    /// Fails with `io` or `decode`.
    pub fn open_file(
        path: String,
        options: CoreChatOptions,
    ) -> Result<CoreChatReplay, CoreChatError> {
        #[cfg(pa_twitch_native)]
        {
            Ok(Self {
                inner: std::sync::Arc::new(native::Replay::file(&path, options)?),
            })
        }
        #[cfg(not(pa_twitch_native))]
        {
            let _ = (path, options);
            Err(store())
        }
    }

    /// Channel login (or name) for the header.
    #[frb(sync)]
    pub fn channel(&self) -> String {
        #[cfg(pa_twitch_native)]
        {
            self.inner.channel()
        }
        #[cfg(not(pa_twitch_native))]
        {
            String::new()
        }
    }

    /// The lines around `offset_ms`: from `lookback_ms` before it to the
    /// end of what is cached there. Online, fetches the pages that are
    /// missing (a seek, or Dart's prefetch ahead of the playhead). Fails
    /// with `network`, `gql_hash_invalid`, `integrity_required`,
    /// `not_found`, `gql_error`, `decode` or `closed`.
    pub fn chunk_at(
        &self,
        offset_ms: i64,
        lookback_ms: i64,
    ) -> Result<CoreReplayChunk, CoreChatError> {
        #[cfg(pa_twitch_native)]
        {
            self.inner.chunk_at(offset_ms, lookback_ms)
        }
        #[cfg(not(pa_twitch_native))]
        {
            let _ = (offset_ms, lookback_ms);
            Err(store())
        }
    }

    /// Badges and channel emotes from Helix when the user is signed in
    /// (the chat looks like the live chat); without a sign-in badges stay
    /// marks drawn by the app. Lines rendered from now on use them.
    pub fn load_channel_assets(&self, twitch: &CoreTwitch) {
        #[cfg(pa_twitch_native)]
        {
            if let Ok(access) = twitch.chat_access() {
                self.inner.load_channel_assets(&access);
            }
        }
        #[cfg(not(pa_twitch_native))]
        {
            let _ = twitch;
        }
    }

    /// Viewer filters (TWI-024) for lines rendered from now on. Returns
    /// the problems of word rules that were skipped.
    #[frb(sync)]
    pub fn set_filters(&self, filters: CoreChatFilters) -> Vec<String> {
        #[cfg(pa_twitch_native)]
        {
            self.inner.set_filters(&filters)
        }
        #[cfg(not(pa_twitch_native))]
        {
            let _ = filters;
            Vec::new()
        }
    }

    /// 7TV, BetterTTV and FFZ emotes on or off (TWI-023).
    #[frb(sync)]
    pub fn set_third_party_emotes(&self, enabled: bool) {
        #[cfg(pa_twitch_native)]
        {
            self.inner.set_third_party(enabled);
        }
        #[cfg(not(pa_twitch_native))]
        {
            let _ = enabled;
        }
    }

    /// Images of badges and emotes as they land in the chat cache.
    pub fn images(&self, sink: StreamSink<Vec<CoreChatImage>>) -> Result<(), CoreChatError> {
        #[cfg(pa_twitch_native)]
        {
            self.inner.set_image_sink(sink);
            Ok(())
        }
        #[cfg(not(pa_twitch_native))]
        {
            let _ = sink;
            Err(store())
        }
    }

    /// Stops downloads and the emote watcher; the image stream ends.
    #[frb(sync)]
    pub fn close(&self) {
        #[cfg(pa_twitch_native)]
        {
            self.inner.close();
        }
    }
}

#[cfg(pa_twitch_native)]
mod native {
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use pa_imagecache::UreqFetcher;
    use pa_twitch::chat_replay::{
        ChatReplayError, ChatReplayFile, ChatReplayGql, Comment, OnlineReplay, ReplayTimeline,
    };
    use pa_twitch_chat as tc;

    use super::super::twitch_chat::{
        ChatAccess, CoreChatError, CoreChatFilters, CoreChatImage, CoreChatMessage,
        CoreChatOptions, USER_AGENT, image_prefs, lock, message as core_message,
    };
    use super::{CoreReplayChunk, CoreReplayLine};
    use crate::frb_generated::StreamSink;

    /// At most this many lines in one chunk; the rest comes with the next.
    const MAX_CHUNK_LINES: usize = 3000;
    /// Image download threads.
    const IMAGE_WORKERS: usize = 2;

    enum Source {
        Online {
            replay: OnlineReplay,
            rt: tokio::runtime::Runtime,
        },
        File(ReplayTimeline),
    }

    type Sink = Arc<Mutex<Option<StreamSink<Vec<CoreChatImage>>>>>;

    pub(super) struct Replay {
        source: Source,
        channel: String,
        channel_id: Mutex<Option<String>>,
        options: CoreChatOptions,
        prefs: tc::ImagePrefs,
        filters: Mutex<Arc<tc::ChatFilters>>,
        /// Twitch catalogs (Helix when signed in, else CDN by id).
        base: Mutex<Arc<tc::ChannelAssets>>,
        third_party: Arc<Mutex<Option<Arc<tc::ThirdPartyEmotes>>>>,
        third_party_on: Mutex<bool>,
        watcher: Mutex<Option<tc::ThirdPartyWatcher>>,
        /// Rendered lines by comment id; `None`: hidden by a filter.
        /// Cleared when filters or emotes change.
        rendered: Arc<Mutex<HashMap<String, Option<CoreChatMessage>>>>,
        images: Mutex<Option<mpsc::Sender<tc::ImageRequest>>>,
        requested: Mutex<HashSet<String>>,
        sink: Sink,
        /// Images that landed before Dart listened.
        early: Arc<Mutex<Vec<CoreChatImage>>>,
        closed: Mutex<bool>,
    }

    fn error(e: &ChatReplayError) -> CoreChatError {
        CoreChatError::new(e.code(), e)
    }

    impl Replay {
        pub(super) fn online(
            gql: &ChatReplayGql,
            video_id: &str,
            channel: String,
            channel_id: Option<String>,
            options: CoreChatOptions,
        ) -> Result<Self, CoreChatError> {
            let transport = pa_twitch::ReqwestTransport::new(Duration::from_secs(15))
                .map_err(|e| CoreChatError::new("io", e))?;
            let replay =
                OnlineReplay::new(gql, video_id, Arc::new(transport)).map_err(|e| match e {
                    ChatReplayError::Decode(m) => CoreChatError::new("invalid", m),
                    other => error(&other),
                })?;
            // Several Dart calls (a seek and a prefetch) may wait on it at
            // once: a small multi-thread runtime, not a current-thread one.
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("twitch-chat-replay")
                .enable_all()
                .build()
                .map_err(|e| CoreChatError::new("io", e))?;
            Ok(Self::with(
                Source::Online { replay, rt },
                channel,
                channel_id.filter(|id| !id.is_empty()),
                options,
            ))
        }

        pub(super) fn file(path: &str, options: CoreChatOptions) -> Result<Self, CoreChatError> {
            let bytes = std::fs::read(path).map_err(|e| CoreChatError::new("io", e))?;
            let file =
                ChatReplayFile::parse(&bytes).map_err(|e| CoreChatError::new("decode", e))?;
            let channel = file.channel_name();
            let channel_id = file.channel_id();
            let timeline = ReplayTimeline::from_comments(file.comments);
            Ok(Self::with(
                Source::File(timeline),
                channel,
                channel_id,
                options,
            ))
        }

        fn with(
            source: Source,
            channel: String,
            channel_id: Option<String>,
            options: CoreChatOptions,
        ) -> Self {
            let prefs = image_prefs(&options);
            let third_party_on = options.third_party_emotes;
            let replay = Self {
                source,
                channel,
                channel_id: Mutex::new(channel_id),
                prefs,
                filters: Mutex::new(Arc::new(tc::ChatFilters::default())),
                base: Mutex::new(Arc::new(tc::ChannelAssets::cdn_only())),
                third_party: Arc::new(Mutex::new(None)),
                third_party_on: Mutex::new(third_party_on),
                watcher: Mutex::new(None),
                rendered: Arc::new(Mutex::new(HashMap::new())),
                images: Mutex::new(None),
                requested: Mutex::new(HashSet::new()),
                sink: Arc::new(Mutex::new(None)),
                early: Arc::new(Mutex::new(Vec::new())),
                closed: Mutex::new(false),
                options,
            };
            replay.start_images();
            replay
        }

        pub(super) fn channel(&self) -> String {
            self.channel.clone()
        }

        /// Image downloads on a few threads, each key once.
        fn start_images(&self) {
            let cache = PathBuf::from(&self.options.cache_dir).join("twitch-chat");
            let Ok(images) = tc::ChatImages::open(
                cache,
                tc::DEFAULT_CHAT_CACHE_BYTES,
                Arc::new(UreqFetcher::new(USER_AGENT)),
            ) else {
                return;
            };
            let images = Arc::new(images);
            let (tx, rx) = mpsc::channel::<tc::ImageRequest>();
            let rx = Arc::new(Mutex::new(rx));
            for i in 0..IMAGE_WORKERS {
                let (rx, images, sink, early) = (
                    Arc::clone(&rx),
                    Arc::clone(&images),
                    Arc::clone(&self.sink),
                    Arc::clone(&self.early),
                );
                let spawned = thread::Builder::new()
                    .name(format!("twitch-replay-img-{i}"))
                    .spawn(move || {
                        loop {
                            let next = lock(&rx).recv();
                            let Ok(request) = next else { return };
                            let Ok(path) = images.get(&request) else {
                                continue;
                            };
                            let animated = request.animated.unwrap_or_else(|| is_gif(&path));
                            let ready = CoreChatImage {
                                key: request.key,
                                path: path.to_string_lossy().into_owned(),
                                animated,
                            };
                            let mut sink = lock(&sink);
                            match sink.as_ref() {
                                Some(s) => {
                                    if s.add(vec![ready]).is_err() {
                                        sink.take();
                                    }
                                }
                                None => lock(&early).push(ready),
                            }
                        }
                    });
                if spawned.is_err() {
                    return;
                }
            }
            *lock(&self.images) = Some(tx);
        }

        pub(super) fn set_image_sink(&self, sink: StreamSink<Vec<CoreChatImage>>) {
            let early: Vec<CoreChatImage> = std::mem::take(&mut *lock(&self.early));
            if !early.is_empty() {
                let _ = sink.add(early);
            }
            *lock(&self.sink) = Some(sink);
            // The third-party sets load once someone looks.
            if *lock(&self.third_party_on) {
                self.start_third_party();
            }
        }

        pub(super) fn close(&self) {
            *lock(&self.closed) = true;
            lock(&self.watcher).take();
            lock(&self.images).take();
            lock(&self.sink).take();
        }

        pub(super) fn set_filters(&self, filters: &CoreChatFilters) -> Vec<String> {
            let (compiled, errors) = tc::ChatFilters::new(&filters.settings());
            *lock(&self.filters) = Arc::new(compiled);
            lock(&self.rendered).clear();
            errors
                .into_iter()
                .map(|e| format!("{}: {}", e.index, e.message))
                .collect()
        }

        pub(super) fn set_third_party(&self, enabled: bool) {
            {
                let mut on = lock(&self.third_party_on);
                if *on == enabled {
                    return;
                }
                *on = enabled;
            }
            if enabled {
                self.start_third_party();
            } else {
                lock(&self.watcher).take();
                lock(&self.third_party).take();
                lock(&self.rendered).clear();
            }
        }

        pub(super) fn load_channel_assets(&self, access: &ChatAccess) {
            let id = lock(&self.channel_id)
                .clone()
                .or_else(|| self.learned_channel_id());
            let Some(id) = id else { return };
            let store = tc::AssetStore::new(Arc::clone(&access.emotes));
            let fetched = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()
                .and_then(|rt| rt.block_on(store.channel(&id)).ok());
            if let Some(assets) = fetched {
                *lock(&self.base) = assets;
                lock(&self.rendered).clear();
            }
        }

        fn learned_channel_id(&self) -> Option<String> {
            match &self.source {
                Source::Online { replay, .. } => replay.channel_id(),
                Source::File(_) => None,
            }
        }

        /// 7TV, BTTV and FFZ sets of the channel (once its id is known).
        fn start_third_party(&self) {
            if lock(&self.watcher).is_some() {
                return;
            }
            let Some(id) = lock(&self.channel_id)
                .clone()
                .or_else(|| self.learned_channel_id())
            else {
                return;
            };
            let Ok(transport) = pa_twitch::ReqwestTransport::new(Duration::from_secs(20)) else {
                return;
            };
            let cache = PathBuf::from(&self.options.cache_dir).join("twitch-emote-sets");
            let client = Arc::new(tc::EmoteSetsClient::new(
                Arc::new(transport),
                (!self.options.cache_dir.is_empty()).then_some(cache),
                USER_AGENT,
            ));
            let (slot, rendered) = (Arc::clone(&self.third_party), Arc::clone(&self.rendered));
            // A VOD needs no live updates of the sets: no EventAPI.
            let config = tc::ThirdPartyConfig {
                seventv_events: false,
                ..tc::ThirdPartyConfig::default()
            };
            let on_update = move |tp: Arc<tc::ThirdPartyEmotes>| {
                *lock(&slot) = Some(tp);
                // Lines are rendered again with the new emotes.
                lock(&rendered).clear();
            };
            if let Ok(w) = tc::ThirdPartyWatcher::start(client, config, Some(id), None, on_update) {
                *lock(&self.watcher) = Some(w);
            }
        }

        pub(super) fn chunk_at(
            &self,
            offset_ms: i64,
            lookback_ms: i64,
        ) -> Result<CoreReplayChunk, CoreChatError> {
            if *lock(&self.closed) {
                return Err(CoreChatError::new("closed", "the replay is closed"));
            }
            let at = u64::try_from(offset_ms).unwrap_or(0);
            let lookback = u64::try_from(lookback_ms).unwrap_or(0);
            let start = at.saturating_sub(lookback);
            match &self.source {
                Source::File(timeline) => Ok(self.chunk(timeline, start, 0, None)),
                Source::Online { replay, rt } => {
                    let coverage = rt.block_on(replay.ensure(at)).map_err(|e| error(&e))?;
                    if lock(&self.channel_id).is_none()
                        && let Some(id) = replay.channel_id()
                    {
                        *lock(&self.channel_id) = Some(id);
                        if *lock(&self.third_party_on) && lock(&self.sink).is_some() {
                            self.start_third_party();
                        }
                    }
                    let from = coverage.from_ms.max(start.min(at));
                    Ok(replay
                        .with_timeline(|t| self.chunk(t, from, coverage.from_ms, coverage.to_ms)))
                }
            }
        }

        /// Lines shown in `[from, to)` (`to` `None`: to the end), at most
        /// [`MAX_CHUNK_LINES`].
        fn chunk(
            &self,
            timeline: &ReplayTimeline,
            from: u64,
            covered_from: u64,
            to: Option<u64>,
        ) -> CoreReplayChunk {
            let from = from.max(covered_from);
            let start = if from == 0 {
                0
            } else {
                timeline.count_until(from - 1)
            };
            let end = to.map_or(timeline.len(), |t| {
                if t == 0 {
                    0
                } else {
                    timeline.count_until(t - 1)
                }
            });
            let items = &timeline.items()[start..end.max(start)];
            let (items, to_ms) = if items.len() > MAX_CHUNK_LINES {
                let cut = &items[..MAX_CHUNK_LINES];
                (cut, Some(items[MAX_CHUNK_LINES].at_ms))
            } else {
                (items, to)
            };
            let lines = items
                .iter()
                .filter_map(|t| {
                    self.render(&t.comment).map(|message| CoreReplayLine {
                        offset_ms: i64::try_from(t.at_ms).unwrap_or(i64::MAX),
                        message,
                    })
                })
                .collect();
            CoreReplayChunk {
                from_ms: i64::try_from(from).unwrap_or(i64::MAX),
                to_ms: to_ms.map(|t| i64::try_from(t).unwrap_or(i64::MAX)),
                lines,
            }
        }

        fn render(&self, comment: &Comment) -> Option<CoreChatMessage> {
            if let Some(done) = lock(&self.rendered).get(&comment.id) {
                return done.clone();
            }
            let channel_id = lock(&self.channel_id).clone().unwrap_or_default();
            let mut message = chat_message(comment, &channel_id, &self.channel);
            let filters = Arc::clone(&lock(&self.filters));
            let verdict = filters.apply(&mut message);
            let out = if verdict.hidden.is_some() {
                None
            } else {
                let base = Arc::clone(&lock(&self.base));
                let assets = match lock(&self.third_party).clone() {
                    Some(tp) => Arc::new(base.with_third_party(Some(tp))),
                    None => base,
                };
                let rendered = assets.render(&message);
                self.request_images(&rendered);
                Some(core_message(
                    tc::ChatLine {
                        message: Box::new(message),
                        rendered,
                        mention: verdict.mention,
                    },
                    &self.prefs,
                ))
            };
            if !comment.id.is_empty() {
                lock(&self.rendered).insert(comment.id.clone(), out.clone());
            }
            out
        }

        fn request_images(&self, rendered: &tc::RenderedMessage) {
            let images = lock(&self.images);
            let Some(tx) = images.as_ref() else { return };
            let mut requested = lock(&self.requested);
            for r in tc::image_requests([rendered], &self.prefs) {
                if requested.insert(r.key.clone()) {
                    let _ = tx.send(r);
                }
            }
        }
    }

    fn is_gif(path: &std::path::Path) -> bool {
        use std::io::Read;
        let mut head = [0u8; 12];
        std::fs::File::open(path)
            .and_then(|mut f| f.read(&mut head))
            .map(|n| pa_imagecache::sniff_image(&head[..n]) == Some("gif"))
            .unwrap_or(false)
    }

    /// A TwitchDownloader comment as a chat message of the live chat model:
    /// emote fragments become character ranges, badges set and version.
    pub(super) fn chat_message(c: &Comment, channel_id: &str, channel: &str) -> tc::ChatMessage {
        let mut text = String::new();
        let mut emotes = Vec::new();
        let mut at = 0usize;
        for f in &c.message.fragments {
            let n = f.text.chars().count();
            if let Some(e) = f.emoticon.as_ref().filter(|e| !e.emoticon_id.is_empty())
                && n > 0
            {
                emotes.push(tc::EmoteRange {
                    id: e.emoticon_id.clone(),
                    start: at,
                    end: at + n,
                });
            }
            text.push_str(&f.text);
            at += n;
        }
        if c.message.fragments.is_empty() {
            text.clone_from(&c.message.body);
        }
        let (text, is_action) = match text
            .strip_prefix("\u{1}ACTION ")
            .and_then(|t| t.strip_suffix('\u{1}'))
        {
            Some(inner) => (inner.to_owned(), true),
            None => (text, false),
        };
        tc::ChatMessage {
            id: c.id.clone(),
            ts_ms: c
                .created_at
                .as_deref()
                .and_then(pa_twitch::parse_rfc3339_ms)
                .unwrap_or(0),
            channel: tc::ChannelRef {
                id: c
                    .channel_id
                    .clone()
                    .unwrap_or_else(|| channel_id.to_owned()),
                login: channel.to_lowercase(),
            },
            user: tc::ChatUser {
                id: c.commenter.id.clone(),
                login: c.commenter.name.to_lowercase(),
                display_name: c.commenter.display_name.clone(),
                color: c.message.user_color.clone().filter(|s| !s.is_empty()),
                badges: c
                    .message
                    .user_badges
                    .iter()
                    .filter(|b| !b.id.is_empty())
                    .map(|b| tc::Badge {
                        set_id: b.id.clone(),
                        version: b.version.clone(),
                        info: None,
                    })
                    .collect(),
            },
            text,
            emotes: if is_action { Vec::new() } else { emotes },
            is_action,
            bits: c.message.bits_spent.filter(|b| *b > 0),
            ..tc::ChatMessage::default()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn options(dir: &std::path::Path) -> CoreChatOptions {
            CoreChatOptions {
                cache_dir: dir.to_string_lossy().into_owned(),
                device_pixel_ratio: 1.0,
                dark: true,
                animate: false,
                flush_ms: 0,
                third_party_emotes: false,
            }
        }

        const FILE: &str = r##"{"streamer":{"name":"Fake_Streamer","id":7001},"comments":[
          {"_id":"a","content_offset_seconds":1.0,"commenter":{"_id":"9","name":"Viewer_One","display_name":"Viewer_One"},
           "message":{"body":"привіт Kappa","fragments":[{"text":"привіт ","emoticon":null},{"text":"Kappa","emoticon":{"emoticon_id":"25"}}],
                      "user_badges":[{"_id":"subscriber","version":"12"}],"user_color":"#1E90FF"}},
          {"_id":"b","content_offset_seconds":2.0,"state":"deleted","commenter":{"_id":"8","name":"troll"},"message":{"body":"x"}},
          {"_id":"c","content_offset_seconds":3.0,"commenter":{"_id":"7","name":"blocked_one"},"message":{"body":"spam","fragments":[{"text":"spam"}]}},
          {"_id":"d","content_offset_seconds":4.0,"commenter":{"_id":"6","name":"me"},"message":{"body":"\u0001ACTION waves\u0001"}}
        ]}"##;

        #[test]
        fn fragments_become_emote_ranges_in_characters() {
            let file = ChatReplayFile::parse(FILE.as_bytes()).unwrap();
            let m = chat_message(&file.comments[0], "7001", "fake_streamer");
            assert_eq!(m.text, "привіт Kappa");
            assert_eq!(
                m.emotes,
                [tc::EmoteRange {
                    id: "25".into(),
                    start: 7,
                    end: 12
                }]
            );
            assert_eq!(m.user.login, "viewer_one");
            assert_eq!(m.user.badges[0].set_id, "subscriber");
            assert_eq!(m.user.color.as_deref(), Some("#1E90FF"));
            assert_eq!(m.channel.id, "7001");
            let action = chat_message(&file.comments[3], "7001", "fake_streamer");
            assert!(action.is_action);
            assert_eq!(action.text, "waves");
        }

        #[test]
        fn a_file_replays_rendered_without_deleted_or_filtered_lines() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("1000000001.chat.json");
            std::fs::write(&path, FILE).unwrap();
            let media = dir.path().join("1000000001.mkv");
            assert_eq!(
                super::super::find_chat_replay_file(media.to_string_lossy().into_owned()),
                Some(path.to_string_lossy().into_owned())
            );
            let r = Replay::file(&path.to_string_lossy(), options(dir.path())).unwrap();
            assert_eq!(r.channel(), "Fake_Streamer");
            let errors = r.set_filters(&CoreChatFilters {
                blocked: vec!["blocked_one".into()],
                words: vec![],
                own_login: None,
                highlight_keywords: vec![],
                hide_bots: false,
                extra_bots: vec![],
            });
            assert!(errors.is_empty());
            let chunk = r.chunk_at(10_000, 60_000).unwrap();
            assert_eq!((chunk.from_ms, chunk.to_ms), (0, None));
            let ids: Vec<&str> = chunk.lines.iter().map(|l| l.message.id.as_str()).collect();
            assert_eq!(
                ids,
                ["a", "d"],
                "deleted and blocked lines never reach Dart"
            );
            assert_eq!(chunk.lines[0].offset_ms, 1000);
            let first = &chunk.lines[0].message;
            assert_eq!(first.fragments.len(), 2);
            assert_eq!(first.fragments[1].text, "Kappa");
            assert!(first.fragments[1].emote_key.is_some(), "the Twitch emote");
            let later = r.chunk_at(3_500, 0).unwrap();
            assert_eq!(later.from_ms, 3_500);
            assert_eq!(later.lines.len(), 1);
            r.close();
            assert_eq!(r.chunk_at(0, 0).unwrap_err().code, "closed");
        }
    }
}
