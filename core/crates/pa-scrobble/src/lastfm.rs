// SPDX-License-Identifier: MPL-2.0
//! Last.fm Scrobbling 2.0 (sans-IO): <https://www.last.fm/api/scrobbling>.
//!
//! - Desktop sign-in (<https://www.last.fm/api/desktopauth>): `auth.getToken`,
//!   the user approves the token in the browser at [`auth_url`], then
//!   `auth.getSession` gives a session key that does not expire (the user
//!   can revoke it on Last.fm).
//! - Every call is signed: `api_sig` = MD5 of the parameters sorted by
//!   name, each `name` + `value`, then the shared secret (`format` is not
//!   signed).
//! - `track.updateNowPlaying` is not retried; `track.scrobble` takes up to
//!   [`MAX_BATCH`] scrobbles. Errors 11 and 16 are retried, 9 needs a new
//!   sign-in, 29 (rate limit) waits; any other error drops the batch.
//!   Scrobbles the service filters («ignored») are not sent again.

use md5::{Digest, Md5};
use serde_json::Value;

use crate::http::{self, Request, Response};
use crate::key::LastFmKey;
use crate::{ApiError, Listen, Outcome, Track};

/// API root.
pub const API_ROOT: &str = "https://ws.audioscrobbler.com/2.0/";
/// Where the user approves a token.
pub const AUTH_ROOT: &str = "https://www.last.fm/api/auth/";
/// Most scrobbles in one `track.scrobble`.
pub const MAX_BATCH: usize = 50;
/// Wait after a rate limit (error 29), ms.
pub const RATE_LIMIT_WAIT_MS: u64 = 5 * 60_000;

const FORM: &str = "application/x-www-form-urlencoded";

/// `api_sig` of `params` (without `format`).
pub fn sign(params: &[(String, String)], secret: &str) -> String {
    let mut sorted: Vec<&(String, String)> = params
        .iter()
        .filter(|(k, _)| k != "format" && k != "callback")
        .collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Md5::new();
    for (k, v) in sorted {
        h.update(k.as_bytes());
        h.update(v.as_bytes());
    }
    h.update(secret.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn p(k: &str, v: impl Into<String>) -> (String, String) {
    (k.to_owned(), v.into())
}

/// A signed call of `method` with `params`: `POST` for writes, `GET`
/// for the sign-in steps.
fn call(key: &LastFmKey, method: &str, mut params: Vec<(String, String)>, post: bool) -> Request {
    params.push(p("method", method));
    params.push(p("api_key", key.api_key.clone()));
    let sig = sign(&params, &key.secret);
    params.push(p("api_sig", sig));
    params.push(p("format", "json"));
    let body = http::form(&params);
    if post {
        Request {
            method: "POST",
            url: API_ROOT.to_owned(),
            headers: Vec::new(),
            body: Some((FORM, body)),
        }
    } else {
        Request {
            method: "GET",
            url: format!("{API_ROOT}?{body}"),
            headers: Vec::new(),
            body: None,
        }
    }
}

/// Step 1 of the sign-in: a token to approve.
pub fn get_token(key: &LastFmKey) -> Request {
    call(key, "auth.getToken", Vec::new(), false)
}

/// The token of a [`get_token`] answer.
pub fn parse_token(r: &Response) -> Result<String, ApiError> {
    let v = json(r)?;
    v.get("token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .ok_or(ApiError::Malformed)
}

/// Step 2: the page where the user approves `token` (valid for 60 min).
pub fn auth_url(key: &LastFmKey, token: &str) -> String {
    let mut out = format!("{AUTH_ROOT}?api_key=");
    http::encode(&key.api_key, &mut out);
    out.push_str("&token=");
    http::encode(token, &mut out);
    out
}

/// Step 3: the session of an approved `token` (usable once).
pub fn get_session(key: &LastFmKey, token: &str) -> Request {
    call(key, "auth.getSession", vec![p("token", token)], false)
}

/// A signed-in Last.fm account. `Debug` hides the key.
#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    pub name: String,
    pub key: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// The session of a [`get_session`] answer.
pub fn parse_session(r: &Response) -> Result<Session, ApiError> {
    let v = json(r)?;
    let s = v.get("session").ok_or(ApiError::Malformed)?;
    let name = s.get("name").and_then(Value::as_str);
    let key = s.get("key").and_then(Value::as_str);
    match (name, key) {
        (Some(n), Some(k)) if !k.is_empty() => Ok(Session {
            name: n.to_owned(),
            key: k.to_owned(),
        }),
        _ => Err(ApiError::Malformed),
    }
}

fn track_params(t: &Track, i: Option<usize>) -> Vec<(String, String)> {
    let n = |k: &str| match i {
        Some(i) => format!("{k}[{i}]"),
        None => k.to_owned(),
    };
    let mut out = vec![(n("artist"), t.artist.clone()), (n("track"), t.title.clone())];
    if let Some(a) = t.album.as_deref().filter(|a| !a.is_empty()) {
        out.push((n("album"), a.to_owned()));
    }
    if let Some(a) = t.album_artist.as_deref().filter(|a| !a.is_empty()) {
        out.push((n("albumArtist"), a.to_owned()));
    }
    if let Some(d) = t.duration_ms {
        out.push((n("duration"), (d / 1000).to_string()));
    }
    if let Some(no) = t.track_number {
        out.push((n("trackNumber"), no.to_string()));
    }
    if let Some(m) = t.mbid.as_deref().filter(|m| !m.is_empty()) {
        out.push((n("mbid"), m.to_owned()));
    }
    out
}

/// `track.updateNowPlaying` of `track` with the session key `sk`.
pub fn now_playing(key: &LastFmKey, sk: &str, track: &Track) -> Request {
    let mut params = track_params(track, None);
    params.push(p("sk", sk));
    call(key, "track.updateNowPlaying", params, true)
}

/// `track.scrobble` of up to [`MAX_BATCH`] `listens`.
pub fn scrobble(key: &LastFmKey, sk: &str, listens: &[Listen]) -> Request {
    let mut params = Vec::new();
    for (i, l) in listens.iter().take(MAX_BATCH).enumerate() {
        params.extend(track_params(&l.track, Some(i)));
        params.push((format!("timestamp[{i}]"), l.started_at.to_string()));
    }
    params.push(p("sk", sk));
    call(key, "track.scrobble", params, true)
}

fn json(r: &Response) -> Result<Value, ApiError> {
    let v: Value = serde_json::from_str(&r.body).map_err(|_| {
        if r.status >= 500 {
            ApiError::Unavailable(format!("HTTP {}", r.status))
        } else {
            ApiError::Malformed
        }
    })?;
    if let Some(code) = v.get("error").and_then(Value::as_i64) {
        let message = v
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        return Err(if matches!(code, 11 | 16 | 29) {
            ApiError::Unavailable(message)
        } else {
            ApiError::Refused { code, message }
        });
    }
    Ok(v)
}

/// What an answer to [`scrobble`] or [`now_playing`] means.
pub fn outcome(r: &Response) -> Outcome {
    let Ok(v) = serde_json::from_str::<Value>(&r.body) else {
        return if r.status >= 500 || r.status == 429 {
            Outcome::Retry { after_ms: None }
        } else {
            Outcome::Drop
        };
    };
    match v.get("error").and_then(Value::as_i64) {
        None if (200..300).contains(&r.status) => Outcome::Done,
        None if r.status >= 500 => Outcome::Retry { after_ms: None },
        None => Outcome::Drop,
        // Service offline, temporarily unavailable.
        Some(11 | 16) => Outcome::Retry { after_ms: None },
        Some(29) => Outcome::Retry {
            after_ms: Some(RATE_LIMIT_WAIT_MS),
        },
        // Invalid session, bad API key, suspended key, auth required.
        Some(4 | 9 | 10 | 14 | 26) => Outcome::Auth,
        Some(_) => Outcome::Drop,
    }
}

/// How many scrobbles of a `track.scrobble` answer the service filtered.
pub fn ignored(r: &Response) -> u64 {
    serde_json::from_str::<Value>(&r.body)
        .ok()
        .and_then(|v| {
            let n = v.pointer("/scrobbles/@attr/ignored")?;
            n.as_u64().or_else(|| n.as_str()?.parse().ok())
        })
        .unwrap_or(0)
}
