// SPDX-License-Identifier: MPL-2.0
//! trace.moe client (META-031): which anime, which episode and which
//! moment a video frame comes from.
//!
//! Only the recognition fallback of `pa-catalog` (`frame_search`) uses it,
//! and only after the user has turned the setting on: a frame of the
//! user's own file leaves the device. Nothing here decides that; the
//! caller does.
//!
//! **Rules** (<https://soruly.github.io/trace.moe-api/>, «Account and
//! Limits», «Terms»):
//! - no API key: the free tier is identified by the IP address and has a
//!   **daily quota of 100 searches** in a rolling 24-hour window and a
//!   **concurrency of 1**; [`TraceMoeClient::me`] tells what is left and
//!   costs no quota;
//! - **100 requests a minute** per IP for every endpoint — the client
//!   sends one request at a time and at most one every
//!   [`TraceMoeConfig::min_interval`];
//! - HTTP 402 is either the quota used up ([`TraceMoeError::QuotaDepleted`]:
//!   never retried, the caller stops until later) or a second request
//!   while one is running; that, 429 and 503/504 (queue full, overloaded)
//!   are retried with backoff ([`RetryPolicy`], `Retry-After` honoured)
//!   and end as [`TraceMoeError::Busy`];
//! - failed searches (4xx, 5xx) do not count against the quota;
//! - images up to 25 MB; 640×360 is the recommended size (larger does not
//!   search better), JPEG artefacts do not hurt;
//! - not for commercial use: Kinolune is free and asks on the user's
//!   behalf from the user's device.
//!
//! Responses are not cached here: `pa-catalog` keeps the outcome per file
//! fingerprint, so a file is never searched twice.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

use crate::clock::Clock;
use crate::http::{HttpRequest, HttpResponse, HttpTransport};
use crate::rate::{RetryPolicy, parse_retry_after};

/// Provider id (logs, explanations).
pub const PROVIDER: &str = "trace_moe";

/// Production API base.
pub const DEFAULT_BASE_URL: &str = "https://api.trace.moe";

/// A frame of this width is what trace.moe recommends (640×360).
pub const RECOMMENDED_WIDTH: u32 = 640;

/// Largest image the API accepts.
pub const MAX_IMAGE_BYTES: usize = 25 * 1024 * 1024;

/// Client settings.
#[derive(Debug, Clone)]
pub struct TraceMoeConfig {
    /// API base, without a trailing slash.
    pub base_url: String,
    /// Least time between two requests (the published limit is 100 a
    /// minute; one a second leaves room for other apps on the same IP).
    pub min_interval: Duration,
    /// Retries of 429, 402 «concurrency» and 5xx.
    pub retry: RetryPolicy,
}

impl Default for TraceMoeConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            min_interval: Duration::from_secs(1),
            retry: RetryPolicy {
                max_retries: 3,
                base_delay: Duration::from_secs(2),
                max_delay: Duration::from_secs(60),
            },
        }
    }
}

/// Failures of [`TraceMoeClient`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TraceMoeError {
    /// HTTP 402: the daily quota is used up. Not retried; searches can go
    /// on when [`TraceMoeClient::me`] shows quota left.
    #[error("trace.moe search quota used up ({used} of {quota} in 24 hours)")]
    QuotaDepleted {
        /// Searches allowed in 24 hours (0 if the response did not say).
        quota: u32,
        /// Searches made in the last 24 hours.
        used: u32,
    },
    /// Still 429, 402 «concurrency» or 5xx after all retries.
    #[error("trace.moe is busy: HTTP {status} after {attempts} attempts")]
    Busy {
        /// Last status.
        status: u16,
        /// Requests made.
        attempts: u32,
    },
    /// Another HTTP error (400 broken image, 413 too large, 403 key…).
    #[error("trace.moe HTTP {status}: {message}")]
    Http {
        /// Status code.
        status: u16,
        /// The API's `error` text or a short description.
        message: String,
    },
    /// No response at all.
    #[error("network error: {0}")]
    Transport(String),
    /// The response did not have the expected shape.
    #[error("unexpected trace.moe response: {0}")]
    Decode(String),
}

/// The caller's quota (`GET /me`; the IP address it reports is not kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    /// Searches allowed in 24 hours.
    #[serde(default)]
    pub quota: u32,
    /// Searches made in the last 24 hours.
    #[serde(default)]
    pub quota_used: u32,
    /// Parallel searches allowed.
    #[serde(default)]
    pub concurrency: u32,
    /// Queue priority (0: lowest, the free tier).
    #[serde(default)]
    pub priority: u32,
}

impl Me {
    /// Searches left in the current window.
    #[must_use]
    pub fn left(&self) -> u32 {
        self.quota.saturating_sub(self.quota_used)
    }
}

/// Titles of an AniList entry.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct AniListTitle {
    /// Japanese.
    #[serde(default)]
    pub native: Option<String>,
    /// Romaji.
    #[serde(default)]
    pub romaji: Option<String>,
    /// English.
    #[serde(default)]
    pub english: Option<String>,
}

/// The AniList entry of a hit, as `anilistInfo` returns it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AniListInfo {
    /// AniList id.
    pub id: u64,
    /// MyAnimeList id.
    #[serde(default)]
    pub id_mal: Option<u64>,
    /// Native, romaji and English titles.
    #[serde(default)]
    pub title: AniListTitle,
    /// Other names.
    #[serde(default)]
    pub synonyms: Vec<String>,
    /// 18+ entry.
    #[serde(default)]
    pub is_adult: bool,
}

/// `anilist`: an id, or the entry with `anilistInfo`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum AniListRef {
    /// Just the id.
    Id(u64),
    /// The entry.
    Info(AniListInfo),
}

impl AniListRef {
    /// The AniList id.
    #[must_use]
    pub fn id(&self) -> u64 {
        match self {
            Self::Id(id) => *id,
            Self::Info(info) => info.id,
        }
    }

    /// The entry, if the search asked for it.
    #[must_use]
    pub fn info(&self) -> Option<&AniListInfo> {
        match self {
            Self::Id(_) => None,
            Self::Info(info) => Some(info),
        }
    }

    /// A title to show: English, romaji, native, else `AniList <id>`.
    #[must_use]
    pub fn title(&self) -> String {
        self.info()
            .and_then(|i| {
                i.title
                    .english
                    .clone()
                    .or_else(|| i.title.romaji.clone())
                    .or_else(|| i.title.native.clone())
            })
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| format!("AniList {}", self.id()))
    }
}

/// One search result. Preview URLs (`video`, `image`) are not kept: they
/// expire in minutes and are not needed.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Hit {
    /// The AniList entry.
    pub anilist: AniListRef,
    /// trace.moe's own file name (not ours).
    #[serde(default)]
    pub filename: String,
    /// Parsed from `filename` with aniep: a number, a string, an array or
    /// null.
    #[serde(default)]
    pub episode: serde_json::Value,
    /// First and last episode of trace.moe's file in AniList order; null
    /// for specials and unknown files.
    #[serde(default)]
    pub episode_start: Option<u32>,
    /// See `episode_start`.
    #[serde(default)]
    pub episode_end: Option<u32>,
    /// Length of trace.moe's file, seconds.
    #[serde(default)]
    pub duration: Option<f64>,
    /// Time of the matching frame in trace.moe's file, seconds.
    #[serde(default)]
    pub at: Option<f64>,
    /// Start of the matching scene, seconds.
    #[serde(default)]
    pub from: Option<f64>,
    /// End of the matching scene, seconds.
    #[serde(default)]
    pub to: Option<f64>,
    /// `0..=1`; below 0.9 the API says the hit is most likely wrong.
    pub similarity: f64,
}

impl Hit {
    /// The one episode the hit is from, in AniList order: `episode_start`
    /// when the file holds one episode, else a whole-number `episode`.
    #[must_use]
    pub fn episode_number(&self) -> Option<u32> {
        match (self.episode_start, self.episode_end) {
            (Some(s), Some(e)) if s == e && s > 0 => return Some(s),
            (Some(_), Some(_)) => return None,
            _ => {}
        }
        self.episode
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
    }
}

/// `POST /search` response.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    /// Frames searched.
    #[serde(default)]
    pub frame_count: u64,
    /// Empty when there is no error.
    #[serde(default)]
    pub error: String,
    /// Quota of the 24-hour window, when the server says.
    #[serde(default)]
    pub quota: Option<u32>,
    /// Searches made in the window.
    #[serde(default)]
    pub quota_used: Option<u32>,
    /// Best first.
    #[serde(default)]
    pub result: Vec<Hit>,
}

impl SearchResponse {
    /// The best hit.
    #[must_use]
    pub fn best(&self) -> Option<&Hit> {
        self.result.first()
    }
}

/// See the module docs. `Send + Sync`; methods block, and requests from
/// several threads go out one after another.
pub struct TraceMoeClient {
    config: TraceMoeConfig,
    transport: Arc<dyn HttpTransport>,
    clock: Arc<dyn Clock>,
    /// Monotonic time of the last request; held while a request runs
    /// (concurrency 1).
    gate: Mutex<Option<Duration>>,
}

impl std::fmt::Debug for TraceMoeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceMoeClient")
            .field("base_url", &self.config.base_url)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    error: String,
    #[serde(default)]
    quota: Option<u32>,
    #[serde(default, rename = "quotaUsed")]
    quota_used: Option<u32>,
}

impl TraceMoeClient {
    /// A client over `transport`.
    pub fn new(
        config: TraceMoeConfig,
        transport: Arc<dyn HttpTransport>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            config,
            transport,
            clock,
            gate: Mutex::new(None),
        }
    }

    /// The live client (HTTPS, [`crate::USER_AGENT`], 60 s timeout: the
    /// free tier waits in the lowest-priority queue).
    #[cfg(feature = "http")]
    pub fn live(config: TraceMoeConfig) -> Self {
        Self::new(
            config,
            Arc::new(crate::http::UreqTransport::with_timeout(
                crate::USER_AGENT,
                Duration::from_secs(60),
            )),
            Arc::new(crate::clock::SystemClock::default()),
        )
    }

    /// Settings in use.
    pub fn config(&self) -> &TraceMoeConfig {
        &self.config
    }

    fn request(&self, path: &str, query: &[(&str, &str)]) -> HttpRequest {
        HttpRequest {
            base: self.config.base_url.trim_end_matches('/').to_owned(),
            path: path.to_owned(),
            query: query
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            headers: Vec::new(),
        }
    }

    /// The quota of this IP address. Costs no search.
    ///
    /// # Errors
    ///
    /// As [`search`](Self::search), never `QuotaDepleted`.
    pub fn me(&self) -> Result<Me, TraceMoeError> {
        let request = self.request("/me", &[]);
        let response = self.send(|t| t.get(&request))?;
        serde_json::from_str(&response.body).map_err(|e| TraceMoeError::Decode(e.to_string()))
    }

    /// Searches one JPEG image (`/search?anilistInfo`): hits best first,
    /// each with its AniList entry.
    ///
    /// # Errors
    ///
    /// [`TraceMoeError::QuotaDepleted`] when the daily quota is used up,
    /// [`TraceMoeError::Busy`] when the server stayed busy,
    /// [`TraceMoeError::Http`] for a rejected image,
    /// [`TraceMoeError::Transport`] and [`TraceMoeError::Decode`].
    pub fn search(&self, jpeg: &[u8]) -> Result<SearchResponse, TraceMoeError> {
        if jpeg.len() > MAX_IMAGE_BYTES {
            return Err(TraceMoeError::Http {
                status: 413,
                message: "image larger than 25 MB".to_owned(),
            });
        }
        let request = self.request("/search", &[("anilistInfo", "")]);
        let response = self.send(|t| t.post_bytes(&request, "image/jpeg", jpeg))?;
        let parsed: SearchResponse = serde_json::from_str(&response.body)
            .map_err(|e| TraceMoeError::Decode(e.to_string()))?;
        if !parsed.error.is_empty() {
            return Err(TraceMoeError::Http {
                status: response.status,
                message: parsed.error,
            });
        }
        Ok(parsed)
    }

    /// One request with pacing, one at a time, and retries.
    fn send(
        &self,
        call: impl Fn(&dyn HttpTransport) -> Result<HttpResponse, crate::http::TransportError>,
    ) -> Result<HttpResponse, TraceMoeError> {
        let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut attempts = 0u32;
        loop {
            if let Some(last) = *gate {
                let next = last + self.config.min_interval;
                let now = self.clock.monotonic();
                if next > now {
                    self.clock.sleep(next - now);
                }
            }
            *gate = Some(self.clock.monotonic());
            attempts += 1;
            let response = call(self.transport.as_ref())
                .map_err(|e| TraceMoeError::Transport(e.0))?;
            let status = response.status;
            if (200..300).contains(&status) {
                return Ok(response);
            }
            let body: Option<ErrorBody> = serde_json::from_str(&response.body).ok();
            let message = body
                .as_ref()
                .map(|b| b.error.clone())
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| format!("HTTP {status}"));
            if status == 402 && is_quota(body.as_ref()) {
                let b = body.unwrap_or(ErrorBody {
                    error: String::new(),
                    quota: None,
                    quota_used: None,
                });
                tracing::warn!(target: "pa_metadata::tracemoe", "trace.moe: quota used up");
                return Err(TraceMoeError::QuotaDepleted {
                    quota: b.quota.unwrap_or(0),
                    used: b.quota_used.unwrap_or(0),
                });
            }
            let retryable = matches!(status, 402 | 429 | 500 | 502 | 503 | 504);
            if !retryable {
                return Err(TraceMoeError::Http { status, message });
            }
            if attempts > self.config.retry.max_retries {
                return Err(TraceMoeError::Busy { status, attempts });
            }
            let delay = self.config.retry.delay(
                attempts - 1,
                parse_retry_after(response.header("retry-after")),
            );
            tracing::info!(
                target: "pa_metadata::tracemoe",
                status,
                delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                "trace.moe: busy, backing off"
            );
            self.clock.sleep(delay);
        }
    }
}

/// A 402 that means the quota, not concurrency.
fn is_quota(body: Option<&ErrorBody>) -> bool {
    let Some(b) = body else {
        return false;
    };
    matches!((b.quota, b.quota_used), (Some(q), Some(u)) if u >= q)
        || b.error.to_ascii_lowercase().contains("quota")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::replay::ReplayTransport;

    const SEARCH: &str = "/search?anilistInfo= <image/jpeg>";

    fn response(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            headers: vec![],
            body: body.to_owned(),
        }
    }

    fn client(t: &Arc<ReplayTransport>, clock: &Arc<ManualClock>) -> TraceMoeClient {
        TraceMoeClient::new(
            TraceMoeConfig::default(),
            Arc::clone(t) as Arc<dyn HttpTransport>,
            Arc::clone(clock) as Arc<dyn Clock>,
        )
    }

    #[test]
    fn parses_hits_with_anilist_info() {
        let t = Arc::new(ReplayTransport::default());
        t.push(
            SEARCH,
            response(
                200,
                r#"{"frameCount":745506,"error":"","quota":100,"quotaUsed":1,"result":[
                {"anilist":{"id":99939,"idMal":34658,"title":{"native":"ネコぱらOVA","romaji":"Nekopara OVA","english":null},"synonyms":["Neko Para OVA"],"isAdult":false},
                 "filename":"Nekopara - OVA.mp4","episode":null,"episode_start":null,"episode_end":null,
                 "duration":3486.48,"from":97.72,"to":98.89,"at":98.22,"similarity":0.944,
                 "video":"https://api.trace.moe/video/x","image":"https://api.trace.moe/image/x"},
                {"anilist":21,"filename":"x.mp4","episode":[1,2],"episode_start":3,"episode_end":3,"similarity":0.5}]}"#,
            ),
        );
        let clock = Arc::new(ManualClock::default());
        let r = client(&t, &clock).search(b"jpeg").unwrap();
        assert_eq!(r.quota_used, Some(1));
        let best = r.best().unwrap();
        assert_eq!(best.anilist.id(), 99939);
        assert_eq!(best.anilist.title(), "Nekopara OVA");
        assert_eq!(best.episode_number(), None);
        assert_eq!(r.result[1].anilist.id(), 21);
        assert_eq!(r.result[1].episode_number(), Some(3));
        assert_eq!(t.requests()[0].url(), "https://api.trace.moe/search?anilistInfo=");
    }

    #[test]
    fn episode_from_aniep_when_no_range() {
        let hit: Hit = serde_json::from_str(r#"{"anilist":1,"episode":5,"similarity":0.95}"#).unwrap();
        assert_eq!(hit.episode_number(), Some(5));
        let hit: Hit =
            serde_json::from_str(r#"{"anilist":1,"episode":"OVA","similarity":0.95}"#).unwrap();
        assert_eq!(hit.episode_number(), None);
        let hit: Hit = serde_json::from_str(
            r#"{"anilist":1,"episode":5,"episode_start":7,"episode_end":8,"similarity":0.95}"#,
        )
        .unwrap();
        assert_eq!(hit.episode_number(), None);
    }

    #[test]
    fn quota_depleted_is_not_retried() {
        let t = Arc::new(ReplayTransport::default());
        t.push(
            SEARCH,
            response(
                402,
                r#"{"quota":100,"quotaUsed":100,"error":"Search quota depleted (quota per 24 hours: 100, used: 100)"}"#,
            ),
        );
        let clock = Arc::new(ManualClock::default());
        let e = client(&t, &clock).search(b"jpeg").unwrap_err();
        assert_eq!(
            e,
            TraceMoeError::QuotaDepleted {
                quota: 100,
                used: 100
            }
        );
        assert_eq!(t.requests().len(), 1);
    }

    #[test]
    fn concurrency_and_429_back_off_then_succeed() {
        let t = Arc::new(ReplayTransport::default());
        t.push(
            SEARCH,
            response(402, r#"{"error":"Concurrency limit exceeded"}"#),
        );
        t.push(
            SEARCH,
            HttpResponse {
                status: 429,
                headers: vec![("retry-after".into(), "7".into())],
                body: String::new(),
            },
        );
        t.push(SEARCH, response(200, r#"{"error":"","result":[]}"#));
        let clock = Arc::new(ManualClock::default());
        let r = client(&t, &clock).search(b"jpeg").unwrap();
        assert!(r.result.is_empty());
        assert_eq!(t.requests().len(), 3);
        // 2 s exponential backoff, then the server's 7 s.
        let sleeps = clock.sleeps();
        assert!(sleeps.contains(&Duration::from_secs(2)), "{sleeps:?}");
        assert!(sleeps.contains(&Duration::from_secs(7)), "{sleeps:?}");
    }

    #[test]
    fn stays_busy_after_retries() {
        let t = Arc::new(ReplayTransport::default());
        t.push(SEARCH, response(503, r#"{"error":"Search queue is full"}"#));
        let clock = Arc::new(ManualClock::default());
        let e = client(&t, &clock).search(b"jpeg").unwrap_err();
        assert_eq!(
            e,
            TraceMoeError::Busy {
                status: 503,
                attempts: 4
            }
        );
    }

    #[test]
    fn requests_are_paced() {
        let t = Arc::new(ReplayTransport::default());
        t.push(SEARCH, response(200, r#"{"error":"","result":[]}"#));
        let clock = Arc::new(ManualClock::default());
        let c = client(&t, &clock);
        c.search(b"a").unwrap();
        c.search(b"b").unwrap();
        assert_eq!(clock.sleeps(), vec![Duration::from_secs(1)]);
    }

    #[test]
    fn me_reports_quota_left() {
        let t = Arc::new(ReplayTransport::default());
        t.push(
            "/me",
            response(
                200,
                r#"{"id":"192.0.2.1","priority":0,"concurrency":1,"quota":100,"quotaUsed":43}"#,
            ),
        );
        let clock = Arc::new(ManualClock::default());
        let me = client(&t, &clock).me().unwrap();
        assert_eq!(me.left(), 57);
        assert_eq!(me.concurrency, 1);
    }

    #[test]
    fn bad_image_is_an_http_error() {
        let t = Arc::new(ReplayTransport::default());
        t.push(SEARCH, response(400, r#"{"error":"Failed to process image"}"#));
        let clock = Arc::new(ManualClock::default());
        let e = client(&t, &clock).search(b"jpeg").unwrap_err();
        assert_eq!(
            e,
            TraceMoeError::Http {
                status: 400,
                message: "Failed to process image".into()
            }
        );
    }
}
