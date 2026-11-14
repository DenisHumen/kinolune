// SPDX-License-Identifier: MPL-2.0
//! Emote and badge images on disk through `pa-imagecache` (LIB-013).
//!
//! Chat images live in their own cache directory (not the artwork one):
//! the Twitch Developer Agreement allows caching Twitch content for at most
//! 24 hours (docs/legal/providers.md §2.4), so this cache uses
//! [`pa_twitch::MAX_CACHE_TTL`] as its age limit instead of the six months
//! of the artwork cache. Files are only used on this device.
//!
//! Calls download synchronously (`ureq` in the app's fetcher); run them off
//! the UI thread. [`ChatImages::cached`] never touches the network, for the
//! first frame.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pa_imagecache::{CacheOptions, ImageCache, ImageFetcher};
use pa_twitch::MAX_CACHE_TTL;

use crate::assets::{BadgeAsset, EmoteAsset, ImagePrefs};
use crate::render::RenderedMessage;

/// Default size limit of the chat image cache: 64 MiB, thousands of emotes
/// at 2×.
pub const DEFAULT_CHAT_CACHE_BYTES: u64 = 64 * 1024 * 1024;

/// One image to show: the asset's key (one decode per key), the URL for
/// the current [`ImagePrefs`] and whether it animates.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageRequest {
    /// [`EmoteAsset::key`] or [`BadgeAsset::key`], plus `#static` when the
    /// static variant of an animated emote is asked for.
    pub key: String,
    /// Image URL.
    pub url: String,
    /// `Some(true)`: animated (GIF), `Some(false)`: still, `None`: sniff
    /// the file (`pa_imagecache::sniff_image`).
    pub animated: Option<bool>,
}

impl ImageRequest {
    /// Request for an emote.
    pub fn emote(asset: &EmoteAsset, prefs: &ImagePrefs) -> Self {
        let animated = asset.animates(prefs);
        let mut key = asset.key();
        if animated == Some(false) && asset.animation != crate::assets::Animation::Static {
            key.push_str("#static");
        }
        Self {
            key,
            url: asset.url(prefs),
            animated,
        }
    }

    /// Request for a badge (always still).
    pub fn badge(asset: &BadgeAsset, prefs: &ImagePrefs) -> Self {
        Self {
            key: asset.key(),
            url: asset.url(prefs.scale).to_owned(),
            animated: Some(false),
        }
    }
}

/// Distinct images of `messages` for `prefs`, in first-use order: what a
/// batch of chat lines needs, each image once however often it is used.
pub fn image_requests<'a>(
    messages: impl IntoIterator<Item = &'a RenderedMessage>,
    prefs: &ImagePrefs,
) -> Vec<ImageRequest> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for m in messages {
        for b in &m.badges {
            let r = ImageRequest::badge(&b.asset, prefs);
            if !r.url.is_empty() && seen.insert(r.key.clone()) {
                out.push(r);
            }
        }
        for e in m.emotes() {
            let r = ImageRequest::emote(e, prefs);
            if seen.insert(r.key.clone()) {
                out.push(r);
            }
        }
    }
    out
}

/// Chat image cache: a `pa-imagecache` instance with the 24-hour limit.
#[derive(Debug, Clone)]
pub struct ChatImages {
    cache: Arc<ImageCache>,
}

impl ChatImages {
    /// Opens (creating) the chat image cache in `dir`, e.g.
    /// `PlatformDirs::cache/twitch-chat`, with `max_bytes` as size limit.
    pub fn open(
        dir: impl Into<PathBuf>,
        max_bytes: u64,
        fetcher: Arc<dyn ImageFetcher>,
    ) -> Result<Self, pa_imagecache::Error> {
        let options = CacheOptions {
            max_bytes,
            max_age: MAX_CACHE_TTL,
        };
        Ok(Self {
            cache: Arc::new(ImageCache::open(dir, options, fetcher)?),
        })
    }

    /// The cache directory.
    pub fn dir(&self) -> &Path {
        self.cache.dir()
    }

    /// The underlying cache (usage, limit, clear).
    pub fn cache(&self) -> &ImageCache {
        &self.cache
    }

    /// Local file of `request`: cached, or downloaded now (blocking).
    pub fn get(&self, request: &ImageRequest) -> Result<PathBuf, pa_imagecache::Error> {
        self.cache.get(&request.url, None)
    }

    /// Local file of `request` if it is cached; never touches the network.
    pub fn cached(&self, request: &ImageRequest) -> Option<PathBuf> {
        self.cache.cached(&request.url, None)
    }
}
