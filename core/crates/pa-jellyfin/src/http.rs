// SPDX-License-Identifier: MPL-2.0
//! The HTTP side of [`Client`]: the `Authorization: MediaBrowser …` header,
//! failover between the server's addresses and error mapping.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{PoisonError, RwLock};
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::{Error, Result};

/// Largest JSON answer read (a big library page stays far below it).
const MAX_JSON: u64 = 64 << 20;
/// Largest binary answer read at once (an image, a trickplay sheet, a
/// read-ahead block of a media file).
pub(crate) const MAX_BLOB: u64 = 64 << 20;

/// How this app introduces itself to the server. Jellyfin keys sessions
/// and access tokens by `device_id`: keep it stable per installation and
/// server, or every sign-in shows up as a new device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Client name shown in the server dashboard (`Kinolune`).
    pub client: String,
    /// Device name shown in the dashboard (`Linux`, `Pixel 8`).
    pub device: String,
    /// Stable device id.
    pub device_id: String,
    /// Client version.
    pub version: String,
}

impl DeviceInfo {
    /// Kinolune on `device` with the id `device_id`, versioned as this crate.
    pub fn kinolune(device: impl Into<String>, device_id: impl Into<String>) -> Self {
        Self {
            client: "Kinolune".to_owned(),
            device: device.into(),
            device_id: device_id.into(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

/// An access token. `Debug` does not print it.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// Wraps a token string.
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// The token itself; do not log it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(***)")
    }
}

/// HTTP method of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Post,
    Delete,
}

/// An answer: status, the headers the client needs, body bytes.
#[derive(Debug)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) content_range: Option<String>,
    pub(crate) body: Vec<u8>,
}

/// A Jellyfin server reached at one of several addresses (a LAN address
/// and a public one, say): a request goes to the address that answered
/// last; if that one cannot be reached, the others are tried in order and
/// the first one that answers becomes the current one.
///
/// Methods block; call them off the UI thread. A `Client` is `Send + Sync`
/// and cheap to share in an `Arc`.
pub struct Client {
    agent: ureq::Agent,
    addresses: Vec<String>,
    active: AtomicUsize,
    device: DeviceInfo,
    token: RwLock<Option<Token>>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("addresses", &self.addresses)
            .field("active", &self.active.load(Ordering::Relaxed))
            .field("device", &self.device)
            .field("signed_in", &self.token().is_some())
            .finish_non_exhaustive()
    }
}

/// `http(s)://host[:port][/base]` without a trailing slash; anything else
/// (credentials, query, other schemes) is refused.
pub fn normalize_address(address: &str) -> Result<String> {
    let a = address.trim().trim_end_matches('/');
    let rest = a
        .strip_prefix("http://")
        .or_else(|| a.strip_prefix("https://"))
        .ok_or_else(|| Error::Invalid(format!("not an http(s) address: {a:?}")))?;
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() {
        return Err(Error::Invalid(format!("no host in {a:?}")));
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

/// Percent-encodes `text` for a header value or a query parameter.
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

/// The error kind of a transport failure, never the URL: a URL may hold
/// an `api_key`.
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

fn is_timeout(e: &ureq::Error) -> bool {
    matches!(e, ureq::Error::Timeout(_))
}

impl Client {
    /// A client for the server at `addresses` (the first is tried first),
    /// not signed in. Each address is checked with [`normalize_address`].
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if there is no address or one is malformed.
    pub fn new(addresses: &[String], device: DeviceInfo) -> Result<Self> {
        Self::with_timeouts(
            addresses,
            device,
            Duration::from_secs(5),
            Duration::from_secs(60),
        )
    }

    /// Like [`new`](Self::new) with a connect timeout (how long one
    /// address is tried before the next) and an overall request timeout.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_timeouts(
        addresses: &[String],
        device: DeviceInfo,
        connect: Duration,
        overall: Duration,
    ) -> Result<Self> {
        let mut list = Vec::new();
        for a in addresses {
            let a = normalize_address(a)?;
            if !list.contains(&a) {
                list.push(a);
            }
        }
        if list.is_empty() {
            return Err(Error::Invalid("no server address".into()));
        }
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(connect))
            .timeout_global(Some(overall))
            .user_agent(format!("{}/{}", device.client, device.version))
            .build();
        Ok(Self {
            agent: ureq::Agent::new_with_config(config),
            addresses: list,
            active: AtomicUsize::new(0),
            device,
            token: RwLock::new(None),
        })
    }

    /// The addresses, in the order they are tried.
    pub fn addresses(&self) -> &[String] {
        &self.addresses
    }

    /// The address that answered last (the first one before any request).
    pub fn active_address(&self) -> &str {
        let i = self.active.load(Ordering::Relaxed);
        &self.addresses[i.min(self.addresses.len() - 1)]
    }

    /// Who this client says it is.
    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    /// The access token in use, if signed in.
    pub fn token(&self) -> Option<Token> {
        self.token
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Uses `token` for the following requests (a stored sign-in), or
    /// none.
    pub fn set_token(&self, token: Option<Token>) {
        *self.token.write().unwrap_or_else(PoisonError::into_inner) = token;
    }

    /// The `Authorization` header value.
    fn authorization(&self) -> String {
        let d = &self.device;
        let mut v = format!(
            "MediaBrowser Client=\"{}\", Device=\"{}\", DeviceId=\"{}\", Version=\"{}\"",
            encode(&d.client),
            encode(&d.device),
            encode(&d.device_id),
            encode(&d.version)
        );
        if let Some(t) = self.token() {
            v.push_str(&format!(", Token=\"{}\"", t.expose()));
        }
        v
    }

    /// Absolute URL of `path_query` (`/Items?…`) on the current address,
    /// with the token as `api_key` for clients that cannot send headers
    /// (a player opening an HLS stream). Contains a secret: do not log it.
    pub fn url_with_key(&self, path_query: &str) -> String {
        let mut url = format!("{}{path_query}", self.active_address());
        if let Some(t) = self.token() {
            let lower = url.to_ascii_lowercase();
            if !lower.contains("api_key=") && !lower.contains("apikey=") {
                url.push(if url.contains('?') { '&' } else { '?' });
                url.push_str("api_key=");
                url.push_str(&encode(t.expose()));
            }
        }
        url
    }

    /// Sends one request, trying the other addresses when the current one
    /// cannot be reached. `range`: `(offset, len)` of a byte range.
    pub(crate) fn send(
        &self,
        method: Method,
        path_query: &str,
        json: Option<&serde_json::Value>,
        range: Option<(u64, u64)>,
        limit: u64,
    ) -> Result<Response> {
        let n = self.addresses.len();
        let start = self.active.load(Ordering::Relaxed) % n;
        let body = json.map(serde_json::Value::to_string);
        let mut last = None;
        for k in 0..n {
            let i = (start + k) % n;
            let url = format!("{}{path_query}", self.addresses[i]);
            match self.send_to(method, &url, body.as_deref(), range, limit) {
                Ok(r) => {
                    self.active.store(i, Ordering::Relaxed);
                    return Ok(r);
                }
                // The server answered but the body broke: not an address
                // problem.
                Err(Fail::Read(e)) => return Err(e),
                Err(Fail::Reach(e)) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| Error::Unreachable("no address".into())))
    }

    fn send_to(
        &self,
        method: Method,
        url: &str,
        body: Option<&str>,
        range: Option<(u64, u64)>,
        limit: u64,
    ) -> std::result::Result<Response, Fail> {
        let auth = self.authorization();
        let range = range.map(|(off, len)| format!("bytes={off}-{}", off + len.max(1) - 1));
        let reach = |e: ureq::Error| {
            Fail::Reach(if is_timeout(&e) {
                Error::TimedOut
            } else {
                Error::Unreachable(describe(&e))
            })
        };
        let result = match method {
            Method::Get | Method::Delete => {
                let mut b = if method == Method::Get {
                    self.agent.get(url)
                } else {
                    self.agent.delete(url)
                };
                b = b
                    .header("authorization", auth.as_str())
                    .header("accept", "application/json");
                if let Some(r) = &range {
                    b = b.header("range", r.as_str());
                }
                b.call()
            }
            Method::Post => {
                let b = self
                    .agent
                    .post(url)
                    .header("authorization", auth.as_str())
                    .header("accept", "application/json")
                    .header("content-type", "application/json");
                match body {
                    Some(text) => b.send(text),
                    None => b.send_empty(),
                }
            }
        };
        let mut response = result.map_err(reach)?;
        let status = response.status().as_u16();
        let content_range = response
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = response
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(|e| {
                Fail::Read(if is_timeout(&e) {
                    Error::TimedOut
                } else {
                    Error::Unreachable(describe(&e))
                })
            })?;
        Ok(Response {
            status,
            content_range,
            body,
        })
    }

    /// Sends a request and fails on a non-2xx status.
    pub(crate) fn call(
        &self,
        method: Method,
        path_query: &str,
        json: Option<&serde_json::Value>,
    ) -> Result<Response> {
        let r = self.send(method, path_query, json, None, MAX_JSON)?;
        check(r)
    }

    /// GET and parse a JSON answer.
    pub(crate) fn get_json<T: DeserializeOwned>(&self, path_query: &str) -> Result<T> {
        parse(&self.call(Method::Get, path_query, None)?.body)
    }

    /// POST `body` and parse a JSON answer.
    pub(crate) fn post_json<T: DeserializeOwned>(
        &self,
        path_query: &str,
        body: &serde_json::Value,
    ) -> Result<T> {
        parse(&self.call(Method::Post, path_query, Some(body))?.body)
    }

    /// GET a whole binary answer (an image, a trickplay sheet).
    pub(crate) fn get_bytes(&self, path_query: &str) -> Result<Vec<u8>> {
        Ok(check(self.send(Method::Get, path_query, None, None, MAX_BLOB)?)?.body)
    }
}

enum Fail {
    /// The address could not be reached: try the next one.
    Reach(Error),
    /// The server answered, reading the answer failed.
    Read(Error),
}

/// Maps a non-2xx status to an [`Error`].
pub(crate) fn check(r: Response) -> Result<Response> {
    match r.status {
        200..=299 => Ok(r),
        401 => Err(Error::Unauthorized),
        403 => Err(Error::Forbidden),
        404 => Err(Error::NotFound),
        status => {
            let text = String::from_utf8_lossy(&r.body);
            let message: String = text.chars().take(200).collect();
            Err(Error::Status { status, message })
        }
    }
}

pub(crate) fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<T> {
    serde_json::from_slice(body).map_err(|e| Error::Protocol(format!("bad JSON: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_addresses() {
        assert_eq!(
            normalize_address(" http://nas:8096/ ").unwrap(),
            "http://nas:8096"
        );
        assert_eq!(
            normalize_address("https://media.example/jellyfin/").unwrap(),
            "https://media.example/jellyfin"
        );
        for bad in [
            "nas:8096",
            "ftp://nas",
            "http://",
            "http://a:b@nas",
            "http://nas/?x=1",
        ] {
            assert!(normalize_address(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn authorization_header_escapes_and_hides_token() {
        let c = Client::new(
            &["http://nas:8096".into()],
            DeviceInfo::kinolune("Anna's \"TV\"", "dev-1"),
        )
        .unwrap();
        assert!(!c.authorization().contains("Token"));
        c.set_token(Some(Token::new("abc")));
        let h = c.authorization();
        assert!(h.starts_with("MediaBrowser Client=\"Kinolune\", Device=\"Anna%27s%20%22TV%22\""));
        assert!(h.ends_with("Token=\"abc\""));
        assert!(!format!("{c:?}").contains("abc"));
        assert_eq!(format!("{:?}", Token::new("abc")), "Token(***)");
    }

    #[test]
    fn url_with_key_appends_once() {
        let c = Client::new(&["http://nas:8096".into()], DeviceInfo::kinolune("d", "i")).unwrap();
        c.set_token(Some(Token::new("t0k")));
        assert_eq!(
            c.url_with_key("/a?x=1"),
            "http://nas:8096/a?x=1&api_key=t0k"
        );
        assert_eq!(c.url_with_key("/a"), "http://nas:8096/a?api_key=t0k");
        assert_eq!(c.url_with_key("/a?ApiKey=z"), "http://nas:8096/a?ApiKey=z");
    }

    #[test]
    fn fails_over_to_the_next_address() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let good = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for stream in listener.incoming().take(2) {
                let mut s = stream.unwrap();
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let body = r#"{"ok":true}"#;
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        // Port 9 (discard) on loopback is closed: connection refused.
        let c = Client::new(
            &["http://127.0.0.1:9".into(), good.clone()],
            DeviceInfo::kinolune("d", "i"),
        )
        .unwrap();
        let v: serde_json::Value = c.get_json("/x").unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(c.active_address(), good);
        // The working address is used first from now on.
        let _: serde_json::Value = c.get_json("/x").unwrap();
        assert_eq!(c.active_address(), good);
    }
}
