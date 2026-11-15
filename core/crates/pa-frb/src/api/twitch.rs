// SPDX-License-Identifier: MPL-2.0
//! The "Streams" section (Twitch, TWI-013) for Dart: Helix data
//! (`pa-twitch`, TWI-001), Device Code Flow sign-in (TWI-002) and the
//! Twitch model of the library (`pa-library::twitch`, TWI-007) with VOD
//! chapters (TWI-008).
//!
//! # Without a Twitch login (DEC-006, ADR-0011 §5)
//!
//! Helix answers only with a token, and the project has no registered
//! client id yet. So [`CoreTwitch`] works in three steps:
//!
//! 1. **No client id** ([`CoreTwitch::open`] with `None`): local data only —
//!    cached streamers, VODs and chapters, favourites, progress, search of
//!    the cache. The app keeps favourites added by channel name itself
//!    (by login, ADR-NNNN) and resolves them once Helix is reachable.
//! 2. **Client id, signed out**: [`CoreTwitch::start_login`] and
//!    [`CoreTwitch::poll_login`] (QR and `user_code`).
//! 3. **Signed in**: every Helix call below; answers are also written into
//!    the library cache (24-hour cap, [`pa_library::twitch::purge_expired`]).
//!
//! Network calls run on a small tokio runtime owned by the object (Helix
//! over reqwest needs one); FRB calls them on its worker threads, so they
//! are futures in Dart.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use flutter_rust_bridge::frb;
use pa_library::progress::{self, ContinueQuery, LocalOnly, Thresholds};
use pa_library::rusqlite::Connection;
use pa_library::twitch as lt;
use pa_model::{BroadcasterType, MediaKey, MediaKind, VideoType};
use pa_twitch as tw;
use pa_twitch::chapters::store as chapter_store;

/// The library profile (one profile until profiles land, as `pa-catalog`).
const PROFILE: i64 = 1;

/// File in the data dir with the user id of the signed-in account (not a
/// secret: the tokens are in the secret store).
const ACCOUNT_FILE: &str = "twitch-account";

/// What went wrong; Dart receives it as an exception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwitchErrorKind {
    /// No Twitch client id in this build: only local data.
    NotConfigured,
    /// The call needs a Twitch sign-in.
    SignedOut,
    /// The sign-in expired: sign in again.
    ReauthRequired,
    /// The user declined on twitch.tv/activate.
    LoginDenied,
    /// The sign-in code expired before it was confirmed.
    LoginExpired,
    /// No network or Twitch unreachable.
    Network,
    /// Twitch rate limit; try later.
    RateLimited,
    /// Twitch answered with an error.
    Twitch,
    /// The local database failed.
    Storage,
    /// Bad argument (not a Twitch id, empty query).
    Invalid,
    /// The active profile is a kid or restricted profile: no Twitch
    /// notifications for it (TWI-009, UI-015).
    Restricted,
}

/// Error of a Twitch call (no tokens in the message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwitchApiError {
    pub kind: TwitchErrorKind,
    pub message: String,
}

impl TwitchApiError {
    fn new(kind: TwitchErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl From<pa_library::Error> for TwitchApiError {
    fn from(e: pa_library::Error) -> Self {
        let kind = match e {
            pa_library::Error::Invalid(_) => TwitchErrorKind::Invalid,
            _ => TwitchErrorKind::Storage,
        };
        Self::new(kind, e.to_string())
    }
}

impl From<tw::Error> for TwitchApiError {
    fn from(e: tw::Error) -> Self {
        let kind = match &e {
            tw::Error::Transport(_) => TwitchErrorKind::Network,
            tw::Error::MissingToken | tw::Error::Unauthorized { .. } => TwitchErrorKind::SignedOut,
            tw::Error::RateLimited { .. } => TwitchErrorKind::RateLimited,
            tw::Error::InvalidArgument(_) => TwitchErrorKind::Invalid,
            tw::Error::Api { .. } | tw::Error::Decode(_) => TwitchErrorKind::Twitch,
        };
        Self::new(kind, e.to_string())
    }
}

impl From<tw::AuthError> for TwitchApiError {
    fn from(e: tw::AuthError) -> Self {
        let kind = match &e {
            tw::AuthError::Transport(_) => TwitchErrorKind::Network,
            tw::AuthError::NotSignedIn => TwitchErrorKind::SignedOut,
            tw::AuthError::ReauthRequired => TwitchErrorKind::ReauthRequired,
            tw::AuthError::AccessDenied => TwitchErrorKind::LoginDenied,
            tw::AuthError::DeviceCodeExpired => TwitchErrorKind::LoginExpired,
            tw::AuthError::Helix(inner) => return inner.clone().into(),
            tw::AuthError::Twitch { .. } | tw::AuthError::Decode(_) => TwitchErrorKind::Twitch,
            tw::AuthError::Secrets(_) => TwitchErrorKind::Storage,
        };
        Self::new(kind, e.to_string())
    }
}

type TwitchResult<T> = std::result::Result<T, TwitchApiError>;

/// A channel: profile from Helix or the library cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTwitchUser {
    pub user_id: String,
    /// Lower-case login (`twitch.tv/<login>`).
    pub login: String,
    pub display_name: String,
    /// `partner`, `affiliate` or `normal` (decides VOD retention).
    pub broadcaster_type: String,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub offline_banner_url: Option<String>,
    /// Unix ms of the last broadcast we saw.
    pub last_live_at_ms: Option<i64>,
}

/// A live stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTwitchStream {
    pub stream_id: String,
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    pub game_id: Option<String>,
    pub game_name: Option<String>,
    pub title: String,
    pub viewer_count: i64,
    pub started_at_ms: Option<i64>,
    /// Preview frame template with `{width}`/`{height}`.
    pub thumbnail_url: Option<String>,
}

/// A VOD (archive, highlight or upload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTwitchVideo {
    pub video_id: String,
    pub user_id: String,
    /// Empty when only the library cache knows the VOD.
    pub user_login: String,
    pub user_name: String,
    /// `archive`, `highlight` or `upload`.
    pub video_type: String,
    pub title: String,
    pub published_at_ms: i64,
    pub duration_ms: i64,
    pub view_count: i64,
    /// Template with `%{width}`/`%{height}`.
    pub thumbnail_url: Option<String>,
    /// Subscribers only (known from a 403 of the playback path or GQL).
    pub is_sub_only: Option<bool>,
    /// When Twitch deletes it (archives only).
    pub expires_at_ms: Option<i64>,
    /// Saved position under `twitch:vod:<id>`.
    pub position_ms: Option<i64>,
    pub progress_updated_at_ms: Option<i64>,
    pub watched: bool,
    /// Downloads and recordings of it (TWI-017+).
    pub saved_files: i64,
}

/// A chapter: the category on air between two offsets of a VOD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTwitchChapter {
    pub game_id: String,
    pub game_name: Option<String>,
    /// Template with `{width}`/`{height}`.
    pub box_art_url: Option<String>,
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    /// The channel's current category, not a timeline ("≈").
    pub approximate: bool,
}

/// A clip.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreTwitchClip {
    pub slug: String,
    pub title: String,
    pub creator_name: Option<String>,
    pub view_count: i64,
    pub created_at_ms: Option<i64>,
    pub duration_secs: f64,
    pub thumbnail_url: Option<String>,
    pub game_id: Option<String>,
}

/// A scheduled broadcast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTwitchScheduleSegment {
    pub id: String,
    pub title: String,
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    pub category_id: Option<String>,
    pub category_name: Option<String>,
    pub canceled: bool,
    pub recurring: bool,
}

/// A category (game).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTwitchCategory {
    pub game_id: String,
    pub name: String,
    /// Template with `{width}`/`{height}`.
    pub box_art_url: Option<String>,
}

/// A favourite channel stored in the library (synced by `pa-sync`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTwitchFavorite {
    pub user_id: String,
    /// `local` or `twitch_follow`.
    pub source: String,
    pub notify: bool,
    pub added_at_ms: i64,
    pub user: Option<CoreTwitchUser>,
}

/// A cached search hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTwitchSearchHit {
    /// `streamer`, `video`, `category`, `clip` or `live`.
    pub kind: String,
    pub twitch_id: String,
    pub text: String,
}

/// A started VOD for the Continue rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTwitchProgress {
    pub video_id: String,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub updated_at_ms: i64,
    /// Cached metadata; `None` after the 24-hour cache dropped it.
    pub video: Option<CoreTwitchVideo>,
    pub user: Option<CoreTwitchUser>,
}

/// The signed-in account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTwitchAccount {
    pub user_id: String,
    pub login: String,
}

/// A pending Device Code Flow sign-in: show `user_code` and a QR code of
/// `verification_uri`, then call [`CoreTwitch::poll_login`] every
/// `interval_secs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTwitchDeviceLogin {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in_secs: i64,
    pub interval_secs: i64,
}

enum Api {
    Signed(tw::AuthorizedHelix),
    /// A client with a fixed token (tests).
    Plain(Arc<tw::HelixClient>),
}

/// The Twitch section of one library database.
#[frb(opaque)]
pub struct CoreTwitch {
    conn: Mutex<Connection>,
    rt: tokio::runtime::Runtime,
    api: Option<Api>,
    pending: Mutex<Option<tw::DeviceAuthorization>>,
    data_dir: Option<PathBuf>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Dart has no unsigned 64-bit int: counts and offsets go as `i64`.
fn signed(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}

fn runtime() -> TwitchResult<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| TwitchApiError::new(TwitchErrorKind::Storage, e.to_string()))
}

fn broadcaster_type(s: &str) -> BroadcasterType {
    match s {
        "partner" => BroadcasterType::Partner,
        "affiliate" => BroadcasterType::Affiliate,
        _ => BroadcasterType::Normal,
    }
}

impl From<pa_model::Streamer> for CoreTwitchUser {
    fn from(s: pa_model::Streamer) -> Self {
        Self {
            broadcaster_type: lt::broadcaster_type_str(s.broadcaster_type).to_owned(),
            user_id: s.user_id,
            login: s.login,
            display_name: s.display_name,
            description: s.description,
            avatar_url: s.avatar_url,
            offline_banner_url: s.offline_banner_url,
            last_live_at_ms: s.last_seen_live_at_ms,
        }
    }
}

fn model_streamer(u: &tw::User, now: i64) -> pa_model::Streamer {
    pa_model::Streamer {
        user_id: u.id.clone(),
        login: u.login.clone(),
        display_name: u.display_name.clone(),
        broadcaster_type: broadcaster_type(&u.broadcaster_type),
        description: non_empty(&u.description),
        avatar_url: non_empty(&u.profile_image_url),
        offline_banner_url: non_empty(&u.offline_image_url),
        last_seen_live_at_ms: None,
        updated_at_ms: now,
    }
}

fn stream_of(s: &tw::Stream) -> CoreTwitchStream {
    CoreTwitchStream {
        stream_id: s.id.clone(),
        user_id: s.user_id.clone(),
        user_login: s.user_login.clone(),
        user_name: s.user_name.clone(),
        game_id: non_empty(&s.game_id),
        game_name: non_empty(&s.game_name),
        title: s.title.clone(),
        viewer_count: signed(s.viewer_count),
        started_at_ms: tw::parse_rfc3339_ms(&s.started_at),
        thumbnail_url: non_empty(&s.thumbnail_url),
    }
}

fn category_of(c: &tw::Category) -> CoreTwitchCategory {
    CoreTwitchCategory {
        game_id: c.id.clone(),
        name: c.name.clone(),
        box_art_url: non_empty(&c.box_art_url),
    }
}

impl CoreTwitch {
    /// Opens the Twitch part of the library database at `db_path` (the
    /// same file as `CoreLibrary.open`). With `client_id` and `data_dir`
    /// Helix and sign-in are available and a stored sign-in is restored
    /// (no network); without them only local data is.
    pub fn open(
        db_path: String,
        data_dir: Option<String>,
        client_id: Option<String>,
    ) -> TwitchResult<CoreTwitch> {
        let conn = pa_library::open(&db_path)?;
        let rt = runtime()?;
        let data_dir = data_dir.map(PathBuf::from);
        let api = match (client_id.filter(|c| !c.is_empty()), &data_dir) {
            (Some(id), Some(dir)) => Some(Self::signed_api(&rt, &id, dir)?),
            _ => None,
        };
        Ok(Self {
            conn: Mutex::new(conn),
            rt,
            api,
            pending: Mutex::new(None),
            data_dir,
        })
    }

    fn signed_api(rt: &tokio::runtime::Runtime, client_id: &str, dir: &Path) -> TwitchResult<Api> {
        let id = tw::ClientId::new(client_id)?;
        let opened = crate::shared::secret_store(dir)
            .map_err(|e| TwitchApiError::new(TwitchErrorKind::Storage, e.to_string()))?;
        let auth =
            tw::TwitchAuth::with_default_transport(tw::AuthConfig::new(id.clone()), opened.store)?;
        let helix = tw::HelixClient::with_default_transport(tw::HelixConfig::new(id))?;
        let auth = Arc::new(auth);
        if let Ok(user_id) = std::fs::read_to_string(dir.join(ACCOUNT_FILE)) {
            // A missing or unreadable token only means "signed out".
            let _ = rt.block_on(auth.restore(user_id.trim()));
        }
        Ok(Api::Signed(tw::AuthorizedHelix::new(auth, Arc::new(helix))))
    }

    /// Opens with a Helix client that already has a token (tests).
    #[frb(ignore)]
    pub fn with_helix(
        conn: Connection,
        helix: Option<Arc<tw::HelixClient>>,
    ) -> TwitchResult<CoreTwitch> {
        Ok(Self {
            conn: Mutex::new(conn),
            rt: runtime()?,
            api: helix.map(Api::Plain),
            pending: Mutex::new(None),
            data_dir: None,
        })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn call<T, F, Fut>(&self, op: F) -> TwitchResult<T>
    where
        F: Fn(Arc<tw::HelixClient>) -> Fut,
        Fut: std::future::Future<Output = tw::Result<T>>,
    {
        match &self.api {
            None => Err(TwitchApiError::new(
                TwitchErrorKind::NotConfigured,
                "no Twitch client id in this build",
            )),
            Some(Api::Plain(helix)) => Ok(self.rt.block_on(op(helix.clone()))?),
            Some(Api::Signed(api)) => {
                if api.auth().account().is_none() {
                    return Err(TwitchApiError::new(
                        TwitchErrorKind::SignedOut,
                        "not signed in to Twitch",
                    ));
                }
                Ok(self.rt.block_on(api.call(op))?)
            }
        }
    }

    // ----- sign-in ---------------------------------------------------------

    /// Whether Helix is configured (a client id): sign-in is possible.
    #[frb(sync)]
    pub fn can_sign_in(&self) -> bool {
        self.api.is_some()
    }

    /// The signed-in account, if any (no network).
    #[frb(sync)]
    pub fn account(&self) -> Option<CoreTwitchAccount> {
        match &self.api {
            Some(Api::Signed(api)) => api.auth().account().map(|a| CoreTwitchAccount {
                user_id: a.user_id,
                login: a.login,
            }),
            _ => None,
        }
    }

    /// Starts a Device Code Flow sign-in.
    pub fn start_login(&self) -> TwitchResult<CoreTwitchDeviceLogin> {
        self.start_login_with(&[])
    }

    /// Starts a sign-in that also asks for `user:write:chat`: the user
    /// turned on sending chat messages (TWI-024).
    pub fn start_chat_login(&self) -> TwitchResult<CoreTwitchDeviceLogin> {
        self.start_login_with(&[tw::SCOPE_WRITE_CHAT])
    }

    /// The signed-in account may send chat messages (`user:write:chat`
    /// granted). No network.
    #[frb(sync)]
    pub fn can_send_chat(&self) -> bool {
        match &self.api {
            Some(Api::Signed(api)) => api
                .auth()
                .account()
                .is_some_and(|a| a.scopes.iter().any(|s| s == tw::SCOPE_WRITE_CHAT)),
            _ => false,
        }
    }

    /// Helix for the signed-in chat (TWI-024): reading through EventSub,
    /// sending, the channel's emotes. Needs a sign-in with
    /// `user:read:chat`.
    #[frb(ignore)]
    pub(crate) fn chat_access(&self) -> TwitchResult<super::twitch_chat::ChatAccess> {
        let Some(Api::Signed(api)) = &self.api else {
            return Err(TwitchApiError::new(
                TwitchErrorKind::NotConfigured,
                "no Twitch client id in this build",
            ));
        };
        let Some(account) = api.auth().account() else {
            return Err(TwitchApiError::new(
                TwitchErrorKind::SignedOut,
                "not signed in to Twitch",
            ));
        };
        if !account.scopes.iter().any(|s| s == tw::SCOPE_READ_CHAT) {
            return Err(TwitchApiError::new(
                TwitchErrorKind::ReauthRequired,
                "the sign-in does not allow reading chat",
            ));
        }
        let api = Arc::new(api.clone());
        Ok(super::twitch_chat::ChatAccess {
            chat: api.clone(),
            emotes: api,
            can_send: account.scopes.iter().any(|s| s == tw::SCOPE_WRITE_CHAT),
            user_id: account.user_id,
            login: account.login,
        })
    }

    fn start_login_with(&self, extra: &[&str]) -> TwitchResult<CoreTwitchDeviceLogin> {
        let Some(Api::Signed(api)) = &self.api else {
            return Err(TwitchApiError::new(
                TwitchErrorKind::NotConfigured,
                "no Twitch client id in this build",
            ));
        };
        let device = self
            .rt
            .block_on(api.auth().start_device_login_with(extra))?;
        let expires_in_secs = device
            .expires_at
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO)
            .as_secs();
        let out = CoreTwitchDeviceLogin {
            user_code: device.user_code.clone(),
            verification_uri: device.verification_uri.clone(),
            expires_in_secs: signed(expires_in_secs),
            interval_secs: signed(device.interval.as_secs().max(1)),
        };
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(device);
        Ok(out)
    }

    /// Polls the pending sign-in once: the account once the user
    /// confirmed, `None` while waiting.
    pub fn poll_login(&self) -> TwitchResult<Option<CoreTwitchAccount>> {
        let Some(Api::Signed(api)) = &self.api else {
            return Err(TwitchApiError::new(
                TwitchErrorKind::NotConfigured,
                "no client id",
            ));
        };
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let Some(device) = pending.as_mut() else {
            return Err(TwitchApiError::new(
                TwitchErrorKind::LoginExpired,
                "no sign-in started",
            ));
        };
        let result = self.rt.block_on(api.auth().poll_device_login(device));
        match result {
            Ok(None) => Ok(None),
            Ok(Some(account)) => {
                *pending = None;
                if let Some(dir) = &self.data_dir {
                    let _ = std::fs::write(dir.join(ACCOUNT_FILE), &account.user_id);
                }
                Ok(Some(CoreTwitchAccount {
                    user_id: account.user_id,
                    login: account.login,
                }))
            }
            Err(e) => {
                *pending = None;
                Err(e.into())
            }
        }
    }

    /// Signs out (revokes the token) and forgets the account.
    pub fn sign_out(&self) -> TwitchResult<()> {
        if let Some(Api::Signed(api)) = &self.api {
            let result = self.rt.block_on(api.sign_out());
            if let Some(dir) = &self.data_dir {
                let _ = std::fs::remove_file(dir.join(ACCOUNT_FILE));
            }
            result?;
        }
        Ok(())
    }

    // ----- Helix (written into the library cache) ---------------------------

    /// Profiles by ids and logins (resolves favourites added by name).
    pub fn users(
        &self,
        ids: Vec<String>,
        logins: Vec<String>,
    ) -> TwitchResult<Vec<CoreTwitchUser>> {
        let users = self.call(|h| {
            let (ids, logins) = (ids.clone(), logins.clone());
            async move {
                let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
                let logins: Vec<&str> = logins.iter().map(String::as_str).collect();
                h.get_users(&ids, &logins).await
            }
        })?;
        let now = now_ms();
        let conn = self.db();
        let mut out = Vec::new();
        for u in &users {
            let model = model_streamer(u, now);
            // Keep the "last live" time the cache already has.
            let _ = lt::upsert_streamer(&conn, &model, now);
            let cached = lt::streamer(&conn, &u.id).ok().flatten();
            out.push(cached.unwrap_or(model).into());
        }
        Ok(out)
    }

    /// Live streams of these channels (≤ 100 per call).
    pub fn live_streams(&self, user_ids: Vec<String>) -> TwitchResult<Vec<CoreTwitchStream>> {
        if user_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for chunk in user_ids.chunks(100) {
            let q = tw::StreamsQuery {
                user_ids: chunk.to_vec(),
                first: Some(100),
                ..tw::StreamsQuery::default()
            };
            let page = self.call(|h| {
                let q = q.clone();
                async move { h.get_streams(&q).await }
            })?;
            out.extend(page.items.iter().map(stream_of));
        }
        let now = now_ms();
        let conn = self.db();
        for s in &out {
            if let Some(started) = s.started_at_ms {
                let _ = lt::upsert_live_session(
                    &conn,
                    &lt::LiveSession {
                        stream_id: s.stream_id.clone(),
                        user_id: s.user_id.clone(),
                        started_at_ms: started,
                        ended_at_ms: None,
                        title_last: Some(s.title.clone()),
                        peak_viewers: u64::try_from(s.viewer_count).ok(),
                        video_id: None,
                    },
                    now,
                );
                let _ = conn.execute(
                    "UPDATE twitch_streamers SET last_seen_live_at_ms = ?2 WHERE user_id = ?1",
                    pa_library::rusqlite::params![s.user_id, now],
                );
            }
        }
        Ok(out)
    }

    /// Channels the signed-in user follows, with profiles.
    pub fn followed(&self) -> TwitchResult<Vec<CoreTwitchUser>> {
        let Some(account) = self.account() else {
            return Err(TwitchApiError::new(
                TwitchErrorKind::SignedOut,
                "not signed in",
            ));
        };
        let followed = self.call(|h| {
            let user_id = account.user_id.clone();
            async move { h.followed_overview(&user_id, 500).await }
        })?;
        let now = now_ms();
        let conn = self.db();
        let mut out = Vec::new();
        for f in followed {
            let Some(user) = f.user else { continue };
            let model = model_streamer(&user, now);
            let _ = lt::upsert_streamer(&conn, &model, now);
            out.push(model.into());
        }
        Ok(out)
    }

    /// VODs of a channel from Helix (`archive`, `highlight` or `upload`;
    /// empty = all), stored in the cache and returned with progress,
    /// expiry and saved files.
    pub fn channel_videos(
        &self,
        user_id: String,
        video_type: String,
        limit: u32,
    ) -> TwitchResult<Vec<CoreTwitchVideo>> {
        let kind = match video_type.as_str() {
            "archive" => Some(tw::VideoType::Archive),
            "highlight" => Some(tw::VideoType::Highlight),
            "upload" => Some(tw::VideoType::Upload),
            _ => None,
        };
        let q = tw::VideosQuery {
            video_type: kind,
            first: Some(limit.clamp(1, 100)),
            ..tw::VideosQuery::by_user(user_id.clone())
        };
        let page = self.call(|h| {
            let q = q.clone();
            async move { h.get_videos(&q).await }
        })?;
        let now = now_ms();
        let conn = self.db();
        let mut ids = Vec::new();
        for v in &page.items {
            if chapter_store::store_video(&conn, v, now)? {
                ids.push(v.id.clone());
            }
        }
        let names: std::collections::HashMap<&str, (&str, &str)> = page
            .items
            .iter()
            .map(|v| (v.id.as_str(), (v.user_login.as_str(), v.user_name.as_str())))
            .collect();
        let all = lt::channel_videos(&conn, PROFILE, &user_id, &lt::VideoQuery::default())?;
        Ok(all
            .into_iter()
            .filter(|e| ids.contains(&e.video.video_id))
            .map(|e| {
                let (login, name) = names
                    .get(e.video.video_id.as_str())
                    .copied()
                    .unwrap_or(("", ""));
                video_of(e, login, name)
            })
            .collect())
    }

    /// Clips of a channel, most viewed first.
    pub fn clips(&self, user_id: String, limit: u32) -> TwitchResult<Vec<CoreTwitchClip>> {
        let q = tw::ClipsQuery {
            broadcaster_id: Some(user_id),
            first: Some(limit.clamp(1, 100)),
            ..tw::ClipsQuery::default()
        };
        let page = self.call(|h| {
            let q = q.clone();
            async move { h.get_clips(&q).await }
        })?;
        Ok(page
            .items
            .into_iter()
            .map(|c| CoreTwitchClip {
                created_at_ms: tw::parse_rfc3339_ms(&c.created_at),
                creator_name: non_empty(&c.creator_name),
                thumbnail_url: non_empty(&c.thumbnail_url),
                game_id: non_empty(&c.game_id),
                slug: c.id,
                title: c.title,
                view_count: signed(c.view_count),
                duration_secs: c.duration,
            })
            .collect())
    }

    /// Upcoming broadcasts of a channel (empty without a schedule).
    pub fn schedule(&self, user_id: String) -> TwitchResult<Vec<CoreTwitchScheduleSegment>> {
        let page = self.call(|h| {
            let id = user_id.clone();
            async move { h.get_schedule(&id, Some(25), None).await }
        })?;
        let Some(page) = page else {
            return Ok(Vec::new());
        };
        Ok(page
            .schedule
            .segments
            .into_iter()
            .filter_map(|s| {
                Some(CoreTwitchScheduleSegment {
                    start_ms: tw::parse_rfc3339_ms(&s.start_time)?,
                    end_ms: s.end_time.as_deref().and_then(tw::parse_rfc3339_ms),
                    canceled: s.canceled_until.is_some(),
                    recurring: s.is_recurring,
                    category_id: s.category.as_ref().map(|c| c.id.clone()),
                    category_name: s.category.map(|c| c.name),
                    id: s.id,
                    title: s.title,
                })
            })
            .collect())
    }

    /// Channel search on Twitch.
    pub fn search_channels(&self, query: String, limit: u32) -> TwitchResult<Vec<CoreTwitchUser>> {
        let q = tw::SearchChannelsQuery {
            query,
            live_only: false,
            first: Some(limit.clamp(1, 100)),
            after: None,
        };
        let page = self.call(|h| {
            let q = q.clone();
            async move { h.search_channels(&q).await }
        })?;
        // Search results carry no avatar: fetch the profiles (cached 24 h).
        let ids: Vec<String> = page.items.iter().map(|c| c.id.clone()).collect();
        let mut users = self.users(ids.clone(), Vec::new())?;
        users.sort_by_key(|u| ids.iter().position(|i| *i == u.user_id));
        Ok(users)
    }

    /// Category search on Twitch.
    pub fn search_categories(
        &self,
        query: String,
        limit: u32,
    ) -> TwitchResult<Vec<CoreTwitchCategory>> {
        let page = self.call(|h| {
            let q = query.clone();
            async move {
                h.search_categories(&q, Some(limit.clamp(1, 100)), None)
                    .await
            }
        })?;
        let now = now_ms();
        let conn = self.db();
        for c in &page.items {
            let _ = lt::upsert_category(
                &conn,
                &lt::Category {
                    game_id: c.id.clone(),
                    name: c.name.clone(),
                    box_art_url: non_empty(&c.box_art_url),
                    igdb_id: c.igdb_id.clone(),
                },
                now,
            );
        }
        Ok(page.items.iter().map(category_of).collect())
    }

    // ----- local (no network) --------------------------------------------------

    /// A cached channel.
    #[frb(sync)]
    pub fn cached_user(&self, user_id: String) -> TwitchResult<Option<CoreTwitchUser>> {
        Ok(lt::streamer(&self.db(), &user_id)?.map(Into::into))
    }

    /// Favourites stored in the library (synced between devices).
    pub fn favorites(&self) -> TwitchResult<Vec<CoreTwitchFavorite>> {
        Ok(lt::favorites(&self.db(), PROFILE)?
            .into_iter()
            .map(|f| CoreTwitchFavorite {
                user_id: f.user_id,
                source: f.source.as_str().to_owned(),
                notify: f.notify,
                added_at_ms: f.added_at_ms,
                user: f.streamer.map(Into::into),
            })
            .collect())
    }

    /// Adds or removes a library favourite (a resolved channel). With
    /// `user` its profile is cached first, so the favourite keeps a name
    /// and avatar after the 24-hour cache expires.
    pub fn set_favorite(
        &self,
        user_id: String,
        favorite: bool,
        user: Option<CoreTwitchUser>,
    ) -> TwitchResult<bool> {
        let now = now_ms();
        let mut conn = self.db();
        if let Some(u) = user.filter(|u| u.user_id == user_id) {
            lt::upsert_streamer(
                &conn,
                &pa_model::Streamer {
                    user_id: u.user_id,
                    login: u.login,
                    display_name: u.display_name,
                    broadcaster_type: broadcaster_type(&u.broadcaster_type),
                    description: u.description,
                    avatar_url: u.avatar_url,
                    offline_banner_url: u.offline_banner_url,
                    last_seen_live_at_ms: u.last_live_at_ms,
                    updated_at_ms: now,
                },
                now,
            )?;
        }
        let mut sink = LocalOnly::new("local");
        Ok(lt::set_favorite(
            &mut conn,
            &mut sink,
            PROFILE,
            &user_id,
            lt::FavoriteSource::Local,
            favorite,
            now,
        )?)
    }

    /// Cached VODs of a channel (newest first) with progress and expiry.
    pub fn cached_videos(&self, user_id: String, limit: u32) -> TwitchResult<Vec<CoreTwitchVideo>> {
        let q = lt::VideoQuery {
            limit: limit as usize,
            ..lt::VideoQuery::default()
        };
        let conn = self.db();
        let login = lt::streamer(&conn, &user_id)?;
        let (login, name) = login.map_or((String::new(), String::new()), |s| {
            (s.login, s.display_name)
        });
        Ok(lt::channel_videos(&conn, PROFILE, &user_id, &q)?
            .into_iter()
            .map(|e| video_of(e, &login, &name))
            .collect())
    }

    /// Cached VODs of every channel with a chapter in `game_id` (TWI-008).
    pub fn category_videos(
        &self,
        game_id: String,
        limit: u32,
    ) -> TwitchResult<Vec<CoreTwitchVideo>> {
        let q = lt::VideoQuery {
            limit: limit as usize,
            ..lt::VideoQuery::default()
        };
        let conn = self.db();
        let entries = lt::category_videos(&conn, PROFILE, &game_id, &q)?;
        Ok(entries
            .into_iter()
            .map(|e| {
                let s = lt::streamer(&conn, &e.video.user_id).ok().flatten();
                let (login, name) = s.map_or((String::new(), String::new()), |s| {
                    (s.login, s.display_name)
                });
                video_of(e, &login, &name)
            })
            .collect())
    }

    /// Stored chapters of a VOD with category names and box art.
    pub fn chapters(&self, video_id: String) -> TwitchResult<Vec<CoreTwitchChapter>> {
        let conn = self.db();
        let segments = lt::segments(&conn, &lt::SegmentOwner::Video(video_id))?;
        let mut out = Vec::new();
        for s in segments {
            let cat = lt::category(&conn, &s.game_id)?;
            out.push(CoreTwitchChapter {
                approximate: s.source == tw::chapters::ChapterSource::Fallback.as_str(),
                game_name: cat.as_ref().map(|c| c.name.clone()),
                box_art_url: cat.and_then(|c| c.box_art_url),
                game_id: s.game_id,
                start_ms: signed(s.start_ms),
                end_ms: s.end_ms.map(signed),
            });
        }
        Ok(out)
    }

    /// Full-text search of the cache: streamers, VODs, categories.
    pub fn search_cached(
        &self,
        query: String,
        limit: u32,
    ) -> TwitchResult<Vec<CoreTwitchSearchHit>> {
        Ok(lt::search(&self.db(), &query, limit as usize)?
            .into_iter()
            .map(|h| CoreTwitchSearchHit {
                kind: match h.kind {
                    lt::SearchKind::Streamer => "streamer",
                    lt::SearchKind::Video => "video",
                    lt::SearchKind::Category => "category",
                    lt::SearchKind::Clip => "clip",
                    lt::SearchKind::LiveSession => "live",
                }
                .to_owned(),
                twitch_id: h.twitch_id,
                text: h.text,
            })
            .collect())
    }

    /// Started VODs, latest first: the Twitch part of "Continue watching".
    pub fn continue_vods(&self, limit: u32) -> TwitchResult<Vec<CoreTwitchProgress>> {
        let conn = self.db();
        let entries = progress::continue_watching(
            &conn,
            &Thresholds::default(),
            PROFILE,
            &ContinueQuery {
                kinds: vec![MediaKind::TwitchVod],
                up_next: false,
                limit: limit as usize,
            },
        )?;
        let mut out = Vec::new();
        for e in entries {
            let MediaKey::TwitchVod { video_id } = &e.item.media_key else {
                continue;
            };
            let cached = lt::video(&conn, video_id)?;
            let user = match &cached {
                Some(v) => lt::streamer(&conn, &v.user_id)?,
                None => None,
            };
            let video = cached.map(|v| {
                let bt = user
                    .as_ref()
                    .map_or(BroadcasterType::Normal, |s| s.broadcaster_type);
                let (login, name) = user.as_ref().map_or((String::new(), String::new()), |s| {
                    (s.login.clone(), s.display_name.clone())
                });
                video_of(
                    lt::VideoEntry {
                        expires_at_ms: lt::vod_expires_at_ms(v.video_type, bt, v.published_at_ms),
                        progress: Some(e.item.clone()),
                        saved_files: 0,
                        video: v,
                    },
                    &login,
                    &name,
                )
            });
            out.push(CoreTwitchProgress {
                video_id: video_id.clone(),
                position_ms: signed(e.item.position_ms),
                duration_ms: signed(e.item.duration_ms.unwrap_or(0)),
                updated_at_ms: e.item.updated_at_ms,
                video,
                user: user.map(Into::into),
            });
        }
        Ok(out)
    }

    // ----- "X is live" (TWI-009) ----------------------------------------------

    /// Turns "X is live" notifications of a library favourite of the
    /// active profile on or off (local to this device). False if it is not
    /// a favourite.
    pub fn set_favorite_notify(&self, user_id: String, notify: bool) -> TwitchResult<bool> {
        let profile = self.live_profile()?;
        Ok(lt::set_notify(&self.db(), profile.id, &user_id, notify)?)
    }

    /// The profile in use now (LIB-012): the most recently switched-to one,
    /// re-read on every call because profiles are switched on another
    /// connection.
    fn live_profile(&self) -> TwitchResult<pa_library::profiles::Profile> {
        Ok(pa_library::profiles::activate_recent(&self.db())?)
    }

    /// Whether the active profile may get "X is live" notifications: not a
    /// kid profile and no parental filter (UI-015 hides Twitch for them).
    #[frb(sync)]
    pub fn live_notifications_allowed(&self) -> TwitchResult<bool> {
        let p = self.live_profile()?;
        Ok(!p.is_kid && !p.is_restricted())
    }

    fn require_live_allowed(&self) -> TwitchResult<()> {
        if self.live_notifications_allowed()? {
            Ok(())
        } else {
            Err(TwitchApiError::new(
                TwitchErrorKind::Restricted,
                "no Twitch notifications for a restricted profile",
            ))
        }
    }

    /// Helix for the live monitor; needs a sign-in.
    fn live_api(&self) -> TwitchResult<Arc<dyn pa_twitch_live::LiveApi>> {
        match &self.api {
            None => Err(TwitchApiError::new(
                TwitchErrorKind::NotConfigured,
                "no Twitch client id in this build",
            )),
            Some(Api::Plain(helix)) => Ok(helix.clone()),
            Some(Api::Signed(api)) => {
                if api.auth().account().is_none() {
                    return Err(TwitchApiError::new(
                        TwitchErrorKind::SignedOut,
                        "not signed in to Twitch",
                    ));
                }
                Ok(Arc::new(api.clone()))
            }
        }
    }

    /// Helix and the database file for the VOD downloader (TWI-017, full
    /// builds); needs a sign-in.
    #[cfg(all(pa_twitch_archive, pa_twitch_native))]
    pub(crate) fn archive_access(&self) -> TwitchResult<(pa_twitch_archive::HelixAccess, String)> {
        use pa_twitch_archive::HelixAccess;
        let helix = match &self.api {
            None => {
                return Err(TwitchApiError::new(
                    TwitchErrorKind::NotConfigured,
                    "no Twitch client id in this build",
                ));
            }
            Some(Api::Plain(helix)) => HelixAccess::Client(helix.clone()),
            Some(Api::Signed(api)) => {
                if api.auth().account().is_none() {
                    return Err(TwitchApiError::new(
                        TwitchErrorKind::SignedOut,
                        "not signed in to Twitch",
                    ));
                }
                HelixAccess::Authorized(Arc::new(api.clone()))
            }
        };
        let db = self.db_path().ok_or_else(|| {
            TwitchApiError::new(TwitchErrorKind::Storage, "the library is not a file")
        })?;
        Ok((helix, db))
    }

    fn db_path(&self) -> Option<String> {
        self.db()
            .path()
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
    }

    /// The live monitor of the running app (EventSub + polling, TWI-009);
    /// start it with `events`. Needs a sign-in.
    #[frb(sync)]
    pub fn live_monitor(&self) -> TwitchResult<super::twitch_live::CoreTwitchLive> {
        self.require_live_allowed()?;
        Ok(super::twitch_live::CoreTwitchLive::with(
            super::twitch_live::Backend::Twitch {
                api: self.live_api()?,
                db_path: self.db_path(),
            },
        ))
    }

    /// One background check (Android WorkManager): polls `channels` once,
    /// stores the timelines and returns the notifications to show.
    /// `max_age_secs` should cover the time since the previous check.
    pub fn check_live_once(
        &self,
        channels: Vec<super::twitch_live::CoreLiveChannel>,
        max_age_secs: u32,
    ) -> TwitchResult<Vec<super::twitch_live::CoreLiveAlert>> {
        // A kid profile active when the job wakes up: nothing to show.
        if !self.live_notifications_allowed()? {
            return Ok(Vec::new());
        }
        let api = self.live_api()?;
        let db_path = self.db_path();
        super::twitch_live::check_once_blocking(&*api, db_path.as_deref(), channels, max_age_secs)
            .map_err(|e| {
                let kind = match e.code.as_str() {
                    "signed_out" => TwitchErrorKind::SignedOut,
                    "network" => TwitchErrorKind::Network,
                    "io" => TwitchErrorKind::Storage,
                    _ => TwitchErrorKind::Twitch,
                };
                TwitchApiError::new(kind, e.message)
            })
    }

    /// Drops Twitch metadata older than 24 hours (Developer Agreement);
    /// favourites, saved files and progress stay.
    pub fn purge_expired(&self) -> TwitchResult<()> {
        lt::purge_expired(&self.db(), now_ms())?;
        Ok(())
    }
}

fn video_of(e: lt::VideoEntry, login: &str, name: &str) -> CoreTwitchVideo {
    let v = e.video;
    CoreTwitchVideo {
        video_type: match v.video_type {
            VideoType::Archive => "archive",
            VideoType::Highlight => "highlight",
            VideoType::Upload => "upload",
        }
        .to_owned(),
        position_ms: e.progress.as_ref().map(|p| signed(p.position_ms)),
        progress_updated_at_ms: e.progress.as_ref().map(|p| p.updated_at_ms),
        watched: e.progress.as_ref().is_some_and(|p| p.watched),
        expires_at_ms: e.expires_at_ms,
        saved_files: signed(e.saved_files),
        user_login: login.to_owned(),
        user_name: name.to_owned(),
        video_id: v.video_id,
        user_id: v.user_id,
        title: v.title,
        published_at_ms: v.published_at_ms,
        duration_ms: signed(v.duration_ms),
        view_count: signed(v.view_count),
        thumbnail_url: v.thumbnail_url,
        is_sub_only: v.is_sub_only,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pa_twitch::testing::FixtureTransport;

    // Made-up channels; no real Twitch data.
    const USERS: &str = r#"{"status":200,"body":{"data":[{"id":"7001","login":"pixelheron","display_name":"PixelHeron",
        "type":"","broadcaster_type":"affiliate","description":"Cozy games",
        "profile_image_url":"https://img.test/heron.png","offline_image_url":"",
        "view_count":0,"created_at":"2020-01-01T00:00:00Z"}]}}"#;
    const VIDEOS: &str = r#"{"status":200,"body":{"data":[{"id":"9001","stream_id":"5001","user_id":"7001",
        "user_login":"pixelheron","user_name":"PixelHeron","title":"Night run",
        "description":"","created_at":"2026-11-12T18:00:00Z","published_at":"2026-11-12T18:00:00Z",
        "url":"https://www.twitch.tv/videos/9001","thumbnail_url":"https://img.test/%{width}x%{height}.jpg",
        "viewable":"public","view_count":42,"language":"en","type":"archive","duration":"1h2m3s",
        "muted_segments":null}],"pagination":{}}}"#;

    fn core(transport: Arc<FixtureTransport>) -> CoreTwitch {
        let conn = pa_library::open_in_memory().unwrap();
        let mut config =
            tw::HelixConfig::new(tw::ClientId::new("testclientid0000000000000000000").unwrap());
        config.base_url = "https://helix.test/helix".to_owned();
        let helix = tw::HelixClient::new(config, transport);
        helix.set_access_token(tw::AccessToken::new("not-a-real-token"));
        CoreTwitch::with_helix(conn, Some(Arc::new(helix))).unwrap()
    }

    #[test]
    fn kid_profile_gets_no_live_notifications() {
        let core = core(Arc::new(FixtureTransport::new()));
        assert!(core.live_notifications_allowed().unwrap());
        assert!(core.live_monitor().is_ok());
        {
            let conn = core.db();
            let kid = pa_library::profiles::create(
                &conn,
                &pa_library::profiles::ProfileSettings {
                    name: "Made-up kid".into(),
                    is_kid: true,
                    max_age: Some(6),
                    ..Default::default()
                },
                1,
            )
            .unwrap();
            // Switched to on another connection: recorded, then re-read.
            pa_library::profiles::touch(&conn, kid.id, 2).unwrap();
        }
        assert!(!core.live_notifications_allowed().unwrap());
        let err = core.live_monitor().err().expect("refused");
        assert_eq!(err.kind, TwitchErrorKind::Restricted);
        // The background check shows nothing, without a Helix request.
        let channel = super::super::twitch_live::CoreLiveChannel {
            user_id: "81000001".into(),
            login: "made_up".into(),
            display_name: "Made Up".into(),
            notify: true,
        };
        assert!(core.check_live_once(vec![channel], 600).unwrap().is_empty());
    }

    #[test]
    fn local_favorites_work_without_helix() {
        let conn = pa_library::open_in_memory().unwrap();
        let t = CoreTwitch::with_helix(conn, None).unwrap();
        assert!(!t.can_sign_in());
        assert!(t.account().is_none());
        let err = t.users(vec![], vec!["pixelheron".into()]).unwrap_err();
        assert_eq!(err.kind, TwitchErrorKind::NotConfigured);

        let user = CoreTwitchUser {
            user_id: "7001".into(),
            login: "pixelheron".into(),
            display_name: "PixelHeron".into(),
            broadcaster_type: "affiliate".into(),
            description: None,
            avatar_url: None,
            offline_banner_url: None,
            last_live_at_ms: None,
        };
        assert!(t.set_favorite("7001".into(), true, Some(user)).unwrap());
        let favs = t.favorites().unwrap();
        assert_eq!(favs.len(), 1);
        assert_eq!(favs[0].user.as_ref().unwrap().login, "pixelheron");
        let hits = t.search_cached("pixel".into(), 10).unwrap();
        assert_eq!(hits[0].kind, "streamer");
        assert!(t.set_favorite("7001".into(), false, None).unwrap());
        assert!(t.favorites().unwrap().is_empty());
        assert_eq!(
            t.set_favorite("not-an-id".into(), true, None)
                .unwrap_err()
                .kind,
            TwitchErrorKind::Invalid
        );
    }

    #[test]
    fn helix_answers_are_cached_with_expiry() {
        let transport = Arc::new(FixtureTransport::new());
        transport.push_fixture(USERS).push_fixture(VIDEOS);
        let t = core(transport);
        let users = t.users(vec![], vec!["pixelheron".into()]).unwrap();
        assert_eq!(users[0].display_name, "PixelHeron");
        assert_eq!(users[0].broadcaster_type, "affiliate");
        let vods = t
            .channel_videos("7001".into(), "archive".into(), 20)
            .unwrap();
        assert_eq!(vods.len(), 1);
        let v = &vods[0];
        assert_eq!(v.title, "Night run");
        assert_eq!(v.user_login, "pixelheron");
        assert_eq!(v.duration_ms, 3_723_000);
        // Affiliate archive: 14 days after publishing.
        assert_eq!(
            v.expires_at_ms.unwrap() - v.published_at_ms,
            14 * 24 * 60 * 60 * 1000
        );
        // Cached: readable without Helix.
        assert_eq!(t.cached_videos("7001".into(), 10).unwrap().len(), 1);
        assert!(t.cached_user("7001".into()).unwrap().is_some());
        assert!(t.chapters("9001".into()).unwrap().is_empty());
    }

    #[test]
    fn continue_lists_started_vods() {
        let transport = Arc::new(FixtureTransport::new());
        transport.push_fixture(USERS).push_fixture(VIDEOS);
        let t = core(transport);
        t.users(vec!["7001".into()], vec![]).unwrap();
        t.channel_videos("7001".into(), String::new(), 20).unwrap();
        {
            let mut conn = t.db();
            let mut sink = LocalOnly::new("test");
            progress::report_position(
                &mut conn,
                &mut sink,
                &Thresholds::default(),
                &progress::PositionReport {
                    profile_id: PROFILE,
                    key: MediaKey::TwitchVod {
                        video_id: "9001".into(),
                    },
                    kind: MediaKind::TwitchVod,
                    position_ms: 600_000,
                    duration_ms: Some(3_723_000),
                },
                now_ms(),
            )
            .unwrap();
        }
        let rows = t.continue_vods(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].position_ms, 600_000);
        assert_eq!(rows[0].video.as_ref().unwrap().title, "Night run");
        assert_eq!(rows[0].user.as_ref().unwrap().login, "pixelheron");
    }
}
