// SPDX-License-Identifier: MPL-2.0
//! MusicBrainz Web Service v2 client (META-021): releases, release groups
//! and artists by MBID, and release / artist search.
//!
//! Requests are plain `GET`s with `fmt=json` through the shared pipeline
//! ([`fetch`](crate::fetch)): cache → rate limit → retries → stale fallback.
//!
//! **Rules** (docs/legal/providers.md, MusicBrainz):
//! - **one request per second** on average per IP, otherwise MusicBrainz
//!   answers 503 to every request of the IP. The default bucket
//!   ([`MusicBrainzConfig::rate_limit`]) sends at most one request per
//!   second, without a burst; a 503 is retried with backoff;
//! - the `User-Agent` must let MusicBrainz contact the authors:
//!   `App/Version ( contact-url )` — [`USER_AGENT`] carries the app name,
//!   version and the project URL, never a personal e-mail address;
//! - core data is CC0; genres and tags are supplementary data
//!   (CC BY-NC-SA 3.0): the "About" screen credits MusicBrainz (UI-016).
//!
//! No key is needed. Cover images come from the Cover Art Archive, a
//! separate host with its own client ([`coverart`](crate::coverart)).

pub mod model;

use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::cache::{MAX_CACHE_AGE, ProviderCache};
use crate::clock::Clock;
use crate::fetch::{FetchError, Fetcher, Hooks};
use crate::http::{HttpRequest, HttpResponse, HttpTransport};
use crate::rate::{RateLimit, RateLimiter, RetryPolicy};

pub use model::*;

/// Provider id in cache rows.
pub const PROVIDER: &str = "musicbrainz";

/// Production API base.
pub const DEFAULT_BASE_URL: &str = "https://musicbrainz.org/ws/2";

/// `User-Agent` in the form MusicBrainz asks for:
/// `Kinolune/<version> ( <project URL> )`.
pub const USER_AGENT: &str = concat!(
    "Kinolune/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/DenisHumen/kinolune )"
);

/// `inc` of a release lookup.
const RELEASE_INC: &str = "artist-credits+release-groups+genres+recordings+labels+media";

/// How long responses stay fresh. Every value is capped at
/// [`MAX_CACHE_AGE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheTtl {
    /// Searches.
    pub search: Duration,
    /// Lookups by MBID.
    pub lookup: Duration,
}

impl Default for CacheTtl {
    fn default() -> Self {
        const DAY: u64 = 24 * 3600;
        Self {
            search: Duration::from_secs(7 * DAY),
            lookup: Duration::from_secs(30 * DAY),
        }
    }
}

/// Client settings.
#[derive(Debug, Clone)]
pub struct MusicBrainzConfig {
    /// API base URL.
    pub base_url: String,
    /// `User-Agent` of every request.
    pub user_agent: String,
    /// Client-side rate limit: one request per second, no burst.
    pub rate_limit: RateLimit,
    /// Results per search (MusicBrainz allows up to 100).
    pub search_limit: u32,
    /// Retry policy for 503 and 429.
    pub retry: RetryPolicy,
    /// Freshness of cached responses.
    pub ttl: CacheTtl,
}

impl Default for MusicBrainzConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            user_agent: USER_AGENT.to_owned(),
            rate_limit: RateLimit {
                burst: 1,
                per_second: 1.0,
            },
            search_limit: 25,
            retry: RetryPolicy::default(),
            ttl: CacheTtl::default(),
        }
    }
}

/// The MusicBrainz client. `Send + Sync`; methods take `&self` and block.
pub struct MusicBrainzClient {
    config: MusicBrainzConfig,
    fetcher: Fetcher,
}

impl std::fmt::Debug for MusicBrainzClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MusicBrainzClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Lucene phrase: the text in double quotes, `"` and `\` escaped.
fn phrase(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.trim().chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

impl MusicBrainzClient {
    /// A client with injected transport, cache and clock.
    pub fn new(
        config: MusicBrainzConfig,
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
    pub fn live(config: MusicBrainzConfig, cache: Arc<dyn ProviderCache>) -> Self {
        let transport = crate::http::UreqTransport::new(&config.user_agent);
        Self::new(
            config,
            Arc::new(transport),
            cache,
            Arc::new(crate::clock::SystemClock::default()),
        )
    }

    /// Settings in use.
    pub fn config(&self) -> &MusicBrainzConfig {
        &self.config
    }

    /// Deletes all cached MusicBrainz responses.
    pub fn purge_all(&self) -> usize {
        self.fetcher.purge_all()
    }

    /// `/release/{mbid}` with credits, release group, genres, labels and
    /// the tracks of every medium.
    pub fn release(&self, mbid: &str) -> Result<Release, FetchError> {
        self.lookup("release", mbid, RELEASE_INC)
    }

    /// `/release-group/{mbid}` with credits and genres.
    pub fn release_group(&self, mbid: &str) -> Result<ReleaseGroup, FetchError> {
        self.lookup("release-group", mbid, "artist-credits+genres")
    }

    /// `/artist/{mbid}` with genres.
    pub fn artist(&self, mbid: &str) -> Result<Artist, FetchError> {
        self.lookup("artist", mbid, "genres")
    }

    /// `/release?query=`: releases titled `title`, credited to `artist` if
    /// given. Results carry a search score and the media track counts.
    pub fn search_releases(
        &self,
        title: &str,
        artist: Option<&str>,
    ) -> Result<Vec<Release>, FetchError> {
        let mut query = format!("release:{}", phrase(title));
        if let Some(a) = artist.map(str::trim).filter(|a| !a.is_empty()) {
            query.push_str(" AND artist:");
            query.push_str(&phrase(a));
        }
        let found: ReleaseSearch = self.search("release", query)?;
        Ok(found.releases)
    }

    /// `/artist?query=`: artists named `name`.
    pub fn search_artists(&self, name: &str) -> Result<Vec<Artist>, FetchError> {
        let found: ArtistSearch = self.search("artist", format!("artist:{}", phrase(name)))?;
        Ok(found.artists)
    }

    // --- Plumbing --------------------------------------------------------

    fn lookup<T: DeserializeOwned>(
        &self,
        entity: &str,
        mbid: &str,
        inc: &str,
    ) -> Result<T, FetchError> {
        let Some(id) = model::mbid(mbid) else {
            return Err(FetchError::NotFound {
                provider: PROVIDER,
                what: format!("{entity} {mbid:?} (not an MBID)"),
            });
        };
        self.get_json(
            &format!("/{entity}/{id}"),
            vec![("inc".into(), inc.into())],
            self.config.ttl.lookup,
        )
    }

    fn search<T: DeserializeOwned>(&self, entity: &str, query: String) -> Result<T, FetchError> {
        self.get_json(
            &format!("/{entity}"),
            vec![
                ("query".into(), query),
                (
                    "limit".into(),
                    self.config.search_limit.clamp(1, 100).to_string(),
                ),
            ],
            self.config.ttl.search,
        )
    }

    fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        mut query: Vec<(String, String)>,
        ttl: Duration,
    ) -> Result<T, FetchError> {
        query.push(("fmt".into(), "json".into()));
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
            .fetch(&request, None, &MusicBrainzHooks { ttl })?;
        serde_json::from_str(&body)
            .map_err(|e| FetchError::Decode(format!("MusicBrainz {path}: {e}")))
    }
}

struct MusicBrainzHooks {
    ttl: Duration,
}

impl Hooks for MusicBrainzHooks {
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

    #[test]
    fn phrases_are_escaped() {
        assert_eq!(phrase(" a \"b\" \\c "), r#""a \"b\" \\c""#);
    }

    #[test]
    fn user_agent_has_app_and_project_url_only() {
        assert!(USER_AGENT.starts_with("Kinolune/"));
        assert!(USER_AGENT.ends_with("( https://github.com/DenisHumen/kinolune )"));
        assert!(!USER_AGENT.contains('@'));
    }
}
