// SPDX-License-Identifier: MPL-2.0
//! TheTVDB API v4 client (META-012), **with the user's own key only**:
//! series search, the TMDB id of a TheTVDB series, and episodes by air
//! date (daily shows, A11) or by number.
//!
//! **Rules** (<https://thetvdb.github.io/v4-api/>, docs/legal/providers.md,
//! ADR-0132):
//! - the project ships no TheTVDB key: TheTVDB licenses keys per project
//!   for money or lets a person use their own. Without a key from
//!   Settings nothing is sent ([`TvdbError::NoKey`]);
//! - `POST /login` with the key returns a bearer token valid for a month;
//!   it lives in memory only and is renewed after 25 days or when a
//!   request answers 401. The key and the token never enter cache keys,
//!   logs or recordings (`Authorization` is an auth header);
//! - requests go through the shared pipeline ([`fetch`](crate::fetch)):
//!   cache (a week), at most five requests a second, 429/5xx retries;
//! - TheTVDB asks for attribution: [`attribution::THETVDB`](crate::attribution::THETVDB).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::cache::{MAX_CACHE_AGE, ProviderCache};
use crate::clock::Clock;
use crate::fetch::{FetchError, Fetcher, Hooks, year_of};
use crate::http::{HttpRequest, HttpResponse, HttpTransport};
use crate::key::{ApiKey, Keys};
use crate::rate::{RateLimit, RateLimiter, RetryPolicy};

/// Provider id in cache rows and `external_ids`.
pub const PROVIDER: &str = "tvdb";

/// Production API base.
pub const DEFAULT_BASE_URL: &str = "https://api4.thetvdb.com/v4";

/// A token is valid for a month; it is renewed earlier.
const TOKEN_TTL: Duration = Duration::from_secs(25 * 24 * 3600);

/// `remoteIds[].type` of TheMovieDB.com.
const REMOTE_TMDB: u32 = 12;

/// Client settings.
#[derive(Debug, Clone)]
pub struct TvdbConfig {
    /// API base URL.
    pub base_url: String,
    /// `User-Agent` of every request.
    pub user_agent: String,
    /// Client-side rate limit.
    pub rate_limit: RateLimit,
    /// Retry policy for 429 and 5xx.
    pub retry: RetryPolicy,
    /// Freshness of cached responses.
    pub ttl: Duration,
}

impl Default for TvdbConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            user_agent: crate::USER_AGENT.to_owned(),
            rate_limit: RateLimit {
                burst: 5,
                per_second: 5.0,
            },
            retry: RetryPolicy::default(),
            ttl: Duration::from_secs(7 * 24 * 3600),
        }
    }
}

/// Failures of the TheTVDB client.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TvdbError {
    /// No key in Settings: nothing was sent.
    #[error("no TheTVDB key")]
    NoKey,
    /// `/login` refused the key.
    #[error("TheTVDB rejected the key: {0}")]
    KeyRejected(String),
    /// Any other failure.
    #[error(transparent)]
    Fetch(#[from] FetchError),
}

/// A search result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SearchResult {
    /// TheTVDB id as a string (`"81189"`).
    #[serde(default)]
    pub tvdb_id: Option<String>,
    /// Name in the original language.
    #[serde(default)]
    pub name: String,
    /// Other names, translations included.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// First air year.
    #[serde(default)]
    pub year: Option<String>,
    /// `series`, `movie`…
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

impl SearchResult {
    /// The numeric TheTVDB id.
    pub fn id(&self) -> Option<u64> {
        self.tvdb_id.as_deref()?.trim().parse().ok()
    }

    /// First air year.
    pub fn year(&self) -> Option<u32> {
        year_of(self.year.as_deref()?)
    }
}

/// An id of the series at another provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct RemoteId {
    /// The id there.
    #[serde(default)]
    pub id: String,
    /// TheTVDB's code of the provider (12: TheMovieDB.com, 2: IMDB).
    #[serde(default, rename = "type")]
    pub kind: Option<u32>,
    /// Provider name.
    #[serde(default, rename = "sourceName")]
    pub source_name: Option<String>,
}

/// A series record (`/series/{id}/extended?short=true`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Series {
    /// TheTVDB id.
    pub id: u64,
    /// Name in the original language.
    #[serde(default)]
    pub name: Option<String>,
    /// First air date.
    #[serde(default, rename = "firstAired")]
    pub first_aired: Option<String>,
    /// Ids elsewhere.
    #[serde(default, rename = "remoteIds")]
    pub remote_ids: Vec<RemoteId>,
}

impl Series {
    /// The TMDB tv id TheTVDB links the series to.
    pub fn tmdb_id(&self) -> Option<u64> {
        self.remote_ids
            .iter()
            .find(|r| {
                r.kind == Some(REMOTE_TMDB)
                    || r.source_name
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case("TheMovieDB.com"))
            })
            .and_then(|r| r.id.trim().parse().ok())
    }
}

/// An episode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Episode {
    /// TheTVDB episode id.
    pub id: u64,
    /// Title in the original language.
    #[serde(default)]
    pub name: Option<String>,
    /// Overview in the original language.
    #[serde(default)]
    pub overview: Option<String>,
    /// Air date, `YYYY-MM-DD`.
    #[serde(default)]
    pub aired: Option<String>,
    /// Minutes.
    #[serde(default)]
    pub runtime: Option<u32>,
    /// Season in the default (aired) order; 0 for specials.
    #[serde(default, rename = "seasonNumber")]
    pub season_number: Option<u32>,
    /// Number in the season.
    #[serde(default)]
    pub number: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct EpisodesPage {
    #[serde(default)]
    episodes: Vec<Episode>,
}

#[derive(Debug, Deserialize)]
struct Token {
    token: String,
}

#[derive(Clone)]
struct Session {
    token: String,
    until: Duration,
}

/// The TheTVDB client. `Send + Sync`; methods take `&self` and block.
pub struct TvdbClient {
    config: TvdbConfig,
    keys: Keys,
    /// Subscriber PIN, if the key needs one.
    pin: Option<ApiKey>,
    fetcher: Fetcher,
    session: Mutex<Option<Session>>,
}

impl std::fmt::Debug for TvdbClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TvdbClient")
            .field("config", &self.config)
            .field("keys", &self.keys)
            .finish_non_exhaustive()
    }
}

impl TvdbClient {
    /// A client with the user's `key` (no project key exists) over the
    /// injected transport, cache and clock.
    pub fn new(
        config: TvdbConfig,
        key: Option<&str>,
        transport: Arc<dyn HttpTransport>,
        cache: Arc<dyn ProviderCache>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            fetcher: Fetcher {
                provider: PROVIDER,
                transport,
                cache,
                clock,
                limiter: RateLimiter::new(config.rate_limit),
                retry: config.retry,
                flight: Default::default(),
            },
            keys: Keys::none().with_user_key(key),
            pin: None,
            config,
            session: Mutex::new(None),
        }
    }

    /// A live client: HTTPS via `ureq`, the system clock and the given cache.
    #[cfg(feature = "http")]
    pub fn live(config: TvdbConfig, key: Option<&str>, cache: Arc<dyn ProviderCache>) -> Self {
        let transport = crate::http::UreqTransport::new(&config.user_agent);
        Self::new(
            config,
            key,
            Arc::new(transport),
            cache,
            Arc::new(crate::clock::SystemClock::default()),
        )
    }

    /// Adds the subscriber PIN TheTVDB asks for with keys of the
    /// "user-supported" model; keys from a person's own API dashboard
    /// need none. Sent only to `/login`, like the key.
    #[must_use]
    pub fn with_pin(mut self, pin: Option<&str>) -> Self {
        self.pin = pin.and_then(ApiKey::new);
        self
    }

    /// The user gave a key.
    pub fn has_key(&self) -> bool {
        self.keys.active().is_some()
    }

    /// Time until the next request goes out, if the client waits.
    pub fn waiting(&self) -> Option<crate::rate::Wait> {
        self.fetcher.limiter.waiting(self.fetcher.clock.as_ref())
    }

    /// `/search?type=series`: series named like `query`, of `year` if
    /// given.
    pub fn search_series(
        &self,
        query: &str,
        year: Option<u32>,
    ) -> Result<Vec<SearchResult>, TvdbError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut params = vec![
            ("query".to_owned(), query.to_owned()),
            ("type".to_owned(), "series".to_owned()),
        ];
        if let Some(y) = year {
            params.push(("year".into(), y.to_string()));
        }
        let page: Envelope<Vec<SearchResult>> = self.get_json("/search", params)?;
        Ok(page.data)
    }

    /// `/series/{id}/extended?short=true`: name, first air date and the ids
    /// elsewhere; `None` if TheTVDB does not know it.
    pub fn series(&self, id: u64) -> Result<Option<Series>, TvdbError> {
        match self.get_json::<Envelope<Series>>(
            &format!("/series/{id}/extended"),
            vec![("short".into(), "true".into())],
        ) {
            Ok(s) => Ok(Some(s.data)),
            Err(TvdbError::Fetch(FetchError::NotFound { .. })) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Episodes of series `id` in the default (aired) order that aired on
    /// `date` (`YYYY-MM-DD`).
    pub fn episodes_by_air_date(&self, id: u64, date: &str) -> Result<Vec<Episode>, TvdbError> {
        if !crate::tvmaze::is_date(date) {
            return Ok(Vec::new());
        }
        self.episodes(id, vec![("airDate".into(), date.to_owned())])
    }

    fn episodes(
        &self,
        id: u64,
        mut params: Vec<(String, String)>,
    ) -> Result<Vec<Episode>, TvdbError> {
        params.push(("page".into(), "0".into()));
        match self
            .get_json::<Envelope<EpisodesPage>>(&format!("/series/{id}/episodes/default"), params)
        {
            Ok(p) => Ok(p.data.episodes),
            Err(TvdbError::Fetch(FetchError::NotFound { .. })) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Deletes all cached TheTVDB data.
    pub fn purge_all(&self) -> usize {
        self.fetcher.purge_all()
    }

    fn key(&self) -> Result<&ApiKey, TvdbError> {
        self.keys.active().map(|(k, _)| k).ok_or(TvdbError::NoKey)
    }

    fn base(&self) -> String {
        self.config.base_url.trim_end_matches('/').to_owned()
    }

    /// The bearer token, logging in when there is none or it is old.
    fn token(&self) -> Result<String, TvdbError> {
        let now = self.fetcher.clock.monotonic();
        if let Some(s) = self
            .session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .filter(|s| s.until > now)
        {
            return Ok(s.token.clone());
        }
        let mut body = serde_json::json!({ "apikey": self.key()?.expose() });
        if let Some(pin) = &self.pin {
            body["pin"] = pin.expose().into();
        }
        let body = body.to_string();
        let request = HttpRequest {
            base: self.base(),
            path: "/login".into(),
            query: Vec::new(),
            headers: vec![
                ("accept".into(), "application/json".into()),
                ("user-agent".into(), self.config.user_agent.clone()),
            ],
        };
        self.fetcher.limiter.acquire(self.fetcher.clock.as_ref());
        let response = self
            .fetcher
            .transport
            .post_json(&request, &body)
            .map_err(|e| TvdbError::Fetch(FetchError::Transport(e.0)))?;
        match response.status {
            200..=299 => {}
            401 | 403 => {
                return Err(TvdbError::KeyRejected(
                    message(&response).unwrap_or_else(|| format!("HTTP {}", response.status)),
                ));
            }
            status => {
                return Err(TvdbError::Fetch(FetchError::Http {
                    provider: PROVIDER,
                    status,
                    message: message(&response).unwrap_or_else(|| format!("HTTP {status}")),
                }));
            }
        }
        let token: Envelope<Token> = serde_json::from_str(&response.body)
            .map_err(|e| TvdbError::Fetch(FetchError::Decode(format!("TheTVDB /login: {e}"))))?;
        let session = Session {
            token: token.data.token,
            until: now + TOKEN_TTL,
        };
        *self.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(session.clone());
        Ok(session.token)
    }

    fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: Vec<(String, String)>,
    ) -> Result<T, TvdbError> {
        let mut renewed = false;
        loop {
            let token = self.token()?;
            let request = HttpRequest {
                base: self.base(),
                path: path.to_owned(),
                query: query.clone(),
                headers: vec![
                    ("accept".into(), "application/json".into()),
                    ("user-agent".into(), self.config.user_agent.clone()),
                    ("authorization".into(), format!("Bearer {token}")),
                ],
            };
            match self.fetcher.fetch(
                &request,
                None,
                &TvdbHooks {
                    ttl: self.config.ttl,
                },
            ) {
                Ok(body) => {
                    return serde_json::from_str(&body).map_err(|e| {
                        TvdbError::Fetch(FetchError::Decode(format!("TheTVDB {path}: {e}")))
                    });
                }
                // An expired token: log in again, once.
                Err(FetchError::Http { status: 401, .. }) if !renewed => {
                    renewed = true;
                    *self.session.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
                Err(FetchError::Http {
                    status: 401,
                    message,
                    ..
                }) => return Err(TvdbError::KeyRejected(message)),
                Err(e) => return Err(e.into()),
            }
        }
    }
}

fn message(response: &HttpResponse) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(&response.body).ok()?;
    v.get("message")?.as_str().map(str::to_owned)
}

struct TvdbHooks {
    ttl: Duration,
}

impl Hooks for TvdbHooks {
    fn error_message(&self, response: &HttpResponse) -> Option<String> {
        message(response)
    }

    fn ttl(&self, _body: &str) -> Duration {
        self.ttl.min(MAX_CACHE_AGE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::MemoryCache;
    use crate::clock::ManualClock;
    use crate::replay::ReplayTransport;

    fn ok(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: vec![],
            body: body.into(),
        }
    }

    #[test]
    fn without_a_key_nothing_is_sent() {
        let t = Arc::new(ReplayTransport::default());
        let c = TvdbClient::new(
            TvdbConfig::default(),
            None,
            t.clone(),
            Arc::new(MemoryCache::default()),
            Arc::new(ManualClock::at_unix(1_800_000_000)),
        );
        assert!(!c.has_key());
        assert_eq!(c.search_series("x", None), Err(TvdbError::NoKey));
        assert!(t.requests().is_empty());
    }

    #[test]
    fn tmdb_remote_id() {
        let s: Series = serde_json::from_str(
            r#"{"id":1,"remoteIds":[{"id":"tt1","type":2,"sourceName":"IMDB"},
                {"id":"2224","type":12,"sourceName":"TheMovieDB.com"}]}"#,
        )
        .unwrap();
        assert_eq!(s.tmdb_id(), Some(2224));
    }

    #[test]
    fn a_rejected_key_is_reported() {
        let t = ReplayTransport::default();
        t.push(
            r#"/login {"apikey":"bad"}"#,
            HttpResponse {
                status: 401,
                headers: vec![],
                body: r#"{"status":"failure","message":"Unauthorized"}"#.into(),
            },
        );
        t.push("/x", ok("{}"));
        let c = TvdbClient::new(
            TvdbConfig::default(),
            Some("bad"),
            Arc::new(t),
            Arc::new(MemoryCache::default()),
            Arc::new(ManualClock::at_unix(1_800_000_000)),
        );
        assert!(matches!(
            c.search_series("x", None),
            Err(TvdbError::KeyRejected(_))
        ));
    }
}
