// SPDX-License-Identifier: MPL-2.0
//! The HTTP seam: requests are built and answers read sans-IO; a
//! [`Transport`] sends them. Tests inject recorded answers.

use std::fmt::Write as _;

/// An HTTP request. `Debug` hides header values and the body: they carry
/// the session key, the token and the API signature.
#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    /// `GET` or `POST`.
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Content type and body of a `POST`.
    pub body: Option<(&'static str, String)>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let url = self.url.split('?').next().unwrap_or_default();
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &url)
            .finish_non_exhaustive()
    }
}

/// An HTTP answer; non-2xx statuses are answers, not errors.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Response {
    pub status: u16,
    /// Lowercase names.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Response {
    /// The first header `name` (lowercase).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// The network failed (no answer at all). Never carries the URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TransportError(pub String);

/// Sends requests; usable from several threads.
pub trait Transport: Send + Sync {
    fn send(&self, request: &Request) -> Result<Response, TransportError>;
}

/// `application/x-www-form-urlencoded` of `params`, in their order.
pub fn form(params: &[(String, String)]) -> String {
    let mut out = String::new();
    for (i, (k, v)) in params.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        encode(k, &mut out);
        out.push('=');
        encode(v, &mut out);
    }
    out
}

/// Percent-encodes everything except RFC 3986 unreserved characters.
pub fn encode(s: &str, out: &mut String) {
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b));
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
}

#[cfg(feature = "http")]
pub use live::UreqTransport;

#[cfg(feature = "http")]
mod live {
    use std::time::Duration;

    use super::{Request, Response, Transport, TransportError};

    /// Answers of both services are small.
    const MAX_BODY: u64 = 1024 * 1024;

    /// Blocking HTTPS on `ureq` with rustls, like `pa-metadata`.
    #[derive(Debug, Clone)]
    pub struct UreqTransport {
        agent: ureq::Agent,
    }

    impl UreqTransport {
        /// A transport with [`crate::user_agent`] and a 15 s timeout.
        pub fn new() -> Self {
            let config = ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(15)))
                .user_agent(crate::user_agent())
                .build();
            Self {
                agent: ureq::Agent::new_with_config(config),
            }
        }
    }

    impl Default for UreqTransport {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Transport for UreqTransport {
        fn send(&self, request: &Request) -> Result<Response, TransportError> {
            let response = if let Some((content_type, body)) = &request.body {
                let mut b = self
                    .agent
                    .post(&request.url)
                    .header("content-type", *content_type);
                for (k, v) in &request.headers {
                    b = b.header(k.as_str(), v.as_str());
                }
                b.send(body.as_str())
            } else {
                let mut b = self.agent.get(&request.url);
                for (k, v) in &request.headers {
                    b = b.header(k.as_str(), v.as_str());
                }
                b.call()
            };
            // ureq errors may quote the URL (an API key): only the kind.
            let mut response = response.map_err(|e| TransportError(describe(&e)))?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .filter_map(|(k, v)| {
                    Some((k.as_str().to_ascii_lowercase(), v.to_str().ok()?.to_owned()))
                })
                .collect();
            let body = response
                .body_mut()
                .with_config()
                .limit(MAX_BODY)
                .read_to_string()
                .map_err(|e| TransportError(describe(&e)))?;
            Ok(Response {
                status,
                headers,
                body,
            })
        }
    }

    fn describe(e: &ureq::Error) -> String {
        match e {
            ureq::Error::Timeout(_) => "request timed out".to_owned(),
            ureq::Error::HostNotFound => "host not found".to_owned(),
            ureq::Error::ConnectionFailed => "connection failed".to_owned(),
            ureq::Error::Io(io) => format!("I/O error: {}", io.kind()),
            ureq::Error::Tls(_) | ureq::Error::Rustls(_) => "TLS error".to_owned(),
            _ => "HTTP transport error".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_encoding_and_debug_hides_secrets() {
        let f = form(&[
            ("artist[0]".into(), "Sigur Rós".into()),
            ("track[0]".into(), "a&b=c".into()),
        ]);
        assert_eq!(f, "artist%5B0%5D=Sigur%20R%C3%B3s&track%5B0%5D=a%26b%3Dc");
        let r = Request {
            method: "POST",
            url: "https://x.test/2.0/?api_key=secret".into(),
            headers: vec![("authorization".into(), "Token secret".into())],
            body: Some(("text/plain", "sk=secret".into())),
        };
        assert!(!format!("{r:?}").contains("secret"));
    }
}
