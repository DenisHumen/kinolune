// SPDX-License-Identifier: MPL-2.0
//! Trakt endpoints over a [`Transport`]: OAuth on `auth.trakt.tv`, the API
//! on `api.trakt.tv`, the required headers, the write rate limit and
//! `429` handling.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::auth::{Credentials, DeviceCode, Poll, TokenAnswer, Tokens};
use crate::http::{Method, Request, Response, Transport};
use crate::model::{Ids, Listed, Rated, Target, Watched};
use crate::time::{format_ms, parse_ms};
use crate::{Error, Result};

/// Redirect URI of apps without one (the device flow); Trakt asks for it
/// in a refresh.
const OOB: &str = "urn:ietf:wg:oauth:2.0:oob";

/// Writes are spaced at least this much (Trakt: 1 per second per user).
const WRITE_GAP: Duration = Duration::from_millis(1050);

/// A `429` on a write is waited out and retried once if `Retry-After` is
/// at most this long.
const MAX_RETRY_WAIT: u64 = 30;

/// Items per `/sync/*` write.
pub(crate) const BATCH: usize = 100;

/// Hosts of the Trakt API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// `https://api.trakt.tv`.
    pub api: String,
    /// `https://auth.trakt.tv` (Trakt: all OAuth requests go there).
    pub auth: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            api: "https://api.trakt.tv".into(),
            auth: "https://auth.trakt.tv".into(),
        }
    }
}

/// A scrobble event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrobbleAction {
    Start,
    Pause,
    /// Stop or finish: at 80 % and more Trakt records a play.
    Stop,
}

impl ScrobbleAction {
    fn path(self) -> &'static str {
        match self {
            Self::Start => "/scrobble/start",
            Self::Pause => "/scrobble/pause",
            Self::Stop => "/scrobble/stop",
        }
    }
}

type Sleeper = Arc<dyn Fn(Duration) + Send + Sync>;

/// The Trakt client. Holds no tokens: authorized calls take the access
/// token ([`crate::Session`] keeps and refreshes it).
pub struct Client {
    credentials: Credentials,
    transport: Arc<dyn Transport>,
    endpoints: Endpoints,
    last_write: Mutex<Option<Instant>>,
    sleep: Sleeper,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("credentials", &self.credentials)
            .field("endpoints", &self.endpoints)
            .finish_non_exhaustive()
    }
}

// ----- answers -------------------------------------------------------------

#[derive(Deserialize)]
struct MediaRef {
    #[serde(default)]
    ids: Ids,
}

#[derive(Deserialize)]
struct WatchedMovie {
    last_watched_at: Option<String>,
    movie: MediaRef,
}

#[derive(Deserialize)]
struct WatchedEpisode {
    number: u32,
    last_watched_at: Option<String>,
}

#[derive(Deserialize)]
struct WatchedSeason {
    number: u32,
    #[serde(default)]
    episodes: Vec<WatchedEpisode>,
}

#[derive(Deserialize)]
struct WatchedShow {
    show: MediaRef,
    #[serde(default)]
    seasons: Vec<WatchedSeason>,
}

/// An entry of the watchlist or ratings: `type` and the matching object.
#[derive(Deserialize)]
struct Entry {
    #[serde(rename = "type")]
    kind: String,
    movie: Option<MediaRef>,
    show: Option<MediaRef>,
    listed_at: Option<String>,
    rated_at: Option<String>,
    rating: Option<u8>,
}

impl Entry {
    fn target(&self) -> Option<Target> {
        match self.kind.as_str() {
            "movie" => Some(Target::Movie(self.movie.as_ref()?.ids.clone())),
            "show" => Some(Target::Show(self.show.as_ref()?.ids.clone())),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct Settings {
    user: SettingsUser,
}

#[derive(Deserialize)]
struct SettingsUser {
    username: String,
}

fn ts(text: Option<&str>) -> i64 {
    text.and_then(parse_ms).unwrap_or(0)
}

/// `{"movies": [...], "shows": [...]}` of targets; `item` adds fields of
/// each (`watched_at`, `rating`). Episodes are grouped by show and season.
pub(crate) fn sync_body(items: &[(Target, Value)]) -> Value {
    let mut movies = Vec::new();
    let mut shows: Vec<(Ids, Value)> = Vec::new();
    let mut episodes: Vec<(Ids, BTreeMap<u32, Vec<Value>>)> = Vec::new();
    for (target, extra) in items {
        let mut obj = extra.as_object().cloned().unwrap_or_default();
        match target {
            Target::Movie(ids) => {
                obj.insert("ids".into(), json!(ids));
                movies.push(Value::Object(obj));
            }
            Target::Show(ids) => {
                obj.insert("ids".into(), json!(ids));
                shows.push((ids.clone(), Value::Object(obj)));
            }
            Target::Episode {
                show,
                season,
                number,
            } => {
                obj.insert("number".into(), json!(number));
                let i = match episodes.iter().position(|(s, _)| s == show) {
                    Some(i) => i,
                    None => {
                        episodes.push((show.clone(), BTreeMap::new()));
                        episodes.len() - 1
                    }
                };
                episodes[i]
                    .1
                    .entry(*season)
                    .or_default()
                    .push(Value::Object(obj));
            }
        }
    }
    let mut show_list: Vec<Value> = shows.into_iter().map(|(_, v)| v).collect();
    for (ids, seasons) in episodes {
        let seasons: Vec<Value> = seasons
            .into_iter()
            .map(|(number, eps)| json!({ "number": number, "episodes": eps }))
            .collect();
        show_list.push(json!({ "ids": ids, "seasons": seasons }));
    }
    let mut body = serde_json::Map::new();
    if !movies.is_empty() {
        body.insert("movies".into(), Value::Array(movies));
    }
    if !show_list.is_empty() {
        body.insert("shows".into(), Value::Array(show_list));
    }
    Value::Object(body)
}

impl Client {
    /// A client of `credentials` over `transport`.
    pub fn new(
        credentials: Credentials,
        transport: Arc<dyn Transport>,
        endpoints: Endpoints,
    ) -> Self {
        Self {
            credentials,
            transport,
            endpoints,
            last_write: Mutex::new(None),
            sleep: Arc::new(std::thread::sleep),
        }
    }

    /// The same client with a custom sleep (tests: none).
    #[must_use]
    pub fn with_sleep(mut self, sleep: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        self.sleep = Arc::new(sleep);
        self
    }

    /// The credentials in use.
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    fn headers(&self, token: Option<&str>) -> Vec<(String, String)> {
        let mut h = vec![
            ("content-type".into(), "application/json".into()),
            ("trakt-api-version".into(), "2".into()),
            ("trakt-api-key".into(), self.credentials.client_id().into()),
        ];
        if let Some(t) = token {
            h.push(("authorization".into(), format!("Bearer {t}")));
        }
        h
    }

    /// Waits until a write may go (1 per second).
    fn gate(&self) {
        let mut last = self.last_write.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = *last {
            let since = at.elapsed();
            if since < WRITE_GAP {
                (self.sleep)(WRITE_GAP - since);
            }
        }
        *last = Some(Instant::now());
    }

    /// Sends one request; a write waits for the gate and a `429` with a
    /// short `Retry-After` is waited out and retried once.
    fn send(
        &self,
        method: Method,
        url: String,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Response> {
        let request = Request {
            method,
            url,
            headers: self.headers(token),
            body: body.map(Value::to_string),
        };
        for attempt in 0..2 {
            if method.is_write() {
                self.gate();
            }
            let r = self.transport.send(&request)?;
            if r.status == 429 {
                let wait = r.retry_after.unwrap_or(1);
                if attempt == 0 && wait <= MAX_RETRY_WAIT {
                    (self.sleep)(Duration::from_secs(wait));
                    continue;
                }
                return Err(Error::RateLimited {
                    retry_after_secs: wait,
                });
            }
            return Ok(r);
        }
        unreachable!("the loop returns")
    }

    fn check(r: Response) -> Result<Response> {
        match r.status {
            200..=299 => Ok(r),
            401 | 403 => Err(Error::Unauthorized),
            s => Err(Error::Http(s)),
        }
    }

    fn parse<T: DeserializeOwned>(r: &Response) -> Result<T> {
        Ok(serde_json::from_slice(&r.body)?)
    }

    fn api(&self, path: &str) -> String {
        format!("{}{path}", self.endpoints.api)
    }

    fn get<T: DeserializeOwned>(&self, path: &str, token: &str) -> Result<T> {
        let r = Self::check(self.send(Method::Get, self.api(path), Some(token), None)?)?;
        Self::parse(&r)
    }

    fn post(&self, path: &str, token: &str, body: &Value) -> Result<Response> {
        Self::check(self.send(Method::Post, self.api(path), Some(token), Some(body))?)
    }

    fn oauth_body(&self, mut body: Value) -> Value {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("client_id".into(), json!(self.credentials.client_id()));
            if let Some(secret) = self.credentials.client_secret() {
                obj.insert("client_secret".into(), json!(secret));
            }
        }
        body
    }

    // ----- OAuth --------------------------------------------------------------

    /// Starts a device sign-in.
    pub fn device_code(&self) -> Result<DeviceCode> {
        let body = json!({ "client_id": self.credentials.client_id() });
        let url = format!("{}/oauth/device/code", self.endpoints.auth);
        let r = Self::check(self.send(Method::Post, url, None, Some(&body))?)?;
        Self::parse(&r)
    }

    /// Polls a device sign-in once.
    pub fn poll_device(&self, code: &DeviceCode) -> Result<Poll> {
        let body = self.oauth_body(json!({ "code": code.device_code }));
        let url = format!("{}/oauth/device/token", self.endpoints.auth);
        let request = Request {
            method: Method::Post,
            url,
            headers: self.headers(None),
            body: Some(body.to_string()),
        };
        // No retry on 429 here: it means "poll slower".
        let r = self.transport.send(&request)?;
        Ok(match r.status {
            200 => Poll::Approved(Self::parse::<TokenAnswer>(&r)?.into_tokens()),
            400 => Poll::Pending,
            404 => Poll::InvalidCode,
            409 => Poll::AlreadyUsed,
            410 => Poll::Expired,
            418 => Poll::Denied,
            429 => Poll::SlowDown,
            s => return Err(Error::Http(s)),
        })
    }

    /// Exchanges the single-use `refresh_token` for new tokens. A `400`
    /// (`invalid_grant`) means the sign-in is gone.
    pub fn refresh(&self, refresh_token: &str) -> Result<Tokens> {
        let body = self.oauth_body(json!({
            "refresh_token": refresh_token,
            "grant_type": "refresh_token",
            "redirect_uri": OOB,
        }));
        let url = format!("{}/oauth/token", self.endpoints.auth);
        let r = self.send(Method::Post, url, None, Some(&body))?;
        match r.status {
            200 => Ok(Self::parse::<TokenAnswer>(&r)?.into_tokens()),
            400 | 401 => Err(Error::SignedOut),
            s => Err(Error::Http(s)),
        }
    }

    /// Revokes an access token (sign-out).
    pub fn revoke(&self, access_token: &str) -> Result<()> {
        let body = self.oauth_body(json!({ "token": access_token }));
        let url = format!("{}/oauth/revoke", self.endpoints.auth);
        Self::check(self.send(Method::Post, url, None, Some(&body))?)?;
        Ok(())
    }

    // ----- API ----------------------------------------------------------------

    /// The user name of the token's account.
    pub fn username(&self, token: &str) -> Result<String> {
        Ok(self.get::<Settings>("/users/settings", token)?.user.username)
    }

    /// A scrobble event of `target` at `progress` percent (0–100). A
    /// `409` (the same item was just scrobbled) counts as done.
    pub fn scrobble(
        &self,
        token: &str,
        action: ScrobbleAction,
        target: &Target,
        progress: f64,
    ) -> Result<()> {
        let mut body = target.scrobble_json();
        body["progress"] = json!((progress.clamp(0.0, 100.0) * 100.0).round() / 100.0);
        let r = self.send(
            Method::Post,
            self.api(action.path()),
            Some(token),
            Some(&body),
        )?;
        if r.status == 409 {
            return Ok(());
        }
        Self::check(r)?;
        Ok(())
    }

    /// Every watched movie and episode with its last watch time.
    pub fn watched(&self, token: &str) -> Result<Vec<Watched>> {
        let mut out = Vec::new();
        let movies: Vec<WatchedMovie> = self.get("/sync/watched/movies", token)?;
        for m in movies {
            out.push(Watched {
                target: Target::Movie(m.movie.ids),
                watched_at_ms: ts(m.last_watched_at.as_deref()),
            });
        }
        let shows: Vec<WatchedShow> = self.get("/sync/watched/shows", token)?;
        for s in shows {
            for season in s.seasons {
                for e in season.episodes {
                    out.push(Watched {
                        target: Target::Episode {
                            show: s.show.ids.clone(),
                            season: season.number,
                            number: e.number,
                        },
                        watched_at_ms: ts(e.last_watched_at.as_deref()),
                    });
                }
            }
        }
        Ok(out)
    }

    /// Adds plays to the history, in batches.
    pub fn add_history(&self, token: &str, items: &[Watched]) -> Result<()> {
        for chunk in items.chunks(BATCH) {
            let list: Vec<(Target, Value)> = chunk
                .iter()
                .map(|w| {
                    (
                        w.target.clone(),
                        json!({ "watched_at": format_ms(w.watched_at_ms) }),
                    )
                })
                .collect();
            self.post("/sync/history", token, &sync_body(&list))?;
        }
        Ok(())
    }

    /// Removes every play of `targets` from the history.
    pub fn remove_history(&self, token: &str, targets: &[Target]) -> Result<()> {
        for chunk in targets.chunks(BATCH) {
            let list: Vec<(Target, Value)> =
                chunk.iter().map(|t| (t.clone(), json!({}))).collect();
            self.post("/sync/history/remove", token, &sync_body(&list))?;
        }
        Ok(())
    }

    /// The watchlist: movies and shows.
    pub fn watchlist(&self, token: &str) -> Result<Vec<Listed>> {
        let mut out = Vec::new();
        for path in ["/sync/watchlist/movies", "/sync/watchlist/shows"] {
            let entries: Vec<Entry> = self.get(path, token)?;
            out.extend(entries.iter().filter_map(|e| {
                Some(Listed {
                    target: e.target()?,
                    listed_at_ms: ts(e.listed_at.as_deref()),
                })
            }));
        }
        Ok(out)
    }

    /// Adds a movie or show to the watchlist.
    pub fn watchlist_add(&self, token: &str, target: &Target) -> Result<()> {
        let body = sync_body(&[(target.clone(), json!({}))]);
        self.post("/sync/watchlist", token, &body)?;
        Ok(())
    }

    /// Removes a movie or show from the watchlist.
    pub fn watchlist_remove(&self, token: &str, target: &Target) -> Result<()> {
        let body = sync_body(&[(target.clone(), json!({}))]);
        self.post("/sync/watchlist/remove", token, &body)?;
        Ok(())
    }

    /// Ratings of movies and shows.
    pub fn ratings(&self, token: &str) -> Result<Vec<Rated>> {
        let mut out = Vec::new();
        for path in ["/sync/ratings/movies", "/sync/ratings/shows"] {
            let entries: Vec<Entry> = self.get(path, token)?;
            out.extend(entries.iter().filter_map(|e| {
                Some(Rated {
                    target: e.target()?,
                    rating: e.rating.filter(|r| (1..=10).contains(r))?,
                    rated_at_ms: ts(e.rated_at.as_deref()),
                })
            }));
        }
        Ok(out)
    }

    /// Adds or replaces ratings, in batches.
    pub fn add_ratings(&self, token: &str, items: &[Rated]) -> Result<()> {
        for chunk in items.chunks(BATCH) {
            let list: Vec<(Target, Value)> = chunk
                .iter()
                .map(|r| {
                    (
                        r.target.clone(),
                        json!({ "rating": r.rating, "rated_at": format_ms(r.rated_at_ms) }),
                    )
                })
                .collect();
            self.post("/sync/ratings", token, &sync_body(&list))?;
        }
        Ok(())
    }

    /// Removes the ratings of `targets`.
    pub fn remove_ratings(&self, token: &str, targets: &[Target]) -> Result<()> {
        for chunk in targets.chunks(BATCH) {
            let list: Vec<(Target, Value)> =
                chunk.iter().map(|t| (t.clone(), json!({}))).collect();
            self.post("/sync/ratings/remove", token, &sync_body(&list))?;
        }
        Ok(())
    }
}
