// SPDX-License-Identifier: MPL-2.0
//! The HTTP side shared by [`PlexTv`](crate::PlexTv) and
//! [`Server`](crate::Server): the `X-Plex-*` headers that name this app,
//! the token header, byte ranges and error mapping.

use std::fmt;
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::{Error, Result};

/// The plex.tv API.
pub const PLEX_TV: &str = "https://plex.tv";

/// Largest JSON answer read (a big library page stays far below it).
pub(crate) const MAX_JSON: u64 = 64 << 20;
/// Largest binary answer read at once (an image, a read-ahead block).
pub(crate) const MAX_BLOB: u64 = 64 << 20;

/// How this app introduces itself to plex.tv and the servers. Plex lists
/// "authorized devices" by `client_id`: keep it stable per sign-in, or
/// every start shows up as a new device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    /// Product name shown at plex.tv/link and in the device list.
    pub product: String,
    /// Product version.
    pub version: String,
    /// Stable id of this installation (`X-Plex-Client-Identifier`).
    pub client_id: String,
    /// Device name shown in the device list (`Linux`, `Pixel 8`).
    pub device: String,
    /// Platform (`Linux`, `Android`, …).
    pub platform: String,
}

impl ClientInfo {
    /// Kinolune on `device` with the id `client_id`, versioned as this
    /// crate; the platform is the OS this build runs on.
    pub fn kinolune(device: impl Into<String>, client_id: impl Into<String>) -> Self {
        Self {
            product: "Kinolune".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            client_id: client_id.into(),
            device: device.into(),
            platform: platform().to_owned(),
        }
    }

    /// The `X-Plex-*` headers, values cleaned to visible ASCII.
    pub(crate) fn headers(&self) -> [(&'static str, String); 6] {
        [
            ("x-plex-product", header_value(&self.product)),
            ("x-plex-version", header_value(&self.version)),
            ("x-plex-client-identifier", header_value(&self.client_id)),
            ("x-plex-device", header_value(&self.device)),
            ("x-plex-device-name", header_value(&self.device)),
            ("x-plex-platform", header_value(&self.platform)),
        ]
    }
}

fn platform() -> &'static str {
    match std::env::consts::OS {
        "linux" => "Linux",
        "android" => "Android",
        "macos" => "macOS",
        "ios" => "iOS",
        "windows" => "Windows",
        _ => "Other",
    }
}

/// A header value of visible ASCII (other characters become `_`).
fn header_value(text: &str) -> String {
    let v: String = text
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if v.is_empty() { "_".to_owned() } else { v }
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

/// `http(s)://host[:port][/base]` without a trailing slash; anything else
/// (credentials, query, other schemes) is refused.
///
/// # Errors
///
/// [`Error::Invalid`].
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

/// Percent-encodes `text` for a query parameter or a path segment.
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

/// HTTP method of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Post,
}

/// An answer: status, the headers the client needs, body bytes.
#[derive(Debug)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) content_range: Option<String>,
    pub(crate) body: Vec<u8>,
}

/// Why a request failed.
pub(crate) enum Fail {
    /// The address could not be reached: another connection may work.
    Reach(Error),
    /// The server answered, reading the answer failed.
    Read(Error),
}

impl Fail {
    pub(crate) fn into_error(self) -> Error {
        match self {
            Self::Reach(e) | Self::Read(e) => e,
        }
    }
}

/// The error kind of a transport failure, never the URL: a URL may hold
/// a token.
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

/// An HTTP agent that sends the `X-Plex-*` headers of `info`.
pub(crate) struct Http {
    agent: ureq::Agent,
    pub(crate) info: ClientInfo,
}

impl fmt::Debug for Http {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Http")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl Http {
    pub(crate) fn new(info: ClientInfo, connect: Duration, overall: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(connect))
            .timeout_global(Some(overall))
            .user_agent(format!(
                "{}/{}",
                header_value(&info.product),
                header_value(&info.version)
            ))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            info,
        }
    }

    /// Sends one request to `url` (which must not be logged: Plex
    /// accepts tokens in queries) with the token as `X-Plex-Token`.
    /// `range`: `(offset, len)` of a byte range.
    pub(crate) fn send(
        &self,
        method: Method,
        url: &str,
        token: Option<&Token>,
        range: Option<(u64, u64)>,
        limit: u64,
    ) -> std::result::Result<Response, Fail> {
        let range = range.map(|(off, len)| format!("bytes={off}-{}", off + len.max(1) - 1));
        let headers = self.info.headers();
        let result = match method {
            Method::Get => {
                let mut b = self.agent.get(url).header("accept", "application/json");
                for (k, v) in &headers {
                    b = b.header(*k, v.as_str());
                }
                if let Some(t) = token {
                    b = b.header("x-plex-token", t.expose());
                }
                if let Some(r) = &range {
                    b = b.header("range", r.as_str());
                }
                b.call()
            }
            Method::Post => {
                let mut b = self.agent.post(url).header("accept", "application/json");
                for (k, v) in &headers {
                    b = b.header(*k, v.as_str());
                }
                if let Some(t) = token {
                    b = b.header("x-plex-token", t.expose());
                }
                b.send_empty()
            }
        };
        let mut response = result.map_err(|e| Fail::Reach(transport(&e)))?;
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
            .map_err(|e| Fail::Read(transport(&e)))?;
        Ok(Response {
            status,
            content_range,
            body,
        })
    }
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
            normalize_address(" http://nas:32400/ ").unwrap(),
            "http://nas:32400"
        );
        for bad in [
            "nas:32400",
            "ftp://nas",
            "http://",
            "http://a:b@nas",
            "http://nas/?x=1",
        ] {
            assert!(normalize_address(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn headers_are_visible_ascii_and_token_hidden() {
        let info = ClientInfo::kinolune("Anna's\nТВ", "id-1");
        let h = info.headers();
        assert_eq!(h[0], ("x-plex-product", "Kinolune".to_owned()));
        assert_eq!(h[3].1, "Anna's___");
        assert_eq!(h[2].1, "id-1");
        assert_eq!(format!("{:?}", Token::new("abc")), "Token(***)");
        assert_eq!(header_value("  "), "_");
    }
}
