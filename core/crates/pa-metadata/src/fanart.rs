// SPDX-License-Identifier: MPL-2.0
//! Fanart.tv client (META-012): the clear logo and the clear art of a movie
//! (by TMDB id) or a series (by TheTVDB id) — transparent title art TMDB
//! does not have.
//!
//! **Keys** (<https://fanart.tv/api-docs/>, docs/legal/providers.md, ADR-0132):
//! Fanart.tv gives a *project* key (`api_key`) and lets a person add their
//! own *personal* key (`client_key`), which shortens the delay before new
//! images reach the API. The project key is read at build time from
//! `PA_FANART_API_KEY` / `CARGO_PA_FANART_API_KEY`
//! ([`Keys::fanart_from_build`]); the personal key comes from Settings.
//! With both, both are sent; with only the personal key it is sent as
//! `api_key`; with neither nothing is sent and the client is off. Keys are
//! auth query parameters: never in cache keys, fixtures or logs.
//!
//! Requests go through the shared pipeline ([`fetch`](crate::fetch)):
//! cache (a month), two requests a second, 429/5xx retries. The images
//! belong to their artists; they are shown and cached on the device only,
//! with credit to Fanart.tv ([`attribution::FANART_TV`](crate::attribution::FANART_TV)).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

use crate::cache::{MAX_CACHE_AGE, ProviderCache};
use crate::clock::Clock;
use crate::fetch::{FetchError, Fetcher, Hooks};
use crate::http::{HttpRequest, HttpResponse, HttpTransport};
use crate::key::{ApiKey, KeySource, Keys};
use crate::rate::{RateLimit, RateLimiter, RetryPolicy};

/// Provider id in cache rows and `artwork.provider`.
pub const PROVIDER: &str = "fanart";

/// Production API base.
pub const DEFAULT_BASE_URL: &str = "https://webservice.fanart.tv/v3";

/// Client settings.
#[derive(Debug, Clone)]
pub struct FanartConfig {
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

impl Default for FanartConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            user_agent: crate::USER_AGENT.to_owned(),
            rate_limit: RateLimit {
                burst: 4,
                per_second: 2.0,
            },
            retry: RetryPolicy::default(),
            ttl: Duration::from_secs(30 * 24 * 3600),
        }
    }
}

/// One image.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Image {
    /// Fanart.tv image id.
    #[serde(default)]
    pub id: String,
    /// Full URL of the PNG.
    #[serde(default)]
    pub url: String,
    /// ISO 639-1 of the text on it; `00` or empty: no text.
    #[serde(default)]
    pub lang: String,
    /// Votes, as a string.
    #[serde(default)]
    pub likes: String,
}

impl Image {
    fn likes(&self) -> u64 {
        self.likes.trim().parse().unwrap_or(0)
    }
}

/// The art a work gets: the best clear logo and clear art for a language.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Art {
    /// Transparent title logo (`hdmovielogo`/`hdtvlogo`, else the SD one).
    pub logo: Option<Image>,
    /// Transparent character art (`hdmovieclearart`/`hdclearart`, else the
    /// SD one).
    pub clearart: Option<Image>,
}

impl Art {
    /// Nothing to show.
    pub fn is_empty(&self) -> bool {
        self.logo.is_none() && self.clearart.is_none()
    }
}

/// What a lookup is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A movie by TMDB id.
    Movie(u64),
    /// A series by TheTVDB id.
    Series(u64),
}

/// Image lists of an answer, by type (`hdmovielogo`, `clearart`…).
type Lists = BTreeMap<String, serde_json::Value>;

/// The best image of the first list in `types` that has one: in the
/// language `lang` (ISO 639-1), else English, else without text, else any;
/// most liked first.
fn best(lists: &Lists, types: &[&str], lang: &str) -> Option<Image> {
    let rank = |i: &Image| match i.lang.as_str() {
        l if l == lang => 0,
        "en" => 1,
        "00" | "" => 2,
        _ => 3,
    };
    types.iter().find_map(|t| {
        let images: Vec<Image> = serde_json::from_value(lists.get(*t)?.clone()).ok()?;
        images
            .into_iter()
            .filter(|i| i.url.starts_with("https://"))
            .min_by(|a, b| {
                rank(a)
                    .cmp(&rank(b))
                    .then_with(|| b.likes().cmp(&a.likes()))
                    .then_with(|| a.id.cmp(&b.id))
            })
    })
}

/// The Fanart.tv client. `Send + Sync`; methods take `&self` and block.
pub struct FanartClient {
    config: FanartConfig,
    keys: Keys,
    fetcher: Fetcher,
}

impl std::fmt::Debug for FanartClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FanartClient")
            .field("config", &self.config)
            .field("keys", &self.keys)
            .finish_non_exhaustive()
    }
}

impl FanartClient {
    /// A client with `keys` (project key plus the user's personal key) over
    /// the injected transport, cache and clock.
    pub fn new(
        config: FanartConfig,
        keys: Keys,
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
            keys,
            config,
        }
    }

    /// A live client with the build's project key and the user's personal
    /// key, if any.
    #[cfg(feature = "http")]
    pub fn live(
        config: FanartConfig,
        personal_key: Option<&str>,
        cache: Arc<dyn ProviderCache>,
    ) -> Self {
        let transport = crate::http::UreqTransport::new(&config.user_agent);
        Self::new(
            config,
            Keys::fanart_from_build().with_user_key(personal_key),
            Arc::new(transport),
            cache,
            Arc::new(crate::clock::SystemClock::default()),
        )
    }

    /// Where the `api_key` comes from; `None`: no key, the client is off.
    pub fn key_source(&self) -> Option<KeySource> {
        self.api_key().map(|(_, s)| s)
    }

    /// `api_key`: the project's, else the personal key.
    fn api_key(&self) -> Option<(&ApiKey, KeySource)> {
        self.keys
            .project()
            .map(|k| (k, KeySource::Project))
            .or_else(|| self.keys.user().map(|k| (k, KeySource::User)))
    }

    /// Clear logo and clear art of `target`, best for language `lang`
    /// (ISO 639-1); empty when Fanart.tv has none.
    pub fn art(&self, target: Target, lang: &str) -> Result<Art, FetchError> {
        let Some((api_key, source)) = self.api_key() else {
            return Ok(Art::default());
        };
        let path = match target {
            Target::Movie(id) => format!("/movies/{id}"),
            Target::Series(id) => format!("/tv/{id}"),
        };
        let mut query = vec![("api_key".to_owned(), api_key.expose().to_owned())];
        if source == KeySource::Project
            && let Some(personal) = self.keys.user()
        {
            query.push(("client_key".to_owned(), personal.expose().to_owned()));
        }
        let request = HttpRequest {
            base: self.config.base_url.trim_end_matches('/').to_owned(),
            path,
            query,
            headers: vec![
                ("accept".into(), "application/json".into()),
                ("user-agent".into(), self.config.user_agent.clone()),
            ],
        };
        let body = match self.fetcher.fetch(
            &request,
            None,
            &FanartHooks {
                ttl: self.config.ttl,
            },
        ) {
            Ok(b) => b,
            Err(FetchError::NotFound { .. }) => return Ok(Art::default()),
            Err(e) => return Err(e),
        };
        let lists: Lists = serde_json::from_str(&body)
            .map_err(|e| FetchError::Decode(format!("Fanart.tv {}: {e}", request.path)))?;
        let (logos, arts): (&[&str], &[&str]) = match target {
            Target::Movie(_) => (
                &["hdmovielogo", "movielogo"],
                &["hdmovieclearart", "movieart"],
            ),
            Target::Series(_) => (&["hdtvlogo", "clearlogo"], &["hdclearart", "clearart"]),
        };
        Ok(Art {
            logo: best(&lists, logos, lang),
            clearart: best(&lists, arts, lang),
        })
    }

    /// Deletes all cached Fanart.tv answers.
    pub fn purge_all(&self) -> usize {
        self.fetcher.purge_all()
    }
}

struct FanartHooks {
    ttl: Duration,
}

impl Hooks for FanartHooks {
    fn error_message(&self, response: &HttpResponse) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(&response.body).ok()?;
        v.get("error message")?.as_str().map(str::to_owned)
    }

    /// Fanart.tv answers 200 with `{"status":"error"}` for unknown ids in
    /// some versions: such bodies are not cached.
    fn check_ok(&self, body: &str) -> Result<(), FetchError> {
        let v: serde_json::Value =
            serde_json::from_str(body).map_err(|e| FetchError::Decode(e.to_string()))?;
        if v.get("status").and_then(serde_json::Value::as_str) == Some("error") {
            return Err(FetchError::NotFound {
                provider: PROVIDER,
                what: v
                    .get("error message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("not found")
                    .to_owned(),
            });
        }
        Ok(())
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

    fn client(keys: Keys, t: Arc<ReplayTransport>) -> FanartClient {
        FanartClient::new(
            FanartConfig::default(),
            keys,
            t,
            Arc::new(MemoryCache::default()),
            Arc::new(ManualClock::at_unix(1_800_000_000)),
        )
    }

    #[test]
    fn without_any_key_the_client_is_off() {
        let t = Arc::new(ReplayTransport::default());
        let c = client(Keys::none(), t.clone());
        assert_eq!(c.key_source(), None);
        assert!(c.art(Target::Movie(1), "ru").unwrap().is_empty());
        assert!(t.requests().is_empty());
    }

    #[test]
    fn language_then_english_then_textless_then_likes() {
        let lists: Lists = serde_json::from_str(
            r#"{"hdmovielogo":[
                {"id":"1","url":"https://a/1.png","lang":"de","likes":"9"},
                {"id":"2","url":"https://a/2.png","lang":"en","likes":"1"},
                {"id":"3","url":"https://a/3.png","lang":"en","likes":"4"},
                {"id":"4","url":"http://a/4.png","lang":"ru","likes":"9"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(best(&lists, &["hdmovielogo"], "ru").unwrap().id, "3");
        assert_eq!(best(&lists, &["hdmovielogo"], "de").unwrap().id, "1");
        assert!(best(&lists, &["movielogo"], "ru").is_none());
    }

    #[test]
    fn keys_go_as_api_key_and_client_key_and_never_into_the_cache_key() {
        let t = Arc::new(ReplayTransport::default());
        t.push(
            "/movies/7",
            HttpResponse {
                status: 200,
                headers: vec![],
                body:
                    r#"{"movieart":[{"id":"9","url":"https://a/9.png","lang":"00","likes":"0"}]}"#
                        .into(),
            },
        );
        let c = client(
            Keys::none()
                .with_project_key(Some("project"))
                .with_user_key(Some("mine")),
            t.clone(),
        );
        let art = c.art(Target::Movie(7), "ru").unwrap();
        assert_eq!(art.clearart.unwrap().id, "9");
        assert!(art.logo.is_none());
        let r = &t.requests()[0];
        assert_eq!(
            r.query,
            vec![
                ("api_key".to_owned(), "project".to_owned()),
                ("client_key".to_owned(), "mine".to_owned())
            ]
        );
        assert_eq!(r.canonical(), "/movies/7");

        // Only a personal key: it is the api_key.
        let t = Arc::new(ReplayTransport::default());
        t.push(
            "/tv/5",
            HttpResponse {
                status: 404,
                headers: vec![],
                body: "{}".into(),
            },
        );
        let c = client(Keys::none().with_user_key(Some("mine")), t.clone());
        assert_eq!(c.key_source(), Some(KeySource::User));
        assert!(c.art(Target::Series(5), "ru").unwrap().is_empty());
        assert_eq!(
            t.requests()[0].query,
            vec![("api_key".to_owned(), "mine".to_owned())]
        );
    }
}
