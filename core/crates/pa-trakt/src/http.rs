// SPDX-License-Identifier: MPL-2.0
//! The HTTP layer: a [`Transport`] sends one request; [`UreqTransport`]
//! is the real one, [`crate::testing::Recorded`] replays recorded
//! answers in tests.

use std::time::Duration;

use crate::{Error, Result};

/// Largest answer read (a big watched history stays far below it).
const MAX_BODY: u64 = 32 << 20;

/// HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Delete,
}

impl Method {
    /// Whether Trakt counts it as a write (`POST`, `PUT`, `DELETE`): one
    /// per second per user.
    pub fn is_write(self) -> bool {
        !matches!(self, Self::Get)
    }
}

/// One request. Headers may hold the token: never log a request.
#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// JSON body.
    pub body: Option<String>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

/// One answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    /// `Retry-After` in seconds, if sent.
    pub retry_after: Option<u64>,
    pub body: Vec<u8>,
}

/// Sends requests.
pub trait Transport: Send + Sync {
    /// Sends `request` and reads the whole answer, whatever its status.
    fn send(&self, request: &Request) -> Result<Response>;
}

/// The real transport: ureq with rustls.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    /// A transport that introduces itself as `user_agent`
    /// (`Kinolune/0.1.0`, as Trakt asks).
    pub fn new(user_agent: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(user_agent)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

/// The error kind of a transport failure, never the URL.
fn describe(e: &ureq::Error) -> Error {
    match e {
        ureq::Error::Timeout(_) => Error::TimedOut,
        ureq::Error::HostNotFound => Error::Unreachable("host not found".into()),
        ureq::Error::ConnectionFailed => Error::Unreachable("connection failed".into()),
        ureq::Error::Io(io) => Error::Unreachable(format!("I/O error: {}", io.kind())),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => Error::Unreachable("TLS error".into()),
        _ => Error::Unreachable("HTTP transport error".into()),
    }
}

impl Transport for UreqTransport {
    fn send(&self, request: &Request) -> Result<Response> {
        let result = match request.method {
            Method::Get | Method::Delete => {
                let mut b = if request.method == Method::Get {
                    self.agent.get(&request.url)
                } else {
                    self.agent.delete(&request.url)
                };
                for (k, v) in &request.headers {
                    b = b.header(k.as_str(), v.as_str());
                }
                b.call()
            }
            Method::Post => {
                let mut b = self.agent.post(&request.url);
                for (k, v) in &request.headers {
                    b = b.header(k.as_str(), v.as_str());
                }
                match &request.body {
                    Some(text) => b.send(text.as_str()),
                    None => b.send_empty(),
                }
            }
        };
        let mut response = result.map_err(|e| describe(&e))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok());
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_vec()
            .map_err(|e| describe(&e))?;
        Ok(Response {
            status,
            retry_after,
            body,
        })
    }
}
