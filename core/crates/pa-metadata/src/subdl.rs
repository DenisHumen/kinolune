// SPDX-License-Identifier: MPL-2.0
//! Subdl REST API client (PLAY-016): the second source of «Найти
//! субтитры», next to OpenSubtitles.
//!
//! **Rules** (<https://subdl.com/api-doc>, docs/legal/providers.md, ADR о
//! PLAY-016):
//! - every search carries the key in `api_key` (query); the key is the
//!   project's (built in, [`Keys::subdl_from_build`]) or the user's own
//!   one; without any key nothing is sent ([`SubdlError::NoKey`]). The key
//!   never reaches logs or cassettes (`api_key` is a credential parameter
//!   of [`crate::http`]);
//! - search by IMDb or TMDB id (of the movie, or of the show plus season
//!   and episode), else by title; Subdl has no file hash;
//! - a free key has a daily request limit: the client sends one request at
//!   a time, at most one every [`SubdlConfig::min_interval`], and retries
//!   `429 rate_limit`/`service_busy` and 5xx with backoff (`Retry-After`
//!   honoured); a daily limit (`daily_limit`, `quota_exceeded`…) is
//!   [`SubdlError::QuotaExhausted`], never retried;
//! - every subtitle is a ZIP on `dl.subdl.com` (no key needed): it is
//!   unpacked on the device ([`extract_subtitle`]), the file inside is
//!   kept **as it is** (ASS stays ASS); a season pack is searched for the
//!   episode by the names of its files.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde::de::Deserializer;

use crate::clock::Clock;
use crate::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use crate::key::{ApiKey, KeySource, Keys};
use crate::opensubtitles::MAX_FILE_BYTES;
use crate::rate::{RetryPolicy, parse_retry_after};
use crate::zip_archive::{self, ZipArchive, ZipEntry};

/// Provider id (offers, logs).
pub const PROVIDER: &str = "subdl";

/// Production API base.
pub const DEFAULT_BASE_URL: &str = "https://api.subdl.com/api/v1";

/// Where the archives are (`url` of a result is a path on it).
pub const DOWNLOAD_BASE_URL: &str = "https://dl.subdl.com";

/// `User-Agent` of every request.
pub const USER_AGENT: &str = concat!("Kinolune v", env!("CARGO_PKG_VERSION"));

/// Largest archive accepted (a season pack is well below it).
pub const MAX_ARCHIVE_BYTES: usize = 16 * 1024 * 1024;

/// Results asked per search (the API's maximum).
const PER_PAGE: u32 = 30;

/// Subtitle file extensions taken from an archive.
const SUBTITLE_EXTENSIONS: &[&str] = &["ass", "ssa", "srt", "vtt", "sub", "smi"];

/// Client settings.
#[derive(Debug, Clone)]
pub struct SubdlConfig {
    /// API base, without a trailing slash.
    pub base_url: String,
    /// Archive host, without a trailing slash.
    pub download_base_url: String,
    /// Least time between two requests.
    pub min_interval: Duration,
    /// Retries of 429 and 5xx.
    pub retry: RetryPolicy,
}

impl Default for SubdlConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            download_base_url: DOWNLOAD_BASE_URL.to_owned(),
            min_interval: Duration::from_secs(1),
            retry: RetryPolicy {
                max_retries: 2,
                base_delay: Duration::from_secs(2),
                max_delay: Duration::from_secs(20),
            },
        }
    }
}

/// Failures of [`SubdlClient`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SubdlError {
    /// Neither a project key in the build nor a user key: nothing is sent.
    #[error("no Subdl API key")]
    NoKey,
    /// The API refused the key.
    #[error("the Subdl API key was refused: HTTP {0}")]
    KeyRejected(u16),
    /// The daily limit of the key is used up.
    #[error("Subdl daily limit used up")]
    QuotaExhausted,
    /// Still 429 or 5xx after all retries.
    #[error("Subdl is busy: HTTP {status} after {attempts} attempts")]
    Busy {
        /// Last status.
        status: u16,
        /// Requests made.
        attempts: u32,
    },
    /// Another HTTP error, or `status: false` with a message.
    #[error("Subdl HTTP {status}: {message}")]
    Http {
        /// Status code.
        status: u16,
        /// The API's message or a short description.
        message: String,
    },
    /// No response at all.
    #[error("network error: {0}")]
    Transport(String),
    /// The response, or the archive, did not have the expected shape.
    #[error("unexpected Subdl response: {0}")]
    Decode(String),
    /// The archive has no subtitle file for the episode.
    #[error("no subtitle for this episode in the archive")]
    NotInArchive,
}

type Result<T> = std::result::Result<T, SubdlError>;

// ---------------------------------------------------------------------------
// languages

/// ISO 639-2/B (library, player) → Subdl codes, and the short code used in
/// file names (`Movie.pt-br.srt`, like OpenSubtitles' codes). The same
/// languages as the OpenSubtitles table.
const LANGUAGES: &[(&str, &str, &str)] = &[
    ("rus", "RU", "ru"),
    ("eng", "EN", "en"),
    ("ukr", "UK", "uk"),
    ("bel", "BE", "be"),
    ("kaz", "KK", "kk"),
    ("jpn", "JA", "ja"),
    ("ger", "DE", "de"),
    ("fre", "FR", "fr"),
    ("spa", "ES", "es"),
    ("ita", "IT", "it"),
    ("por", "PT", "pt-pt"),
    ("por", "BR_PT", "pt-br"),
    ("chi", "ZH", "zh-cn"),
    ("chi", "ZH_BG", "zh-tw"),
    ("kor", "KO", "ko"),
    ("pol", "PL", "pl"),
    ("cze", "CS", "cs"),
    ("dut", "NL", "nl"),
    ("swe", "SV", "sv"),
    ("nor", "NO", "no"),
    ("dan", "DA", "da"),
    ("fin", "FI", "fi"),
    ("tur", "TR", "tr"),
    ("ara", "AR", "ar"),
    ("heb", "HE", "he"),
    ("hin", "HI", "hi"),
    ("gre", "EL", "el"),
    ("hun", "HU", "hu"),
    ("rum", "RO", "ro"),
    ("bul", "BG", "bg"),
    ("srp", "SR", "sr"),
    ("hrv", "HR", "hr"),
    ("tha", "TH", "th"),
    ("vie", "VI", "vi"),
    ("ind", "ID", "id"),
];

/// Subdl codes to search for an ISO 639-2/B code (`por` → `PT`, `BR_PT`);
/// empty for a language Subdl's table here does not know.
#[must_use]
pub fn search_codes(iso639_2: &str) -> Vec<&'static str> {
    let iso = iso639_2.trim().to_ascii_lowercase();
    LANGUAGES
        .iter()
        .filter(|(i, _, _)| *i == iso)
        .map(|(_, code, _)| *code)
        .collect()
}

/// ISO 639-2/B code and short code of a Subdl language code (`BR_PT` →
/// `por`, `pt-br`), or `None` for an unknown one.
#[must_use]
pub fn language_of(code: &str) -> Option<(&'static str, &'static str)> {
    let code = code.trim().to_ascii_uppercase().replace('-', "_");
    LANGUAGES
        .iter()
        .find(|(_, c, _)| *c == code)
        .map(|(iso, _, short)| (*iso, *short))
}

// ---------------------------------------------------------------------------
// search

/// What to search for: ids when the file is recognised, else a title.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubdlQuery {
    /// IMDb id of the movie, or of the show of an episode.
    pub imdb_id: Option<String>,
    /// TMDB id of the movie, or of the show of an episode.
    pub tmdb_id: Option<u64>,
    /// Title, used only without ids.
    pub film_name: Option<String>,
    /// Year of a movie (with a title).
    pub year: Option<u32>,
    /// `movie` or `tv`.
    pub kind: Option<String>,
    /// Season of an episode.
    pub season_number: Option<u32>,
    /// Episode number in its season.
    pub episode_number: Option<u32>,
    /// Subdl language codes ([`search_codes`]); empty: all.
    pub languages: Vec<String>,
}

impl SubdlQuery {
    /// True when the query names nothing to search for.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.imdb_id.as_deref().and_then(imdb_tt).is_none()
            && self.tmdb_id.is_none()
            && self
                .film_name
                .as_deref()
                .is_none_or(|t| t.trim().is_empty())
    }

    /// The query parameters (without the key), sorted.
    #[must_use]
    pub fn params(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut put = |k: &str, v: String| out.push((k.to_owned(), v));
        let imdb = self.imdb_id.as_deref().and_then(imdb_tt);
        if let Some(id) = &imdb {
            put("imdb_id", id.clone());
        } else if let Some(id) = self.tmdb_id {
            put("tmdb_id", id.to_string());
        } else if let Some(name) = self.film_name.as_deref().map(clean_title) {
            put("film_name", name);
            if let Some(y) = self.year {
                put("year", y.to_string());
            }
        }
        if let Some(kind) = &self.kind {
            put("type", kind.clone());
        }
        if let Some(n) = self.season_number {
            put("season_number", n.to_string());
        }
        if let Some(n) = self.episode_number {
            put("episode_number", n.to_string());
        }
        if !self.languages.is_empty() {
            let mut langs: Vec<String> = self
                .languages
                .iter()
                .map(|l| l.trim().to_ascii_uppercase())
                .filter(|l| !l.is_empty())
                .collect();
            langs.sort();
            langs.dedup();
            put("languages", langs.join(","));
        }
        put("subs_per_page", PER_PAGE.to_string());
        put("comment", "1".to_owned());
        put("releases", "1".to_owned());
        put("hi", "1".to_owned());
        out.sort();
        out
    }
}

/// `tt0133093` for `tt133093`, `133093` or `tt0133093`; `None` for junk.
#[must_use]
pub fn imdb_tt(id: &str) -> Option<String> {
    let digits = id.trim().trim_start_matches("tt");
    let n = digits.parse::<u64>().ok().filter(|n| *n > 0)?;
    Some(format!("tt{n:07}"))
}

/// The API refuses quotes, brackets and slashes in a title; they become
/// spaces (a removed apostrophe would change the words).
fn clean_title(title: &str) -> String {
    title
        .chars()
        .map(|c| {
            if "<>{}[]'\"`´’;\\/".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Booleans arrive as `true`, `1` or `"1"`.
fn lenient_bool<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<bool, D::Error> {
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match v {
        Some(serde_json::Value::Bool(b)) => b,
        Some(serde_json::Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        Some(serde_json::Value::String(s)) => matches!(s.trim(), "1" | "true" | "True"),
        _ => false,
    })
}

/// Numbers arrive as numbers or strings; 0 and null mean "none".
fn lenient_opt_u32<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<u32>, D::Error> {
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match v {
        Some(serde_json::Value::Number(n)) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        Some(serde_json::Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n| *n > 0))
}

/// Counts arrive as numbers or strings.
fn lenient_u64<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<u64, D::Error> {
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match v {
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(serde_json::Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    })
}

/// Strings that may be `null` or something else.
fn lenient_string_list<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<String>, D::Error> {
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match v {
        Some(serde_json::Value::Array(items)) => items
            .into_iter()
            .filter_map(|i| match i {
                serde_json::Value::String(s) if !s.trim().is_empty() => Some(s),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    })
}

/// One subtitle of a search.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SubdlSubtitle {
    /// Release name.
    #[serde(default)]
    pub release_name: Option<String>,
    /// Archive name (`SUBDL::subtitle_123.zip`).
    #[serde(default)]
    pub name: Option<String>,
    /// Language name (`english`).
    #[serde(default)]
    pub lang: Option<String>,
    /// Language code (`EN`, `BR_PT`).
    #[serde(default)]
    pub language: Option<String>,
    /// Uploader.
    #[serde(default)]
    pub author: Option<String>,
    /// Path of the ZIP on [`DOWNLOAD_BASE_URL`].
    #[serde(default)]
    pub url: String,
    /// Page of the subtitle on subdl.com.
    #[serde(default, rename = "subtitlePage")]
    pub subtitle_page: Option<String>,
    /// Season (none for a movie).
    #[serde(default, deserialize_with = "lenient_opt_u32")]
    pub season: Option<u32>,
    /// Episode (none for a movie or a pack).
    #[serde(default, deserialize_with = "lenient_opt_u32")]
    pub episode: Option<u32>,
    /// First episode of a pack.
    #[serde(default, deserialize_with = "lenient_opt_u32")]
    pub episode_from: Option<u32>,
    /// Last episode of a pack.
    #[serde(default, deserialize_with = "lenient_opt_u32")]
    pub episode_end: Option<u32>,
    /// The archive holds the whole season.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub full_season: bool,
    /// SDH / hearing impaired.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub hi: bool,
    /// The uploader's comment.
    #[serde(default)]
    pub comment: Option<String>,
    /// Releases the subtitle fits.
    #[serde(default, deserialize_with = "lenient_string_list")]
    pub releases: Vec<String>,
    /// Downloads so far, when the API says.
    #[serde(default, alias = "downloads", deserialize_with = "lenient_u64")]
    pub download_count: u64,
    /// Translated by AI.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub ai_translated: bool,
}

impl SubdlSubtitle {
    /// The archive holds several episodes (a season or a range).
    #[must_use]
    pub fn is_pack(&self) -> bool {
        self.full_season
            || matches!((self.episode_from, self.episode_end), (Some(a), Some(b)) if a != b)
    }

    /// Whether the subtitle can hold episode `episode` of season `season`:
    /// a pack whose range includes it, the episode itself, or a result
    /// that does not say. Another season or another episode does not fit.
    #[must_use]
    pub fn fits_episode(&self, season: u32, episode: u32) -> bool {
        if self.season.is_some_and(|s| s != season) {
            return false;
        }
        if self.full_season {
            return true;
        }
        if let (Some(a), Some(b)) = (self.episode_from, self.episode_end)
            && a != b
        {
            return (a.min(b)..=a.max(b)).contains(&episode);
        }
        self.episode.is_none_or(|e| e == episode)
    }

    /// Only the foreign-language lines, by the uploader's words.
    #[must_use]
    pub fn forced(&self) -> bool {
        self.texts().any(|t| {
            let t = t.to_ascii_lowercase();
            has_word(&t, "forced") || t.contains("foreign parts")
        })
    }

    /// SDH, by the API's flag or the uploader's words (`SDH`, `HI`), unless
    /// they say it was removed.
    #[must_use]
    pub fn hearing_impaired(&self) -> bool {
        let words: Vec<String> = self.texts().map(str::to_ascii_lowercase).collect();
        if words.iter().any(|t| {
            t.contains("non-hi")
                || t.contains("non hi")
                || t.contains("non-sdh")
                || t.contains("hi removed")
                || t.contains("sdh removed")
        }) {
            return false;
        }
        self.hi
            || words
                .iter()
                .any(|t| has_word(t, "sdh") || t.contains("hearing impaired"))
    }

    fn texts(&self) -> impl Iterator<Item = &str> {
        self.comment
            .as_deref()
            .into_iter()
            .chain(self.release_name.as_deref())
            .chain(self.releases.iter().map(String::as_str))
    }
}

/// `word` stands alone in `text` (not inside a longer word).
fn has_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| w == word)
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default = "yes", deserialize_with = "lenient_bool")]
    status: bool,
    #[serde(default)]
    error: Option<serde_json::Value>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    subtitles: Vec<SubdlSubtitle>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    error: Option<serde_json::Value>,
    #[serde(default)]
    message: Option<String>,
}

impl ErrorBody {
    fn code(&self) -> String {
        match &self.error {
            Some(serde_json::Value::String(s)) => s.to_ascii_lowercase(),
            Some(other) => other.to_string().to_ascii_lowercase(),
            None => String::new(),
        }
    }

    fn text(&self) -> String {
        self.message
            .clone()
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| self.code())
    }
}

/// A daily or monthly limit, not a short-term rate limit.
fn is_quota(code: &str) -> bool {
    matches!(
        code,
        "daily_limit" | "api_download_limit_exceeded" | "quota_exceeded" | "limit_exceeded"
    )
}

/// The API's verdict on the key.
fn is_key_error(code: &str, message: &str) -> bool {
    let all = format!("{code} {}", message.to_ascii_lowercase());
    all.contains("not_authorized")
        || all.contains("not authorized")
        || all.contains("api key")
        || all.contains("api_key")
        || all.contains("invalid key")
        || all.contains("unauthorized")
}

// ---------------------------------------------------------------------------
// client

/// See the module docs. `Send + Sync`; methods block, and requests from
/// several threads go out one after another.
pub struct SubdlClient {
    config: SubdlConfig,
    keys: Keys,
    transport: Arc<dyn HttpTransport>,
    clock: Arc<dyn Clock>,
    gate: Mutex<Option<Duration>>,
}

impl fmt::Debug for SubdlClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubdlClient")
            .field("base_url", &self.config.base_url)
            .field("keys", &self.keys)
            .finish_non_exhaustive()
    }
}

impl SubdlClient {
    /// A client over `transport`.
    pub fn new(
        config: SubdlConfig,
        keys: Keys,
        transport: Arc<dyn HttpTransport>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            config,
            keys,
            transport,
            clock,
            gate: Mutex::new(None),
        }
    }

    /// The live client (HTTPS, [`USER_AGENT`], 30 s timeout) with the
    /// project key of the build and the user's own key (if any).
    #[cfg(feature = "http")]
    pub fn live(user_key: Option<&str>) -> Self {
        Self::new(
            SubdlConfig::default(),
            Keys::subdl_from_build().with_user_key(user_key),
            Arc::new(crate::http::UreqTransport::with_timeout(
                USER_AGENT,
                Duration::from_secs(30),
            )),
            Arc::new(crate::clock::SystemClock::default()),
        )
    }

    /// Where the key in use comes from; `None`: no key, nothing is sent.
    pub fn key_source(&self) -> Option<KeySource> {
        self.keys.active().map(|(_, s)| s)
    }

    fn key(&self) -> Result<&ApiKey> {
        self.keys.active().map(|(k, _)| k).ok_or(SubdlError::NoKey)
    }

    /// Subtitles for `query`, as the API orders them.
    ///
    /// # Errors
    ///
    /// [`SubdlError::NoKey`] without a key (nothing sent), the key refused,
    /// the daily limit, HTTP, network and decoding errors.
    pub fn search(&self, query: &SubdlQuery) -> Result<Vec<SubdlSubtitle>> {
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut params = query.params();
        params.push(("api_key".to_owned(), self.key()?.expose().to_owned()));
        let request = HttpRequest {
            base: self.config.base_url.trim_end_matches('/').to_owned(),
            path: "/subtitles".to_owned(),
            query: params,
            headers: vec![
                ("User-Agent".to_owned(), USER_AGENT.to_owned()),
                ("Accept".to_owned(), "application/json".to_owned()),
            ],
        };
        let response = self.send(|t| t.get(&request))?;
        let page: SearchResponse =
            serde_json::from_str(&response.body).map_err(|e| SubdlError::Decode(e.to_string()))?;
        if !page.status {
            let body = ErrorBody {
                error: page.error,
                message: page.message,
            };
            let (code, text) = (body.code(), body.text());
            // "Can't find movie or TV" is a normal empty answer.
            if code.contains("can't find") || code.contains("not found") || code.is_empty() {
                return Ok(Vec::new());
            }
            if is_key_error(&code, &text) {
                return Err(SubdlError::KeyRejected(response.status));
            }
            if is_quota(&code) {
                return Err(SubdlError::QuotaExhausted);
            }
            return Err(SubdlError::Http {
                status: response.status,
                message: text,
            });
        }
        Ok(page
            .subtitles
            .into_iter()
            .filter(|s| !s.url.trim().is_empty())
            .collect())
    }

    /// The archive (or the file) behind a result's `url`, byte for byte.
    /// No key is sent: archives are public.
    ///
    /// # Errors
    ///
    /// A link outside Subdl, the daily limit, HTTP, network and size
    /// errors.
    pub fn fetch(&self, url: &str) -> Result<Vec<u8>> {
        let (base, path) = download_target(&self.config.download_base_url, url)
            .ok_or_else(|| SubdlError::Decode(format!("bad link: {url}")))?;
        let request = HttpRequest {
            base,
            path,
            query: Vec::new(),
            headers: vec![("User-Agent".to_owned(), USER_AGENT.to_owned())],
        };
        let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut attempts = 0u32;
        loop {
            self.wait_turn(&mut gate);
            attempts += 1;
            let response = self
                .transport
                .get_bytes(&request)
                .map_err(|e| SubdlError::Transport(e.0))?;
            let status = response.status;
            if (200..300).contains(&status) {
                if response.body.len() > MAX_ARCHIVE_BYTES {
                    return Err(SubdlError::Http {
                        status,
                        message: "archive too large for subtitles".to_owned(),
                    });
                }
                return Ok(response.body);
            }
            let body: ErrorBody = serde_json::from_slice(&response.body).unwrap_or_default();
            if status == 429 && is_quota(&body.code()) {
                return Err(SubdlError::QuotaExhausted);
            }
            if matches!(status, 429 | 500 | 502 | 503 | 504) {
                if attempts > self.config.retry.max_retries {
                    return Err(SubdlError::Busy { status, attempts });
                }
                let wait = parse_retry_after(
                    response
                        .headers
                        .iter()
                        .find(|(k, _)| k == "retry-after")
                        .map(|(_, v)| v.as_str()),
                );
                self.clock
                    .sleep(self.config.retry.delay(attempts - 1, wait));
                continue;
            }
            return Err(SubdlError::Http {
                status,
                message: "subtitle archive not available".to_owned(),
            });
        }
    }

    /// Waits until [`SubdlConfig::min_interval`] has passed since the last
    /// request and marks this one.
    fn wait_turn(&self, gate: &mut Option<Duration>) {
        if let Some(last) = *gate {
            let next = last + self.config.min_interval;
            let now = self.clock.monotonic();
            if next > now {
                self.clock.sleep(next - now);
            }
        }
        *gate = Some(self.clock.monotonic());
    }

    /// One request with pacing, one at a time, and retries.
    fn send(
        &self,
        call: impl Fn(&dyn HttpTransport) -> std::result::Result<HttpResponse, TransportError>,
    ) -> Result<HttpResponse> {
        let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut attempts = 0u32;
        loop {
            self.wait_turn(&mut gate);
            attempts += 1;
            let response = call(self.transport.as_ref()).map_err(|e| SubdlError::Transport(e.0))?;
            let status = response.status;
            if (200..300).contains(&status) {
                return Ok(response);
            }
            let body: Option<ErrorBody> = serde_json::from_str(&response.body).ok();
            let code = body.as_ref().map(ErrorBody::code).unwrap_or_default();
            let text = body
                .as_ref()
                .map(ErrorBody::text)
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| format!("HTTP {status}"));
            if matches!(status, 401 | 403) && body.is_some() && is_key_error(&code, &text) {
                return Err(SubdlError::KeyRejected(status));
            }
            if is_quota(&code) {
                tracing::warn!(target: "pa_metadata::subdl", "Subdl daily limit used up");
                return Err(SubdlError::QuotaExhausted);
            }
            if !matches!(status, 429 | 500 | 502 | 503 | 504) {
                return Err(SubdlError::Http {
                    status,
                    message: text,
                });
            }
            if attempts > self.config.retry.max_retries {
                return Err(SubdlError::Busy { status, attempts });
            }
            let delay = self.config.retry.delay(
                attempts - 1,
                parse_retry_after(response.header("retry-after")),
            );
            tracing::info!(
                target: "pa_metadata::subdl",
                status,
                delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                "Subdl: busy, backing off"
            );
            self.clock.sleep(delay);
        }
    }
}

/// `(base, path)` of a result's `url`: a path on `download_base`, or an
/// absolute `https` link on a `subdl.com` host. Anything else is refused.
fn download_target(download_base: &str, url: &str) -> Option<(String, String)> {
    let url = url.trim();
    if url.starts_with('/') {
        return Some((
            download_base.trim_end_matches('/').to_owned(),
            url.to_owned(),
        ));
    }
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_at(rest.find('/')?);
    (host == "subdl.com" || host.ends_with(".subdl.com"))
        .then(|| (format!("https://{host}"), path.to_owned()))
}

// ---------------------------------------------------------------------------
// archives

/// Which file of an archive is wanted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Wanted {
    /// Season of the episode the video is (none for a movie).
    pub season: Option<u32>,
    /// Episode number in its season (none for a movie).
    pub episode: Option<u32>,
    /// The offer was SDH: prefer an SDH file of a mixed archive.
    pub hearing_impaired: bool,
}

/// A subtitle file taken out of a download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// Its name (in the archive, or the link's last part).
    pub name: String,
    /// Its bytes, as they were packed.
    pub bytes: Vec<u8>,
}

fn extension(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .filter(|e| SUBTITLE_EXTENSIONS.contains(&e.as_str()))
}

/// The subtitle file of a download for `wanted`: the only subtitle in a
/// ZIP, or, in an archive with several, the one whose name is the episode
/// (by the name parser of `pa-recognition`), the SDH one when `wanted`
/// says so, ASS before SRT (it keeps its styles). A download that is not
/// an archive is the file itself.
///
/// # Errors
///
/// [`SubdlError::NotInArchive`] when no file fits the episode, and
/// [`SubdlError::Decode`] for a RAR, a broken archive or one without
/// subtitles.
pub fn extract_subtitle(bytes: &[u8], link: &str, wanted: Wanted) -> Result<Extracted> {
    if zip_archive::is_rar(bytes) {
        return Err(SubdlError::Decode(
            "RAR archives are not supported".to_owned(),
        ));
    }
    if !zip_archive::is_zip(bytes) {
        let name = link
            .rsplit('/')
            .next()
            .filter(|n| extension(n).is_some())
            .unwrap_or("subtitle.srt")
            .to_owned();
        if bytes.len() > MAX_FILE_BYTES || bytes.is_empty() {
            return Err(SubdlError::Decode("not a subtitle file".to_owned()));
        }
        return Ok(Extracted {
            name,
            bytes: bytes.to_vec(),
        });
    }
    let zip = ZipArchive::open(bytes).map_err(|e| SubdlError::Decode(e.to_string()))?;
    let entry = pick_entry(zip.entries(), wanted)?;
    let data = zip
        .read(entry, MAX_FILE_BYTES)
        .map_err(|e| SubdlError::Decode(e.to_string()))?;
    Ok(Extracted {
        name: entry.base_name().to_owned(),
        bytes: data,
    })
}

/// See [`extract_subtitle`].
fn pick_entry(entries: &[ZipEntry], wanted: Wanted) -> Result<&ZipEntry> {
    let has_idx = |e: &ZipEntry| {
        let stem = e.name.rsplit_once('.').map_or(e.name.as_str(), |(s, _)| s);
        entries
            .iter()
            .any(|o| o.name.eq_ignore_ascii_case(&format!("{stem}.idx")))
    };
    let candidates: Vec<&ZipEntry> = entries
        .iter()
        .filter(|e| !e.is_dir() && e.size > 0)
        .filter(|e| !e.name.starts_with("__MACOSX/") && !e.base_name().starts_with("._"))
        .filter(|e| match extension(e.base_name()).as_deref() {
            // A VobSub `.sub` beside its `.idx` is pictures, not text.
            Some("sub") => !has_idx(e),
            Some(_) => true,
            None => false,
        })
        .collect();
    if candidates.is_empty() {
        return Err(SubdlError::Decode(
            "no subtitle file in the archive".to_owned(),
        ));
    }
    let preference = |e: &ZipEntry| {
        let lower = e.base_name().to_ascii_lowercase();
        let sdh = has_word(&lower, "sdh") || has_word(&lower, "hi") || lower.contains("_hi_");
        let ext = extension(&lower).unwrap_or_default();
        (
            sdh != wanted.hearing_impaired,
            !matches!(ext.as_str(), "ass" | "ssa"),
            lower,
        )
    };
    let Some(episode) = wanted.episode else {
        return candidates
            .into_iter()
            .min_by_key(|e| preference(e))
            .ok_or(SubdlError::NotInArchive);
    };
    // Each candidate's episode numbers, as its name says.
    let mut named: Vec<(&ZipEntry, bool)> = Vec::new();
    let mut any_numbered = false;
    for e in &candidates {
        let parsed = pa_recognition::parse::parse(e.base_name());
        let best = &parsed.best().parsed;
        let numbers: Vec<u32> = best
            .episodes
            .iter()
            .chain(best.absolute.iter())
            .filter(|n| n.fraction.is_none())
            .map(|n| n.number)
            .collect();
        if numbers.is_empty() {
            continue;
        }
        any_numbered = true;
        let season_ok = match (best.season, wanted.season) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        named.push((e, season_ok && numbers.contains(&episode)));
    }
    if let Some(found) = named
        .iter()
        .filter(|(_, fits)| *fits)
        .map(|(e, _)| *e)
        .min_by_key(|e| preference(e))
    {
        return Ok(found);
    }
    // One file whose name says nothing: a single-episode archive.
    if !any_numbered && candidates.len() == 1 {
        return Ok(candidates[0]);
    }
    Err(SubdlError::NotInArchive)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_map_both_ways() {
        assert_eq!(search_codes("rus"), vec!["RU"]);
        assert_eq!(search_codes("por"), vec!["PT", "BR_PT"]);
        assert_eq!(search_codes("xxx"), Vec::<&str>::new());
        assert_eq!(language_of("BR_PT"), Some(("por", "pt-br")));
        assert_eq!(language_of("en"), Some(("eng", "en")));
        assert_eq!(language_of("ZZ"), None);
    }

    #[test]
    fn params_prefer_ids_and_clean_titles() {
        let q = SubdlQuery {
            imdb_id: Some("944947".into()),
            tmdb_id: Some(1399),
            kind: Some("tv".into()),
            season_number: Some(1),
            episode_number: Some(2),
            languages: vec!["ru".into(), "EN".into()],
            ..SubdlQuery::default()
        };
        let p: Vec<String> = q.params().iter().map(|(k, v)| format!("{k}={v}")).collect();
        assert_eq!(
            p,
            [
                "comment=1",
                "episode_number=2",
                "hi=1",
                "imdb_id=tt0944947",
                "languages=EN,RU",
                "releases=1",
                "season_number=1",
                "subs_per_page=30",
                "type=tv",
            ]
        );
        let q = SubdlQuery {
            film_name: Some("Grey's [Anatomy]".into()),
            year: Some(2005),
            ..SubdlQuery::default()
        };
        assert!(
            q.params()
                .contains(&("film_name".into(), "Grey s Anatomy".into()))
        );
        assert!(SubdlQuery::default().is_empty());
    }

    #[test]
    fn episodes_fit_packs_and_ranges() {
        let s: SubdlSubtitle = serde_json::from_value(serde_json::json!({
            "url": "/subtitle/1.zip", "season": 1, "episode": 0,
            "episode_from": 1, "episode_end": 10
        }))
        .unwrap();
        assert!(s.is_pack());
        assert!(s.fits_episode(1, 5));
        assert!(!s.fits_episode(1, 11));
        assert!(!s.fits_episode(2, 5));
        let one: SubdlSubtitle = serde_json::from_value(serde_json::json!({
            "url": "/subtitle/2.zip", "season": "1", "episode": "3", "hi": 1,
            "comment": "SDH, forced signs"
        }))
        .unwrap();
        assert!(!one.is_pack());
        assert!(one.fits_episode(1, 3) && !one.fits_episode(1, 4));
        assert!(one.hearing_impaired() && one.forced());
    }

    #[test]
    fn links_stay_on_subdl() {
        assert_eq!(
            download_target(DOWNLOAD_BASE_URL, "/subtitle/1-2.zip"),
            Some((DOWNLOAD_BASE_URL.into(), "/subtitle/1-2.zip".into()))
        );
        assert_eq!(
            download_target(DOWNLOAD_BASE_URL, "https://dl.subdl.com/subtitle/x.zip"),
            Some(("https://dl.subdl.com".into(), "/subtitle/x.zip".into()))
        );
        assert_eq!(
            download_target(DOWNLOAD_BASE_URL, "https://evil.example/x.zip"),
            None
        );
        assert_eq!(
            download_target(DOWNLOAD_BASE_URL, "http://dl.subdl.com/x"),
            None
        );
    }
}
