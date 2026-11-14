// SPDX-License-Identifier: MPL-2.0
//! Where a download gets its data: Helix, chapters, usher, HTTP.

use std::sync::Arc;

use pa_twitch::chapters::Chapter;
use pa_twitch::playback::{
    DeviceCodecs, MasterPlaylist, PlaybackDecision, PlaybackError, PlaybackTarget, PlayerChoice,
    TwitchFeatures, TwitchPlayback,
};
use pa_twitch::{
    AuthError, AuthorizedHelix, HelixClient, HttpRequest, HttpResponse, Method, Transport, Video,
    VideosQuery,
};

/// Why a step of a download failed. `code` is stable (logs, the UI);
/// `retryable` decides between a job retry with backoff and a failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ArchiveError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl ArchiveError {
    /// A permanent failure.
    pub fn fatal(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: false,
        }
    }

    /// A transient failure (network, server busy).
    pub fn retry(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: true,
        }
    }
}

impl From<PlaybackError> for ArchiveError {
    fn from(e: PlaybackError) -> Self {
        let code = e.code();
        match &e {
            PlaybackError::Transport(_) => Self::retry(code, e.to_string()),
            PlaybackError::Restricted { code: usher } => Self::fatal(
                code,
                format!(
                    "Twitch refuses this VOD ({usher}): sub-only, geo-blocked or otherwise \
                     restricted videos are not downloaded"
                ),
            ),
            PlaybackError::Gql { status, .. } | PlaybackError::Usher { status }
                if *status >= 500 || *status == 429 =>
            {
                Self::retry(code, e.to_string())
            }
            _ => Self::fatal(code, e.to_string()),
        }
    }
}

impl From<pa_twitch::Error> for ArchiveError {
    fn from(e: pa_twitch::Error) -> Self {
        use pa_twitch::Error as E;
        match &e {
            E::Transport(_) | E::RateLimited { .. } => Self::retry("helix_network", e.to_string()),
            E::Api { status, .. } if *status >= 500 => Self::retry("helix_error", e.to_string()),
            E::MissingToken | E::Unauthorized { .. } => Self::fatal("signed_out", e.to_string()),
            _ => Self::fatal("helix_error", e.to_string()),
        }
    }
}

impl From<AuthError> for ArchiveError {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::Helix(inner) => inner.into(),
            AuthError::Transport(m) => Self::retry("helix_network", m),
            other => Self::fatal("signed_out", other.to_string()),
        }
    }
}

/// Data a download needs. Methods block; they are called from the job's
/// worker threads (several at once for [`get`](Self::get)).
pub trait ArchiveSource: Send + Sync {
    /// Whether archiving is allowed now: full build, remote-config
    /// `twitch.archive` on, native playback (GQL + usher) allowed. Checked
    /// at the start of every run, so turning the flag off stops queued
    /// downloads.
    fn check_allowed(&self) -> Result<(), ArchiveError>;

    /// The VOD from Helix (`GET /videos?id=`).
    fn video(&self, video_id: &str) -> Result<Video, ArchiveError>;

    /// The owner's avatar URL from Helix, if any.
    fn avatar_url(&self, user_id: &str) -> Result<Option<String>, ArchiveError>;

    /// Chapters of the VOD (TWI-008): what the library stored for it.
    fn chapters(&self, video: &Video) -> Vec<Chapter>;

    /// usher's master playlist of the VOD (TWI-004). A refusal (sub-only,
    /// geo block) is an error with code `usher_403`; it is never worked
    /// around.
    fn master_playlist(&self, video_id: &str) -> Result<MasterPlaylist, ArchiveError>;

    /// One HTTP GET (playlists, segments, images).
    fn get(&self, url: &str) -> Result<HttpResponse, ArchiveError>;

    /// Chat replay as TwitchDownloader-compatible JSON, when chat replay
    /// (TWI-016) is available and its flags allow it; `None` otherwise.
    fn chat_replay(&self, video: &Video) -> Result<Option<Vec<u8>>, ArchiveError> {
        let _ = video;
        Ok(None)
    }
}

/// How the source reaches Helix.
#[derive(Clone)]
pub enum HelixAccess {
    /// A client with a token already set (tests, the server).
    Client(Arc<HelixClient>),
    /// The signed-in user, refreshing the token when needed (the apps).
    Authorized(Arc<AuthorizedHelix>),
}

/// Chapters of a VOD, usually read from the library (TWI-007/008).
pub type ChaptersFn = dyn Fn(&Video) -> Vec<Chapter> + Send + Sync;
/// Chat replay hook for TWI-016.
pub type ChatReplayFn = dyn Fn(&Video) -> Result<Option<Vec<u8>>, ArchiveError> + Send + Sync;

/// [`ArchiveSource`] over Twitch: Helix, the TWI-004 playback resolver and
/// one HTTP transport. Owns a small tokio runtime that every blocking call
/// runs on, so pooled connections stay valid across job runs.
pub struct TwitchSource {
    rt: tokio::runtime::Runtime,
    features: TwitchFeatures,
    playback: TwitchPlayback,
    helix: HelixAccess,
    transport: Arc<dyn Transport>,
    codecs: DeviceCodecs,
    chapters: Option<Box<ChaptersFn>>,
    chat_replay: Option<Box<ChatReplayFn>>,
}

impl TwitchSource {
    /// A source for `features` (resolved from the signed remote config and
    /// the build flavor). Downloads ask usher for H.264 only, the codec
    /// every device of the household decodes.
    pub fn new(
        features: TwitchFeatures,
        helix: HelixAccess,
        transport: Arc<dyn Transport>,
    ) -> std::io::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("pa-twitch-archive")
            .enable_all()
            .build()?;
        Ok(Self {
            rt,
            playback: TwitchPlayback::new(features.clone(), Arc::clone(&transport)),
            features,
            helix,
            transport,
            codecs: DeviceCodecs::H264_ONLY,
            chapters: None,
            chat_replay: None,
        })
    }

    /// Overrides the GQL endpoint and usher base (the fake Twitch of tests).
    pub fn with_endpoints(mut self, gql_url: &str, usher_base: &str) -> Self {
        self.playback =
            TwitchPlayback::new(self.features.clone(), Arc::clone(&self.transport))
                .with_endpoints(gql_url, usher_base);
        self
    }

    /// Codecs to ask usher for.
    pub fn with_codecs(mut self, codecs: DeviceCodecs) -> Self {
        self.codecs = codecs;
        self
    }

    /// Where chapters come from.
    pub fn with_chapters(mut self, f: Box<ChaptersFn>) -> Self {
        self.chapters = Some(f);
        self
    }

    /// The chat replay hook (TWI-016). Called only when the remote config
    /// allows chat replay (`twitch.chat_replay`).
    pub fn with_chat_replay(mut self, f: Box<ChatReplayFn>) -> Self {
        self.chat_replay = Some(f);
        self
    }

    pub fn features(&self) -> &TwitchFeatures {
        &self.features
    }

    fn helix<T, F, Fut>(&self, op: F) -> Result<T, ArchiveError>
    where
        F: Fn(Arc<HelixClient>) -> Fut,
        Fut: std::future::Future<Output = pa_twitch::Result<T>>,
    {
        match &self.helix {
            HelixAccess::Client(c) => Ok(self.rt.block_on(op(Arc::clone(c)))?),
            HelixAccess::Authorized(a) => Ok(self.rt.block_on(a.call(op))?),
        }
    }
}

impl ArchiveSource for TwitchSource {
    fn check_allowed(&self) -> Result<(), ArchiveError> {
        if !self.features.archive {
            return Err(ArchiveError::fatal(
                "archive_disabled",
                "Twitch downloads are turned off (build flavor or remote config twitch.archive)",
            ));
        }
        if let PlayerChoice::Embed(reason) = &self.features.player {
            return Err(ArchiveError::fatal(
                "native_off",
                format!("native Twitch playback is off ({reason:?}); downloads need it"),
            ));
        }
        Ok(())
    }

    fn video(&self, video_id: &str) -> Result<Video, ArchiveError> {
        let id = video_id.to_owned();
        let page = self.helix(|h| {
            let q = VideosQuery::by_ids([id.clone()]);
            async move { h.get_videos(&q).await }
        })?;
        page.items
            .into_iter()
            .find(|v| v.id == video_id)
            .ok_or_else(|| ArchiveError::fatal("not_found", format!("no VOD {video_id} on Twitch")))
    }

    fn avatar_url(&self, user_id: &str) -> Result<Option<String>, ArchiveError> {
        let id = user_id.to_owned();
        let users = self.helix(|h| {
            let id = id.clone();
            async move { h.get_users(&[id.as_str()], &[]).await }
        })?;
        Ok(users
            .into_iter()
            .find(|u| u.id == user_id)
            .map(|u| u.profile_image_url)
            .filter(|u| !u.is_empty()))
    }

    fn chapters(&self, video: &Video) -> Vec<Chapter> {
        self.chapters.as_ref().map_or_else(Vec::new, |f| f(video))
    }

    fn master_playlist(&self, video_id: &str) -> Result<MasterPlaylist, ArchiveError> {
        let target = PlaybackTarget::vod(video_id)?;
        match self.rt.block_on(self.playback.resolve(&target, self.codecs))? {
            PlaybackDecision::Native(n) => Ok(n.playlist),
            PlaybackDecision::Embed(e) => Err(ArchiveError::fatal(
                "native_off",
                format!("native Twitch playback is off ({:?})", e.reason),
            )),
        }
    }

    fn get(&self, url: &str) -> Result<HttpResponse, ArchiveError> {
        let request = HttpRequest {
            method: Method::Get,
            url: url.to_owned(),
            headers: Vec::new(),
            body: None,
        };
        self.rt
            .block_on(self.transport.send(request))
            .map_err(|e| ArchiveError::retry("network", e.0))
    }

    fn chat_replay(&self, video: &Video) -> Result<Option<Vec<u8>>, ArchiveError> {
        match &self.chat_replay {
            Some(f) if self.features.chat_replay => f(video),
            _ => Ok(None),
        }
    }
}
