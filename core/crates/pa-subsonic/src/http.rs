// SPDX-License-Identifier: MPL-2.0
//! The HTTP side: addresses, authentication, the response envelope and
//! Subsonic error codes.

use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use md5::{Digest, Md5};
use serde_json::Value;

use crate::{Error, Result};

/// The Subsonic API version this client speaks (`v`).
pub const API_VERSION: &str = "1.16.1";
/// Client name sent as `c` (servers show it in their player list).
pub const CLIENT_NAME: &str = "Kinolune";
/// Largest JSON answer read (a big `search3` page).
pub(crate) const MAX_JSON: u64 = 64 << 20;
/// Largest binary answer read (cover art, one range of a file).
pub(crate) const MAX_BLOB: u64 = 256 << 20;

/// A password or API key. `Debug` and `Display` do not print it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a secret.
    #[must_use]
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// The secret itself. Do not log it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// How the client proves who it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credentials {
    /// A user name and password: sent as a salted token, or, if the
    /// server cannot check tokens for this user, as the hex-encoded
    /// password.
    Password {
        /// User name.
        user: String,
        /// Password.
        password: Secret,
    },
    /// An OpenSubsonic API key (`apiKeyAuthentication` extension); the
    /// server knows the user from the key.
    ApiKey(Secret),
}

impl Credentials {
    /// A user name and password.
    #[must_use]
    pub fn password(user: impl Into<String>, password: impl Into<String>) -> Self {
        Self::Password {
            user: user.into(),
            password: Secret::new(password),
        }
    }

    /// An API key.
    #[must_use]
    pub fn api_key(key: impl Into<String>) -> Self {
        Self::ApiKey(Secret::new(key))
    }

    /// The secret (password or key), to mask it in texts.
    #[must_use]
    pub fn secret(&self) -> &Secret {
        match self {
            Self::Password { password, .. } => password,
            Self::ApiKey(k) => k,
        }
    }
}

/// How the credentials go into a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// `u`, `t = md5(password + salt)`, `s = salt` (Subsonic 1.13+). The
    /// password itself never leaves the device.
    Token,
    /// `u`, `p = enc:<hex of the password>`: for servers or users that
    /// cannot check tokens (error 41, LDAP users of some servers).
    HexPassword,
    /// `apiKey` (OpenSubsonic).
    ApiKey,
}

impl AuthMode {
    fn to_u8(self) -> u8 {
        match self {
            Self::Token => 0,
            Self::HexPassword => 1,
            Self::ApiKey => 2,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::HexPassword,
            2 => Self::ApiKey,
            _ => Self::Token,
        }
    }
}

/// An answer: status, the headers the client needs, body bytes.
#[derive(Debug)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) content_type: Option<String>,
    pub(crate) content_range: Option<String>,
    pub(crate) body: Vec<u8>,
}

impl Response {
    /// Whether the body is a Subsonic envelope (JSON or XML) rather than
    /// the bytes asked for: binary endpoints answer errors that way, often
    /// with HTTP 200.
    pub(crate) fn is_envelope(&self) -> bool {
        self.content_type.as_deref().is_some_and(|t| {
            let t = t.to_ascii_lowercase();
            t.starts_with("application/json")
                || t.starts_with("text/xml")
                || t.starts_with("application/xml")
        })
    }
}

/// A Subsonic server at one address, with one set of credentials.
///
/// Methods block; call them off the UI thread. A `Client` is `Send + Sync`
/// and cheap to share in an `Arc`.
pub struct Client {
    agent: ureq::Agent,
    address: String,
    credentials: Credentials,
    mode: AtomicU8,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("address", &self.address)
            .field("user", &self.user())
            .field("mode", &self.auth_mode())
            .finish_non_exhaustive()
    }
}

/// `http(s)://host[:port][/base]` without a trailing slash (and without a
/// pasted `/rest`); anything else (credentials, query, other schemes) is
/// refused.
///
/// # Errors
///
/// [`Error::Invalid`] for anything else.
pub fn normalize_address(address: &str) -> Result<String> {
    let mut a = address.trim().trim_end_matches('/');
    if let Some(base) = a.strip_suffix("/rest") {
        a = base.trim_end_matches('/');
    }
    let rest = a
        .strip_prefix("http://")
        .or_else(|| a.strip_prefix("https://"))
        .ok_or_else(|| Error::Invalid("not an http(s) address".into()))?;
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() {
        return Err(Error::Invalid("no host in the address".into()));
    }
    if host.contains('@') {
        return Err(Error::Invalid(
            "the address must not contain credentials".into(),
        ));
    }
    if rest.contains(['?', '#']) {
        return Err(Error::Invalid(
            "the address must not have a query or fragment".into(),
        ));
    }
    Ok(a.to_owned())
}

/// Percent-encodes `text` for a query parameter.
pub(crate) fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    use fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// The token of the Subsonic token auth: lowercase hex of
/// `md5(password + salt)`.
#[must_use]
pub fn salted_token(password: &str, salt: &str) -> String {
    let mut h = Md5::new();
    h.update(password.as_bytes());
    h.update(salt.as_bytes());
    hex(&h.finalize())
}

/// A fresh random salt, 12 hex characters.
fn salt() -> String {
    let mut b = [0u8; 6];
    if getrandom::fill(&mut b).is_err() {
        // No OS randomness: a time-based salt still changes per request
        // (the salt hides nothing; it only keeps tokens from repeating).
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        b.copy_from_slice(&t.to_le_bytes()[..6]);
    }
    hex(&b)
}

/// The kind of a transport failure, never the URL: every URL carries the
/// credentials.
fn describe(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => "request timed out".to_owned(),
        ureq::Error::HostNotFound => "host not found".to_owned(),
        ureq::Error::ConnectionFailed => "connection failed".to_owned(),
        ureq::Error::Io(io) => format!("I/O error: {}", io.kind()),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => "TLS error".to_owned(),
        ureq::Error::BodyExceedsLimit(_) => "response body too large".to_owned(),
        _ => "HTTP transport error".to_owned(),
    }
}

fn transport(e: &ureq::Error) -> Error {
    if matches!(e, ureq::Error::Timeout(_)) {
        Error::TimedOut
    } else {
        Error::Unreachable(describe(e))
    }
}

/// The error of a Subsonic `error` object.
pub(crate) fn subsonic_error(code: i64, message: &str) -> Error {
    let message = message.trim().to_owned();
    match code {
        10 => Error::Invalid(message),
        20 | 30 | 41..=43 => Error::Unsupported(if message.is_empty() {
            format!("error {code}")
        } else {
            message
        }),
        40 | 44 => Error::Unauthorized,
        50 => Error::Forbidden,
        70 => Error::NotFound,
        _ => Error::Server { code, message },
    }
}

/// Subsonic error code of a failed answer, if it is one (41 and 42 make
/// the client retry with the hex password).
fn error_code(body: &[u8]) -> Option<(i64, String)> {
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        let r = v.get("subsonic-response")?;
        if r.get("status").and_then(Value::as_str) != Some("failed") {
            return None;
        }
        let e = r.get("error")?;
        return Some((
            e.get("code").and_then(Value::as_i64).unwrap_or(0),
            e.get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        ));
    }
    // XML (a server ignoring `f=json`): `<error code="40" message="…"/>`.
    let text = String::from_utf8_lossy(body);
    let at = text.find("<error")?;
    let attr = |name: &str| {
        let rest = &text[at..];
        let start = rest.find(&format!("{name}=\""))? + name.len() + 2;
        let end = rest[start..].find('"')?;
        Some(rest[start..start + end].to_owned())
    };
    Some((
        attr("code")?.parse().ok()?,
        attr("message").unwrap_or_default(),
    ))
}

/// The `subsonic-response` object of a successful JSON answer.
pub(crate) fn envelope(body: &[u8]) -> Result<Value> {
    if let Some((code, message)) = error_code(body) {
        return Err(subsonic_error(code, &message));
    }
    let mut v: Value = serde_json::from_slice(body)
        .map_err(|e| Error::Protocol(format!("not a Subsonic JSON answer: {e}")))?;
    let r = v
        .get_mut("subsonic-response")
        .map(Value::take)
        .ok_or_else(|| Error::Protocol("no subsonic-response".into()))?;
    match r.get("status").and_then(Value::as_str) {
        Some("ok") => Ok(r),
        other => Err(Error::Protocol(format!("status {other:?}"))),
    }
}

impl Client {
    /// A client for the server at `address` (see [`normalize_address`]).
    /// Does not touch the network.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if the address is malformed.
    pub fn new(address: &str, credentials: Credentials) -> Result<Self> {
        Self::with_timeouts(
            address,
            credentials,
            Duration::from_secs(5),
            Duration::from_secs(60),
        )
    }

    /// Like [`new`](Self::new) with a connect timeout and an overall
    /// request timeout.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_timeouts(
        address: &str,
        credentials: Credentials,
        connect: Duration,
        overall: Duration,
    ) -> Result<Self> {
        let address = normalize_address(address)?;
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(connect))
            .timeout_global(Some(overall))
            .user_agent(format!("{CLIENT_NAME}/{}", env!("CARGO_PKG_VERSION")))
            .build();
        let mode = match credentials {
            Credentials::Password { .. } => AuthMode::Token,
            Credentials::ApiKey(_) => AuthMode::ApiKey,
        };
        Ok(Self {
            agent: ureq::Agent::new_with_config(config),
            address,
            credentials,
            mode: AtomicU8::new(mode.to_u8()),
        })
    }

    /// The server address.
    #[must_use]
    pub fn address(&self) -> &str {
        &self.address
    }

    /// The user name, `None` for an API key.
    #[must_use]
    pub fn user(&self) -> Option<&str> {
        match &self.credentials {
            Credentials::Password { user, .. } => Some(user),
            Credentials::ApiKey(_) => None,
        }
    }

    /// The credentials.
    #[must_use]
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// How the credentials are sent now: [`AuthMode::Token`] until the
    /// server says it cannot check tokens.
    #[must_use]
    pub fn auth_mode(&self) -> AuthMode {
        AuthMode::from_u8(self.mode.load(Ordering::Relaxed))
    }

    /// Forces a way of sending the password (a stored choice);
    /// [`AuthMode::ApiKey`] is ignored for a password and the other way
    /// round.
    pub fn set_auth_mode(&self, mode: AuthMode) {
        let fits = matches!(
            (&self.credentials, mode),
            (
                Credentials::Password { .. },
                AuthMode::Token | AuthMode::HexPassword
            ) | (Credentials::ApiKey(_), AuthMode::ApiKey)
        );
        if fits {
            self.mode.store(mode.to_u8(), Ordering::Relaxed);
        }
    }

    /// The query of the credentials and the protocol (`u`, `t`, `s` or
    /// `p` or `apiKey`; `v`, `c`, `f`).
    fn auth_query(&self, mode: AuthMode) -> String {
        let auth = match (&self.credentials, mode) {
            (Credentials::Password { user, password }, AuthMode::HexPassword) => format!(
                "u={}&p=enc:{}",
                encode(user),
                hex(password.expose().as_bytes())
            ),
            (Credentials::Password { user, password }, _) => {
                let s = salt();
                format!(
                    "u={}&t={}&s={s}",
                    encode(user),
                    salted_token(password.expose(), &s)
                )
            }
            (Credentials::ApiKey(key), _) => format!("apiKey={}", encode(key.expose())),
        };
        format!("{auth}&v={API_VERSION}&c={CLIENT_NAME}&f=json")
    }

    /// The absolute URL of API `method` (`ping`, `stream`, …) with
    /// `params` and the credentials. Contains a secret: do not log it.
    #[must_use]
    pub fn url(&self, method: &str, params: &[(&str, String)]) -> String {
        self.url_with(self.auth_mode(), method, params)
    }

    fn url_with(&self, mode: AuthMode, method: &str, params: &[(&str, String)]) -> String {
        let mut url = format!(
            "{}/rest/{method}.view?{}",
            self.address,
            self.auth_query(mode)
        );
        for (k, v) in params {
            url.push('&');
            url.push_str(k);
            url.push('=');
            url.push_str(&encode(v));
        }
        url
    }

    /// Sends one GET of `method`. `range`: `(offset, len)` of a byte
    /// range. Retries once with the hex password when the server cannot
    /// check tokens for this user (errors 41, 42).
    pub(crate) fn get(
        &self,
        method: &str,
        params: &[(&str, String)],
        range: Option<(u64, u64)>,
        limit: u64,
    ) -> Result<Response> {
        let mode = self.auth_mode();
        let r = self.get_with(mode, method, params, range, limit)?;
        if mode == AuthMode::Token
            && r.is_envelope()
            && matches!(error_code(&r.body), Some((41 | 42, _)))
        {
            self.mode
                .store(AuthMode::HexPassword.to_u8(), Ordering::Relaxed);
            return self.get_with(AuthMode::HexPassword, method, params, range, limit);
        }
        Ok(r)
    }

    fn get_with(
        &self,
        mode: AuthMode,
        method: &str,
        params: &[(&str, String)],
        range: Option<(u64, u64)>,
        limit: u64,
    ) -> Result<Response> {
        let url = self.url_with(mode, method, params);
        let mut b = self.agent.get(&url);
        if let Some((off, len)) = range {
            b = b.header("range", format!("bytes={off}-{}", off + len.max(1) - 1));
        }
        let mut response = b.call().map_err(|e| transport(&e))?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let status = response.status().as_u16();
        let content_type = header("content-type");
        let content_range = header("content-range");
        let body = response
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(|e| transport(&e))?;
        Ok(Response {
            status,
            content_type,
            content_range,
            body,
        })
    }

    /// Calls `method` and returns its `subsonic-response` object.
    pub(crate) fn call(&self, method: &str, params: &[(&str, String)]) -> Result<Value> {
        let r = self.get(method, params, None, MAX_JSON)?;
        check_status(&r)?;
        envelope(&r.body)
    }

    /// Fetches binary `method` (cover art, a range of a file); a Subsonic
    /// error in place of the bytes is an error.
    pub(crate) fn binary(
        &self,
        method: &str,
        params: &[(&str, String)],
        range: Option<(u64, u64)>,
    ) -> Result<Response> {
        let r = self.get(method, params, range, MAX_BLOB)?;
        if r.is_envelope() {
            if let Some((code, message)) = error_code(&r.body) {
                return Err(subsonic_error(code, &message));
            }
            if r.status < 300 {
                return Err(Error::Protocol(
                    "the server sent a message in place of the file".into(),
                ));
            }
        }
        check_status(&r)?;
        Ok(r)
    }
}

/// Fails for HTTP errors that carry no Subsonic error.
fn check_status(r: &Response) -> Result<()> {
    if r.status < 300 || error_code(&r.body).is_some() {
        return Ok(());
    }
    Err(match r.status {
        401 => Error::Unauthorized,
        403 => Error::Forbidden,
        404 => Error::NotFound,
        416 => Error::Invalid("byte range outside the file".into()),
        s => Error::Status { status: s },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_md5_of_password_and_salt() {
        // The example of the Subsonic API documentation.
        assert_eq!(
            salted_token("sesame", "c19b2d"),
            "26719a1196d2a940705a59634eb18eab"
        );
    }

    #[test]
    fn normalizes_addresses() {
        assert_eq!(
            normalize_address(" http://nas:4533/ ").unwrap(),
            "http://nas:4533"
        );
        assert_eq!(
            normalize_address("https://music.example/navidrome/rest/").unwrap(),
            "https://music.example/navidrome"
        );
        for bad in [
            "nas:4533",
            "ftp://nas",
            "http://",
            "http://u:p@nas",
            "http://nas/?a",
        ] {
            assert!(normalize_address(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn queries_carry_the_chosen_auth() {
        let c = Client::new("http://nas:4533", Credentials::password("anna", "p@ss")).unwrap();
        let url = c.url("ping", &[("id", "a b".into())]);
        assert!(url.starts_with("http://nas:4533/rest/ping.view?u=anna&t="));
        assert!(url.contains("&s=") && url.contains("&v=1.16.1&c=Kinolune&f=json"));
        assert!(url.ends_with("&id=a%20b"));
        assert!(!url.contains("p%40ss") && !url.contains("p@ss"));
        // The token matches the salt in the same URL.
        let q = |k: &str| {
            url.split(['?', '&'])
                .find_map(|p| p.strip_prefix(&format!("{k}=")))
                .unwrap()
                .to_owned()
        };
        assert_eq!(q("t"), salted_token("p@ss", &q("s")));
        // A new salt per request.
        assert_ne!(c.url("ping", &[]), c.url("ping", &[]));

        c.set_auth_mode(AuthMode::HexPassword);
        assert!(c.url("ping", &[]).contains("u=anna&p=enc:70407373&"));
        c.set_auth_mode(AuthMode::ApiKey);
        assert_eq!(c.auth_mode(), AuthMode::HexPassword);

        let k = Client::new("http://nas:4533", Credentials::api_key("k/1")).unwrap();
        assert_eq!(k.auth_mode(), AuthMode::ApiKey);
        let url = k.url("ping", &[]);
        assert!(url.contains("?apiKey=k%2F1&v=") && !url.contains("u="));
        assert!(!format!("{k:?}{:?}", k.credentials()).contains("k/1"));
    }

    #[test]
    fn reads_envelopes_and_errors() {
        let ok = br#"{"subsonic-response":{"status":"ok","version":"1.16.1","type":"navidrome"}}"#;
        assert_eq!(envelope(ok).unwrap()["type"], "navidrome");
        let fail = |code: i64| {
            let body = format!(
                r#"{{"subsonic-response":{{"status":"failed","version":"1.16.1","error":{{"code":{code},"message":"m"}}}}}}"#
            );
            envelope(body.as_bytes()).unwrap_err()
        };
        assert_eq!(fail(40), Error::Unauthorized);
        assert_eq!(fail(44), Error::Unauthorized);
        assert_eq!(fail(50), Error::Forbidden);
        assert_eq!(fail(70), Error::NotFound);
        assert!(matches!(fail(41), Error::Unsupported(_)));
        assert_eq!(
            fail(0),
            Error::Server {
                code: 0,
                message: "m".into()
            }
        );
        let xml = br#"<?xml version="1.0"?><subsonic-response status="failed"><error code="40" message="Wrong username or password"/></subsonic-response>"#;
        assert_eq!(envelope(xml).unwrap_err(), Error::Unauthorized);
        assert!(matches!(envelope(b"<html>"), Err(Error::Protocol(_))));
    }
}
