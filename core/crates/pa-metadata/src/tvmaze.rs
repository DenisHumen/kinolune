// SPDX-License-Identifier: MPL-2.0
//! TVmaze client (META-012): shows by title or by TheTVDB / IMDb id, and
//! the episodes of a show aired on a date — what daily shows need
//! (`The.Daily.Show.2025.03.14…`, research 03 §4.4, A11).
//!
//! Requests are plain `GET`s through the shared pipeline
//! ([`fetch`](crate::fetch)): cache → rate limit → retries → stale fallback.
//!
//! **Rules** (<https://www.tvmaze.com/api>, docs/legal/providers.md, ADR-0132):
//! - no key; at least 20 calls every 10 seconds per IP are allowed, the
//!   client stays at two a second (burst 5) and retries 429 with backoff;
//! - the data is licensed **CC BY-SA 4.0**: the app credits TVmaze with a
//!   link ([`attribution::TVMAZE`](crate::attribution::TVMAZE)) and keeps
//!   the texts as they are (summaries lose only their HTML tags);
//! - images are not used (they are not covered by the data licence);
//! - only a show title or a TheTVDB / IMDb id and a date leave the device.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::cache::{MAX_CACHE_AGE, ProviderCache};
use crate::clock::Clock;
use crate::fetch::{FetchError, Fetcher, Hooks, year_of};
use crate::http::{HttpRequest, HttpResponse, HttpTransport};
use crate::rate::{RateLimit, RateLimiter, RetryPolicy};

/// Provider id in cache rows and `external_ids`.
pub const PROVIDER: &str = "tvmaze";

/// Production API base.
pub const DEFAULT_BASE_URL: &str = "https://api.tvmaze.com";

/// How long responses stay fresh. Every value is capped at
/// [`MAX_CACHE_AGE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheTtl {
    /// Searches and id lookups.
    pub show: Duration,
    /// Episodes of a date.
    pub episodes: Duration,
}

impl Default for CacheTtl {
    fn default() -> Self {
        const DAY: u64 = 24 * 3600;
        Self {
            show: Duration::from_secs(7 * DAY),
            episodes: Duration::from_secs(30 * DAY),
        }
    }
}

/// Client settings.
#[derive(Debug, Clone)]
pub struct TvMazeConfig {
    /// API base URL.
    pub base_url: String,
    /// `User-Agent` of every request.
    pub user_agent: String,
    /// Client-side rate limit.
    pub rate_limit: RateLimit,
    /// Retry policy for 429 and 5xx.
    pub retry: RetryPolicy,
    /// Freshness of cached responses.
    pub ttl: CacheTtl,
}

impl Default for TvMazeConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            user_agent: crate::USER_AGENT.to_owned(),
            rate_limit: RateLimit {
                burst: 5,
                per_second: 2.0,
            },
            retry: RetryPolicy::default(),
            ttl: CacheTtl::default(),
        }
    }
}

/// Ids of a show at other providers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Externals {
    /// TheTVDB series id.
    #[serde(default)]
    pub thetvdb: Option<u64>,
    /// IMDb id (`tt0115147`).
    #[serde(default)]
    pub imdb: Option<String>,
}

/// A show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Show {
    /// TVmaze id.
    pub id: u64,
    /// Name.
    #[serde(default)]
    pub name: String,
    /// `Scripted`, `Talk Show`, `News`…
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    /// Main language (`English`).
    #[serde(default)]
    pub language: Option<String>,
    /// First air date, `YYYY-MM-DD`.
    #[serde(default)]
    pub premiered: Option<String>,
    /// HTML summary.
    #[serde(default)]
    pub summary: Option<String>,
    /// Ids elsewhere.
    #[serde(default)]
    pub externals: Externals,
}

impl Show {
    /// Year of the first air date.
    pub fn year(&self) -> Option<u32> {
        year_of(self.premiered.as_deref()?)
    }
}

/// A search hit.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SearchHit {
    /// TVmaze relevance.
    #[serde(default)]
    pub score: f64,
    /// The show.
    pub show: Show,
}

/// An episode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Episode {
    /// TVmaze episode id.
    pub id: u64,
    /// Title.
    #[serde(default)]
    pub name: Option<String>,
    /// Season in TVmaze numbering (daily shows: often the year).
    #[serde(default)]
    pub season: Option<u32>,
    /// Number in the season; `None` for specials.
    #[serde(default)]
    pub number: Option<u32>,
    /// `regular`, `significant_special`, `insignificant_special`.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    /// Air date, `YYYY-MM-DD`.
    #[serde(default)]
    pub airdate: Option<String>,
    /// Minutes.
    #[serde(default)]
    pub runtime: Option<u32>,
    /// HTML summary.
    #[serde(default)]
    pub summary: Option<String>,
}

impl Episode {
    /// A regular, numbered episode (not a special).
    pub fn is_regular(&self) -> bool {
        self.number.is_some() && self.kind.as_deref().is_none_or(|k| k == "regular")
    }
}

/// An id TVmaze can look a show up by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup<'a> {
    /// TheTVDB series id.
    TheTvdb(u64),
    /// IMDb id (`tt…`).
    Imdb(&'a str),
}

/// `YYYY-MM-DD` with plausible numbers.
pub fn is_date(date: &str) -> bool {
    let b = date.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
        && matches!(date[5..7].parse::<u8>(), Ok(1..=12))
        && matches!(date[8..10].parse::<u8>(), Ok(1..=31))
}

/// The TVmaze client. `Send + Sync`; methods take `&self` and block.
pub struct TvMazeClient {
    config: TvMazeConfig,
    fetcher: Fetcher,
}

impl std::fmt::Debug for TvMazeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TvMazeClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl TvMazeClient {
    /// A client with injected transport, cache and clock.
    pub fn new(
        config: TvMazeConfig,
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
    pub fn live(config: TvMazeConfig, cache: Arc<dyn ProviderCache>) -> Self {
        let transport = crate::http::UreqTransport::new(&config.user_agent);
        Self::new(
            config,
            Arc::new(transport),
            cache,
            Arc::new(crate::clock::SystemClock::default()),
        )
    }

    /// Settings in use.
    pub fn config(&self) -> &TvMazeConfig {
        &self.config
    }

    /// Time until the next request goes out, if the client waits.
    pub fn waiting(&self) -> Option<crate::rate::Wait> {
        self.fetcher.limiter.waiting(self.fetcher.clock.as_ref())
    }

    /// `/search/shows`: shows named like `query`, best first.
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>, FetchError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        self.get_json(
            "/search/shows",
            vec![("q".into(), query.to_owned())],
            self.config.ttl.show,
        )
    }

    /// `/lookup/shows`: the show with this TheTVDB or IMDb id; `None` if
    /// TVmaze does not know it.
    pub fn lookup(&self, id: Lookup<'_>) -> Result<Option<Show>, FetchError> {
        let param = match id {
            Lookup::TheTvdb(id) => ("thetvdb".to_owned(), id.to_string()),
            Lookup::Imdb(id) => {
                let id = id.trim();
                if !id.starts_with("tt") || id.len() < 3 {
                    return Ok(None);
                }
                ("imdb".to_owned(), id.to_owned())
            }
        };
        not_found_is_none(self.get_json("/lookup/shows", vec![param], self.config.ttl.show))
    }

    /// `/shows/{id}/episodesbydate`: the episodes of show `id` aired on
    /// `date` (`YYYY-MM-DD`); empty when none did.
    pub fn episodes_by_date(&self, show: u64, date: &str) -> Result<Vec<Episode>, FetchError> {
        if !is_date(date) {
            return Ok(Vec::new());
        }
        Ok(not_found_is_none(self.get_json(
            &format!("/shows/{show}/episodesbydate"),
            vec![("date".into(), date.to_owned())],
            self.config.ttl.episodes,
        ))?
        .unwrap_or_default())
    }

    /// Deletes all cached TVmaze data.
    pub fn purge_all(&self) -> usize {
        self.fetcher.purge_all()
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
        let body = self.fetcher.fetch(&request, None, &TvMazeHooks { ttl })?;
        serde_json::from_str(&body).map_err(|e| FetchError::Decode(format!("TVmaze {path}: {e}")))
    }
}

fn not_found_is_none<T>(r: Result<T, FetchError>) -> Result<Option<T>, FetchError> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(FetchError::NotFound { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

struct TvMazeHooks {
    ttl: Duration,
}

impl Hooks for TvMazeHooks {
    fn error_message(&self, response: &HttpResponse) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(&response.body).ok()?;
        v.get("message")?.as_str().map(str::to_owned)
    }

    fn ttl(&self, _body: &str) -> Duration {
        self.ttl.min(MAX_CACHE_AGE)
    }
}

/// Plain text of a TVmaze HTML summary: tags dropped (paragraphs and line
/// breaks become spaces), the few entities TVmaze uses decoded, whitespace
/// collapsed.
pub fn plain_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut tag: Option<String> = None;
    for c in html.chars() {
        match (&mut tag, c) {
            (None, '<') => tag = Some(String::new()),
            (Some(t), '>') => {
                let name = t.trim_start_matches('/').to_ascii_lowercase();
                if name.starts_with('p') || name.starts_with("br") || name.starts_with("li") {
                    out.push(' ');
                }
                tag = None;
            }
            (Some(t), c) => t.push(c),
            (None, c) => out.push(c),
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert!(is_date("2025-03-14"));
        assert!(!is_date("2025-13-14"));
        assert!(!is_date("2025-03-00"));
        assert!(!is_date("2025/03/14"));
        assert!(!is_date("25-03-14"));
    }

    #[test]
    fn summaries_lose_their_tags() {
        assert_eq!(
            plain_text("<p>Jon &amp; guests<br>talk.</p>"),
            "Jon & guests talk."
        );
    }

    #[test]
    fn specials_are_not_regular() {
        let e = Episode {
            number: None,
            kind: Some("significant_special".into()),
            ..Default::default()
        };
        assert!(!e.is_regular());
        let e = Episode {
            number: Some(3),
            ..Default::default()
        };
        assert!(e.is_regular());
    }
}
