// SPDX-License-Identifier: MPL-2.0
//! Open Library client (MUS-019): book search by title and author, the
//! description of a work, and cover URLs.
//!
//! Requests are plain `GET`s through the shared pipeline
//! ([`fetch`](crate::fetch)): cache → rate limit → retries → stale fallback.
//!
//! **Rules** (docs/legal/providers.md, Open Library; ADR-0114):
//! - no key; requests carry a `User-Agent` with the app name and the project
//!   URL (never a personal e-mail address), and stay at **one request per
//!   second** — the limit Open Library sets for clients that do not send a
//!   contact e-mail; 429 and 5xx are retried with backoff;
//! - only what a person looks at is asked for: one book at a time, through
//!   `search.json` and `/works/{id}.json`, never bulk downloads; answers
//!   are cached (searches for a month, works for three);
//! - the Internet Archive asserts no rights over the catalogue data; covers
//!   are shown from `covers.openlibrary.org` by cover id (not rate limited)
//!   and cached on the device only, never bundled or committed.
//!
//! Only a title and an author leave the device — read from the names and
//! tags of the user's files, never the files, paths or audio.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::cache::{MAX_CACHE_AGE, ProviderCache};
use crate::clock::Clock;
use crate::fetch::{FetchError, Fetcher, Hooks};
use crate::http::{HttpRequest, HttpResponse, HttpTransport};
use crate::rate::{RateLimit, RateLimiter, RetryPolicy};

/// Provider id in cache rows.
pub const PROVIDER: &str = "openlibrary";

/// Production API base.
pub const DEFAULT_BASE_URL: &str = "https://openlibrary.org";

/// Cover image host.
pub const COVERS_BASE_URL: &str = "https://covers.openlibrary.org";

/// Fields a search asks for (smaller answers, stable cassettes).
pub const SEARCH_FIELDS: &str =
    "key,title,subtitle,author_name,first_publish_year,cover_i,edition_count,language";

/// URL of the cover with Open Library cover id `id`, large size (by id,
/// which the covers host does not rate limit).
pub fn cover_url(id: i64) -> String {
    format!("{COVERS_BASE_URL}/b/id/{id}-L.jpg")
}

/// How long responses stay fresh. Every value is capped at
/// [`MAX_CACHE_AGE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheTtl {
    /// Searches.
    pub search: Duration,
    /// Works.
    pub work: Duration,
}

impl Default for CacheTtl {
    fn default() -> Self {
        const DAY: u64 = 24 * 3600;
        Self {
            search: Duration::from_secs(30 * DAY),
            work: Duration::from_secs(90 * DAY),
        }
    }
}

/// Client settings.
#[derive(Debug, Clone)]
pub struct OpenLibraryConfig {
    /// API base URL.
    pub base_url: String,
    /// `User-Agent` of every request.
    pub user_agent: String,
    /// Client-side rate limit: one request per second, no burst.
    pub rate_limit: RateLimit,
    /// Results per search.
    pub search_limit: u32,
    /// Retry policy for 429 and 5xx.
    pub retry: RetryPolicy,
    /// Freshness of cached responses.
    pub ttl: CacheTtl,
}

impl Default for OpenLibraryConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            user_agent: crate::USER_AGENT.to_owned(),
            rate_limit: RateLimit {
                burst: 1,
                per_second: 1.0,
            },
            search_limit: 10,
            retry: RetryPolicy::default(),
            ttl: CacheTtl::default(),
        }
    }
}

/// A search result: one work.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SearchDoc {
    /// `/works/OL123W`.
    pub key: String,
    /// Title of the work.
    #[serde(default)]
    pub title: String,
    /// Subtitle.
    #[serde(default)]
    pub subtitle: Option<String>,
    /// Authors as Open Library names them.
    #[serde(default)]
    pub author_name: Vec<String>,
    /// Year of the first edition.
    #[serde(default)]
    pub first_publish_year: Option<i64>,
    /// Cover id ([`cover_url`]).
    #[serde(default)]
    pub cover_i: Option<i64>,
    /// Editions Open Library knows.
    #[serde(default)]
    pub edition_count: Option<i64>,
    /// MARC language codes of the editions (`rus`, `eng`).
    #[serde(default)]
    pub language: Vec<String>,
}

impl SearchDoc {
    /// `OL123W` of `/works/OL123W`.
    pub fn work_id(&self) -> &str {
        self.key.rsplit('/').next().unwrap_or(&self.key)
    }
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    #[serde(default)]
    docs: Vec<SearchDoc>,
}

/// A text field that is either a string or `{"type": …, "value": …}`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Text {
    /// A bare string.
    Plain(String),
    /// A typed text value.
    Typed {
        /// The text.
        #[serde(default)]
        value: String,
    },
}

impl Text {
    /// The text itself.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Plain(s) => s,
            Self::Typed { value } => value,
        }
    }
}

/// A work (`/works/{id}.json`): what a search does not give.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Work {
    /// `/works/OL123W`.
    #[serde(default)]
    pub key: String,
    /// Title of the work.
    #[serde(default)]
    pub title: String,
    /// Description, often markdown with reference links.
    #[serde(default)]
    pub description: Option<Text>,
    /// Cover ids.
    #[serde(default)]
    pub covers: Vec<i64>,
    /// Subjects.
    #[serde(default)]
    pub subjects: Vec<String>,
}

/// `OL123W` if `id` is an Open Library work id.
fn work_id(id: &str) -> Option<&str> {
    let id = id.trim().trim_start_matches("/works/");
    let digits = id.strip_prefix("OL")?.strip_suffix('W')?;
    (!digits.is_empty() && digits.len() <= 12 && digits.bytes().all(|b| b.is_ascii_digit()))
        .then_some(id)
}

/// The Open Library client. `Send + Sync`; methods take `&self` and block.
pub struct OpenLibraryClient {
    config: OpenLibraryConfig,
    fetcher: Fetcher,
}

impl std::fmt::Debug for OpenLibraryClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenLibraryClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl OpenLibraryClient {
    /// A client with injected transport, cache and clock.
    pub fn new(
        config: OpenLibraryConfig,
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
            config,
        }
    }

    /// A live client: HTTPS via `ureq`, the system clock and the given cache.
    #[cfg(feature = "http")]
    pub fn live(config: OpenLibraryConfig, cache: Arc<dyn ProviderCache>) -> Self {
        let transport = crate::http::UreqTransport::new(&config.user_agent);
        Self::new(
            config,
            Arc::new(transport),
            cache,
            Arc::new(crate::clock::SystemClock::default()),
        )
    }

    /// Settings in use.
    pub fn config(&self) -> &OpenLibraryConfig {
        &self.config
    }

    /// `search.json`: works titled `title`, by `author` if given.
    pub fn search(&self, title: &str, author: Option<&str>) -> Result<Vec<SearchDoc>, FetchError> {
        let mut query = vec![("title".to_owned(), title.trim().to_owned())];
        if let Some(a) = author.map(str::trim).filter(|a| !a.is_empty()) {
            query.push(("author".into(), a.to_owned()));
        }
        query.push(("fields".into(), SEARCH_FIELDS.into()));
        query.push((
            "limit".into(),
            self.config.search_limit.clamp(1, 100).to_string(),
        ));
        let found: SearchResult = self.get_json("/search.json", query, self.config.ttl.search)?;
        Ok(found.docs)
    }

    /// `/works/{id}.json`: description, covers and subjects of a work.
    pub fn work(&self, id: &str) -> Result<Work, FetchError> {
        let Some(id) = work_id(id) else {
            return Err(FetchError::NotFound {
                provider: PROVIDER,
                what: format!("work {id:?} (not a work id)"),
            });
        };
        self.get_json(
            &format!("/works/{id}.json"),
            Vec::new(),
            self.config.ttl.work,
        )
    }

    fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: Vec<(String, String)>,
        ttl: Duration,
    ) -> Result<T, FetchError> {
        let request = HttpRequest {
            base: self.config.base_url.trim_end_matches('/').to_owned(),
            path: path.to_owned(),
            query,
            headers: vec![
                ("accept".into(), "application/json".into()),
                ("user-agent".into(), self.config.user_agent.clone()),
            ],
        };
        let body = self
            .fetcher
            .fetch(&request, None, &OpenLibraryHooks { ttl })?;
        serde_json::from_str(&body)
            .map_err(|e| FetchError::Decode(format!("Open Library {path}: {e}")))
    }
}

struct OpenLibraryHooks {
    ttl: Duration,
}

impl Hooks for OpenLibraryHooks {
    fn error_message(&self, response: &HttpResponse) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(&response.body).ok()?;
        v.get("error")?.as_str().map(str::to_owned)
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

    fn client(t: ReplayTransport) -> OpenLibraryClient {
        OpenLibraryClient::new(
            OpenLibraryConfig::default(),
            Arc::new(t),
            Arc::new(MemoryCache::default()),
            Arc::new(ManualClock::at_unix(1_700_000_000)),
        )
    }

    fn ok(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: vec![],
            body: body.into(),
        }
    }

    #[test]
    fn work_ids() {
        assert_eq!(work_id("/works/OL45804W"), Some("OL45804W"));
        assert_eq!(work_id("OL45804W"), Some("OL45804W"));
        assert_eq!(work_id("OL45804M"), None);
        assert_eq!(work_id("../OL1W"), None);
    }

    #[test]
    fn search_and_work() {
        let t = ReplayTransport::default();
        t.push(
            &format!(
                "/search.json?author=Author&fields={}&limit=10&title=Book",
                SEARCH_FIELDS.replace(',', "%2C")
            ),
            ok(r#"{"docs":[{"key":"/works/OL1W","title":"Book","author_name":["Author"],"cover_i":7}]}"#),
        );
        t.push(
            "/works/OL1W.json",
            ok(r#"{"key":"/works/OL1W","title":"Book","description":{"type":"/type/text","value":"About."}}"#),
        );
        let c = client(t);
        let docs = c.search("Book", Some("Author")).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].work_id(), "OL1W");
        assert_eq!(
            cover_url(docs[0].cover_i.unwrap()),
            "https://covers.openlibrary.org/b/id/7-L.jpg"
        );
        let w = c.work("/works/OL1W").unwrap();
        assert_eq!(w.description.unwrap().as_str(), "About.");
    }

    #[test]
    fn plain_descriptions() {
        let w: Work = serde_json::from_str(r#"{"description":"Text"}"#).unwrap();
        assert_eq!(w.description.unwrap().as_str(), "Text");
    }
}
