// SPDX-License-Identifier: MPL-2.0
//! The HTTP side shared by the clouds: one blocking agent, requests built
//! from parts, statuses mapped to `pa-vfs` error kinds. Error texts name
//! what failed, never a URL (download links and queries carry tokens).

use std::time::Duration;

use pa_vfs::{ErrorKind, VfsError};

/// `User-Agent` of every request.
pub const USER_AGENT: &str = concat!("Kinolune/", env!("CARGO_PKG_VERSION"));

/// Largest JSON answer read (a listing page stays far below it).
pub(crate) const MAX_JSON: u64 = 32 << 20;
/// Largest binary answer read at once (a read-ahead block).
pub(crate) const MAX_BLOB: u64 = 64 << 20;

/// An answer: status, the `Content-Range` and `Retry-After` headers, body.
#[derive(Debug)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) content_range: Option<String>,
    pub(crate) retry_after: Option<u64>,
    pub(crate) body: Vec<u8>,
}

impl Response {
    pub(crate) fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Parses the body as JSON.
    pub(crate) fn json<T: serde::de::DeserializeOwned>(&self, what: &str) -> pa_vfs::Result<T> {
        serde_json::from_slice(&self.body).map_err(|e| {
            VfsError::new(ErrorKind::Other, format!("{what}: unexpected answer ({e})"))
        })
    }
}

/// A request: method, URL, headers, body.
#[derive(Debug)]
pub(crate) struct Request {
    pub(crate) method: &'static str,
    pub(crate) url: String,
    pub(crate) headers: Vec<(&'static str, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) limit: u64,
}

impl Request {
    pub(crate) fn get(url: impl Into<String>) -> Self {
        Self {
            method: "GET",
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
            limit: MAX_JSON,
        }
    }

    pub(crate) fn post(url: impl Into<String>) -> Self {
        Self {
            method: "POST",
            ..Self::get(url)
        }
    }

    pub(crate) fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    pub(crate) fn json(mut self, value: &serde_json::Value) -> Self {
        self.body = value.to_string().into_bytes();
        self.header("content-type", "application/json")
    }

    pub(crate) fn form(mut self, fields: &[(&str, &str)]) -> Self {
        self.body = form_encode(fields).into_bytes();
        self.header("content-type", "application/x-www-form-urlencoded")
    }

    /// Asks for `len` bytes from `offset` (`Range: bytes=a-b`).
    pub(crate) fn range(mut self, offset: u64, len: u64) -> Self {
        self.limit = MAX_BLOB.max(len + 1);
        let end = offset + len.max(1) - 1;
        self.header("range", format!("bytes={offset}-{end}"))
    }

    pub(crate) fn bearer(self, token: &str) -> Self {
        self.header("authorization", format!("Bearer {token}"))
    }
}

/// `application/x-www-form-urlencoded` of `fields`.
pub(crate) fn form_encode(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent-encodes `text` for a query or a form (RFC 3986 unreserved kept).
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

/// One blocking agent; cheap to share.
#[derive(Debug, Clone)]
pub(crate) struct Http {
    agent: ureq::Agent,
}

impl Default for Http {
    fn default() -> Self {
        Self::new(Duration::from_secs(10), Duration::from_secs(120))
    }
}

/// The error kind of a transport failure, never the URL.
fn describe(e: &ureq::Error) -> VfsError {
    let (kind, what) = match e {
        ureq::Error::Timeout(_) => (ErrorKind::TimedOut, "request timed out".to_owned()),
        ureq::Error::HostNotFound => (ErrorKind::Unavailable, "host not found".to_owned()),
        ureq::Error::ConnectionFailed => (ErrorKind::Unavailable, "connection failed".to_owned()),
        ureq::Error::Io(io) => (ErrorKind::Unavailable, format!("I/O error: {}", io.kind())),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => {
            (ErrorKind::Unavailable, "TLS error".to_owned())
        }
        ureq::Error::BodyExceedsLimit(_) => (ErrorKind::Other, "answer too large".to_owned()),
        _ => (ErrorKind::Unavailable, "HTTP transport error".to_owned()),
    };
    VfsError::new(kind, what)
}

impl Http {
    pub(crate) fn new(connect: Duration, overall: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(connect))
            .timeout_global(Some(overall))
            .user_agent(USER_AGENT)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    /// Sends `req` once.
    pub(crate) fn send(&self, req: Request) -> pa_vfs::Result<Response> {
        let mut b = ureq::http::Request::builder()
            .method(req.method)
            .uri(req.url.as_str());
        for (k, v) in &req.headers {
            b = b.header(*k, v.as_str());
        }
        let request = b
            .body(req.body)
            .map_err(|_| VfsError::new(ErrorKind::InvalidPath, "malformed request"))?;
        let mut response = self.agent.run(request).map_err(|e| describe(&e))?;
        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let content_range = header("content-range");
        let retry_after = header("retry-after").and_then(|v| v.trim().parse().ok());
        let body = response
            .body_mut()
            .with_config()
            .limit(req.limit)
            .read_to_vec()
            .map_err(|e| describe(&e))?;
        Ok(Response {
            status,
            content_range,
            retry_after,
            body,
        })
    }

    /// Sends a request built by `make`, again after a short pause when the
    /// service is busy (429, 5xx), at most `tries` times in all.
    pub(crate) fn send_retrying(
        &self,
        tries: u32,
        make: &dyn Fn() -> Request,
    ) -> pa_vfs::Result<Response> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let r = self.send(make());
            let wait = match &r {
                Ok(resp) if resp.status == 429 || resp.status >= 500 => {
                    Duration::from_secs(resp.retry_after.unwrap_or(1).min(5))
                }
                Err(e) if e.kind() == ErrorKind::Unavailable => Duration::from_millis(300),
                _ => return r,
            };
            if attempt >= tries {
                return r;
            }
            std::thread::sleep(wait * attempt);
        }
    }
}

/// The error of a failed answer of `what`.
pub(crate) fn status_error(what: &str, status: u16) -> VfsError {
    let kind = match status {
        401 => ErrorKind::AuthenticationFailed,
        403 => ErrorKind::PermissionDenied,
        404 | 410 => ErrorKind::NotFound,
        408 | 504 => ErrorKind::TimedOut,
        429 | 500..=599 => ErrorKind::Unavailable,
        _ => ErrorKind::Other,
    };
    VfsError::new(kind, format!("{what}: HTTP {status}"))
}

/// Bytes `[offset, offset + want)` of a range answer: a `206` holds them
/// (checked against `Content-Range`), a `200` the whole file (a server that
/// ignores ranges), a `416` nothing.
pub(crate) fn range_body(
    what: &str,
    r: Response,
    offset: u64,
    want: u64,
) -> pa_vfs::Result<Vec<u8>> {
    match r.status {
        206 => {
            if let Some(start) = r
                .content_range
                .as_deref()
                .and_then(|c| c.trim().strip_prefix("bytes "))
                .and_then(|c| c.split('-').next())
                .and_then(|s| s.trim().parse::<u64>().ok())
                && start != offset
            {
                return Err(VfsError::new(
                    ErrorKind::Other,
                    format!("{what}: server sent the wrong range"),
                ));
            }
            let mut body = r.body;
            body.truncate(usize::try_from(want).unwrap_or(usize::MAX));
            Ok(body)
        }
        200 => {
            let start = usize::try_from(offset).unwrap_or(usize::MAX);
            let body = r.body;
            if start >= body.len() {
                return Ok(Vec::new());
            }
            let end = start.saturating_add(usize::try_from(want).unwrap_or(usize::MAX));
            Ok(body[start..end.min(body.len())].to_vec())
        }
        416 => Ok(Vec::new()),
        s => Err(status_error(what, s)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_and_query_encoding() {
        assert_eq!(encode("a b/ц"), "a%20b%2F%D1%86");
        assert_eq!(
            form_encode(&[("code", "x&y"), ("redirect_uri", "http://127.0.0.1:1/")]),
            "code=x%26y&redirect_uri=http%3A%2F%2F127.0.0.1%3A1%2F"
        );
    }

    #[test]
    fn range_answers() {
        let r = |status, body: &[u8], cr: Option<&str>| Response {
            status,
            content_range: cr.map(str::to_owned),
            retry_after: None,
            body: body.to_vec(),
        };
        assert_eq!(
            range_body("t", r(206, b"cde", Some("bytes 2-4/10")), 2, 3).unwrap(),
            b"cde"
        );
        assert!(range_body("t", r(206, b"abc", Some("bytes 0-2/10")), 2, 3).is_err());
        assert_eq!(
            range_body("t", r(200, b"abcdef", None), 2, 3).unwrap(),
            b"cde"
        );
        assert_eq!(range_body("t", r(200, b"abcdef", None), 9, 3).unwrap(), b"");
        assert_eq!(range_body("t", r(416, b"", None), 9, 3).unwrap(), b"");
        assert_eq!(
            range_body("t", r(404, b"", None), 0, 3).unwrap_err().kind(),
            ErrorKind::NotFound
        );
    }
}
