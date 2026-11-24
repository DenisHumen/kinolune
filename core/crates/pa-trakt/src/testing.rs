// SPDX-License-Identifier: MPL-2.0
//! Recorded answers for tests: no live network.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use crate::http::{Method, Request, Response, Transport};
use crate::{Error, Result};

/// One expected request and its recorded answer.
#[derive(Debug, Clone)]
pub struct Exchange {
    pub method: Method,
    /// The URL path (`/sync/watched/movies`).
    pub path: String,
    pub status: u16,
    pub body: String,
    pub retry_after: Option<u64>,
}

impl Exchange {
    /// `method path` answered `status` with `body`.
    pub fn new(method: Method, path: &str, status: u16, body: &str) -> Self {
        Self {
            method,
            path: path.into(),
            status,
            body: body.into(),
            retry_after: None,
        }
    }

    /// The same answer with `Retry-After`.
    #[must_use]
    pub fn retry_after(mut self, secs: u64) -> Self {
        self.retry_after = Some(secs);
        self
    }
}

/// A transport that answers the expected requests in order and keeps what
/// was sent. An unexpected request fails with [`Error::Unreachable`].
#[derive(Debug, Default)]
pub struct Recorded {
    queue: Mutex<VecDeque<Exchange>>,
    sent: Mutex<Vec<Request>>,
}

impl Recorded {
    /// A transport for `exchanges`, in order.
    pub fn new(exchanges: Vec<Exchange>) -> Self {
        Self {
            queue: Mutex::new(exchanges.into()),
            sent: Mutex::new(Vec::new()),
        }
    }

    /// Requests sent so far.
    pub fn sent(&self) -> Vec<Request> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether every expected request came.
    pub fn is_done(&self) -> bool {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }
}

/// The path of `url` without scheme, host and query.
fn path_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let path = rest.find('/').map_or("", |i| &rest[i..]);
    path.split('?').next().unwrap_or("")
}

impl Transport for Recorded {
    fn send(&self, request: &Request) -> Result<Response> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        let next = queue.pop_front();
        match next {
            Some(e) if e.method == request.method && e.path == path_of(&request.url) => {
                Ok(Response {
                    status: e.status,
                    retry_after: e.retry_after,
                    body: e.body.into_bytes(),
                })
            }
            other => Err(Error::Unreachable(format!(
                "unexpected {:?} {}, expected {:?}",
                request.method,
                path_of(&request.url),
                other.map(|e| (e.method, e.path))
            ))),
        }
    }
}
