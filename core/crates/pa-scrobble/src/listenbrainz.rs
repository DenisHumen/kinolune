// SPDX-License-Identifier: MPL-2.0
//! ListenBrainz (sans-IO):
//! <https://listenbrainz.readthedocs.io/en/latest/users/api/core.html>.
//!
//! - `POST /1/submit-listens` with `Authorization: Token <user token>`:
//!   `playing_now` (no timestamp, not stored for good), `single` and
//!   `import` (up to [`MAX_BATCH`] listens here; the service takes 1000).
//! - `GET /1/validate-token` checks a token and names its user.
//! - 401 needs a new token; 429 waits `X-RateLimit-Reset-In` seconds; 5xx
//!   and network errors are retried; 400 drops the batch.

use serde_json::{Map, Value, json};

use crate::http::{Request, Response};
use crate::{ApiError, Listen, Outcome, Track};

/// API root.
pub const API_ROOT: &str = "https://api.listenbrainz.org";
/// Most listens in one `import` here.
pub const MAX_BATCH: usize = 100;

const JSON: &str = "application/json";

fn auth(token: &str) -> Vec<(String, String)> {
    vec![("authorization".into(), format!("Token {}", token.trim()))]
}

fn metadata(t: &Track) -> Value {
    let mut info = Map::new();
    info.insert("media_player".into(), "Kinolune".into());
    info.insert("submission_client".into(), "Kinolune".into());
    info.insert(
        "submission_client_version".into(),
        env!("CARGO_PKG_VERSION").into(),
    );
    if let Some(d) = t.duration_ms {
        info.insert("duration_ms".into(), d.into());
    }
    if let Some(n) = t.track_number {
        info.insert("tracknumber".into(), n.into());
    }
    if let Some(m) = t.mbid.as_deref().filter(|m| !m.is_empty()) {
        info.insert("recording_mbid".into(), m.into());
    }
    let mut meta = Map::new();
    meta.insert("artist_name".into(), t.artist.clone().into());
    meta.insert("track_name".into(), t.title.clone().into());
    if let Some(a) = t.album.as_deref().filter(|a| !a.is_empty()) {
        meta.insert("release_name".into(), a.into());
    }
    meta.insert("additional_info".into(), Value::Object(info));
    Value::Object(meta)
}

fn submit(token: &str, body: Value) -> Request {
    Request {
        method: "POST",
        url: format!("{API_ROOT}/1/submit-listens"),
        headers: auth(token),
        body: Some((JSON, body.to_string())),
    }
}

/// `playing_now` of `track`.
pub fn now_playing(token: &str, track: &Track) -> Request {
    submit(
        token,
        json!({"listen_type": "playing_now", "payload": [{"track_metadata": metadata(track)}]}),
    )
}

/// `single` for one listen, `import` for more (up to [`MAX_BATCH`]).
pub fn listens(token: &str, listens: &[Listen]) -> Request {
    let batch = &listens[..listens.len().min(MAX_BATCH)];
    let payload: Vec<Value> = batch
        .iter()
        .map(|l| json!({"listened_at": l.started_at, "track_metadata": metadata(&l.track)}))
        .collect();
    let kind = if batch.len() == 1 { "single" } else { "import" };
    submit(token, json!({"listen_type": kind, "payload": payload}))
}

/// Checks `token`.
pub fn validate_token(token: &str) -> Request {
    Request {
        method: "GET",
        url: format!("{API_ROOT}/1/validate-token"),
        headers: auth(token),
        body: None,
    }
}

/// The user of a valid token; `Refused` for an invalid one.
pub fn parse_validate(r: &Response) -> Result<String, ApiError> {
    if r.status >= 500 {
        return Err(ApiError::Unavailable(format!("HTTP {}", r.status)));
    }
    let v: Value = serde_json::from_str(&r.body).map_err(|_| ApiError::Malformed)?;
    if v.get("valid").and_then(Value::as_bool) == Some(true)
        && let Some(user) = v.get("user_name").and_then(Value::as_str)
    {
        return Ok(user.to_owned());
    }
    Err(ApiError::Refused {
        code: i64::from(r.status),
        message: v
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("invalid token")
            .to_owned(),
    })
}

/// What an answer to [`listens`] or [`now_playing`] means.
pub fn outcome(r: &Response) -> Outcome {
    match r.status {
        200..=299 => Outcome::Done,
        401 | 403 => Outcome::Auth,
        429 => Outcome::Retry {
            after_ms: r
                .header("x-ratelimit-reset-in")
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|s| (s + 1) * 1000),
        },
        500..=599 => Outcome::Retry { after_ms: None },
        _ => Outcome::Drop,
    }
}
