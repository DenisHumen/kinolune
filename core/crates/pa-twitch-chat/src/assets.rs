// SPDX-License-Identifier: MPL-2.0
//! Emote and badge catalogs of a channel (TWI-011, research 06 §3.2).
//!
//! Twitch global and channel emotes (`GET /chat/emotes/global`,
//! `GET /chat/emotes`) and badges (`GET /chat/badges/global`,
//! `GET /chat/badges`) are merged into one immutable [`ChannelAssets`] per
//! channel. [`AssetStore`] keeps those snapshots for an hour (the emote-map
//! TTL of the roadmap, well under the 24-hour limit of the Twitch Developer
//! Agreement), so a busy chat does not touch Helix per message.
//!
//! Every emote and badge is one shared [`Arc`]: all messages that use an
//! emote point at the same [`EmoteAsset`], and [`EmoteAsset::key`] is the
//! stable key under which the renderer (TWI-012) keeps one decoded animation
//! per emote rather than one per message.
//!
//! Third-party emotes (7TV, BetterTTV, FrankerFaceZ, TWI-023) are matched
//! by word, not by range: [`ChannelAssets::with_third_party`] adds a
//! [`ThirdPartyEmotes`](crate::ThirdPartyEmotes) snapshot to the Twitch
//! catalogs, and the renderer looks every word of the text up in it.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pa_twitch::{
    AuthorizedHelix, BadgeSet, BoxFuture, Clock, Emote, EmoteList, HelixClient, SystemClock,
    TtlCache,
};

use crate::source::ChatError;
use crate::third_party::ThirdPartyEmotes;

/// How long an emote map stays fresh (roadmap TWI-011: one hour).
pub const EMOTE_MAP_TTL: Duration = Duration::from_secs(60 * 60);

/// How many channels [`AssetStore`] keeps at once.
pub const MAX_CACHED_CHANNELS: usize = 32;

/// Twitch emote CDN template, used when Helix did not send one and for
/// emotes that are in no fetched list (sub emotes of other channels, emotes
/// seen by anonymous viewers).
pub const TWITCH_EMOTE_TEMPLATE: &str =
    "https://static-cdn.jtvnw.net/emoticons/v2/{{id}}/{{format}}/{{theme_mode}}/{{scale}}";

/// Where an emote comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EmoteProvider {
    /// Twitch itself (Helix catalog or the CDN by id).
    Twitch,
    /// 7TV (`7tv.io`, TWI-023).
    SevenTv,
    /// BetterTTV (`api.betterttv.net`, TWI-023).
    Bttv,
    /// FrankerFaceZ (`api.frankerfacez.com`, TWI-023).
    Ffz,
}

impl EmoteProvider {
    /// Short name used in keys: `twitch`, `7tv`, `bttv`, `ffz`.
    pub fn as_str(self) -> &'static str {
        match self {
            EmoteProvider::Twitch => "twitch",
            EmoteProvider::SevenTv => "7tv",
            EmoteProvider::Bttv => "bttv",
            EmoteProvider::Ffz => "ffz",
        }
    }
}

/// Whether an emote moves. The renderer (TWI-012) decodes an animated image
/// once per emote and may show its first frame when motion is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Animation {
    /// A still image (PNG).
    Static,
    /// An animated image (Twitch: GIF); a static variant also exists.
    Animated,
    /// Not in a fetched catalog: the CDN `default` format answers with the
    /// animated image if there is one. The renderer sniffs the file
    /// (`pa_imagecache::sniff_image`: `gif` means animated).
    Unknown,
}

/// Pixel density of an image: 1× (28 px emote, 18 px badge), 2× or 4×.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scale {
    /// Base size.
    X1,
    /// Double.
    X2,
    /// Quadruple (Twitch emote `3.0`).
    X4,
}

impl Scale {
    /// Smallest scale that covers `device_pixel_ratio` (TV and phones are
    /// usually 2×–3× → 4×).
    pub fn for_ratio(device_pixel_ratio: f32) -> Self {
        if device_pixel_ratio <= 1.0 {
            Scale::X1
        } else if device_pixel_ratio <= 2.0 {
            Scale::X2
        } else {
            Scale::X4
        }
    }

    fn twitch_emote(self) -> &'static str {
        match self {
            Scale::X1 => "1.0",
            Scale::X2 => "2.0",
            Scale::X4 => "3.0",
        }
    }
}

/// Light or dark chat background (Twitch has per-theme emote variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Theme {
    /// Light background.
    Light,
    /// Dark background (the app default).
    #[default]
    Dark,
}

impl Theme {
    fn as_str(self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
}

/// How the app wants images: density, theme and whether animations play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImagePrefs {
    /// Density.
    pub scale: Scale,
    /// Background.
    pub theme: Theme,
    /// `false`: always the static variant (reduced motion, thinned TV chat).
    pub animate: bool,
}

impl Default for ImagePrefs {
    fn default() -> Self {
        Self {
            scale: Scale::X2,
            theme: Theme::Dark,
            animate: true,
        }
    }
}

/// One emote, shared by every message that shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmoteAsset {
    /// Source.
    pub provider: EmoteProvider,
    /// Provider id.
    pub id: String,
    /// Text code (`Kappa`); empty when the emote is not in a catalog (the
    /// message text has the code).
    pub code: String,
    /// Static, animated or unknown.
    pub animation: Animation,
    /// Overlay emote drawn on top of the previous one (7TV zero-width,
    /// BTTV `cvMask`-style overlays). Always `false` for Twitch.
    pub zero_width: bool,
    /// Where the image is.
    source: UrlSource,
}

/// Image URLs of an emote.
#[derive(Debug, Clone, PartialEq, Eq)]
enum UrlSource {
    /// Twitch template with `{{id}}`, `{{format}}`, `{{theme_mode}}`,
    /// `{{scale}}`.
    Template(String),
    /// Third-party: still and animated URLs at 1×, 2×, 4× (empty when the
    /// provider has none at that scale).
    Scaled {
        still: Box<[String; 3]>,
        animated: Box<[String; 3]>,
    },
}

fn pick_scaled(urls: &[String; 3], scale: Scale) -> &str {
    let wanted = match scale {
        Scale::X1 => 0,
        Scale::X2 => 1,
        Scale::X4 => 2,
    };
    // The asked scale, else the nearest smaller one, else any.
    (0..=wanted)
        .rev()
        .chain(wanted + 1..3)
        .map(|i| urls[i].as_str())
        .find(|u| !u.is_empty())
        .unwrap_or("")
}

impl EmoteAsset {
    /// A Twitch emote from Helix.
    pub fn from_helix(emote: &Emote, template: &str) -> Self {
        let animated = emote.format.iter().any(|f| f == "animated");
        Self {
            provider: EmoteProvider::Twitch,
            id: emote.id.clone(),
            code: emote.name.clone(),
            animation: if animated {
                Animation::Animated
            } else {
                Animation::Static
            },
            zero_width: false,
            source: UrlSource::Template(if template.is_empty() {
                TWITCH_EMOTE_TEMPLATE.to_owned()
            } else {
                template.to_owned()
            }),
        }
    }

    /// A third-party emote (TWI-023) with its still and animated image
    /// URLs at 1×, 2× and 4×. An emote without animated URLs is static.
    pub fn third_party(
        provider: EmoteProvider,
        id: &str,
        code: &str,
        zero_width: bool,
        still: [String; 3],
        animated: [String; 3],
    ) -> Self {
        let moves = animated.iter().any(|u| !u.is_empty());
        Self {
            provider,
            id: id.to_owned(),
            code: code.to_owned(),
            animation: if moves {
                Animation::Animated
            } else {
                Animation::Static
            },
            zero_width,
            source: UrlSource::Scaled {
                still: Box::new(still),
                animated: Box::new(animated),
            },
        }
    }

    /// A Twitch emote known only by id (from a message): CDN template with
    /// the `default` format.
    pub fn twitch_by_id(id: &str) -> Self {
        Self {
            provider: EmoteProvider::Twitch,
            id: id.to_owned(),
            code: String::new(),
            animation: Animation::Unknown,
            zero_width: false,
            source: UrlSource::Template(TWITCH_EMOTE_TEMPLATE.to_owned()),
        }
    }

    /// Stable key of the image (`twitch:emote:<id>`, `7tv:emote:<id>`…): one decoded animation
    /// per key in the renderer, whatever the number of messages.
    pub fn key(&self) -> String {
        format!("{}:emote:{}", self.provider.as_str(), self.id)
    }

    /// Whether the image for `prefs` animates (`None`: unknown until the
    /// file is sniffed).
    pub fn animates(&self, prefs: &ImagePrefs) -> Option<bool> {
        match self.animation {
            Animation::Static => Some(false),
            Animation::Animated => Some(prefs.animate),
            Animation::Unknown if prefs.animate => None,
            Animation::Unknown => Some(false),
        }
    }

    /// Image URL for `prefs`.
    pub fn url(&self, prefs: &ImagePrefs) -> String {
        match &self.source {
            UrlSource::Template(template) => {
                let format = match (self.animation, prefs.animate) {
                    (Animation::Animated, true) => "animated",
                    (Animation::Unknown, true) => "default",
                    _ => "static",
                };
                template
                    .replace("{{id}}", &self.id)
                    .replace("{{format}}", format)
                    .replace("{{theme_mode}}", prefs.theme.as_str())
                    .replace("{{scale}}", prefs.scale.twitch_emote())
            }
            UrlSource::Scaled { still, animated } => {
                let moving = self.animation == Animation::Animated && prefs.animate;
                let url = if moving {
                    pick_scaled(animated, prefs.scale)
                } else {
                    ""
                };
                if url.is_empty() {
                    // No still variant of an animated emote: its animation
                    // (the renderer stops it when motion is off).
                    let still = pick_scaled(still, prefs.scale);
                    if still.is_empty() {
                        pick_scaled(animated, prefs.scale).to_owned()
                    } else {
                        still.to_owned()
                    }
                } else {
                    url.to_owned()
                }
            }
        }
    }
}

/// One badge version (`subscriber` / `12`), shared by every message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadgeAsset {
    /// Set id.
    pub set_id: String,
    /// Version id.
    pub version: String,
    /// Broadcaster id for a channel badge (custom sub/bits badge), `None`
    /// for a global one.
    pub channel_id: Option<String>,
    /// Title (tooltip).
    pub title: String,
    /// Description.
    pub description: String,
    /// 1× (18 px), 2× and 4× images.
    urls: [String; 3],
}

impl BadgeAsset {
    /// Badge versions of a Helix set.
    pub fn from_helix(set: &BadgeSet, channel_id: Option<&str>) -> Vec<Self> {
        set.versions
            .iter()
            .map(|v| Self {
                set_id: set.set_id.clone(),
                version: v.id.clone(),
                channel_id: channel_id.map(str::to_owned),
                title: v.title.clone(),
                description: v.description.clone(),
                urls: [
                    v.image_url_1x.clone(),
                    v.image_url_2x.clone(),
                    v.image_url_4x.clone(),
                ],
            })
            .collect()
    }

    /// Stable key: `twitch:badge:global:<set>/<version>` or
    /// `twitch:badge:<channel>:<set>/<version>`.
    pub fn key(&self) -> String {
        format!(
            "twitch:badge:{}:{}/{}",
            self.channel_id.as_deref().unwrap_or("global"),
            self.set_id,
            self.version
        )
    }

    /// Image URL at `scale`, falling back to a smaller one Twitch sent.
    pub fn url(&self, scale: Scale) -> &str {
        let wanted = match scale {
            Scale::X1 => 0,
            Scale::X2 => 1,
            Scale::X4 => 2,
        };
        (0..=wanted)
            .rev()
            .map(|i| self.urls[i].as_str())
            .find(|u| !u.is_empty())
            .unwrap_or("")
    }
}

/// Everything needed to render a channel's chat: an immutable snapshot of
/// global + channel emotes and badges. Cheap to share (`Arc`).
#[derive(Debug, Default)]
pub struct ChannelAssets {
    /// Broadcaster id (empty for [`ChannelAssets::cdn_only`]).
    pub channel_id: String,
    emotes: HashMap<String, Arc<EmoteAsset>>,
    /// Ids of the channel's own emotes (listed first in autocomplete).
    channel_emotes: HashSet<String>,
    badges: HashMap<(String, String), Arc<BadgeAsset>>,
    /// Emotes of messages that were in no catalog, made once per id.
    unknown: Mutex<HashMap<String, Arc<EmoteAsset>>>,
    /// 7TV, BTTV and FFZ emotes by code (TWI-023); `None` when off.
    third_party: Option<Arc<ThirdPartyEmotes>>,
}

/// Unknown emote ids remembered per channel; beyond this they are made per
/// call (still correct, only not shared).
const MAX_UNKNOWN_EMOTES: usize = 4096;

impl ChannelAssets {
    /// Merges catalogs. Channel emotes and badges override global ones with
    /// the same id (custom subscriber and bits badges).
    pub fn new(
        channel_id: &str,
        global_emotes: &EmoteList,
        channel_emotes: &EmoteList,
        global_badges: &[BadgeSet],
        channel_badges: &[BadgeSet],
    ) -> Self {
        let mut emotes = HashMap::new();
        for list in [global_emotes, channel_emotes] {
            for e in &list.emotes {
                emotes.insert(
                    e.id.clone(),
                    Arc::new(EmoteAsset::from_helix(e, &list.template)),
                );
            }
        }
        let channel_emotes = channel_emotes.emotes.iter().map(|e| e.id.clone()).collect();
        let mut badges = HashMap::new();
        let channel = (!channel_id.is_empty()).then_some(channel_id);
        for (sets, owner) in [(global_badges, None), (channel_badges, channel)] {
            for set in sets {
                for b in BadgeAsset::from_helix(set, owner) {
                    badges.insert((b.set_id.clone(), b.version.clone()), Arc::new(b));
                }
            }
        }
        Self {
            channel_id: channel_id.to_owned(),
            emotes,
            channel_emotes,
            badges,
            unknown: Mutex::new(HashMap::new()),
            third_party: None,
        }
    }

    /// The same Twitch catalogs with `third_party` emotes (TWI-023)
    /// instead of the current ones; `None` turns them off. Cheap: the
    /// catalogs share their assets.
    pub fn with_third_party(&self, third_party: Option<Arc<ThirdPartyEmotes>>) -> Self {
        let unknown = lock(&self.unknown).clone();
        Self {
            channel_id: self.channel_id.clone(),
            emotes: self.emotes.clone(),
            channel_emotes: self.channel_emotes.clone(),
            badges: self.badges.clone(),
            unknown: Mutex::new(unknown),
            third_party: third_party.filter(|t| !t.is_empty()),
        }
    }

    /// The third-party emotes in use (TWI-023).
    pub fn third_party(&self) -> Option<&Arc<ThirdPartyEmotes>> {
        self.third_party.as_ref()
    }

    /// A third-party emote whose code is exactly `word`.
    #[inline]
    pub fn third_party_emote(&self, word: &str) -> Option<&Arc<EmoteAsset>> {
        self.third_party.as_ref().and_then(|t| t.get(word))
    }

    /// The emote belongs to the channel's own set (Twitch or third-party):
    /// autocomplete lists it first.
    pub fn is_channel_emote(&self, emote: &EmoteAsset) -> bool {
        match emote.provider {
            EmoteProvider::Twitch => self.channel_emotes.contains(&emote.id),
            _ => self
                .third_party
                .as_ref()
                .is_some_and(|t| t.is_channel(emote)),
        }
    }

    /// No catalogs (anonymous viewer without Helix access): emotes still
    /// render from the Twitch CDN by id; badges do not.
    pub fn cdn_only() -> Self {
        Self::default()
    }

    /// Emote by id: from the catalogs, else a shared CDN-only asset.
    pub fn emote(&self, id: &str) -> Arc<EmoteAsset> {
        if let Some(e) = self.emotes.get(id) {
            return Arc::clone(e);
        }
        let mut unknown = self
            .unknown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(e) = unknown.get(id) {
            return Arc::clone(e);
        }
        let e = Arc::new(EmoteAsset::twitch_by_id(id));
        if unknown.len() < MAX_UNKNOWN_EMOTES {
            unknown.insert(id.to_owned(), Arc::clone(&e));
        }
        e
    }

    /// Badge by set and version.
    pub fn badge(&self, set_id: &str, version: &str) -> Option<Arc<BadgeAsset>> {
        self.badges
            .get(&(set_id.to_owned(), version.to_owned()))
            .cloned()
    }

    /// Catalog emotes (for the emote picker and autocomplete, TWI-012),
    /// third-party ones included, sorted by code.
    pub fn emotes(&self) -> Vec<Arc<EmoteAsset>> {
        let mut all: Vec<_> = self.emotes.values().cloned().collect();
        if let Some(t) = &self.third_party {
            all.extend(t.all().cloned());
        }
        all.sort_by(|a, b| a.code.cmp(&b.code).then_with(|| a.id.cmp(&b.id)));
        all
    }

    /// Ids of the channel's own emotes (TWI-024 autocomplete lists them
    /// before the global ones).
    pub fn channel_emote_ids(&self) -> &HashSet<String> {
        &self.channel_emotes
    }

    /// Number of catalog emotes and badges.
    pub fn len(&self) -> (usize, usize) {
        (self.emotes.len(), self.badges.len())
    }

    /// No catalog emotes and no badges.
    pub fn is_empty(&self) -> bool {
        self.emotes.is_empty() && self.badges.is_empty()
    }
}

/// Helix reads behind [`AssetStore`]. Implemented for
/// [`pa_twitch::HelixClient`] and [`pa_twitch::AuthorizedHelix`]; tests use
/// either over the fixture transport.
pub trait EmoteApi: Send + Sync {
    /// `GET /chat/emotes/global`.
    fn global_emotes(&self) -> BoxFuture<'_, Result<EmoteList, ChatError>>;
    /// `GET /chat/emotes?broadcaster_id=`.
    fn channel_emotes<'a>(
        &'a self,
        broadcaster_id: &'a str,
    ) -> BoxFuture<'a, Result<EmoteList, ChatError>>;
    /// `GET /chat/badges/global`.
    fn global_badges(&self) -> BoxFuture<'_, Result<Vec<BadgeSet>, ChatError>>;
    /// `GET /chat/badges?broadcaster_id=`.
    fn channel_badges<'a>(
        &'a self,
        broadcaster_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<BadgeSet>, ChatError>>;
}

impl EmoteApi for HelixClient {
    fn global_emotes(&self) -> BoxFuture<'_, Result<EmoteList, ChatError>> {
        Box::pin(async move { Ok(self.get_global_emotes().await?) })
    }

    fn channel_emotes<'a>(
        &'a self,
        broadcaster_id: &'a str,
    ) -> BoxFuture<'a, Result<EmoteList, ChatError>> {
        Box::pin(async move { Ok(self.get_channel_emotes(broadcaster_id).await?) })
    }

    fn global_badges(&self) -> BoxFuture<'_, Result<Vec<BadgeSet>, ChatError>> {
        Box::pin(async move { Ok(self.get_global_badges().await?) })
    }

    fn channel_badges<'a>(
        &'a self,
        broadcaster_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<BadgeSet>, ChatError>> {
        Box::pin(async move { Ok(self.get_channel_badges(broadcaster_id).await?) })
    }
}

impl EmoteApi for AuthorizedHelix {
    fn global_emotes(&self) -> BoxFuture<'_, Result<EmoteList, ChatError>> {
        Box::pin(async move {
            Ok(self
                .call(|h| async move { h.get_global_emotes().await })
                .await?)
        })
    }

    fn channel_emotes<'a>(
        &'a self,
        broadcaster_id: &'a str,
    ) -> BoxFuture<'a, Result<EmoteList, ChatError>> {
        Box::pin(async move {
            Ok(self
                .call(|h| async move { h.get_channel_emotes(broadcaster_id).await })
                .await?)
        })
    }

    fn global_badges(&self) -> BoxFuture<'_, Result<Vec<BadgeSet>, ChatError>> {
        Box::pin(async move {
            Ok(self
                .call(|h| async move { h.get_global_badges().await })
                .await?)
        })
    }

    fn channel_badges<'a>(
        &'a self,
        broadcaster_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<BadgeSet>, ChatError>> {
        Box::pin(async move {
            Ok(self
                .call(|h| async move { h.get_channel_badges(broadcaster_id).await })
                .await?)
        })
    }
}

#[derive(Clone)]
struct GlobalCatalog {
    emotes: Arc<EmoteList>,
    badges: Arc<Vec<BadgeSet>>,
}

/// Fetches and keeps [`ChannelAssets`] for an hour ([`EMOTE_MAP_TTL`]).
///
/// The global catalogs are fetched once per TTL and shared by all
/// channels. Two concurrent calls for a cold channel may both fetch; the
/// result is the same.
pub struct AssetStore {
    api: Arc<dyn EmoteApi>,
    clock: Arc<dyn Clock>,
    global: Mutex<TtlCache<(), GlobalCatalog>>,
    channels: Mutex<TtlCache<String, Arc<ChannelAssets>>>,
}

impl std::fmt::Debug for AssetStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetStore").finish_non_exhaustive()
    }
}

impl AssetStore {
    /// Store over `api` with the system clock.
    pub fn new(api: Arc<dyn EmoteApi>) -> Self {
        Self::with_clock(api, Arc::new(SystemClock))
    }

    /// Store with an injected clock (tests) and the default TTL.
    pub fn with_clock(api: Arc<dyn EmoteApi>, clock: Arc<dyn Clock>) -> Self {
        Self::with_ttl(api, clock, EMOTE_MAP_TTL)
    }

    /// Store with a custom TTL (clamped to 24 h by `TtlCache`).
    pub fn with_ttl(api: Arc<dyn EmoteApi>, clock: Arc<dyn Clock>, ttl: Duration) -> Self {
        Self {
            api,
            clock,
            global: Mutex::new(TtlCache::new(ttl, 1)),
            channels: Mutex::new(TtlCache::new(ttl, MAX_CACHED_CHANNELS)),
        }
    }

    /// Assets of a channel: from the cache while fresh, else four Helix
    /// calls (two when the global catalog is still fresh).
    pub async fn channel(&self, broadcaster_id: &str) -> Result<Arc<ChannelAssets>, ChatError> {
        if broadcaster_id.is_empty() {
            return Err(ChatError::InvalidArgument("broadcaster id is empty"));
        }
        let now = self.clock.now();
        if let Some(hit) = lock(&self.channels).get(&broadcaster_id.to_owned(), now) {
            return Ok(hit);
        }
        let global = self.global().await?;
        let emotes = self.api.channel_emotes(broadcaster_id).await?;
        let badges = self.api.channel_badges(broadcaster_id).await?;
        let assets = Arc::new(ChannelAssets::new(
            broadcaster_id,
            &global.emotes,
            &emotes,
            &global.badges,
            &badges,
        ));
        lock(&self.channels).insert(
            broadcaster_id.to_owned(),
            Arc::clone(&assets),
            self.clock.now(),
        );
        Ok(assets)
    }

    /// Drops cached catalogs (e.g. after the streamer changed emotes, or on
    /// sign-out).
    pub fn invalidate(&self) {
        let ttl = lock(&self.channels).ttl();
        *lock(&self.channels) = TtlCache::new(ttl, MAX_CACHED_CHANNELS);
        *lock(&self.global) = TtlCache::new(ttl, 1);
    }

    async fn global(&self) -> Result<GlobalCatalog, ChatError> {
        if let Some(hit) = lock(&self.global).get(&(), self.clock.now()) {
            return Ok(hit);
        }
        let emotes = self.api.global_emotes().await?;
        let badges = self.api.global_badges().await?;
        let catalog = GlobalCatalog {
            emotes: Arc::new(emotes),
            badges: Arc::new(badges),
        };
        lock(&self.global).insert((), catalog.clone(), self.clock.now());
        Ok(catalog)
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
