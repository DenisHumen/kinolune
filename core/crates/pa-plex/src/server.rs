// SPDX-License-Identifier: MPL-2.0
//! A Plex Media Server reached over one of its connections: which
//! connection to use, and the library API (sections, items, playlists).
//!
//! A server published on plex.tv has several connections: **local** (its
//! LAN address, as an `https://…plex.direct` name and as a plain IP),
//! **remote** (the public address through the router) and **relay**
//! (Plex's relay, which works through any NAT but is slow and capped).
//! [`order_connections`] puts them best first — local, remote, relay;
//! HTTPS before plain HTTP except on a LAN whose DNS hides `plex.direct`
//! names; IPv4 before IPv6 — and [`Server`] uses the first that answers,
//! moving to the next when one cannot be reached (the same failover as the
//! Jellyfin client, ADR-0075). Plain HTTP is tried only on the LAN, never
//! over the internet.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::http::{ClientInfo, Fail, Http, MAX_BLOB, MAX_JSON, Method, Response, Token, check};
use crate::model::{Envelope, Identity, IdentityEnvelope, Item, Playlist, Resource, Section};
use crate::{Error, Result};

/// Items asked for per page of a library listing.
const PAGE: usize = 500;

/// Where a connection goes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConnectionKind {
    /// On the LAN of the server.
    Local,
    /// The server's public address.
    Remote,
    /// Plex's relay (slow).
    Relay,
}

impl ConnectionKind {
    /// `local`, `remote` or `relay`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Relay => "relay",
        }
    }
}

/// The URIs to try for `server`, best first, without duplicates (see the
/// module docs).
#[must_use]
pub fn order_connections(server: &Resource) -> Vec<(String, ConnectionKind)> {
    // (kind, plain-HTTP rank, IPv6, position) → uri.
    let mut ranked: Vec<((ConnectionKind, u8, bool, usize), String)> = Vec::new();
    for (i, c) in server.connections.iter().enumerate() {
        let kind = if c.relay {
            ConnectionKind::Relay
        } else if c.local {
            ConnectionKind::Local
        } else {
            ConnectionKind::Remote
        };
        let Ok(uri) = crate::normalize_address(&c.uri) else {
            continue;
        };
        let https = uri.starts_with("https://");
        // On the LAN with DNS rebinding protection, the plain IP first.
        let rank = |plain: bool| -> u8 {
            match (
                plain,
                kind == ConnectionKind::Local && server.dns_rebinding_protection,
            ) {
                (true, true) | (false, false) => 0,
                _ => 1,
            }
        };
        ranked.push(((kind, rank(!https), c.ipv6, i), uri));
        // The plain-IP twin of a LAN `plex.direct` name.
        if kind == ConnectionKind::Local
            && https
            && !server.https_required
            && let Some(port) = c.port.filter(|p| (1..=65_535).contains(p))
            && !c.address.trim().is_empty()
        {
            let host = if c.address.contains(':') {
                format!("[{}]", c.address.trim())
            } else {
                c.address.trim().to_owned()
            };
            if let Ok(plain) = crate::normalize_address(&format!("http://{host}:{port}")) {
                ranked.push(((kind, rank(true), c.ipv6, i), plain));
            }
        }
    }
    ranked.sort_by_key(|r| r.0);
    let mut out: Vec<(String, ConnectionKind)> = Vec::new();
    for ((kind, ..), uri) in ranked {
        // Plain HTTP only on the LAN: the token would cross the internet
        // in clear text otherwise.
        if kind != ConnectionKind::Local && uri.starts_with("http://") {
            continue;
        }
        if !out.iter().any(|(u, _)| *u == uri) {
            out.push((uri, kind));
        }
    }
    out
}

/// Checks every URI of `uris` at once (`GET /identity` with `token`,
/// `timeout` each) and returns, in the same order, whether it answered as
/// the server `machine_id` (any server when empty).
#[must_use]
pub fn probe_connections(
    uris: &[String],
    info: &ClientInfo,
    token: Option<&Token>,
    machine_id: &str,
    timeout: Duration,
) -> Vec<bool> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = uris
            .iter()
            .map(|uri| {
                let info = info.clone();
                scope.spawn(move || {
                    let http = Http::new(info, timeout, timeout + timeout);
                    let url = format!("{uri}/identity");
                    http.send(Method::Get, &url, token, None, 1 << 20)
                        .ok()
                        .and_then(|r| check(r).ok())
                        .and_then(|r| serde_json::from_slice::<IdentityEnvelope>(&r.body).ok())
                        .is_some_and(|id| {
                            machine_id.is_empty() || id.container.machine_identifier == machine_id
                        })
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or(false))
            .collect()
    })
}

/// A Plex Media Server at one of several URIs (best first): a request
/// goes to the URI that answered last; if that one cannot be reached, the
/// others are tried in order and the first that answers becomes the
/// current one. Methods block; a `Server` is `Send + Sync`.
pub struct Server {
    http: Http,
    addresses: Vec<String>,
    active: AtomicUsize,
    token: Token,
}

impl fmt::Debug for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Server")
            .field("addresses", &self.addresses)
            .field("active", &self.active.load(Ordering::Relaxed))
            .field("info", &self.http.info)
            .finish_non_exhaustive()
    }
}

impl Server {
    /// The server at `addresses` (best first) with the server `token`.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] if there is no address or one is malformed.
    pub fn new(addresses: &[String], info: ClientInfo, token: Token) -> Result<Self> {
        Self::with_timeouts(
            addresses,
            info,
            token,
            Duration::from_secs(4),
            Duration::from_secs(60),
        )
    }

    /// Like [`new`](Self::new) with a connect timeout (how long one URI
    /// is tried before the next) and an overall request timeout.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_timeouts(
        addresses: &[String],
        info: ClientInfo,
        token: Token,
        connect: Duration,
        overall: Duration,
    ) -> Result<Self> {
        let mut list = Vec::new();
        for a in addresses {
            let a = crate::normalize_address(a)?;
            if !list.contains(&a) {
                list.push(a);
            }
        }
        if list.is_empty() {
            return Err(Error::Invalid("no server address".into()));
        }
        Ok(Self {
            http: Http::new(info, connect, overall),
            addresses: list,
            active: AtomicUsize::new(0),
            token,
        })
    }

    /// The URIs, in the order they are tried.
    #[must_use]
    pub fn addresses(&self) -> &[String] {
        &self.addresses
    }

    /// The URI that answered last (the first one before any request).
    #[must_use]
    pub fn active_address(&self) -> &str {
        let i = self.active.load(Ordering::Relaxed);
        &self.addresses[i.min(self.addresses.len() - 1)]
    }

    /// Who this app says it is.
    #[must_use]
    pub fn info(&self) -> &ClientInfo {
        &self.http.info
    }

    /// The server token (for URLs the player opens itself).
    pub(crate) fn token(&self) -> &Token {
        &self.token
    }

    /// Sends a GET, trying the other URIs when the current one cannot be
    /// reached. `range`: `(offset, len)` of a byte range.
    pub(crate) fn send(
        &self,
        path_query: &str,
        range: Option<(u64, u64)>,
        limit: u64,
    ) -> Result<Response> {
        self.send_as(Method::Get, path_query, range, limit)
    }

    /// Like [`send`](Self::send) with another method (`PUT` of the
    /// playback API).
    pub(crate) fn send_as(
        &self,
        method: Method,
        path_query: &str,
        range: Option<(u64, u64)>,
        limit: u64,
    ) -> Result<Response> {
        let n = self.addresses.len();
        let start = self.active.load(Ordering::Relaxed) % n;
        let mut last = None;
        for k in 0..n {
            let i = (start + k) % n;
            let url = format!("{}{path_query}", self.addresses[i]);
            match self
                .http
                .send(method, &url, Some(&self.token), range, limit)
            {
                Ok(r) => {
                    self.active.store(i, Ordering::Relaxed);
                    return Ok(r);
                }
                Err(Fail::Read(e)) => return Err(e),
                Err(Fail::Reach(e)) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| Error::Unreachable("no address".into())))
    }

    fn get_json<T: DeserializeOwned>(&self, path_query: &str) -> Result<T> {
        crate::http::parse(&check(self.send(path_query, None, MAX_JSON)?)?.body)
    }

    /// GET a whole binary answer (artwork).
    pub(crate) fn get_bytes(&self, path_query: &str) -> Result<Vec<u8>> {
        Ok(check(self.send(path_query, None, MAX_BLOB)?)?.body)
    }

    /// Who the server is.
    ///
    /// # Errors
    ///
    /// Network errors.
    pub fn identity(&self) -> Result<Identity> {
        Ok(self.get_json::<IdentityEnvelope>("/identity")?.container)
    }

    /// The libraries the token may see.
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`]; network errors.
    pub fn sections(&self) -> Result<Vec<Section>> {
        Ok(self
            .get_json::<Envelope<Section>>("/library/sections")?
            .container
            .directory)
    }

    /// Every item of Plex type `plex_type` in section `key` (1 movie,
    /// 2 show, 4 episode, 10 track), with external ids, paged.
    ///
    /// # Errors
    ///
    /// Network errors.
    pub fn items(&self, key: &str, plex_type: u8) -> Result<Vec<Item>> {
        let key = crate::http::encode(key);
        self.paged(&format!(
            "/library/sections/{key}/all?type={plex_type}&includeGuids=1"
        ))
    }

    /// The user's playlists of `kind` (`audio`, `video`).
    ///
    /// # Errors
    ///
    /// Network errors.
    pub fn playlists(&self, kind: &str) -> Result<Vec<Playlist>> {
        Ok(self
            .get_json::<Envelope<Playlist>>(&format!(
                "/playlists?playlistType={}",
                crate::http::encode(kind)
            ))?
            .container
            .metadata)
    }

    /// The items of playlist `id`, in order.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for a deleted playlist; network errors.
    pub fn playlist_items(&self, id: &str) -> Result<Vec<Item>> {
        self.paged(&format!("/playlists/{}/items?", crate::http::encode(id)))
    }

    fn paged(&self, path_query: &str) -> Result<Vec<Item>> {
        let sep = if path_query.ends_with('?') {
            ""
        } else if path_query.contains('?') {
            "&"
        } else {
            "?"
        };
        let mut out: Vec<Item> = Vec::new();
        loop {
            let page = self
                .get_json::<Envelope<Item>>(&format!(
                    "{path_query}{sep}X-Plex-Container-Start={}&X-Plex-Container-Size={PAGE}",
                    out.len()
                ))?
                .container;
            let got = page.metadata.len();
            out.extend(page.metadata);
            let total = page
                .total_size
                .and_then(|t| usize::try_from(t).ok())
                .unwrap_or(0);
            if got == 0 || out.len() >= total {
                return Ok(out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Connection;

    fn conn(uri: &str, address: &str, local: bool, relay: bool, ipv6: bool) -> Connection {
        Connection {
            protocol: uri.split(':').next().unwrap_or_default().to_owned(),
            address: address.to_owned(),
            port: Some(32400),
            uri: uri.to_owned(),
            local,
            relay,
            ipv6,
        }
    }

    fn server(dns_protection: bool, https_required: bool) -> Resource {
        Resource {
            name: "NAS".into(),
            provides: "server".into(),
            dns_rebinding_protection: dns_protection,
            https_required,
            connections: vec![
                conn(
                    "https://138-201-1-2.abc.plex.direct:8443",
                    "138.201.1.2",
                    true,
                    true,
                    false,
                ),
                conn(
                    "https://1-2-3-4.abc.plex.direct:32400",
                    "1.2.3.4",
                    false,
                    false,
                    false,
                ),
                conn("http://1.2.3.4:32400", "1.2.3.4", false, false, false),
                conn(
                    "https://fd00--1.abc.plex.direct:32400",
                    "fd00::1",
                    true,
                    false,
                    true,
                ),
                conn(
                    "https://192-168-1-2.abc.plex.direct:32400",
                    "192.168.1.2",
                    true,
                    false,
                    false,
                ),
            ],
            ..Resource::default()
        }
    }

    #[test]
    fn orders_local_remote_relay() {
        let order: Vec<_> = order_connections(&server(false, false));
        let uris: Vec<&str> = order.iter().map(|(u, _)| u.as_str()).collect();
        assert_eq!(
            uris,
            [
                "https://192-168-1-2.abc.plex.direct:32400",
                "https://fd00--1.abc.plex.direct:32400",
                "http://192.168.1.2:32400",
                "http://[fd00::1]:32400",
                "https://1-2-3-4.abc.plex.direct:32400",
                "https://138-201-1-2.abc.plex.direct:8443",
            ]
        );
        assert_eq!(order[0].1, ConnectionKind::Local);
        assert_eq!(order[4].1, ConnectionKind::Remote);
        assert_eq!(order[5].1, ConnectionKind::Relay);
    }

    #[test]
    fn plain_ip_first_behind_dns_rebinding_protection() {
        let uris: Vec<String> = order_connections(&server(true, false))
            .into_iter()
            .map(|(u, _)| u)
            .collect();
        assert_eq!(uris[0], "http://192.168.1.2:32400");
        assert_eq!(uris[2], "https://192-168-1-2.abc.plex.direct:32400");
        // HTTPS required: no plain-IP twins.
        let strict: Vec<String> = order_connections(&server(true, true))
            .into_iter()
            .map(|(u, _)| u)
            .collect();
        assert!(strict.iter().all(|u| u.starts_with("https://")));
    }
}
