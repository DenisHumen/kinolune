// SPDX-License-Identifier: MPL-2.0
//! The HTTP client of links: one `ureq` agent bound to a [`LinkPolicy`].

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use pa_vfs::{ErrorKind, VfsError};
use ureq::ResponseExt;
use url::Url;

use crate::file::HttpFile;
use crate::policy::{Blocked, LinkPolicy, PolicyResolver};

/// User agent of every request.
pub const USER_AGENT: &str = concat!("Kinolune/", env!("CARGO_PKG_VERSION"));

/// What a `HEAD` (or a one-byte `GET`) said about a link.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Probe {
    /// Size of the resource; `None` for a stream without an end (a live
    /// channel) or a server that does not say.
    pub size: Option<u64>,
    /// `Content-Type` without parameters, lowercase.
    pub content_type: Option<String>,
}

/// Timeouts of an [`HttpClient`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// Name lookup and TCP/TLS connect.
    pub connect: Duration,
    /// Waiting for the response headers, and for each read of a body.
    pub read: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            read: Duration::from_secs(30),
        }
    }
}

/// A blocking HTTP client whose connections can only reach what its
/// [`LinkPolicy`] allows (checked on the resolved addresses of the first
/// request and of every redirect). Error messages name the host, never the
/// URL: links often carry tokens in their query.
#[derive(Debug, Clone)]
pub struct HttpClient {
    agent: ureq::Agent,
    policy: LinkPolicy,
}

/// Only `http` and `https` links, with a host and without credentials.
///
/// # Errors
///
/// [`ErrorKind::InvalidPath`] otherwise.
pub fn check_link(url: &Url) -> pa_vfs::Result<()> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            format!(
                "only http and https links are played, not {}:",
                url.scheme()
            ),
        ));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            "the link has no host",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(VfsError::new(
            ErrorKind::InvalidPath,
            "links with a user name or password are not played",
        ));
    }
    Ok(())
}

/// The host of a link for messages.
pub(crate) fn host(url: &Url) -> String {
    url.host_str().unwrap_or("?").to_owned()
}

impl HttpClient {
    /// A client for links under `policy` with default timeouts.
    #[must_use]
    pub fn new(policy: LinkPolicy) -> Self {
        Self::with_timeouts(policy, Timeouts::default())
    }

    /// A client for links under `policy`.
    #[must_use]
    pub fn with_timeouts(policy: LinkPolicy, t: Timeouts) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            // A system proxy would be resolved instead of the link.
            .proxy(None)
            .max_redirects(5)
            .user_agent(USER_AGENT)
            .timeout_resolve(Some(t.connect))
            .timeout_connect(Some(t.connect))
            .timeout_recv_response(Some(t.read))
            .timeout_per_read(Some(t.read))
            .build();
        let agent = ureq::Agent::with_parts(
            config,
            ureq::unversioned::transport::DefaultConnector::new(),
            PolicyResolver::new(policy),
        );
        Self { agent, policy }
    }

    /// The policy of this client.
    #[must_use]
    pub fn policy(&self) -> LinkPolicy {
        self.policy
    }

    pub(crate) fn agent(&self) -> &ureq::Agent {
        &self.agent
    }

    /// Size and type of `url`: a `HEAD`, then a one-byte ranged `GET` for
    /// servers that do not answer `HEAD` or do not give a length.
    ///
    /// # Errors
    ///
    /// The link is refused ([`check_link`], the policy), the server cannot
    /// be reached or answers with an error status.
    pub fn probe(&self, url: &Url) -> pa_vfs::Result<Probe> {
        check_link(url)?;
        let head = self
            .agent
            .head(url.as_str())
            .header("accept-encoding", "identity")
            .call()
            .map_err(|e| transport(url, &e))?;
        let status = head.status().as_u16();
        let head_type = content_type(&head);
        if (200..300).contains(&status) {
            if let Some(size) = header_u64(&head, "content-length").filter(|&n| n > 0) {
                return Ok(Probe {
                    size: Some(size),
                    content_type: head_type,
                });
            }
        } else if status != 405 && status != 501 && status != 400 && status != 403 {
            return Err(status_error(url, status));
        }
        // No HEAD or no length: ask for the first byte.
        let resp = self
            .agent
            .get(url.as_str())
            .header("range", "bytes=0-0")
            .header("accept-encoding", "identity")
            .call()
            .map_err(|e| transport(url, &e))?;
        let status = resp.status().as_u16();
        let content_type = content_type(&resp).or(head_type);
        let size = match status {
            206 => resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(total_of_content_range),
            200 => header_u64(&resp, "content-length"),
            s => return Err(status_error(url, s)),
        };
        // Do not download a whole file (or a live stream) to probe it.
        drop(resp);
        Ok(Probe { size, content_type })
    }

    /// Downloads a small text resource (a playlist, a device description):
    /// at most `limit` bytes. Returns the text (invalid UTF-8 replaced, BOM
    /// removed) and the URL it was finally read from (after redirects), the
    /// base of its relative links.
    ///
    /// # Errors
    ///
    /// As [`probe`](Self::probe); [`ErrorKind::Unsupported`] for a body
    /// over `limit`.
    pub fn get_text(&self, url: &Url, limit: u64) -> pa_vfs::Result<(String, Url)> {
        check_link(url)?;
        let resp = self
            .agent
            .get(url.as_str())
            .call()
            .map_err(|e| transport(url, &e))?;
        self.text_of(url, resp, limit)
    }

    /// `POST`s an XML body (a SOAP call) and returns the answer text. A
    /// SOAP fault comes back as `Ok` with its status: UPnP errors are
    /// HTTP 500 with a body.
    ///
    /// # Errors
    ///
    /// As [`get_text`](Self::get_text), except HTTP 500.
    pub fn post_xml(
        &self,
        url: &Url,
        soap_action: &str,
        body: &str,
        limit: u64,
    ) -> pa_vfs::Result<(u16, String)> {
        check_link(url)?;
        let resp = self
            .agent
            .post(url.as_str())
            .header("content-type", "text/xml; charset=\"utf-8\"")
            .header("soapaction", soap_action)
            .send(body)
            .map_err(|e| transport(url, &e))?;
        let status = resp.status().as_u16();
        if status == 500 {
            let mut resp = resp;
            let text = read_limited(url, resp.body_mut().as_reader(), limit)?;
            return Ok((status, text));
        }
        self.text_of(url, resp, limit).map(|(t, _)| (status, t))
    }

    fn text_of(
        &self,
        url: &Url,
        mut resp: ureq::http::Response<ureq::Body>,
        limit: u64,
    ) -> pa_vfs::Result<(String, Url)> {
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(status_error(url, status));
        }
        let fin = Url::parse(&resp.get_uri().to_string()).unwrap_or_else(|_| url.clone());
        let text = read_limited(url, resp.body_mut().as_reader(), limit)?;
        Ok((text, fin))
    }

    /// Opens `url` as a [`HttpFile`] of `size` bytes (from [`probe`](Self::probe)
    /// or a server listing). Nothing is requested until the first read.
    ///
    /// # Errors
    ///
    /// [`check_link`].
    pub fn open(self: &Arc<Self>, url: Url, size: u64) -> pa_vfs::Result<HttpFile> {
        check_link(&url)?;
        Ok(HttpFile::new(Arc::clone(self), url, size))
    }
}

fn read_limited(url: &Url, reader: impl Read, limit: u64) -> pa_vfs::Result<String> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| VfsError::from_io(e, host(url)))?;
    if bytes.len() as u64 > limit {
        return Err(VfsError::new(
            ErrorKind::Unsupported,
            format!("{}: the answer is larger than {limit} bytes", host(url)),
        ));
    }
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned())
}

fn content_type(resp: &ureq::http::Response<ureq::Body>) -> Option<String> {
    let v = resp.headers().get("content-type")?.to_str().ok()?;
    let v = v.split(';').next()?.trim().to_ascii_lowercase();
    (!v.is_empty()).then_some(v)
}

fn header_u64(resp: &ureq::http::Response<ureq::Body>, name: &str) -> Option<u64> {
    resp.headers().get(name)?.to_str().ok()?.trim().parse().ok()
}

/// `bytes 0-0/1234` → `1234`; `None` for `*` (unknown length).
pub(crate) fn total_of_content_range(v: &str) -> Option<u64> {
    v.trim().rsplit_once('/')?.1.trim().parse().ok()
}

/// `bytes 100-199/1234` → `100`.
pub(crate) fn start_of_content_range(v: &str) -> Option<u64> {
    let v = v.trim().strip_prefix("bytes")?.trim();
    v.split_once('-')?.0.trim().parse().ok()
}

/// An HTTP status as a `pa-vfs` error.
pub(crate) fn status_error(url: &Url, status: u16) -> VfsError {
    let kind = match status {
        401 => ErrorKind::AuthenticationFailed,
        403 => ErrorKind::PermissionDenied,
        404 | 410 => ErrorKind::NotFound,
        408 | 504 => ErrorKind::TimedOut,
        416 => ErrorKind::InvalidPath,
        502 | 503 => ErrorKind::Unavailable,
        _ => ErrorKind::Other,
    };
    VfsError::new(kind, format!("{}: HTTP {status}", host(url)))
}

/// A transport error as a `pa-vfs` error, without the URL.
pub(crate) fn transport(url: &Url, e: &ureq::Error) -> VfsError {
    let h = host(url);
    match e {
        ureq::Error::Other(inner) if inner.downcast_ref::<Blocked>().is_some() => VfsError::new(
            ErrorKind::PermissionDenied,
            format!("{h}: blocked: {Blocked}"),
        ),
        ureq::Error::Timeout(_) => VfsError::new(ErrorKind::TimedOut, format!("{h}: timed out")),
        ureq::Error::HostNotFound => {
            VfsError::new(ErrorKind::Unavailable, format!("{h}: host not found"))
        }
        ureq::Error::ConnectionFailed => {
            VfsError::new(ErrorKind::Unavailable, format!("{h}: connection failed"))
        }
        ureq::Error::Io(io) => VfsError::new(
            match io.kind() {
                std::io::ErrorKind::TimedOut => ErrorKind::TimedOut,
                _ => ErrorKind::Unavailable,
            },
            format!("{h}: I/O error: {}", io.kind()),
        ),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => {
            VfsError::new(ErrorKind::Unavailable, format!("{h}: TLS error"))
        }
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => {
            VfsError::new(ErrorKind::Other, format!("{h}: too many or bad redirects"))
        }
        ureq::Error::BadUri(_) => VfsError::new(ErrorKind::InvalidPath, format!("{h}: bad link")),
        _ => VfsError::new(ErrorKind::Other, format!("{h}: HTTP error")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_links() {
        let ok = Url::parse("https://cdn.example/a.mkv?token=1").unwrap();
        check_link(&ok).unwrap();
        for bad in [
            "file:///etc/passwd",
            "ftp://h/a.mkv",
            "rtsp://h/live",
            "http://user:pw@h/a.mkv",
            "http://user@h/a.mkv",
        ] {
            let u = Url::parse(bad).unwrap();
            let e = check_link(&u).unwrap_err();
            assert_eq!(e.kind(), ErrorKind::InvalidPath, "{bad}");
            assert!(!e.to_string().contains("pw"), "{e}");
        }
    }

    #[test]
    fn parses_content_range() {
        assert_eq!(total_of_content_range("bytes 0-0/1234"), Some(1234));
        assert_eq!(total_of_content_range("bytes 0-0/*"), None);
        assert_eq!(start_of_content_range("bytes 100-199/1234"), Some(100));
        assert_eq!(start_of_content_range("items 1-2/3"), None);
    }

    #[test]
    fn errors_never_carry_the_url() {
        let u = Url::parse("http://h.example/a.mkv?token=secret").unwrap();
        let e = status_error(&u, 404);
        assert_eq!(e.kind(), ErrorKind::NotFound);
        assert!(!e.to_string().contains("secret"));
        let e = transport(&u, &ureq::Error::Other(Box::new(Blocked)));
        assert_eq!(e.kind(), ErrorKind::PermissionDenied);
        assert!(!e.to_string().contains("secret"));
    }
}
