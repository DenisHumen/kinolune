// SPDX-License-Identifier: MPL-2.0
//! Proxies of this process, so a URL can be opened without the socket.
//!
//! A player that lives in the same process as the core (Media3 through
//! UniFFI, PLAY-AND-003) gets the same URL as any other player. Instead of
//! fetching it over HTTP it asks [`open_local`]: when the URL belongs to a
//! running [`Proxy`](crate::Proxy) of this process, the proxy's opener
//! returns the [`ByteSource`] directly. Anything else (another process,
//! a stopped proxy, a foreign URL) is `None`, and the player falls back to
//! HTTP.

use std::sync::{Arc, Mutex, MutexGuard, Weak};

use pa_vfs::{ByteSource, ErrorKind, VfsError};

use crate::{Shared, decode_path};

struct Entry {
    port: u16,
    shared: Weak<Shared>,
}

static PROXIES: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

fn proxies() -> MutexGuard<'static, Vec<Entry>> {
    PROXIES.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn register(shared: &Arc<Shared>, port: u16) {
    let mut all = proxies();
    all.retain(|e| e.shared.strong_count() > 0);
    all.push(Entry {
        port,
        shared: Arc::downgrade(shared),
    });
}

pub(crate) fn unregister(shared: &Arc<Shared>) {
    let target = Arc::downgrade(shared);
    proxies().retain(|e| e.shared.strong_count() > 0 && !Weak::ptr_eq(&e.shared, &target));
}

/// Opens what a URL of a proxy running in this process serves, without
/// HTTP.
///
/// - `None`: the URL is not served by a live proxy of this process (wrong
///   scheme or host, unknown port, wrong token). Fetch it over HTTP.
/// - `Some(Err(_))`: the URL is ours but naming nothing (empty or broken
///   path: [`ErrorKind::InvalidPath`]) or the opener failed, with the same
///   error kinds the proxy turns into `4xx`/`5xx`.
///
/// Query and fragment are ignored, as by the server. The token is a secret
/// and is never part of an error message.
#[must_use]
pub fn open_local(url: &str) -> Option<pa_vfs::Result<Arc<dyn ByteSource>>> {
    let (port, path) = split_url(url)?;
    let path = path.split(['?', '#']).next().unwrap_or("");
    let (token, resource) = match path.split_once('/') {
        Some((t, r)) => (t, r),
        None => (path, ""),
    };
    let shared = proxies()
        .iter()
        .filter(|e| e.port == port)
        .filter_map(|e| e.shared.upgrade())
        .find(|s| s.token == token && !s.stop.load(std::sync::atomic::Ordering::Relaxed))?;
    if resource.is_empty() {
        return Some(Err(VfsError::new(
            ErrorKind::InvalidPath,
            "proxy URL without a resource",
        )));
    }
    let Some(resource) = decode_path(resource) else {
        return Some(Err(VfsError::new(
            ErrorKind::InvalidPath,
            "proxy URL with a broken percent escape",
        )));
    };
    Some(shared.opener.open(&resource))
}

/// `http://127.0.0.1:<port>/<rest>` or `http://localhost:<port>/<rest>`:
/// the port and `<rest>` (after the slash).
fn split_url(url: &str) -> Option<(u16, &str)> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = rest.split_once('/')?;
    let (host, port) = authority.rsplit_once(':')?;
    if host != "127.0.0.1" && !host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    Some((port.parse().ok()?, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_loopback_urls_only() {
        assert_eq!(split_url("http://127.0.0.1:8080/t/a"), Some((8080, "t/a")));
        assert_eq!(split_url("http://LOCALHOST:1/t"), Some((1, "t")));
        assert_eq!(split_url("https://127.0.0.1:8080/t/a"), None);
        assert_eq!(split_url("http://192.0.2.2:8080/t/a"), None);
        assert_eq!(split_url("http://127.0.0.1/t/a"), None);
        assert_eq!(split_url("http://127.0.0.1:x/t/a"), None);
        assert_eq!(split_url("http://127.0.0.1:80"), None);
        assert_eq!(split_url("file:///tmp/a.mkv"), None);
    }
}
