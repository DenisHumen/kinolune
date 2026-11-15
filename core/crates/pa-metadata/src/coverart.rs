// SPDX-License-Identifier: MPL-2.0
//! Cover Art Archive client (META-021): which cover images a release group
//! has.
//!
//! Only the image index (`/release-group/{mbid}`, JSON) is fetched here; the
//! images themselves are downloaded by the image cache (`pa-imagecache`,
//! LIB-013) when the UI shows them, from the stable size URLs
//! [`front_url`] builds (`…/release-group/{mbid}/front-500`; the archive
//! redirects them to the current file).
//!
//! **Rules** (docs/legal/providers.md, Cover Art Archive): no published
//! limit, but the archive answers 503 when overloaded — the client keeps to
//! one request per second and retries 503 with backoff. The images are not
//! licensed by the archive: they are shown and cached on the user's device
//! only, never bundled or committed.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

use crate::cache::{MAX_CACHE_AGE, ProviderCache};
use crate::clock::Clock;
use crate::fetch::{FetchError, Fetcher, Hooks};
use crate::http::{HttpRequest, HttpResponse, HttpTransport};
use crate::musicbrainz::model::mbid;
use crate::rate::{RateLimit, RateLimiter, RetryPolicy};

/// Provider id in cache rows.
pub const PROVIDER: &str = "coverartarchive";

/// Production base.
pub const DEFAULT_BASE_URL: &str = "https://coverartarchive.org";

/// Thumbnail widths the archive serves (`front-250`, `front-500`,
/// `front-1200`).
pub const SIZES: [u32; 3] = [250, 500, 1200];

/// Size stored in the library: the album pages show covers up to ~500 px.
pub const STORED_SIZE: u32 = 500;

/// URL of the front cover of a release group at `size` (one of [`SIZES`]).
pub fn front_url(release_group_mbid: &str, size: u32) -> String {
    format!("{DEFAULT_BASE_URL}/release-group/{release_group_mbid}/front-{size}")
}

/// Client settings.
#[derive(Debug, Clone)]
pub struct CoverArtConfig {
    /// Base URL.
    pub base_url: String,
    /// `User-Agent` of every request.
    pub user_agent: String,
    /// Client-side rate limit.
    pub rate_limit: RateLimit,
    /// Retry policy for 503.
    pub retry: RetryPolicy,
    /// Freshness of cached indexes.
    pub ttl: Duration,
}

impl Default for CoverArtConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            user_agent: crate::musicbrainz::USER_AGENT.to_owned(),
            rate_limit: RateLimit {
                burst: 1,
                per_second: 1.0,
            },
            retry: RetryPolicy::default(),
            ttl: Duration::from_secs(30 * 24 * 3600),
        }
    }
}

/// One image of an index.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Image {
    /// The main front cover.
    pub front: bool,
    /// `Front`, `Back`, `Booklet`…
    pub types: Vec<String>,
    /// Approved by an editor.
    pub approved: bool,
    /// Full-size URL.
    pub image: String,
}

/// `/release-group/{mbid}` response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Index {
    /// Images of the release chosen for the group.
    pub images: Vec<Image>,
    /// URL of that release on musicbrainz.org.
    pub release: String,
}

impl Index {
    /// Whether a front cover exists.
    pub fn has_front(&self) -> bool {
        self.images
            .iter()
            .any(|i| i.front || i.types.iter().any(|t| t == "Front"))
    }
}

/// The Cover Art Archive client. `Send + Sync`; methods block.
pub struct CoverArtClient {
    config: CoverArtConfig,
    fetcher: Fetcher,
}

impl std::fmt::Debug for CoverArtClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoverArtClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl CoverArtClient {
    /// A client with injected transport, cache and clock.
    pub fn new(
        config: CoverArtConfig,
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
            },
            config,
        }
    }

    /// A live client: HTTPS via `ureq`, the system clock and the given cache.
    #[cfg(feature = "http")]
    pub fn live(config: CoverArtConfig, cache: Arc<dyn ProviderCache>) -> Self {
        let transport = crate::http::UreqTransport::new(&config.user_agent);
        Self::new(
            config,
            Arc::new(transport),
            cache,
            Arc::new(crate::clock::SystemClock::default()),
        )
    }

    /// The image index of a release group; [`FetchError::NotFound`] when it
    /// has no images.
    pub fn release_group(&self, release_group_mbid: &str) -> Result<Index, FetchError> {
        let Some(id) = mbid(release_group_mbid) else {
            return Err(FetchError::NotFound {
                provider: PROVIDER,
                what: format!("release group {release_group_mbid:?} (not an MBID)"),
            });
        };
        let request = HttpRequest {
            base: self.config.base_url.trim_end_matches('/').to_owned(),
            path: format!("/release-group/{id}"),
            query: Vec::new(),
            headers: vec![
                ("accept".into(), "application/json".into()),
                ("user-agent".into(), self.config.user_agent.clone()),
            ],
        };
        let body = self.fetcher.fetch(
            &request,
            None,
            &CoverArtHooks {
                ttl: self.config.ttl,
            },
        )?;
        serde_json::from_str(&body)
            .map_err(|e| FetchError::Decode(format!("Cover Art Archive {id}: {e}")))
    }

    /// URL of the release group's front cover at [`STORED_SIZE`], if it has
    /// one. `Ok(None)`: no images.
    pub fn front(&self, release_group_mbid: &str) -> Result<Option<String>, FetchError> {
        match self.release_group(release_group_mbid) {
            Ok(index) if index.has_front() => Ok(Some(front_url(
                &release_group_mbid.trim().to_ascii_lowercase(),
                STORED_SIZE,
            ))),
            Ok(_) | Err(FetchError::NotFound { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

struct CoverArtHooks {
    ttl: Duration,
}

impl Hooks for CoverArtHooks {
    fn error_message(&self, response: &HttpResponse) -> Option<String> {
        let body = response.body.trim();
        (!body.is_empty() && body.len() < 200 && !body.starts_with('<')).then(|| body.to_owned())
    }

    fn ttl(&self, _body: &str) -> Duration {
        self.ttl.min(MAX_CACHE_AGE)
    }
}
