// SPDX-License-Identifier: MPL-2.0
//! Music scrobbling to Last.fm and ListenBrainz (MUS-015, `pa-scrobble`).
//!
//! Dart reports what the music player plays ([`CoreScrobbler::track_started`],
//! [`CoreScrobbler::progress`]); the core decides when a play counts, keeps
//! the offline queue in the app's data folder and sends. The accounts (the
//! Last.fm session key, the ListenBrainz token) are kept by the app in the
//! system keychain and passed in with [`CoreScrobbler::configure`].
//! Every call does blocking network I/O: Dart awaits it off the UI thread.

use std::path::PathBuf;
use std::sync::Mutex;

use flutter_rust_bridge::frb;
use pa_scrobble::key::LastFmKey;
use pa_scrobble::http::UreqTransport;
use pa_scrobble::{ApiError, Config, LastFmConfig, Scrobbler, Status, Track};

/// An error of a sign-in step or a token check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrobbleError {
    /// `no_key` (no Last.fm API key), `unavailable` (network or service),
    /// `refused` (the service said no: a token not approved yet, a bad
    /// token), `malformed`.
    pub code: String,
    /// For logs; never carries a key or a token.
    pub message: String,
}

impl From<ApiError> for ScrobbleError {
    fn from(e: ApiError) -> Self {
        let code = match &e {
            ApiError::NoKey => "no_key",
            ApiError::Unavailable(_) => "unavailable",
            ApiError::Refused { .. } => "refused",
            ApiError::Malformed => "malformed",
        };
        Self {
            code: code.into(),
            message: e.to_string(),
        }
    }
}

type ScrobbleResult<T> = Result<T, ScrobbleError>;

/// A track as the player has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreScrobbleTrack {
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub duration_ms: Option<u64>,
    pub track_number: Option<u32>,
    /// MusicBrainz recording id.
    pub mbid: Option<String>,
}

impl From<CoreScrobbleTrack> for Track {
    fn from(t: CoreScrobbleTrack) -> Self {
        Self {
            artist: t.artist,
            title: t.title,
            album: t.album,
            album_artist: t.album_artist,
            duration_ms: t.duration_ms,
            track_number: t.track_number,
            mbid: t.mbid,
        }
    }
}

/// Queues and accounts as the settings show them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoreScrobbleStatus {
    /// Scrobbles waiting to go out.
    pub lastfm_pending: u64,
    pub listenbrainz_pending: u64,
    /// The account must be connected again (revoked, bad token).
    pub lastfm_needs_auth: bool,
    pub listenbrainz_needs_auth: bool,
}

impl From<Status> for CoreScrobbleStatus {
    fn from(s: Status) -> Self {
        Self {
            lastfm_pending: s.lastfm_pending,
            listenbrainz_pending: s.listenbrainz_pending,
            lastfm_needs_auth: s.lastfm_needs_auth,
            listenbrainz_needs_auth: s.listenbrainz_needs_auth,
        }
    }
}

/// Step 1 of the Last.fm sign-in: open `url` in the browser, then call
/// [`CoreScrobbler::lastfm_finish`] with `token`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLastFmAuth {
    pub token: String,
    pub url: String,
}

/// A connected Last.fm account: keep `session_key` in the keychain.
#[derive(Clone, PartialEq, Eq)]
pub struct CoreLastFmSession {
    pub name: String,
    pub session_key: String,
}

impl std::fmt::Debug for CoreLastFmSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreLastFmSession")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

fn key(api_key: Option<String>, secret: Option<String>) -> ScrobbleResult<LastFmKey> {
    LastFmKey::choose(api_key.as_deref(), secret.as_deref()).ok_or_else(|| ApiError::NoKey.into())
}

/// The scrobbler of the app (one per process).
#[frb(opaque)]
pub struct CoreScrobbler {
    inner: Mutex<Scrobbler<UreqTransport>>,
}

impl CoreScrobbler {
    /// A scrobbler that keeps its offline queues in `data_dir`.
    #[frb(sync)]
    pub fn open(data_dir: String) -> CoreScrobbler {
        let dir = PathBuf::from(data_dir);
        let _ = std::fs::create_dir_all(&dir);
        Self {
            inner: Mutex::new(Scrobbler::new(UreqTransport::new(), Some(&dir))),
        }
    }

    /// Whether this build has a Last.fm API key of its own (then the user
    /// needs no key, only the sign-in).
    #[frb(sync)]
    pub fn has_project_lastfm_key() -> bool {
        LastFmKey::project().is_some()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Scrobbler<UreqTransport>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sets the accounts of the active profile. Last.fm: the session key
    /// and the user's own API key and secret (`None`: the build's key);
    /// ListenBrainz: the user token. `None` turns a service off and drops
    /// its queue.
    pub fn configure(
        &self,
        lastfm_session_key: Option<String>,
        lastfm_api_key: Option<String>,
        lastfm_secret: Option<String>,
        listenbrainz_token: Option<String>,
    ) -> CoreScrobbleStatus {
        let lastfm = lastfm_session_key
            .filter(|s| !s.trim().is_empty())
            .and_then(|sk| {
                Some(LastFmConfig {
                    key: key(lastfm_api_key, lastfm_secret).ok()?,
                    session_key: sk.trim().to_owned(),
                })
            });
        let listenbrainz = listenbrainz_token
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty());
        let mut s = self.lock();
        s.configure(Config {
            lastfm,
            listenbrainz,
        });
        s.flush(now_ms()).into()
    }

    /// The music player moved to a track (`id`: its library id or URI) at
    /// `position_ms`.
    pub fn track_started(&self, id: String, track: CoreScrobbleTrack, position_ms: u64) {
        self.lock()
            .track_started(&id, track.into(), position_ms, now_ms());
    }

    /// The playhead and whether it plays (every few seconds and on every
    /// change).
    pub fn progress(&self, position_ms: u64, playing: bool) {
        self.lock().progress(position_ms, playing, now_ms());
    }

    /// The music stopped (the queue is gone).
    pub fn stopped(&self) {
        self.lock().stopped();
    }

    /// Sends what waits in the offline queues and is due.
    pub fn flush(&self) -> CoreScrobbleStatus {
        self.lock().flush(now_ms()).into()
    }

    pub fn status(&self) -> CoreScrobbleStatus {
        self.lock().status().into()
    }

    /// Step 1 of the Last.fm desktop sign-in with the user's key (or the
    /// build's).
    pub fn lastfm_begin(
        &self,
        api_key: Option<String>,
        secret: Option<String>,
    ) -> ScrobbleResult<CoreLastFmAuth> {
        let key = key(api_key, secret)?;
        let (token, url) = self.lock().lastfm_begin(&key)?;
        Ok(CoreLastFmAuth { token, url })
    }

    /// Step 2: the session once the user approved `token` in the browser
    /// (`refused` while not approved yet).
    pub fn lastfm_finish(
        &self,
        token: String,
        api_key: Option<String>,
        secret: Option<String>,
    ) -> ScrobbleResult<CoreLastFmSession> {
        let key = key(api_key, secret)?;
        let s = self.lock().lastfm_finish(&key, &token)?;
        Ok(CoreLastFmSession {
            name: s.name,
            session_key: s.key,
        })
    }

    /// The ListenBrainz user of `token` (`refused` for a bad token).
    pub fn listenbrainz_user(&self, token: String) -> ScrobbleResult<String> {
        Ok(self.lock().listenbrainz_user(token.trim())?)
    }
}
