// SPDX-License-Identifier: MPL-2.0
//! Third-party emotes: 7TV, BetterTTV and FrankerFaceZ (TWI-023, research
//! 06 §3.2; terms in docs/legal/providers.md §2.4, decision in the ADR
//! "third-party chat emotes").
//!
//! - **Sans-IO parsers** ([`parse`]) turn the providers' JSON into
//!   [`EmoteSet`]s of shared [`EmoteAsset`]s; tested on recorded answers.
//! - [`ThirdPartyEmotes::merge`] makes one immutable code → emote map with
//!   a fixed precedence: channel sets over global ones; within a scope 7TV
//!   over BetterTTV over FFZ. Twitch's own emotes (ranges from the server)
//!   always win: the renderer only looks words of the remaining text up.
//! - [`EmoteSetsClient`] fetches the sets straight from the providers over
//!   an injected [`Transport`] (no servers of ours, no tokens: requests
//!   carry nothing but a `User-Agent`) and keeps the raw answers on disk
//!   for at most [`SET_CACHE_MAX_AGE`], so a chat opens with its emotes
//!   before the network answers.
//! - [`ThirdPartyWatcher`] keeps a channel's sets current on its own
//!   thread: cache first, then the network, then BTTV and FFZ every
//!   [`ThirdPartyConfig::channel_refresh`] and 7TV when its EventAPI says
//!   the channel's set changed (a periodic re-fetch when the socket is
//!   down). Each change is handed over as a new [`ThirdPartyEmotes`].

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};

use pa_twitch::{Clock, HttpRequest, Method, SystemClock, Transport};
use serde::Deserialize;
use tokio::sync::{Notify, mpsc};

use crate::assets::{EmoteAsset, EmoteProvider};
use crate::ws::WsConnector;

/// 7TV REST API (v3).
pub const SEVENTV_API: &str = "https://7tv.io/v3";
/// 7TV EventAPI (v3): pushes emote set changes.
pub const SEVENTV_EVENTS_URL: &str = "wss://events.7tv.io/v3";
/// BetterTTV API (v3, cached endpoints).
pub const BTTV_API: &str = "https://api.betterttv.net/3";
/// FrankerFaceZ API (v1).
pub const FFZ_API: &str = "https://api.frankerfacez.com/v1";

/// BetterTTV emotes drawn over the previous emote. BTTV's API has no flag
/// for them; its extension and other chat clients keep the same list.
pub const BTTV_OVERLAYS: &[&str] = &[
    "cvMask",
    "cvHazmat",
    "SoSnowy",
    "IceCold",
    "SantaHat",
    "TopHat",
    "ReinDeer",
    "CandyCane",
];

/// Oldest emote list read from the disk cache (the same 24 hours as the
/// Twitch data of the chat).
pub const SET_CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Largest answer accepted (a big 7TV channel is about 2.5 MB).
const MAX_BODY: usize = 16 * 1024 * 1024;

/// The three providers, in the order of [`ThirdPartyConfig::default`].
pub const THIRD_PARTY_PROVIDERS: [EmoteProvider; 3] = [
    EmoteProvider::SevenTv,
    EmoteProvider::Bttv,
    EmoteProvider::Ffz,
];

/// Global set of a provider or a channel's set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SetScope {
    /// Everyone sees it.
    Global,
    /// The channel's own emotes.
    Channel,
}

/// Emotes of one provider and scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmoteSet {
    /// Provider.
    pub provider: EmoteProvider,
    /// Global or channel.
    pub scope: SetScope,
    /// Emotes in the provider's order.
    pub emotes: Vec<Arc<EmoteAsset>>,
    /// 7TV: id of the emote set (EventAPI subscription).
    pub set_id: Option<String>,
    /// 7TV: id of the channel's 7TV user (EventAPI: active set changed).
    pub owner_id: Option<String>,
}

impl EmoteSet {
    /// No emotes (a channel without an account at the provider).
    pub fn empty(provider: EmoteProvider, scope: SetScope) -> Self {
        Self {
            provider,
            scope,
            emotes: Vec::new(),
            set_id: None,
            owner_id: None,
        }
    }
}

/// Why a provider call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// DNS, TLS, timeout.
    #[error("{0}: network error: {1}")]
    Network(&'static str, String),
    /// Non-2xx answer other than "no such channel".
    #[error("{0}: HTTP {1}")]
    Status(&'static str, u16),
    /// The answer is not what the provider documents.
    #[error("{0}: unexpected answer: {1}")]
    Parse(&'static str, String),
    /// A channel id that is not a Twitch user id.
    #[error("invalid channel id")]
    InvalidChannel,
}

/// Merged third-party emotes of a channel: code → emote.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ThirdPartyEmotes {
    by_code: HashMap<String, Arc<EmoteAsset>>,
    /// Keys of emotes that came from a channel set.
    channel: HashSet<String>,
    /// Longest code: longer words are not looked up.
    max_code_len: usize,
}

fn rank(provider: EmoteProvider) -> u8 {
    match provider {
        EmoteProvider::Ffz => 0,
        EmoteProvider::Bttv => 1,
        EmoteProvider::SevenTv => 2,
        EmoteProvider::Twitch => 3,
    }
}

impl ThirdPartyEmotes {
    /// Merges sets: channel over global, 7TV over BTTV over FFZ.
    pub fn merge<'a>(sets: impl IntoIterator<Item = &'a EmoteSet>) -> Self {
        let mut order: Vec<&EmoteSet> = sets.into_iter().collect();
        order.sort_by_key(|s| (s.scope, rank(s.provider)));
        let mut out = Self::default();
        for set in order {
            for e in &set.emotes {
                if e.code.is_empty() || e.code.contains(char::is_whitespace) {
                    continue;
                }
                if let Some(old) = out.by_code.insert(e.code.clone(), Arc::clone(e)) {
                    out.channel.remove(&old.key());
                }
                if set.scope == SetScope::Channel {
                    out.channel.insert(e.key());
                }
                out.max_code_len = out.max_code_len.max(e.code.len());
            }
        }
        out
    }

    /// The emote whose code is exactly `word`.
    #[inline]
    pub fn get(&self, word: &str) -> Option<&Arc<EmoteAsset>> {
        if word.is_empty() || word.len() > self.max_code_len {
            return None;
        }
        self.by_code.get(word)
    }

    /// All emotes (no order).
    pub fn all(&self) -> impl Iterator<Item = &Arc<EmoteAsset>> {
        self.by_code.values()
    }

    /// The emote came from a channel set.
    pub fn is_channel(&self, emote: &EmoteAsset) -> bool {
        self.channel.contains(&emote.key())
    }

    /// Number of distinct codes.
    pub fn len(&self) -> usize {
        self.by_code.len()
    }

    /// No emotes.
    pub fn is_empty(&self) -> bool {
        self.by_code.is_empty()
    }
}

/// Parsers of the providers' answers (no I/O).
pub mod parse {
    use super::*;

    fn bad(provider: &'static str, e: serde_json::Error) -> ProviderError {
        ProviderError::Parse(provider, e.to_string())
    }

    // --- 7TV ---------------------------------------------------------------

    #[derive(Deserialize)]
    struct StvUser {
        emote_set: Option<StvSet>,
        user: Option<StvId>,
    }

    #[derive(Deserialize)]
    struct StvId {
        id: String,
    }

    #[derive(Deserialize)]
    struct StvSet {
        id: String,
        #[serde(default)]
        emotes: Option<Vec<StvActive>>,
    }

    #[derive(Deserialize)]
    struct StvActive {
        id: String,
        name: String,
        #[serde(default)]
        flags: u32,
        data: Option<StvData>,
    }

    #[derive(Deserialize)]
    struct StvData {
        #[serde(default)]
        animated: bool,
        #[serde(default)]
        flags: u32,
        host: Option<StvHost>,
    }

    #[derive(Deserialize)]
    struct StvHost {
        url: String,
        #[serde(default)]
        files: Vec<StvFile>,
    }

    #[derive(Deserialize)]
    struct StvFile {
        name: String,
        #[serde(default)]
        static_name: Option<String>,
        #[serde(default)]
        format: String,
    }

    /// Active emote flag: zero-width.
    const STV_ACTIVE_ZERO_WIDTH: u32 = 1;
    /// Emote data flag: zero-width.
    const STV_DATA_ZERO_WIDTH: u32 = 1 << 8;

    fn seventv_emote(a: &StvActive) -> Option<EmoteAsset> {
        let data = a.data.as_ref()?;
        let host = data.host.as_ref()?;
        let base = if host.url.starts_with("//") {
            format!("https:{}", host.url)
        } else {
            host.url.clone()
        };
        if !base.starts_with("https://") {
            return None;
        }
        // WEBP (animated and still) is what 7TV serves to its own clients;
        // GIF/PNG only when an old emote has no WEBP.
        let format = ["WEBP", "GIF", "PNG"]
            .into_iter()
            .find(|f| host.files.iter().any(|x| x.format == *f))?;
        let mut still: [String; 3] = Default::default();
        let mut animated: [String; 3] = Default::default();
        for f in host.files.iter().filter(|f| f.format == format) {
            let slot = match f.name.split('.').next() {
                Some("1x") => 0,
                Some("2x") => 1,
                Some("4x") => 2,
                _ => continue,
            };
            if data.animated {
                animated[slot] = format!("{base}/{}", f.name);
                if let Some(s) = &f.static_name {
                    still[slot] = format!("{base}/{s}");
                }
            } else {
                still[slot] = format!("{base}/{}", f.name);
            }
        }
        if still.iter().chain(&animated).all(String::is_empty) {
            return None;
        }
        let zero_width =
            a.flags & STV_ACTIVE_ZERO_WIDTH != 0 || data.flags & STV_DATA_ZERO_WIDTH != 0;
        Some(EmoteAsset::third_party(
            EmoteProvider::SevenTv,
            &a.id,
            &a.name,
            zero_width,
            still,
            animated,
        ))
    }

    fn seventv_set_of(set: StvSet, scope: SetScope, owner: Option<String>) -> EmoteSet {
        EmoteSet {
            provider: EmoteProvider::SevenTv,
            scope,
            emotes: set
                .emotes
                .unwrap_or_default()
                .iter()
                .filter_map(seventv_emote)
                .map(Arc::new)
                .collect(),
            set_id: Some(set.id),
            owner_id: owner,
        }
    }

    /// `GET /v3/emote-sets/{id}` (e.g. `global`).
    pub fn seventv_set(body: &[u8], scope: SetScope) -> Result<EmoteSet, ProviderError> {
        let set: StvSet = serde_json::from_slice(body).map_err(|e| bad("7TV", e))?;
        Ok(seventv_set_of(set, scope, None))
    }

    /// `GET /v3/users/twitch/{twitch_id}`: the channel's active set.
    pub fn seventv_user(body: &[u8]) -> Result<EmoteSet, ProviderError> {
        let user: StvUser = serde_json::from_slice(body).map_err(|e| bad("7TV", e))?;
        let owner = user.user.map(|u| u.id);
        Ok(match user.emote_set {
            Some(set) => seventv_set_of(set, SetScope::Channel, owner),
            None => EmoteSet {
                owner_id: owner,
                ..EmoteSet::empty(EmoteProvider::SevenTv, SetScope::Channel)
            },
        })
    }

    // --- BetterTTV ---------------------------------------------------------

    #[derive(Deserialize)]
    struct BttvEmote {
        id: String,
        code: String,
        #[serde(default)]
        animated: bool,
        /// Text modifiers (`h!`, `w!`…): they transform the next emote,
        /// which this chat does not do, so they stay text.
        #[serde(default)]
        modifier: bool,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct BttvUser {
        #[serde(default)]
        channel_emotes: Vec<BttvEmote>,
        #[serde(default)]
        shared_emotes: Vec<BttvEmote>,
    }

    fn bttv_emote(e: &BttvEmote) -> Option<EmoteAsset> {
        if e.modifier || e.id.is_empty() || !e.id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        let base = format!("https://cdn.betterttv.net/emote/{}", e.id);
        let scaled = |f: &dyn Fn(&str) -> String| [f("1x"), f("2x"), f("3x")];
        let (still, animated) = if e.animated {
            (
                scaled(&|s| format!("{base}/static/{s}.webp")),
                scaled(&|s| format!("{base}/{s}")),
            )
        } else {
            (scaled(&|s| format!("{base}/{s}")), Default::default())
        };
        Some(EmoteAsset::third_party(
            EmoteProvider::Bttv,
            &e.id,
            &e.code,
            BTTV_OVERLAYS.contains(&e.code.as_str()),
            still,
            animated,
        ))
    }

    fn bttv_set(emotes: &[BttvEmote], scope: SetScope) -> EmoteSet {
        EmoteSet {
            emotes: emotes.iter().filter_map(bttv_emote).map(Arc::new).collect(),
            ..EmoteSet::empty(EmoteProvider::Bttv, scope)
        }
    }

    /// `GET /3/cached/emotes/global`.
    pub fn bttv_global(body: &[u8]) -> Result<EmoteSet, ProviderError> {
        let list: Vec<BttvEmote> = serde_json::from_slice(body).map_err(|e| bad("BTTV", e))?;
        Ok(bttv_set(&list, SetScope::Global))
    }

    /// `GET /3/cached/users/twitch/{twitch_id}`: channel and shared emotes.
    pub fn bttv_user(body: &[u8]) -> Result<EmoteSet, ProviderError> {
        let user: BttvUser = serde_json::from_slice(body).map_err(|e| bad("BTTV", e))?;
        let mut set = bttv_set(&user.shared_emotes, SetScope::Channel);
        // Channel emotes override shared ones with the same code.
        set.emotes.extend(
            user.channel_emotes
                .iter()
                .filter_map(bttv_emote)
                .map(Arc::new),
        );
        Ok(set)
    }

    // --- FrankerFaceZ ------------------------------------------------------

    #[derive(Deserialize)]
    struct FfzSet {
        #[serde(default)]
        emoticons: Vec<FfzEmote>,
    }

    #[derive(Deserialize)]
    struct FfzEmote {
        id: u64,
        name: String,
        #[serde(default)]
        hidden: bool,
        /// Effect modifiers (`ffzSpin`, `ffzRainbow`…) restyle the previous
        /// emote; not supported, they stay text.
        #[serde(default)]
        modifier: bool,
        #[serde(default)]
        urls: HashMap<String, String>,
        #[serde(default)]
        animated: Option<HashMap<String, String>>,
    }

    #[derive(Deserialize)]
    struct FfzGlobal {
        #[serde(default)]
        default_sets: Vec<u64>,
        #[serde(default)]
        sets: HashMap<String, FfzSet>,
    }

    #[derive(Deserialize)]
    struct FfzRoom {
        #[serde(default)]
        sets: HashMap<String, FfzSet>,
    }

    fn ffz_urls(map: &HashMap<String, String>) -> [String; 3] {
        let get = |k: &str| {
            map.get(k)
                .filter(|u| u.starts_with("https://"))
                .cloned()
                .unwrap_or_default()
        };
        [get("1"), get("2"), get("4")]
    }

    fn ffz_emote(e: &FfzEmote) -> Option<EmoteAsset> {
        if e.hidden || e.modifier {
            return None;
        }
        let still = ffz_urls(&e.urls);
        let animated = e.animated.as_ref().map(ffz_urls).unwrap_or_default();
        if still.iter().chain(&animated).all(String::is_empty) {
            return None;
        }
        Some(EmoteAsset::third_party(
            EmoteProvider::Ffz,
            &e.id.to_string(),
            &e.name,
            false,
            still,
            animated,
        ))
    }

    fn ffz_set<'a>(sets: impl Iterator<Item = &'a FfzSet>, scope: SetScope) -> EmoteSet {
        EmoteSet {
            emotes: sets
                .flat_map(|s| s.emoticons.iter())
                .filter_map(ffz_emote)
                .map(Arc::new)
                .collect(),
            ..EmoteSet::empty(EmoteProvider::Ffz, scope)
        }
    }

    /// `GET /v1/set/global`: only the default sets (the others belong to
    /// particular users).
    pub fn ffz_global(body: &[u8]) -> Result<EmoteSet, ProviderError> {
        let g: FfzGlobal = serde_json::from_slice(body).map_err(|e| bad("FFZ", e))?;
        let mut ids = g.default_sets.clone();
        ids.sort_unstable();
        Ok(ffz_set(
            ids.iter().filter_map(|id| g.sets.get(&id.to_string())),
            SetScope::Global,
        ))
    }

    /// `GET /v1/room/id/{twitch_id}`.
    pub fn ffz_room(body: &[u8]) -> Result<EmoteSet, ProviderError> {
        let r: FfzRoom = serde_json::from_slice(body).map_err(|e| bad("FFZ", e))?;
        let mut keys: Vec<&String> = r.sets.keys().collect();
        keys.sort();
        Ok(ffz_set(
            keys.into_iter().filter_map(|k| r.sets.get(k)),
            SetScope::Channel,
        ))
    }

    /// Parses an answer of `provider` for `scope`.
    pub fn set(
        provider: EmoteProvider,
        scope: SetScope,
        body: &[u8],
    ) -> Result<EmoteSet, ProviderError> {
        match (provider, scope) {
            (EmoteProvider::SevenTv, SetScope::Global) => seventv_set(body, scope),
            (EmoteProvider::SevenTv, SetScope::Channel) => seventv_user(body),
            (EmoteProvider::Bttv, SetScope::Global) => bttv_global(body),
            (EmoteProvider::Bttv, SetScope::Channel) => bttv_user(body),
            (EmoteProvider::Ffz, SetScope::Global) => ffz_global(body),
            (EmoteProvider::Ffz, SetScope::Channel) => ffz_room(body),
            (EmoteProvider::Twitch, _) => Ok(EmoteSet::empty(provider, scope)),
        }
    }
}

fn provider_name(p: EmoteProvider) -> &'static str {
    match p {
        EmoteProvider::SevenTv => "7TV",
        EmoteProvider::Bttv => "BTTV",
        EmoteProvider::Ffz => "FFZ",
        EmoteProvider::Twitch => "Twitch",
    }
}

fn valid_channel(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

/// Fetches emote sets from the providers and caches the answers on disk.
pub struct EmoteSetsClient {
    transport: Arc<dyn Transport>,
    cache_dir: Option<PathBuf>,
    user_agent: String,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for EmoteSetsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmoteSetsClient")
            .field("cache_dir", &self.cache_dir)
            .finish_non_exhaustive()
    }
}

impl EmoteSetsClient {
    /// Client over `transport`; answers are cached in `cache_dir` when
    /// given (e.g. `<app cache>/twitch-emote-sets`).
    pub fn new(
        transport: Arc<dyn Transport>,
        cache_dir: Option<PathBuf>,
        user_agent: &str,
    ) -> Self {
        Self::with_clock(transport, cache_dir, user_agent, Arc::new(SystemClock))
    }

    /// [`Self::new`] with an injected clock (cache age in tests).
    pub fn with_clock(
        transport: Arc<dyn Transport>,
        cache_dir: Option<PathBuf>,
        user_agent: &str,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            transport,
            cache_dir,
            user_agent: user_agent.to_owned(),
            clock,
        }
    }

    /// API URL of a set; `channel_id` is the Twitch user id.
    pub fn url(provider: EmoteProvider, scope: SetScope, channel_id: &str) -> Option<String> {
        Some(match (provider, scope) {
            (EmoteProvider::SevenTv, SetScope::Global) => {
                format!("{SEVENTV_API}/emote-sets/global")
            }
            (EmoteProvider::SevenTv, SetScope::Channel) => {
                format!("{SEVENTV_API}/users/twitch/{channel_id}")
            }
            (EmoteProvider::Bttv, SetScope::Global) => format!("{BTTV_API}/cached/emotes/global"),
            (EmoteProvider::Bttv, SetScope::Channel) => {
                format!("{BTTV_API}/cached/users/twitch/{channel_id}")
            }
            (EmoteProvider::Ffz, SetScope::Global) => format!("{FFZ_API}/set/global"),
            (EmoteProvider::Ffz, SetScope::Channel) => format!("{FFZ_API}/room/id/{channel_id}"),
            (EmoteProvider::Twitch, _) => return None,
        })
    }

    fn cache_file(
        &self,
        provider: EmoteProvider,
        scope: SetScope,
        channel_id: &str,
    ) -> Option<PathBuf> {
        let dir = self.cache_dir.as_ref()?;
        Some(dir.join(match scope {
            SetScope::Global => format!("{}-global.json", provider.as_str()),
            SetScope::Channel => format!("{}-channel-{channel_id}.json", provider.as_str()),
        }))
    }

    /// The set from the disk cache if it is younger than
    /// [`SET_CACHE_MAX_AGE`]; never touches the network.
    pub fn cached(
        &self,
        provider: EmoteProvider,
        scope: SetScope,
        channel_id: &str,
    ) -> Option<EmoteSet> {
        if scope == SetScope::Channel && !valid_channel(channel_id) {
            return None;
        }
        let file = self.cache_file(provider, scope, channel_id)?;
        let modified = std::fs::metadata(&file).and_then(|m| m.modified()).ok()?;
        let age = self
            .clock
            .now()
            .duration_since(modified)
            .unwrap_or(Duration::ZERO);
        if age > SET_CACHE_MAX_AGE {
            let _ = std::fs::remove_file(&file);
            return None;
        }
        let body = std::fs::read(&file).ok()?;
        parse::set(provider, scope, &body).ok()
    }

    /// Fetches a set now. A channel the provider does not know (HTTP 404)
    /// is an empty set.
    pub async fn fetch(
        &self,
        provider: EmoteProvider,
        scope: SetScope,
        channel_id: &str,
    ) -> Result<EmoteSet, ProviderError> {
        if scope == SetScope::Channel && !valid_channel(channel_id) {
            return Err(ProviderError::InvalidChannel);
        }
        let Some(url) = Self::url(provider, scope, channel_id) else {
            return Ok(EmoteSet::empty(provider, scope));
        };
        let name = provider_name(provider);
        let response = self
            .transport
            .send(HttpRequest {
                method: Method::Get,
                url,
                headers: vec![
                    ("Accept".into(), "application/json".into()),
                    ("User-Agent".into(), self.user_agent.clone()),
                ],
                body: None,
            })
            .await
            .map_err(|e| ProviderError::Network(name, e.0))?;
        let body: &[u8] = match response.status {
            200..=299 => &response.body,
            404 if scope == SetScope::Channel => b"",
            s => return Err(ProviderError::Status(name, s)),
        };
        if body.len() > MAX_BODY {
            return Err(ProviderError::Parse(name, "answer too large".into()));
        }
        let set = if body.is_empty() {
            EmoteSet::empty(provider, scope)
        } else {
            parse::set(provider, scope, body)?
        };
        if let Some(file) = self.cache_file(provider, scope, channel_id) {
            let write = || -> std::io::Result<()> {
                if let Some(dir) = file.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                let tmp = file.with_extension("tmp");
                // An empty channel is cached as `{}`: the parsers read it
                // as "no emotes".
                let data: &[u8] = if body.is_empty() { b"{}" } else { body };
                std::fs::write(&tmp, data)?;
                std::fs::rename(&tmp, &file)
            };
            let _ = write();
        }
        Ok(set)
    }
}

/// What [`ThirdPartyWatcher`] loads and how often.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThirdPartyConfig {
    /// Providers to load (any of [`THIRD_PARTY_PROVIDERS`]).
    pub providers: Vec<EmoteProvider>,
    /// BTTV and FFZ channel sets (and 7TV while its EventAPI is down) are
    /// fetched again this often. Default 10 minutes.
    pub channel_refresh: Duration,
    /// Global sets are fetched again this often. Default 6 hours.
    pub global_refresh: Duration,
    /// Follow 7TV set changes through the EventAPI (needs a connector).
    pub seventv_events: bool,
    /// EventAPI URL (tests point it elsewhere).
    pub events_url: String,
    /// After a 7TV change event, wait this long for more before fetching
    /// (a streamer adding ten emotes causes one fetch). Default 3 s.
    pub event_debounce: Duration,
}

impl Default for ThirdPartyConfig {
    fn default() -> Self {
        Self {
            providers: THIRD_PARTY_PROVIDERS.to_vec(),
            channel_refresh: Duration::from_secs(10 * 60),
            global_refresh: Duration::from_secs(6 * 60 * 60),
            seventv_events: true,
            events_url: SEVENTV_EVENTS_URL.to_owned(),
            event_debounce: Duration::from_secs(3),
        }
    }
}

/// Keeps a channel's third-party emotes current on its own thread; stops
/// on drop.
#[derive(Debug)]
pub struct ThirdPartyWatcher {
    stop: Arc<Notify>,
    channel: mpsc::UnboundedSender<String>,
}

impl ThirdPartyWatcher {
    /// Starts watching. `channel_id` is the Twitch user id when known
    /// (signed-in chat); the anonymous chat learns it from its first
    /// message and passes it to [`Self::set_channel`]. Global sets load
    /// at once. `on_update` gets every new merged map (cache first, then
    /// the network, then changes).
    pub fn start<F>(
        client: Arc<EmoteSetsClient>,
        config: ThirdPartyConfig,
        channel_id: Option<String>,
        events: Option<Arc<dyn WsConnector>>,
        on_update: F,
    ) -> std::io::Result<Self>
    where
        F: FnMut(Arc<ThirdPartyEmotes>) + Send + 'static,
    {
        let stop = Arc::new(Notify::new());
        let (tx, rx) = mpsc::unbounded_channel();
        if let Some(id) = channel_id {
            let _ = tx.send(id);
        }
        let stop_rx = Arc::clone(&stop);
        thread::Builder::new()
            .name("twitch-emote-sets".into())
            .spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                let watch = Watch {
                    client,
                    config,
                    events,
                    on_update: Box::new(on_update),
                    sets: HashMap::new(),
                    channel_id: None,
                    last: None,
                };
                rt.block_on(watch.run(rx, stop_rx));
            })?;
        Ok(Self { stop, channel: tx })
    }

    /// The channel is now known (or changed): load its sets.
    pub fn set_channel(&self, channel_id: &str) {
        let _ = self.channel.send(channel_id.to_owned());
    }

    /// Stops the thread.
    pub fn stop(&self) {
        self.stop.notify_one();
    }
}

impl Drop for ThirdPartyWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

type UpdateFn = Box<dyn FnMut(Arc<ThirdPartyEmotes>) + Send>;

struct Watch {
    client: Arc<EmoteSetsClient>,
    config: ThirdPartyConfig,
    events: Option<Arc<dyn WsConnector>>,
    on_update: UpdateFn,
    sets: HashMap<(EmoteProvider, SetScope), EmoteSet>,
    channel_id: Option<String>,
    last: Option<Arc<ThirdPartyEmotes>>,
}

impl Watch {
    async fn run(mut self, mut channel_rx: mpsc::UnboundedReceiver<String>, stop: Arc<Notify>) {
        // A channel id that is already queued is taken before the first
        // emit, so the chat gets one map, not a global-only one first.
        if let Ok(id) = channel_rx.try_recv() {
            self.channel_id = valid_channel(&id).then_some(id);
        }
        self.load_cached();
        self.emit();
        self.fetch_scope(SetScope::Global).await;
        if self.channel_id.is_some() {
            self.fetch_scope(SetScope::Channel).await;
        }
        self.emit();

        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<()>();
        let mut events = self.start_events(&event_tx);
        let mut channel_tick = tokio::time::interval(self.config.channel_refresh);
        channel_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        channel_tick.reset();
        let mut global_tick = tokio::time::interval(self.config.global_refresh);
        global_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        global_tick.reset();
        loop {
            tokio::select! {
                biased;
                () = stop.notified() => break,
                id = channel_rx.recv() => {
                    let Some(id) = id else { break };
                    if !valid_channel(&id) || self.channel_id.as_deref() == Some(id.as_str()) {
                        continue;
                    }
                    self.channel_id = Some(id);
                    self.sets.retain(|(_, scope), _| *scope == SetScope::Global);
                    self.load_cached();
                    self.emit();
                    self.fetch_scope(SetScope::Channel).await;
                    self.emit();
                    if let Some(e) = events.take() { e.abort(); }
                    events = self.start_events(&event_tx);
                }
                Some(()) = event_rx.recv() => {
                    // Debounce: one fetch for a burst of changes.
                    tokio::time::sleep(self.config.event_debounce).await;
                    while event_rx.try_recv().is_ok() {}
                    let old_set = self.seventv_ids();
                    self.fetch_one(EmoteProvider::SevenTv, SetScope::Channel).await;
                    self.emit();
                    if self.seventv_ids() != old_set {
                        // The channel switched its active set.
                        if let Some(e) = events.take() { e.abort(); }
                        events = self.start_events(&event_tx);
                    }
                }
                _ = channel_tick.tick() => {
                    if self.channel_id.is_some() {
                        for p in self.config.providers.clone() {
                            let pushed = p == EmoteProvider::SevenTv
                                && events.as_ref().is_some_and(|e| !e.is_finished());
                            if !pushed {
                                self.fetch_one(p, SetScope::Channel).await;
                            }
                        }
                        self.emit();
                    }
                }
                _ = global_tick.tick() => {
                    self.fetch_scope(SetScope::Global).await;
                    self.emit();
                }
            }
        }
        if let Some(e) = events {
            e.abort();
        }
    }

    fn seventv_ids(&self) -> (Option<String>, Option<String>) {
        self.sets
            .get(&(EmoteProvider::SevenTv, SetScope::Channel))
            .map(|s| (s.set_id.clone(), s.owner_id.clone()))
            .unwrap_or_default()
    }

    fn start_events(
        &self,
        notify: &mpsc::UnboundedSender<()>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        if !self.config.seventv_events || !self.config.providers.contains(&EmoteProvider::SevenTv) {
            return None;
        }
        let connector = self.events.clone()?;
        let (set_id, owner_id) = self.seventv_ids();
        if set_id.is_none() && owner_id.is_none() {
            return None;
        }
        let url = self.config.events_url.clone();
        let notify = notify.clone();
        Some(tokio::spawn(seventv_events(
            connector, url, set_id, owner_id, notify,
        )))
    }

    fn load_cached(&mut self) {
        for p in self.config.providers.clone() {
            let key = (p, SetScope::Global);
            if !self.sets.contains_key(&key)
                && let Some(s) = self.client.cached(p, SetScope::Global, "")
            {
                self.sets.insert(key, s);
            }
            if let Some(id) = self.channel_id.clone() {
                let key = (p, SetScope::Channel);
                if !self.sets.contains_key(&key)
                    && let Some(s) = self.client.cached(p, SetScope::Channel, &id)
                {
                    self.sets.insert(key, s);
                }
            }
        }
    }

    async fn fetch_scope(&mut self, scope: SetScope) {
        for p in self.config.providers.clone() {
            self.fetch_one(p, scope).await;
        }
    }

    /// Fetches one set; on failure the previous one (cache) stays.
    async fn fetch_one(&mut self, provider: EmoteProvider, scope: SetScope) {
        let channel = match scope {
            SetScope::Global => String::new(),
            SetScope::Channel => match &self.channel_id {
                Some(id) => id.clone(),
                None => return,
            },
        };
        if let Ok(set) = self.client.fetch(provider, scope, &channel).await {
            self.sets.insert((provider, scope), set);
        }
    }

    /// Hands over a new map when it differs from the last one.
    fn emit(&mut self) {
        if self.sets.is_empty() && self.last.is_none() {
            return;
        }
        let merged = Arc::new(ThirdPartyEmotes::merge(self.sets.values()));
        if self.last.as_deref() == Some(&*merged) {
            return;
        }
        self.last = Some(Arc::clone(&merged));
        (self.on_update)(merged);
    }
}

// --- 7TV EventAPI -----------------------------------------------------------

#[derive(Deserialize)]
struct EventFrame {
    op: u8,
    #[serde(default)]
    d: serde_json::Value,
}

/// Op codes of the 7TV EventAPI v3.
mod op {
    pub const DISPATCH: u8 = 0;
    pub const HELLO: u8 = 1;
    pub const RECONNECT: u8 = 4;
    pub const END_OF_STREAM: u8 = 7;
    pub const SUBSCRIBE: u8 = 35;
}

/// Subscribe frames for a channel: changes of its set and of its user (a
/// new active set).
pub fn seventv_subscriptions(set_id: Option<&str>, owner_id: Option<&str>) -> Vec<String> {
    let sub = |kind: &str, id: &str| {
        serde_json::json!({
            "op": op::SUBSCRIBE,
            "d": { "type": kind, "condition": { "object_id": id } },
        })
        .to_string()
    };
    let mut out = Vec::new();
    if let Some(id) = set_id {
        out.push(sub("emote_set.update", id));
    }
    if let Some(id) = owner_id {
        out.push(sub("user.update", id));
    }
    out
}

/// What an EventAPI frame means for the watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SevenTvEvent {
    /// Hello: heartbeat interval.
    Hello(Duration),
    /// The set or the user changed: fetch again.
    Changed,
    /// The server asks to reconnect or ends the stream.
    Reconnect,
    /// Heartbeat, acknowledgement, anything else.
    Other,
}

/// Classifies one EventAPI text frame (sans-IO).
pub fn seventv_event(frame: &str) -> SevenTvEvent {
    let Ok(f) = serde_json::from_str::<EventFrame>(frame) else {
        return SevenTvEvent::Other;
    };
    match f.op {
        op::HELLO => SevenTvEvent::Hello(Duration::from_millis(
            f.d.get("heartbeat_interval")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(25_000),
        )),
        op::DISPATCH => match f.d.get("type").and_then(serde_json::Value::as_str) {
            Some("emote_set.update" | "user.update") => SevenTvEvent::Changed,
            _ => SevenTvEvent::Other,
        },
        op::RECONNECT | op::END_OF_STREAM => SevenTvEvent::Reconnect,
        _ => SevenTvEvent::Other,
    }
}

/// Listens to the EventAPI and sends `()` on every change; reconnects with
/// backoff. Ends when the watcher drops `notify` or aborts the task.
async fn seventv_events(
    connector: Arc<dyn WsConnector>,
    url: String,
    set_id: Option<String>,
    owner_id: Option<String>,
    notify: mpsc::UnboundedSender<()>,
) {
    let subs = seventv_subscriptions(set_id.as_deref(), owner_id.as_deref());
    let mut backoff = Duration::from_secs(5);
    let mut failures = 0u32;
    loop {
        let start = SystemTime::now();
        if let Ok(mut conn) = connector.connect(&url).await {
            let mut idle = Duration::from_secs(75);
            let mut subscribed = false;
            loop {
                let next = tokio::time::timeout(idle, conn.recv_text()).await;
                let Ok(Some(Ok(frame))) = next else { break };
                match seventv_event(&frame) {
                    SevenTvEvent::Hello(beat) => {
                        idle = beat.saturating_mul(3).max(Duration::from_secs(10));
                        if !subscribed {
                            subscribed = true;
                            for s in &subs {
                                if conn.send_text(s.clone()).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    SevenTvEvent::Changed => {
                        if notify.send(()).is_err() {
                            conn.close().await;
                            return;
                        }
                    }
                    SevenTvEvent::Reconnect => break,
                    SevenTvEvent::Other => {}
                }
            }
            conn.close().await;
            // A connection that lived a while resets the backoff.
            if start.elapsed().unwrap_or_default() > Duration::from_secs(60) {
                failures = 0;
                backoff = Duration::from_secs(5);
            }
        }
        failures += 1;
        if notify.is_closed() {
            return;
        }
        // The watcher falls back to periodic fetches while this is down
        // for long: give up after many failures in a row.
        if failures > 8 {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(10 * 60));
    }
}
