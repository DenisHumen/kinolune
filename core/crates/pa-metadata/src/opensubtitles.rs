// SPDX-License-Identifier: MPL-2.0
//! OpenSubtitles.com REST API v1 client (PLAY-008): subtitles for a video
//! by its moviehash (the OSHash of LIB-004) and by its IMDb/TMDB id, the
//! user's download quota and the download itself.
//!
//! **Rules** (<https://opensubtitles.stoplight.io/docs/opensubtitles-api>,
//! docs/legal/providers.md):
//! - every request carries the consumer key in `Api-Key` and a
//!   `User-Agent` of the form `AppName v1.2.3` ([`USER_AGENT`]);
//! - the key is the project's (built in, [`Keys::opensubtitles_from_build`])
//!   or the user's own one; without any key nothing is sent
//!   ([`OpenSubtitlesError::NoKey`]);
//! - searching is free; **downloads count against a quota**: per IP
//!   address without an account, per account after `POST /login` (the
//!   user's own login and password; the JWT it returns lives in memory
//!   only). `POST /download` answers how many are left and when the
//!   window resets; HTTP 406 means the quota is used up
//!   ([`OpenSubtitlesError::QuotaExhausted`], never retried);
//! - the API asks for query parameters in lower case and sorted by name
//!   (otherwise it redirects), IMDb ids without `tt` and leading zeros;
//! - about five requests a second per IP: the client sends one request at
//!   a time, at most one every [`OpenSubtitlesConfig::min_interval`], and
//!   retries 429 and 5xx with backoff (`Retry-After` honoured);
//! - the subtitle file is fetched from the temporary link without key or
//!   token, **as it is**: no `sub_format` is asked for, so an ASS file
//!   stays ASS with its styles.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde::de::Deserializer;

use crate::clock::Clock;
use crate::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use crate::key::{ApiKey, KeySource, Keys};
use crate::rate::{RetryPolicy, parse_retry_after};

/// Provider id (logs, explanations).
pub const PROVIDER: &str = "opensubtitles";

/// Production API base.
pub const DEFAULT_BASE_URL: &str = "https://api.opensubtitles.com/api/v1";

/// `User-Agent` in the form the API asks for: `AppName v1.2.3`.
pub const USER_AGENT: &str = concat!("Kinolune v", env!("CARGO_PKG_VERSION"));

/// A JWT from `/login` is valid for 24 hours; it is renewed a bit earlier.
const SESSION_TTL: Duration = Duration::from_secs(23 * 60 * 60);

/// Largest subtitle file accepted (subtitles are a few hundred KiB; a
/// bigger body is not a subtitle).
pub const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;

/// Client settings.
#[derive(Debug, Clone)]
pub struct OpenSubtitlesConfig {
    /// API base, without a trailing slash.
    pub base_url: String,
    /// Least time between two requests.
    pub min_interval: Duration,
    /// Retries of 429 and 5xx.
    pub retry: RetryPolicy,
}

impl Default for OpenSubtitlesConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            min_interval: Duration::from_millis(250),
            retry: RetryPolicy {
                max_retries: 3,
                base_delay: Duration::from_secs(1),
                max_delay: Duration::from_secs(20),
            },
        }
    }
}

/// The user's OpenSubtitles account. `Debug` hides the password.
#[derive(Clone, PartialEq, Eq)]
pub struct Account {
    /// Login name.
    pub username: String,
    /// Password; only sent to `POST /login`.
    pub password: String,
}

impl Account {
    /// An account, or `None` when either part is blank.
    pub fn new(username: &str, password: &str) -> Option<Self> {
        let username = username.trim();
        (!username.is_empty() && !password.is_empty()).then(|| Self {
            username: username.to_owned(),
            password: password.to_owned(),
        })
    }
}

impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Account")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Failures of [`OpenSubtitlesClient`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenSubtitlesError {
    /// Neither a project key in the build nor a user key: nothing is sent.
    #[error("no OpenSubtitles API key")]
    NoKey,
    /// The API refused the key (401/403 without an account involved).
    #[error("the OpenSubtitles API key was refused: HTTP {0}")]
    KeyRejected(u16),
    /// `POST /login` refused the username or password.
    #[error("OpenSubtitles login failed: {0}")]
    Login(String),
    /// The download quota is used up (HTTP 406, or `remaining` below 0).
    #[error("OpenSubtitles download quota used up")]
    QuotaExhausted {
        /// When the quota is renewed (ISO 8601, UTC), if the API said.
        reset_time_utc: Option<String>,
    },
    /// Still 429 or 5xx after all retries.
    #[error("OpenSubtitles is busy: HTTP {status} after {attempts} attempts")]
    Busy {
        /// Last status.
        status: u16,
        /// Requests made.
        attempts: u32,
    },
    /// Another HTTP error.
    #[error("OpenSubtitles HTTP {status}: {message}")]
    Http {
        /// Status code.
        status: u16,
        /// The API's message or a short description.
        message: String,
    },
    /// No response at all.
    #[error("network error: {0}")]
    Transport(String),
    /// The response did not have the expected shape.
    #[error("unexpected OpenSubtitles response: {0}")]
    Decode(String),
}

type Result<T> = std::result::Result<T, OpenSubtitlesError>;

// ---------------------------------------------------------------------------
// languages

/// ISO 639-2/B (what the library and the player use) ↔ the codes of
/// OpenSubtitles (ISO 639-1, plus regional `pt-br`, `pt-pt`, `zh-cn`,
/// `zh-tw`). The same languages as the player's table (`languages.dart`).
const LANGUAGES: &[(&str, &[&str])] = &[
    ("rus", &["ru"]),
    ("eng", &["en"]),
    ("ukr", &["uk"]),
    ("bel", &["be"]),
    ("kaz", &["kk"]),
    ("jpn", &["ja"]),
    ("ger", &["de"]),
    ("fre", &["fr"]),
    ("spa", &["es", "ea"]),
    ("ita", &["it"]),
    ("por", &["pt-pt", "pt-br", "pt"]),
    ("chi", &["zh-cn", "zh-tw", "zh"]),
    ("kor", &["ko"]),
    ("pol", &["pl"]),
    ("cze", &["cs"]),
    ("dut", &["nl"]),
    ("swe", &["sv"]),
    ("nor", &["no", "nb"]),
    ("dan", &["da"]),
    ("fin", &["fi"]),
    ("tur", &["tr"]),
    ("ara", &["ar"]),
    ("heb", &["he"]),
    ("hin", &["hi"]),
    ("gre", &["el"]),
    ("hun", &["hu"]),
    ("rum", &["ro"]),
    ("bul", &["bg"]),
    ("srp", &["sr"]),
    ("hrv", &["hr"]),
    ("tha", &["th"]),
    ("vie", &["vi"]),
    ("ind", &["id"]),
];

/// OpenSubtitles codes to search for a language (ISO 639-1/2 or BCP 47:
/// `rus`, `ru`, `pt-BR`); empty for an unknown one. Portuguese and Chinese
/// ask for both regional variants.
#[must_use]
pub fn search_codes(language: &str) -> Vec<&'static str> {
    let tag = language.trim().to_ascii_lowercase().replace('_', "-");
    let base = tag.split('-').next().unwrap_or_default();
    for (iso, codes) in LANGUAGES {
        let two = codes.last().copied().unwrap_or_default();
        if base == *iso || base == two || codes.contains(&tag.as_str()) || codes.contains(&base) {
            return match *iso {
                "por" => vec!["pt-br", "pt-pt"],
                "chi" => vec!["zh-cn", "zh-tw"],
                "spa" => vec!["es"],
                "nor" => vec!["no"],
                _ => vec![codes[0]],
            };
        }
    }
    Vec::new()
}

/// ISO 639-2/B code of an OpenSubtitles language code (`pt-br` → `por`),
/// or `None` for an unknown one.
#[must_use]
pub fn iso639_2(code: &str) -> Option<&'static str> {
    let code = code.trim().to_ascii_lowercase();
    LANGUAGES
        .iter()
        .find(|(_, codes)| codes.contains(&code.as_str()))
        .map(|(iso, _)| *iso)
}

// ---------------------------------------------------------------------------
// search

/// What to search for. At least one of [`moviehash`](Self::moviehash) and
/// the ids must be set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    /// OSHash of the file, 16 hex digits.
    pub moviehash: Option<String>,
    /// IMDb id of a movie or an episode (`tt0133093` or `133093`).
    pub imdb_id: Option<String>,
    /// TMDB id of a movie or an episode.
    pub tmdb_id: Option<u64>,
    /// IMDb id of the show of an episode.
    pub parent_imdb_id: Option<String>,
    /// TMDB id of the show of an episode.
    pub parent_tmdb_id: Option<u64>,
    /// Season of an episode.
    pub season_number: Option<u32>,
    /// Episode number in its season.
    pub episode_number: Option<u32>,
    /// OpenSubtitles language codes ([`search_codes`]); empty: all.
    pub languages: Vec<String>,
    /// `movie` or `episode`, when known.
    pub kind: Option<String>,
}

impl SearchQuery {
    /// True when the query names something to search for.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.moviehash.is_none()
            && self.imdb_id.is_none()
            && self.tmdb_id.is_none()
            && self.parent_imdb_id.is_none()
            && self.parent_tmdb_id.is_none()
    }

    /// The query parameters as the API wants them: lower case, sorted.
    #[must_use]
    pub fn params(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut put = |k: &str, v: String| out.push((k.to_owned(), v.to_ascii_lowercase()));
        if let Some(id) = self.imdb_id.as_deref().and_then(imdb_number) {
            put("imdb_id", id.to_string());
        }
        if let Some(id) = self.tmdb_id {
            put("tmdb_id", id.to_string());
        }
        if let Some(id) = self.parent_imdb_id.as_deref().and_then(imdb_number) {
            put("parent_imdb_id", id.to_string());
        }
        if let Some(id) = self.parent_tmdb_id {
            put("parent_tmdb_id", id.to_string());
        }
        if let Some(n) = self.season_number {
            put("season_number", n.to_string());
        }
        if let Some(n) = self.episode_number {
            put("episode_number", n.to_string());
        }
        if let Some(h) = &self.moviehash {
            put("moviehash", h.clone());
        }
        if !self.languages.is_empty() {
            let mut langs: Vec<String> = self
                .languages
                .iter()
                .map(|l| l.trim().to_ascii_lowercase())
                .filter(|l| !l.is_empty())
                .collect();
            langs.sort();
            langs.dedup();
            put("languages", langs.join(","));
        }
        if let Some(kind) = &self.kind {
            put("type", kind.clone());
        }
        out.sort();
        out
    }
}

/// The number of an IMDb id (`tt0133093` → 133093).
#[must_use]
pub fn imdb_number(id: &str) -> Option<u64> {
    let digits = id.trim().trim_start_matches("tt");
    digits.parse::<u64>().ok().filter(|n| *n > 0)
}

/// Booleans arrive as `true`, `1` or `"1"`.
fn lenient_bool<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<bool, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        serde_json::Value::String(s) => matches!(s.trim(), "1" | "true" | "True"),
        _ => false,
    })
}

/// Numbers sometimes arrive as strings.
fn lenient_u64<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<u64, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(0),
        serde_json::Value::String(s) => s.trim().parse().unwrap_or(0),
        _ => 0,
    })
}

/// One file of a subtitle (several for a release split over CDs).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SubtitleFile {
    /// What `POST /download` takes.
    #[serde(deserialize_with = "lenient_u64")]
    pub file_id: u64,
    /// CD number of a split release.
    #[serde(default)]
    pub cd_number: Option<u32>,
    /// The uploader's file name (often without an extension).
    #[serde(default)]
    pub file_name: Option<String>,
}

/// The uploader.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct Uploader {
    /// Display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Rank (`trusted`, `administrator`…).
    #[serde(default)]
    pub rank: Option<String>,
}

/// What the subtitle is for.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct FeatureDetails {
    /// Title of the movie or episode.
    #[serde(default)]
    pub title: Option<String>,
    /// Year.
    #[serde(default)]
    pub year: Option<u32>,
    /// `Movie`, `Episode`, `Tvshow`.
    #[serde(default)]
    pub feature_type: Option<String>,
    /// Season of an episode.
    #[serde(default)]
    pub season_number: Option<u32>,
    /// Episode number.
    #[serde(default)]
    pub episode_number: Option<u32>,
}

/// One subtitle of a search.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Subtitle {
    /// OpenSubtitles subtitle id.
    #[serde(default)]
    pub subtitle_id: String,
    /// OpenSubtitles language code (`en`, `pt-br`).
    #[serde(default)]
    pub language: Option<String>,
    /// Downloads so far.
    #[serde(default, deserialize_with = "lenient_u64")]
    pub download_count: u64,
    /// SDH / hearing impaired.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub hearing_impaired: bool,
    /// Only the foreign-language lines (forced).
    #[serde(default, deserialize_with = "lenient_bool")]
    pub foreign_parts_only: bool,
    /// Translated by AI.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub ai_translated: bool,
    /// Machine translated.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub machine_translated: bool,
    /// Uploaded by a trusted uploader.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub from_trusted: bool,
    /// Made for a file with this very moviehash: in sync with it.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub moviehash_match: bool,
    /// Frame rate the subtitle was timed for.
    #[serde(default)]
    pub fps: Option<f64>,
    /// Release name.
    #[serde(default)]
    pub release: Option<String>,
    /// Uploader.
    #[serde(default)]
    pub uploader: Option<Uploader>,
    /// What it is for.
    #[serde(default)]
    pub feature_details: Option<FeatureDetails>,
    /// Its files.
    #[serde(default)]
    pub files: Vec<SubtitleFile>,
}

impl Subtitle {
    /// The file to download when the subtitle is one file; `None` for a
    /// release split over several CDs (they do not fit one video file).
    #[must_use]
    pub fn single_file(&self) -> Option<&SubtitleFile> {
        match self.files.as_slice() {
            [one] => Some(one),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Datum {
    #[serde(default)]
    id: Option<serde_json::Value>,
    attributes: Subtitle,
}

#[derive(Debug, Deserialize)]
struct SearchPage {
    #[serde(default)]
    data: Vec<Datum>,
}

// ---------------------------------------------------------------------------
// account and download

/// The account's quota (`GET /infos/user`, `POST /login`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct UserInfo {
    /// Downloads allowed in the window.
    #[serde(default)]
    pub allowed_downloads: Option<u32>,
    /// Downloads left now (may go below 0 after the last one).
    #[serde(default)]
    pub remaining_downloads: Option<i64>,
    /// Downloads made in the window.
    #[serde(default)]
    pub downloads_count: Option<u32>,
    /// Rank name (`Sub leecher`…).
    #[serde(default)]
    pub level: Option<String>,
    /// VIP account.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub vip: bool,
}

/// The download quota as last seen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Quota {
    /// Downloads left (`None`: unknown until the first download without an
    /// account).
    pub remaining: Option<i64>,
    /// Downloads allowed in the window, when known.
    pub allowed: Option<u32>,
    /// When the window resets (ISO 8601, UTC), when known.
    pub reset_time_utc: Option<String>,
    /// The quota is the account's (logged in), not the IP address's.
    pub logged_in: bool,
}

/// `POST /download`: the temporary link and the quota after it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DownloadLink {
    /// Temporary URL of the file (no key or token needed).
    #[serde(default)]
    pub link: String,
    /// File name with its real extension (`Movie.ass`).
    #[serde(default)]
    pub file_name: Option<String>,
    /// Downloads made in the window.
    #[serde(default)]
    pub requests: Option<i64>,
    /// Downloads left.
    #[serde(default)]
    pub remaining: Option<i64>,
    /// Human-readable note about the quota.
    #[serde(default)]
    pub message: Option<String>,
    /// When the window resets (ISO 8601, UTC).
    #[serde(default)]
    pub reset_time_utc: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    user: Option<UserInfo>,
}

#[derive(Debug, Deserialize)]
struct UserResponse {
    data: UserInfo,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    errors: Vec<String>,
    #[serde(default)]
    reset_time_utc: Option<String>,
}

struct Session {
    token: String,
    /// API base for this account (`login` may answer a VIP host).
    base: String,
    /// Monotonic time the token was issued.
    issued: Duration,
}

/// See the module docs. `Send + Sync`; methods block, and requests from
/// several threads go out one after another.
pub struct OpenSubtitlesClient {
    config: OpenSubtitlesConfig,
    keys: Keys,
    account: Option<Account>,
    transport: Arc<dyn HttpTransport>,
    clock: Arc<dyn Clock>,
    gate: Mutex<Option<Duration>>,
    session: Mutex<Option<Session>>,
    quota: Mutex<Option<Quota>>,
}

impl fmt::Debug for OpenSubtitlesClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenSubtitlesClient")
            .field("base_url", &self.config.base_url)
            .field("keys", &self.keys)
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
}

impl OpenSubtitlesClient {
    /// A client over `transport`.
    pub fn new(
        config: OpenSubtitlesConfig,
        keys: Keys,
        account: Option<Account>,
        transport: Arc<dyn HttpTransport>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            config,
            keys,
            account,
            transport,
            clock,
            gate: Mutex::new(None),
            session: Mutex::new(None),
            quota: Mutex::new(None),
        }
    }

    /// The live client (HTTPS, [`USER_AGENT`], 30 s timeout) with the
    /// project key of the build, the user's own key (if any) and account.
    #[cfg(feature = "http")]
    pub fn live(user_key: Option<&str>, account: Option<Account>) -> Self {
        Self::new(
            OpenSubtitlesConfig::default(),
            Keys::opensubtitles_from_build().with_user_key(user_key),
            account,
            Arc::new(crate::http::UreqTransport::with_timeout(
                USER_AGENT,
                Duration::from_secs(30),
            )),
            Arc::new(crate::clock::SystemClock::default()),
        )
    }

    /// Where the key in use comes from; `None`: no key, nothing works.
    pub fn key_source(&self) -> Option<KeySource> {
        self.keys.active().map(|(_, s)| s)
    }

    /// The account in use, if any.
    pub fn account(&self) -> Option<&Account> {
        self.account.as_ref()
    }

    /// The quota as last seen (after [`user_info`](Self::user_info) or a
    /// [`download`](Self::download)).
    pub fn quota(&self) -> Option<Quota> {
        self.quota.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn key(&self) -> Result<&ApiKey> {
        self.keys
            .active()
            .map(|(k, _)| k)
            .ok_or(OpenSubtitlesError::NoKey)
    }

    fn request(
        &self,
        base: &str,
        path: &str,
        query: Vec<(String, String)>,
        token: Option<&str>,
    ) -> Result<HttpRequest> {
        let mut headers = vec![
            ("Api-Key".to_owned(), self.key()?.expose().to_owned()),
            ("User-Agent".to_owned(), USER_AGENT.to_owned()),
            ("Accept".to_owned(), "application/json".to_owned()),
        ];
        if let Some(token) = token {
            headers.push(("Authorization".to_owned(), format!("Bearer {token}")));
        }
        Ok(HttpRequest {
            base: base.trim_end_matches('/').to_owned(),
            path: path.to_owned(),
            query,
            headers,
        })
    }

    /// Subtitles for `query`, as the API orders them. Costs no quota.
    ///
    /// # Errors
    ///
    /// [`OpenSubtitlesError::NoKey`] without a key (nothing sent), HTTP,
    /// network and decoding errors.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<Subtitle>> {
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let request = self.request(&self.config.base_url, "/subtitles", query.params(), None)?;
        let response = self.send(|t| t.get(&request))?;
        let page: SearchPage = serde_json::from_str(&response.body)
            .map_err(|e| OpenSubtitlesError::Decode(e.to_string()))?;
        Ok(page
            .data
            .into_iter()
            .map(|d| {
                let mut s = d.attributes;
                if s.subtitle_id.is_empty()
                    && let Some(id) = d.id
                {
                    s.subtitle_id = id.as_str().map_or_else(|| id.to_string(), str::to_owned);
                }
                s
            })
            .collect())
    }

    /// The account's quota, logging in first if needed. `None` without an
    /// account: the quota of an IP address is only known after a download.
    ///
    /// # Errors
    ///
    /// [`OpenSubtitlesError::Login`] for a wrong username or password, and
    /// the errors of [`search`](Self::search).
    pub fn user_info(&self) -> Result<Option<UserInfo>> {
        if self.account.is_none() {
            return Ok(None);
        }
        let info = self.with_session(|token, base| {
            let request = self.request(base, "/infos/user", Vec::new(), Some(token))?;
            let response = self.send(|t| t.get(&request))?;
            let parsed: UserResponse = serde_json::from_str(&response.body)
                .map_err(|e| OpenSubtitlesError::Decode(e.to_string()))?;
            Ok(parsed.data)
        })?;
        self.set_quota(Quota {
            remaining: info.remaining_downloads,
            allowed: info.allowed_downloads,
            reset_time_utc: self.quota().and_then(|q| q.reset_time_utc),
            logged_in: true,
        });
        Ok(Some(info))
    }

    /// Asks for the temporary link of file `file_id`. **Spends one
    /// download of the quota** (the account's when logged in, else the IP
    /// address's).
    ///
    /// # Errors
    ///
    /// [`OpenSubtitlesError::QuotaExhausted`] when the quota is used up,
    /// [`OpenSubtitlesError::Login`], and the errors of
    /// [`search`](Self::search).
    pub fn download(&self, file_id: u64) -> Result<DownloadLink> {
        let body = serde_json::json!({ "file_id": file_id }).to_string();
        let call = |token: Option<&str>, base: &str| -> Result<DownloadLink> {
            let request = self.request(base, "/download", Vec::new(), token)?;
            let response = self.send(|t| t.post_json(&request, &body))?;
            serde_json::from_str::<DownloadLink>(&response.body)
                .map_err(|e| OpenSubtitlesError::Decode(e.to_string()))
        };
        let link = if self.account.is_some() {
            self.with_session(|token, base| call(Some(token), base))
        } else {
            call(None, &self.config.base_url)
        };
        let previous = self.quota();
        let link = match link {
            Err(OpenSubtitlesError::QuotaExhausted { reset_time_utc }) => {
                self.set_quota(Quota {
                    remaining: Some(0),
                    allowed: previous.as_ref().and_then(|q| q.allowed),
                    reset_time_utc: reset_time_utc.clone(),
                    logged_in: self.account.is_some(),
                });
                return Err(OpenSubtitlesError::QuotaExhausted { reset_time_utc });
            }
            other => other?,
        };
        self.set_quota(Quota {
            remaining: link.remaining,
            allowed: link
                .remaining
                .zip(link.requests)
                .and_then(|(r, q)| u32::try_from(r + q).ok())
                .or(previous.as_ref().and_then(|q| q.allowed)),
            reset_time_utc: link.reset_time_utc.clone(),
            logged_in: self.account.is_some(),
        });
        if link.link.is_empty() {
            if link.remaining.is_some_and(|r| r < 0) {
                return Err(OpenSubtitlesError::QuotaExhausted {
                    reset_time_utc: link.reset_time_utc,
                });
            }
            return Err(OpenSubtitlesError::Decode("no download link".to_owned()));
        }
        Ok(link)
    }

    /// The subtitle file behind a [`DownloadLink`], byte for byte (no key
    /// or token is sent: the link is a temporary CDN address).
    ///
    /// # Errors
    ///
    /// HTTP, network and size errors.
    pub fn fetch(&self, link: &DownloadLink) -> Result<Vec<u8>> {
        let (base, path) = split_url(&link.link)
            .ok_or_else(|| OpenSubtitlesError::Decode(format!("bad link: {}", link.link)))?;
        let request = HttpRequest {
            base,
            path,
            query: Vec::new(),
            headers: vec![("User-Agent".to_owned(), USER_AGENT.to_owned())],
        };
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            let response = self
                .transport
                .get_bytes(&request)
                .map_err(|e| OpenSubtitlesError::Transport(e.0))?;
            match response.status {
                200..=299 => {
                    if response.body.len() > MAX_FILE_BYTES {
                        return Err(OpenSubtitlesError::Http {
                            status: response.status,
                            message: "file too large for a subtitle".to_owned(),
                        });
                    }
                    return Ok(response.body);
                }
                429 | 500 | 502 | 503 | 504 if attempts <= self.config.retry.max_retries => {
                    let wait = parse_retry_after(
                        response
                            .headers
                            .iter()
                            .find(|(k, _)| k == "retry-after")
                            .map(|(_, v)| v.as_str()),
                    );
                    self.clock
                        .sleep(self.config.retry.delay(attempts - 1, wait));
                }
                status => {
                    return Err(OpenSubtitlesError::Http {
                        status,
                        message: "subtitle file not available".to_owned(),
                    });
                }
            }
        }
    }

    fn set_quota(&self, quota: Quota) {
        *self.quota.lock().unwrap_or_else(|e| e.into_inner()) = Some(quota);
    }

    /// Runs `f` with a valid token, logging in when there is none or it is
    /// old, and once more after a 401 (the token expired on the server).
    fn with_session<T>(&self, f: impl Fn(&str, &str) -> Result<T>) -> Result<T> {
        for attempt in 0..2 {
            let (token, base) = self.session(attempt > 0)?;
            match f(&token, &base) {
                Err(OpenSubtitlesError::Http { status: 401, .. }) if attempt == 0 => {}
                other => return other,
            }
        }
        Err(OpenSubtitlesError::Login("token refused".to_owned()))
    }

    fn session(&self, renew: bool) -> Result<(String, String)> {
        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let now = self.clock.monotonic();
        if !renew
            && let Some(s) = session.as_ref()
            && now.saturating_sub(s.issued) < SESSION_TTL
        {
            return Ok((s.token.clone(), s.base.clone()));
        }
        let account = self
            .account
            .as_ref()
            .ok_or_else(|| OpenSubtitlesError::Login("no account".to_owned()))?;
        let body = serde_json::json!({
            "username": account.username,
            "password": account.password,
        })
        .to_string();
        let request = self.request(&self.config.base_url, "/login", Vec::new(), None)?;
        let response = match self.send(|t| t.post_json(&request, &body)) {
            Err(OpenSubtitlesError::Http {
                status: 400 | 401,
                message,
            }) => {
                return Err(OpenSubtitlesError::Login(message));
            }
            other => other?,
        };
        let login: LoginResponse = serde_json::from_str(&response.body)
            .map_err(|e| OpenSubtitlesError::Decode(e.to_string()))?;
        let token = login
            .token
            .filter(|t| !t.is_empty())
            .ok_or_else(|| OpenSubtitlesError::Login("no token".to_owned()))?;
        let base = login
            .base_url
            .as_deref()
            .and_then(|host| account_base(&self.config.base_url, host))
            .unwrap_or_else(|| self.config.base_url.clone());
        if let Some(user) = login.user {
            self.set_quota(Quota {
                remaining: user.remaining_downloads,
                allowed: user.allowed_downloads,
                reset_time_utc: None,
                logged_in: true,
            });
        }
        *session = Some(Session {
            token: token.clone(),
            base: base.clone(),
            issued: now,
        });
        Ok((token, base))
    }

    /// One request with pacing, one at a time, and retries.
    fn send(
        &self,
        call: impl Fn(&dyn HttpTransport) -> std::result::Result<HttpResponse, TransportError>,
    ) -> Result<HttpResponse> {
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
            let response =
                call(self.transport.as_ref()).map_err(|e| OpenSubtitlesError::Transport(e.0))?;
            let status = response.status;
            if (200..300).contains(&status) {
                return Ok(response);
            }
            let body: Option<ErrorBody> = serde_json::from_str(&response.body).ok();
            let message = body
                .as_ref()
                .and_then(|b| b.message.clone().or_else(|| b.errors.first().cloned()))
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| format!("HTTP {status}"));
            if status == 406 {
                tracing::warn!(target: "pa_metadata::opensubtitles", "download quota used up");
                return Err(OpenSubtitlesError::QuotaExhausted {
                    reset_time_utc: body.and_then(|b| b.reset_time_utc),
                });
            }
            if status == 403 {
                return Err(OpenSubtitlesError::KeyRejected(status));
            }
            let retryable = matches!(status, 429 | 500 | 502 | 503 | 504);
            if !retryable {
                return Err(OpenSubtitlesError::Http { status, message });
            }
            if attempts > self.config.retry.max_retries {
                return Err(OpenSubtitlesError::Busy { status, attempts });
            }
            let delay = self.config.retry.delay(
                attempts - 1,
                parse_retry_after(response.header("retry-after")),
            );
            tracing::info!(
                target: "pa_metadata::opensubtitles",
                status,
                delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                "OpenSubtitles: busy, backing off"
            );
            self.clock.sleep(delay);
        }
    }
}

/// The API base on the host `login` names (`vip-api.opensubtitles.com`),
/// keeping the scheme and path of `default`. Only OpenSubtitles hosts are
/// taken: a token is never sent anywhere else.
fn account_base(default: &str, host: &str) -> Option<String> {
    let host = host
        .trim()
        .trim_start_matches("https://")
        .trim_end_matches('/');
    let host = host.split('/').next()?;
    if host.is_empty() || !(host == "opensubtitles.com" || host.ends_with(".opensubtitles.com")) {
        return None;
    }
    let (scheme, rest) = default.split_once("://")?;
    let path = rest.find('/').map_or("", |i| &rest[i..]);
    Some(format!("{scheme}://{host}{path}"))
}

/// `https://host/a/b?q` → (`https://host`, `/a/b?q`). The query stays in
/// the path: the link is used as it is.
fn split_url(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme, "https" | "http") {
        return None;
    }
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    (!host.is_empty()).then(|| (format!("{scheme}://{host}"), path.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_map_both_ways() {
        assert_eq!(search_codes("rus"), vec!["ru"]);
        assert_eq!(search_codes("ru"), vec!["ru"]);
        assert_eq!(search_codes("en-US"), vec!["en"]);
        assert_eq!(search_codes("por"), vec!["pt-br", "pt-pt"]);
        assert_eq!(search_codes("pt-BR"), vec!["pt-br", "pt-pt"]);
        assert_eq!(search_codes("ger"), vec!["de"]);
        assert_eq!(search_codes("deu"), Vec::<&str>::new());
        assert_eq!(search_codes("xx"), Vec::<&str>::new());
        assert_eq!(iso639_2("pt-BR"), Some("por"));
        assert_eq!(iso639_2("ru"), Some("rus"));
        assert_eq!(iso639_2("zz"), None);
    }

    #[test]
    fn params_are_lower_case_and_sorted() {
        let q = SearchQuery {
            moviehash: Some("8E245D9679D31E12".into()),
            parent_imdb_id: Some("tt0944947".into()),
            season_number: Some(1),
            episode_number: Some(2),
            languages: vec!["RU".into(), "en".into(), "ru".into()],
            kind: Some("episode".into()),
            ..SearchQuery::default()
        };
        let p: Vec<String> = q.params().iter().map(|(k, v)| format!("{k}={v}")).collect();
        assert_eq!(
            p,
            [
                "episode_number=2",
                "languages=en,ru",
                "moviehash=8e245d9679d31e12",
                "parent_imdb_id=944947",
                "season_number=1",
                "type=episode",
            ]
        );
    }

    #[test]
    fn urls_split_and_hosts_checked() {
        assert_eq!(
            split_url("https://www.opensubtitles.com/download/AB/subfile/x.ass"),
            Some((
                "https://www.opensubtitles.com".into(),
                "/download/AB/subfile/x.ass".into()
            ))
        );
        assert_eq!(split_url("ftp://x/y"), None);
        assert_eq!(
            account_base(DEFAULT_BASE_URL, "vip-api.opensubtitles.com").as_deref(),
            Some("https://vip-api.opensubtitles.com/api/v1")
        );
        assert_eq!(account_base(DEFAULT_BASE_URL, "evil.example"), None);
    }

    #[test]
    fn account_debug_hides_password() {
        let a = Account::new("user", "hunter2").unwrap();
        assert!(!format!("{a:?}").contains("hunter2"));
        assert!(Account::new(" ", "x").is_none());
        assert!(Account::new("u", "").is_none());
    }
}
