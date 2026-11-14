// SPDX-License-Identifier: MPL-2.0
//! Native Twitch playback for the Dart player (TWI-014, full builds only).
//!
//! - [`CoreTwitchPlayback`]: the TWI-004 decision. It resolves a live
//!   channel or a VOD through GQL `PlaybackAccessToken` and usher into the
//!   variants of the master playlist (qualities, audio only). The remote
//!   config decides whether it may (`twitch.gql_playback`, `player`,
//!   kill-switches); the GQL client id and query hash come only from it.
//! - [`CoreLiveProxy`]: the loopback live proxy of TWI-006 in front of one
//!   media playlist. The player opens [`CoreLiveProxy::playlist_url`]; the
//!   Dart `LivePlaybackController` polls `stats_url` and calls `reset_url`.
//! - [`twitch_master_variants`]: parses a master playlist (the player lab
//!   and tests).
//!
//! The API is the same in every flavor so the generated bindings are too.
//! Store builds have no GQL, usher or proxy code (`cfg(pa_twitch_native)`,
//! ADR-0041): every call there fails with the code `store_build`, and the
//! app never calls them, because `decideTwitchRoute` picks the embed.

use flutter_rust_bridge::frb;

use super::remote_config::CoreRemoteConfig;

/// Error of a native playback call; Dart receives it as an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePlaybackError {
    /// `store_build`, `embed` (the config wants the embed: `message` says
    /// why), `invalid_target`, `network`, `not_found`, `unavailable`
    /// (offline channel, deleted VOD), `usher_403` (sub-only, geo block;
    /// never bypassed), `gql_hash_invalid`, `integrity_required`,
    /// `gql_error`, `usher_error`, `decode`, `io`.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// The official embed may still play the target (TWI-005).
    pub falls_back_to_embed: bool,
}

impl CorePlaybackError {
    fn new(code: &str, message: impl ToString, falls_back_to_embed: bool) -> Self {
        Self {
            code: code.into(),
            message: message.to_string(),
            falls_back_to_embed,
        }
    }

    #[cfg(not(pa_twitch_native))]
    fn store() -> Self {
        Self::new(
            "store_build",
            "native Twitch playback is not in this build",
            true,
        )
    }
}

#[cfg(pa_twitch_native)]
impl From<pa_twitch::playback::PlaybackError> for CorePlaybackError {
    fn from(e: pa_twitch::playback::PlaybackError) -> Self {
        Self::new(e.code(), &e, e.falls_back_to_embed())
    }
}

/// One rendition of the master playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreTwitchVariant {
    /// `VIDEO` group id (`chunked`, `720p60`, `audio_only`, …).
    pub group_id: String,
    /// Quality label (`1080p60`, `audio_only`).
    pub quality: String,
    /// The original quality.
    pub is_source: bool,
    /// Audio only (the "listen" mode).
    pub audio_only: bool,
    /// Peak bits per second.
    pub bandwidth: u64,
    /// Picture size, when given.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<f32>,
    /// Media playlist URL.
    pub url: String,
}

/// A resolved stream or VOD: the variants in playlist order.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreTwitchStream {
    pub variants: Vec<CoreTwitchVariant>,
}

/// Live latency mode (`pa_twitch::playback::live::LiveMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreLiveMode {
    /// Prefetch segments, small buffer (≈ 3 s).
    LowLatency,
    /// Complete segments only (≈ 8 s).
    Stable,
}

/// Parses a master playlist (`#EXT-X-STREAM-INF` variants).
#[frb(sync)]
pub fn twitch_master_variants(text: String) -> Result<Vec<CoreTwitchVariant>, CorePlaybackError> {
    #[cfg(pa_twitch_native)]
    {
        let master = pa_twitch::playback::MasterPlaylist::parse(&text)?;
        Ok(master.variants.into_iter().map(variant).collect())
    }
    #[cfg(not(pa_twitch_native))]
    {
        let _ = text;
        Err(CorePlaybackError::store())
    }
}

#[cfg(pa_twitch_native)]
fn variant(v: pa_twitch::playback::Variant) -> CoreTwitchVariant {
    CoreTwitchVariant {
        group_id: v.group_id,
        quality: v.quality,
        is_source: v.is_source,
        audio_only: v.audio_only,
        bandwidth: v.bandwidth,
        width: v.resolution.map(|(w, _)| w),
        height: v.resolution.map(|(_, h)| h),
        frame_rate: v.frame_rate,
        url: v.url,
    }
}

/// The native playback path of this app (remote config ∧ flavor).
#[frb(opaque)]
pub struct CoreTwitchPlayback {
    #[cfg(pa_twitch_native)]
    inner: pa_twitch::playback::TwitchPlayback,
    #[cfg(pa_twitch_native)]
    rt: tokio::runtime::Runtime,
    /// Why the embed is used instead, when it is.
    embed_reason: Option<String>,
}

impl CoreTwitchPlayback {
    /// The decision of [`config`] for this build. No network.
    #[frb(sync)]
    pub fn open(config: &CoreRemoteConfig) -> Result<CoreTwitchPlayback, CorePlaybackError> {
        #[cfg(pa_twitch_native)]
        {
            use std::sync::Arc;
            use std::time::Duration;

            use pa_twitch::playback::{PlayerChoice, TwitchFeatures, TwitchPlayback};

            let (cfg, ctx) = config.config_and_context();
            let features = TwitchFeatures::resolve(&cfg, &ctx);
            let embed_reason = match &features.player {
                PlayerChoice::Embed(reason) => Some(format!("{reason:?}")),
                PlayerChoice::Native(_) => None,
            };
            let transport = pa_twitch::ReqwestTransport::new(Duration::from_secs(15))
                .map_err(|e| CorePlaybackError::new("io", e, false))?;
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| CorePlaybackError::new("io", e, false))?;
            Ok(Self {
                inner: TwitchPlayback::new(features, Arc::new(transport)),
                rt,
                embed_reason,
            })
        }
        #[cfg(not(pa_twitch_native))]
        {
            let _ = config;
            Ok(Self {
                embed_reason: Some("StoreBuild".into()),
            })
        }
    }

    /// Native playback is possible (full build, the config allows it).
    #[frb(sync)]
    pub fn native_allowed(&self) -> bool {
        self.embed_reason.is_none()
    }

    /// Why not, for diagnostics (`StoreBuild`, `GqlPlaybackOff`, …).
    #[frb(sync)]
    pub fn embed_reason(&self) -> Option<String> {
        self.embed_reason.clone()
    }

    /// Variants of a live channel. `av1`/`hevc`: what the device decodes.
    pub fn resolve_live(
        &self,
        login: String,
        av1: bool,
        hevc: bool,
    ) -> Result<CoreTwitchStream, CorePlaybackError> {
        #[cfg(pa_twitch_native)]
        {
            let target = pa_twitch::playback::PlaybackTarget::live(&login)?;
            self.resolve(target, av1, hevc)
        }
        #[cfg(not(pa_twitch_native))]
        {
            let _ = (login, av1, hevc);
            Err(CorePlaybackError::store())
        }
    }

    /// Variants of a VOD.
    pub fn resolve_vod(
        &self,
        video_id: String,
        av1: bool,
        hevc: bool,
    ) -> Result<CoreTwitchStream, CorePlaybackError> {
        #[cfg(pa_twitch_native)]
        {
            let target = pa_twitch::playback::PlaybackTarget::vod(&video_id)?;
            self.resolve(target, av1, hevc)
        }
        #[cfg(not(pa_twitch_native))]
        {
            let _ = (video_id, av1, hevc);
            Err(CorePlaybackError::store())
        }
    }

    #[cfg(pa_twitch_native)]
    fn resolve(
        &self,
        target: pa_twitch::playback::PlaybackTarget,
        av1: bool,
        hevc: bool,
    ) -> Result<CoreTwitchStream, CorePlaybackError> {
        use pa_twitch::playback::{DeviceCodecs, PlaybackDecision};

        let codecs = DeviceCodecs { av1, hevc };
        match self.rt.block_on(self.inner.resolve(&target, codecs))? {
            PlaybackDecision::Native(n) => Ok(CoreTwitchStream {
                variants: n.playlist.variants.into_iter().map(variant).collect(),
            }),
            PlaybackDecision::Embed(e) => Err(CorePlaybackError::new(
                "embed",
                format!("{:?}", e.reason),
                true,
            )),
        }
    }
}

/// The loopback live proxy (TWI-006) for one media playlist. Dropping it
/// (or [`Self::stop`]) stops the server.
#[frb(opaque)]
pub struct CoreLiveProxy {
    #[cfg(pa_twitch_native)]
    inner: std::sync::Mutex<Option<pa_twitch::playback::live::LiveProxy>>,
    playlist_url: String,
    stats_url: String,
    reset_url: String,
}

#[cfg(pa_twitch_native)]
fn live_mode(mode: CoreLiveMode) -> pa_twitch::playback::live::LiveMode {
    match mode {
        CoreLiveMode::LowLatency => pa_twitch::playback::live::LiveMode::LowLatency,
        CoreLiveMode::Stable => pa_twitch::playback::live::LiveMode::Stable,
    }
}

impl CoreLiveProxy {
    /// Starts the proxy on `127.0.0.1:<random port>` for
    /// `media_playlist_url` (a variant's `url`).
    #[frb(sync)]
    pub fn start(
        media_playlist_url: String,
        mode: CoreLiveMode,
    ) -> Result<CoreLiveProxy, CorePlaybackError> {
        #[cfg(pa_twitch_native)]
        {
            use std::sync::Arc;

            use pa_twitch::playback::live::{LiveProxy, ReqwestUpstream};

            let upstream = ReqwestUpstream::new()?;
            let proxy = LiveProxy::start(Arc::new(upstream), &media_playlist_url, live_mode(mode))
                .map_err(|e| CorePlaybackError::new("io", e, false))?;
            Ok(Self {
                playlist_url: proxy.playlist_url(),
                stats_url: proxy.stats_url(),
                reset_url: proxy.reset_url(),
                inner: std::sync::Mutex::new(Some(proxy)),
            })
        }
        #[cfg(not(pa_twitch_native))]
        {
            let _ = (media_playlist_url, mode);
            Err(CorePlaybackError::store())
        }
    }

    /// `http://127.0.0.1:<port>/<token>/live.m3u8`: what the player opens.
    #[frb(sync)]
    pub fn playlist_url(&self) -> String {
        self.playlist_url.clone()
    }

    /// The JSON report (`?ahead=<s>&rate=<x>`).
    #[frb(sync)]
    pub fn stats_url(&self) -> String {
        self.stats_url.clone()
    }

    /// Forgets the playhead ("К эфиру").
    #[frb(sync)]
    pub fn reset_url(&self) -> String {
        self.reset_url.clone()
    }

    /// Switches the latency mode; the player reloads the playlist URL.
    #[frb(sync)]
    pub fn set_mode(&self, mode: CoreLiveMode) {
        #[cfg(pa_twitch_native)]
        if let Some(p) = self.proxy().as_ref() {
            p.set_mode(live_mode(mode));
        }
        #[cfg(not(pa_twitch_native))]
        let _ = mode;
    }

    /// Switches to another variant (quality); the player reloads.
    #[frb(sync)]
    pub fn set_media_playlist(&self, url: String) {
        #[cfg(pa_twitch_native)]
        if let Some(p) = self.proxy().as_ref() {
            p.set_media_playlist(&url);
        }
        #[cfg(not(pa_twitch_native))]
        let _ = url;
    }

    /// Stops the server now (the player closed).
    #[frb(sync)]
    pub fn stop(&self) {
        #[cfg(pa_twitch_native)]
        drop(self.proxy().take());
    }

    #[cfg(pa_twitch_native)]
    fn proxy(
        &self,
    ) -> std::sync::MutexGuard<'_, Option<pa_twitch::playback::live::LiveProxy>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(dir: &tempfile::TempDir) -> CoreRemoteConfig {
        CoreRemoteConfig::open(
            dir.path().to_string_lossy().into_owned(),
            "0.1.0".into(),
            "linux".into(),
        )
    }

    #[test]
    fn built_in_config_keeps_native_playback_off() {
        // The built-in config has GQL off: the embed in every flavor.
        let dir = tempfile::tempdir().unwrap();
        let playback = CoreTwitchPlayback::open(&config(&dir)).unwrap();
        assert!(!playback.native_allowed());
        assert!(playback.embed_reason().is_some());
    }

    #[cfg(not(pa_twitch_native))]
    #[test]
    fn store_build_has_no_native_path() {
        let e = twitch_master_variants("#EXTM3U".into()).unwrap_err();
        assert_eq!(e.code, "store_build");
        assert!(e.falls_back_to_embed);
        let e = CoreLiveProxy::start("http://127.0.0.1:1/x.m3u8".into(), CoreLiveMode::Stable)
            .err()
            .unwrap();
        assert_eq!(e.code, "store_build");
        let dir = tempfile::tempdir().unwrap();
        let playback = CoreTwitchPlayback::open(&config(&dir)).unwrap();
        assert_eq!(playback.embed_reason().as_deref(), Some("StoreBuild"));
        let e = playback
            .resolve_live("somechannel".into(), false, false)
            .unwrap_err();
        assert_eq!(e.code, "store_build");
    }

    #[cfg(pa_twitch_native)]
    #[test]
    fn master_variants_keep_quality_and_audio_only() {
        let text = "#EXTM3U\n\
            #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"1080p60 (source)\",AUTOSELECT=YES,DEFAULT=YES\n\
            #EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080,CODECS=\"avc1.64002A,mp4a.40.2\",VIDEO=\"chunked\",FRAME-RATE=60.000\n\
            https://example.invalid/chunked.m3u8\n\
            #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"audio_only\",NAME=\"audio_only\",AUTOSELECT=NO,DEFAULT=NO\n\
            #EXT-X-STREAM-INF:BANDWIDTH=160000,CODECS=\"mp4a.40.2\",VIDEO=\"audio_only\"\n\
            https://example.invalid/audio.m3u8\n";
        let v = twitch_master_variants(text.into()).unwrap();
        assert_eq!(v.len(), 2);
        assert!(v[0].is_source);
        assert_eq!(v[0].height, Some(1080));
        assert!(v[1].audio_only);
        assert_eq!(v[1].url, "https://example.invalid/audio.m3u8");
        let e = twitch_master_variants("nope".into()).unwrap_err();
        assert_eq!(e.code, "decode");
    }

    #[cfg(pa_twitch_native)]
    #[test]
    fn live_proxy_serves_urls_and_stops() {
        let proxy = CoreLiveProxy::start(
            "http://127.0.0.1:9/live/index.m3u8".into(),
            CoreLiveMode::LowLatency,
        )
        .unwrap();
        assert!(proxy.playlist_url().starts_with("http://127.0.0.1:"));
        assert!(proxy.playlist_url().ends_with("/live.m3u8"));
        assert!(proxy.stats_url().ends_with("/stats.json"));
        assert!(proxy.reset_url().ends_with("/reset"));
        proxy.set_mode(CoreLiveMode::Stable);
        proxy.set_media_playlist("http://127.0.0.1:9/other.m3u8".into());
        proxy.stop();
        proxy.stop();
    }
}
