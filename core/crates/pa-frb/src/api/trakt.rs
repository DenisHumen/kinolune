// SPDX-License-Identifier: MPL-2.0
//! Trakt for Dart (SYNC-011, ADR-NNNN): device code sign-in, scrobbles of
//! the player, two-way sync of watched marks and ratings, the watchlist
//! as a shelf of library titles, and the item menu's Trakt actions.
//!
//! The client id comes from the build (`CARGO_PA_TRAKT_CLIENT_ID`) unless
//! the user entered their own (advanced settings); the user's id and
//! secret live in the secret store, as do the tokens. Every method that
//! talks to Trakt blocks on the network.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use flutter_rust_bridge::frb;
use pa_catalog as c;
use pa_secrets::{SecretKey, SecretStore, SecretString};
use pa_trakt as t;

use super::library::{CoreLibrary, item_ref};

/// How long a fetched watchlist is used before asking Trakt again.
const WATCHLIST_TTL: Duration = Duration::from_secs(10 * 60);

/// Error of a Trakt call; Dart receives it as an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraktError {
    /// `not_configured`, `signed_out`, `expired` (the sign-in code),
    /// `denied`, `rate_limited`, `network`, `library` or `other`.
    pub code: String,
    /// Human-readable message without secrets.
    pub message: String,
}

impl TraktError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<t::Error> for TraktError {
    fn from(e: t::Error) -> Self {
        let code = match &e {
            t::Error::NotConfigured => "not_configured",
            t::Error::SignedOut | t::Error::Unauthorized => "signed_out",
            t::Error::RateLimited { .. } => "rate_limited",
            t::Error::Unreachable(_) | t::Error::TimedOut => "network",
            t::Error::Library(_) => "library",
            _ => "other",
        };
        Self::new(code, e.to_string())
    }
}

impl From<c::Error> for TraktError {
    fn from(e: c::Error) -> Self {
        Self::new("library", e.to_string())
    }
}

impl From<super::library::LibraryError> for TraktError {
    fn from(e: super::library::LibraryError) -> Self {
        Self::new("library", e.message)
    }
}

type TraktResult<T> = std::result::Result<T, TraktError>;

/// A device sign-in in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTraktDeviceLogin {
    /// The code to enter (about 8 characters).
    pub user_code: String,
    /// The page to enter it on (`https://trakt.tv/activate`).
    pub verification_url: String,
    /// The page with the code filled in, for the QR code.
    pub qr_url: String,
    pub expires_in_secs: i64,
    /// Poll this often.
    pub interval_secs: i64,
}

/// What a sync changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoreTraktSyncReport {
    pub watched_imported: u32,
    pub watched_exported: u32,
    pub ratings_imported: u32,
    pub ratings_exported: u32,
}

/// Where the user's own client id and secret are kept.
fn user_id_key() -> SecretKey {
    SecretKey::provider_api_key("trakt_client_id").expect("valid key")
}

fn user_secret_key() -> SecretKey {
    SecretKey::provider_api_key("trakt_client_secret").expect("valid key")
}

fn read(store: &dyn SecretStore, key: &SecretKey) -> Option<String> {
    store
        .get(key)
        .ok()
        .flatten()
        .map(|s| s.expose().to_owned())
        .filter(|s| !s.trim().is_empty())
}

fn ids_of(title: &c::ExternalTitle) -> t::Ids {
    t::Ids {
        trakt: None,
        imdb: title.imdb.clone(),
        tmdb: title.tmdb,
        tvdb: title.tvdb,
    }
}

/// The Trakt target of a library title.
fn target_of(title: &c::ExternalTitle) -> Option<t::Target> {
    let ids = ids_of(title);
    if ids.is_empty() {
        return None;
    }
    Some(match title.kind? {
        c::TitleKind::Movie => t::Target::Movie(ids),
        c::TitleKind::Show => t::Target::Show(ids),
        c::TitleKind::Episode => t::Target::Episode {
            show: ids,
            season: title.season?,
            number: title.episode?,
        },
    })
}

/// The library title of a Trakt target.
fn title_of(target: &t::Target) -> c::ExternalTitle {
    let (kind, ids, season, episode) = match target {
        t::Target::Movie(ids) => (c::TitleKind::Movie, ids, None, None),
        t::Target::Show(ids) => (c::TitleKind::Show, ids, None, None),
        t::Target::Episode {
            show,
            season,
            number,
        } => (c::TitleKind::Episode, show, Some(*season), Some(*number)),
    };
    c::ExternalTitle {
        kind: Some(kind),
        tmdb: ids.tmdb,
        imdb: ids.imdb.clone(),
        tvdb: ids.tvdb,
        season,
        episode,
    }
}

/// The library as [`t::sync::Library`].
struct CatalogLibrary<'a>(&'a c::Catalog);

fn lib_err(e: c::Error) -> t::Error {
    t::Error::Library(e.to_string())
}

impl t::sync::Library for CatalogLibrary<'_> {
    fn watched(&self) -> t::Result<Vec<t::Watched>> {
        Ok(self
            .0
            .watched_titles()
            .map_err(lib_err)?
            .into_iter()
            .filter_map(|(title, at)| {
                Some(t::Watched {
                    target: target_of(&title)?,
                    watched_at_ms: at,
                })
            })
            .collect())
    }

    fn mark_watched(&self, w: &t::Watched) -> t::Result<bool> {
        self.0
            .mark_title_watched(&title_of(&w.target), w.watched_at_ms)
            .map_err(lib_err)
    }

    fn ratings(&self) -> t::Result<Vec<t::sync::LocalRating>> {
        Ok(self
            .0
            .rated_titles()
            .map_err(lib_err)?
            .into_iter()
            .filter_map(|(title, stars, at)| {
                Some(t::sync::LocalRating {
                    target: target_of(&title)?,
                    stars,
                    rated_at_ms: at,
                })
            })
            .collect())
    }

    fn set_rating(&self, target: &t::Target, stars: u8) -> t::Result<bool> {
        self.0
            .rate_title(&title_of(target), stars)
            .map_err(lib_err)
    }
}

/// Trakt for one library.
#[frb(opaque)]
pub struct CoreTrakt {
    catalog: Arc<c::Catalog>,
    store: Arc<dyn SecretStore>,
    session: Mutex<Option<Arc<t::Session>>>,
    pending: Mutex<Option<t::DeviceCode>>,
    watchlist: Mutex<Option<(Instant, Vec<t::Listed>)>>,
    targets: Mutex<HashMap<String, Option<t::Target>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl CoreTrakt {
    /// Opens Trakt for `library`, with the secret store whose fallback
    /// file is in `data_dir` (as for `CoreSources.open`). A stored sign-in
    /// is restored; no network.
    pub fn open(library: &CoreLibrary, data_dir: String) -> TraktResult<CoreTrakt> {
        let opened = crate::shared::secret_store(Path::new(&data_dir))
            .map_err(|e| TraktError::new("other", e.to_string()))?;
        let me = Self {
            catalog: library.catalog(),
            store: Arc::clone(&opened.store),
            session: Mutex::new(None),
            pending: Mutex::new(None),
            watchlist: Mutex::new(None),
            targets: Mutex::new(HashMap::new()),
        };
        me.rebuild()?;
        Ok(me)
    }

    /// Builds the session of the current credentials.
    fn rebuild(&self) -> TraktResult<()> {
        let id = read(self.store.as_ref(), &user_id_key());
        let secret = read(self.store.as_ref(), &user_secret_key());
        let session = match t::Credentials::choose(id.as_deref(), secret.as_deref()) {
            Some(credentials) => {
                let agent = format!("Kinolune/{}", env!("CARGO_PKG_VERSION"));
                let client = t::Client::new(
                    credentials,
                    Arc::new(t::UreqTransport::new(&agent)),
                    t::Endpoints::default(),
                );
                Some(Arc::new(t::Session::open(client, Arc::clone(&self.store))?))
            }
            None => None,
        };
        *lock(&self.session) = session;
        *lock(&self.watchlist) = None;
        Ok(())
    }

    fn session(&self) -> TraktResult<Arc<t::Session>> {
        lock(&self.session)
            .clone()
            .ok_or_else(|| TraktError::from(t::Error::NotConfigured))
    }

    fn signed_in(&self) -> TraktResult<Arc<t::Session>> {
        let s = self.session()?;
        if s.is_signed_in() {
            Ok(s)
        } else {
            Err(t::Error::SignedOut.into())
        }
    }

    /// Whether a client id is set (the build's or the user's): sign-in is
    /// possible.
    #[frb(sync)]
    pub fn is_configured(&self) -> bool {
        lock(&self.session).is_some()
    }

    /// Whether the user entered their own client id.
    #[frb(sync)]
    pub fn has_own_client(&self) -> bool {
        read(self.store.as_ref(), &user_id_key()).is_some()
    }

    /// The signed-in Trakt user name; `Some("")` if signed in without a
    /// known name; `None` when signed out. No network.
    #[frb(sync)]
    pub fn account(&self) -> Option<String> {
        let s = lock(&self.session).clone()?;
        s.is_signed_in().then(|| s.username().unwrap_or_default())
    }

    /// Sets (or clears, with empty values) the user's own client id and
    /// secret. A different client needs a new sign-in.
    pub fn set_own_client(
        &self,
        client_id: Option<String>,
        client_secret: Option<String>,
    ) -> TraktResult<()> {
        let put = |key: SecretKey, value: Option<String>| -> TraktResult<()> {
            match value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) {
                Some(v) => self
                    .store
                    .set(&key, &SecretString::new(v))
                    .map_err(|e| TraktError::new("other", e.to_string())),
                None => self
                    .store
                    .delete(&key)
                    .map(|_| ())
                    .map_err(|e| TraktError::new("other", e.to_string())),
            }
        };
        put(user_id_key(), client_id)?;
        put(user_secret_key(), client_secret)?;
        self.rebuild()
    }

    /// Starts a device sign-in.
    pub fn start_login(&self) -> TraktResult<CoreTraktDeviceLogin> {
        let code = self.session()?.begin()?;
        let out = CoreTraktDeviceLogin {
            user_code: code.user_code.clone(),
            verification_url: code.verification_url.clone(),
            qr_url: code.qr_url(),
            expires_in_secs: i64::try_from(code.expires_in).unwrap_or(600),
            interval_secs: i64::try_from(code.interval.max(1)).unwrap_or(5),
        };
        *lock(&self.pending) = Some(code);
        Ok(out)
    }

    /// Polls the pending sign-in once: the user name once confirmed,
    /// `None` while waiting. `expired` and `denied` end it.
    pub fn poll_login(&self) -> TraktResult<Option<String>> {
        let Some(code) = lock(&self.pending).clone() else {
            return Err(TraktError::new("expired", "no sign-in started"));
        };
        let poll = self.session()?.poll(&code)?;
        let end = |code: &str, msg: &str| {
            *lock(&self.pending) = None;
            Err(TraktError::new(code, msg))
        };
        match poll {
            t::Poll::Pending | t::Poll::SlowDown => Ok(None),
            t::Poll::Approved(tokens) => {
                *lock(&self.pending) = None;
                Ok(Some(tokens.username.unwrap_or_default()))
            }
            t::Poll::Denied => end("denied", "the sign-in was refused"),
            t::Poll::Expired | t::Poll::InvalidCode | t::Poll::AlreadyUsed => {
                end("expired", "the sign-in code expired")
            }
        }
    }

    /// Forgets the pending sign-in.
    #[frb(sync)]
    pub fn cancel_login(&self) {
        *lock(&self.pending) = None;
    }

    /// Signs out and forgets the tokens.
    pub fn sign_out(&self) -> TraktResult<()> {
        *lock(&self.watchlist) = None;
        if let Some(s) = lock(&self.session).clone() {
            s.sign_out()?;
        }
        Ok(())
    }

    /// The Trakt target of library item `item_id` (cached per item).
    fn target(&self, item_id: &str) -> TraktResult<Option<t::Target>> {
        if let Some(hit) = lock(&self.targets).get(item_id) {
            return Ok(hit.clone());
        }
        let title = self.catalog.external_title(item_ref(item_id)?)?;
        let target = title.as_ref().and_then(target_of);
        lock(&self.targets).insert(item_id.to_owned(), target.clone());
        Ok(target)
    }

    /// A scrobble of the player for library item `item_id`: `action` is
    /// `start`, `pause` or `stop`, `progress` 0–100. `false` when signed
    /// out or the item is not a known movie or episode.
    pub fn scrobble(&self, item_id: String, action: String, progress: f64) -> TraktResult<bool> {
        let Ok(session) = self.signed_in() else {
            return Ok(false);
        };
        let action = match action.as_str() {
            "start" => t::ScrobbleAction::Start,
            "pause" => t::ScrobbleAction::Pause,
            "stop" => t::ScrobbleAction::Stop,
            other => return Err(TraktError::new("other", format!("action {other:?}"))),
        };
        match self.target(&item_id)? {
            Some(target @ (t::Target::Movie(_) | t::Target::Episode { .. })) => {
                session.scrobble(action, &target, progress)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Syncs watched marks and ratings both ways.
    pub fn sync(&self) -> TraktResult<CoreTraktSyncReport> {
        let session = self.signed_in()?;
        let r = t::sync::run(&session, &CatalogLibrary(&self.catalog))?;
        Ok(CoreTraktSyncReport {
            watched_imported: r.watched_imported,
            watched_exported: r.watched_exported,
            ratings_imported: r.ratings_imported,
            ratings_exported: r.ratings_exported,
        })
    }

    /// The item menu marked library item `item_id` (a movie or an
    /// episode) watched or unwatched here: adds a play now or removes its
    /// plays on Trakt. `false` when signed out or unknown to Trakt.
    pub fn set_watched(&self, item_id: String, watched: bool, at_ms: i64) -> TraktResult<bool> {
        let Ok(session) = self.signed_in() else {
            return Ok(false);
        };
        let Some(target @ (t::Target::Movie(_) | t::Target::Episode { .. })) =
            self.target(&item_id)?
        else {
            return Ok(false);
        };
        if watched {
            session.add_history(&[t::Watched {
                target,
                watched_at_ms: at_ms,
            }])?;
        } else {
            session.remove_history(&[target])?;
        }
        Ok(true)
    }

    /// Sends a changed rating (1–5 stars, `None`: removed) of library
    /// movie or show `item_id`. `false` when signed out or unknown.
    pub fn set_rating(&self, item_id: String, stars: Option<u8>, at_ms: i64) -> TraktResult<bool> {
        let Ok(session) = self.signed_in() else {
            return Ok(false);
        };
        let Some(target @ (t::Target::Movie(_) | t::Target::Show(_))) = self.target(&item_id)?
        else {
            return Ok(false);
        };
        match stars {
            Some(s) => session.add_ratings(&[t::Rated {
                target,
                rating: t::stars_to_trakt(s),
                rated_at_ms: at_ms,
            }])?,
            None => session.remove_ratings(&[target])?,
        }
        Ok(true)
    }

    fn listed(&self, refresh: bool) -> TraktResult<Vec<t::Listed>> {
        let session = self.signed_in()?;
        if !refresh
            && let Some((at, list)) = lock(&self.watchlist).as_ref()
            && at.elapsed() < WATCHLIST_TTL
        {
            return Ok(list.clone());
        }
        let list = session.watchlist()?;
        *lock(&self.watchlist) = Some((Instant::now(), list.clone()));
        Ok(list)
    }

    /// Library items (movies and shows) on the Trakt watchlist, most
    /// recently listed first; empty when signed out. Fetched at most every
    /// ten minutes unless `refresh`.
    pub fn watchlist_items(&self, refresh: bool) -> TraktResult<Vec<String>> {
        if self.signed_in().is_err() {
            return Ok(Vec::new());
        }
        let mut list = self.listed(refresh)?;
        list.sort_by_key(|l| std::cmp::Reverse(l.listed_at_ms));
        let mut out = Vec::new();
        for l in list {
            if let Some(item) = self.catalog.item_of_title(&title_of(&l.target))? {
                let id = item.to_string();
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
        Ok(out)
    }

    /// Whether library movie or show `item_id` is on the watchlist (from
    /// the cached list). `false` when signed out.
    pub fn in_watchlist(&self, item_id: String) -> TraktResult<bool> {
        if self.signed_in().is_err() {
            return Ok(false);
        }
        let Some(target) = self.target(&item_id)? else {
            return Ok(false);
        };
        let key = target.key();
        Ok(key.is_some() && self.listed(false)?.iter().any(|l| l.target.key() == key))
    }

    /// Adds library movie or show `item_id` to the watchlist or removes it.
    /// `false` when signed out or unknown to Trakt.
    pub fn set_watchlisted(&self, item_id: String, listed: bool) -> TraktResult<bool> {
        let Ok(session) = self.signed_in() else {
            return Ok(false);
        };
        let Some(target @ (t::Target::Movie(_) | t::Target::Show(_))) = self.target(&item_id)?
        else {
            return Ok(false);
        };
        session.set_watchlisted(&target, listed)?;
        *lock(&self.watchlist) = None;
        Ok(true)
    }
}
