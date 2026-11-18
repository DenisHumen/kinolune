// SPDX-License-Identifier: MPL-2.0
//! plex.tv: the PIN sign-in, the account, Plex Home users and the servers
//! of the account.
//!
//! The PIN sign-in is the one Plex apps on TVs use: [`PlexTv::create_pin`]
//! gets a short code, the app shows it, the user opens
//! <https://plex.tv/link> on a phone or computer where they are signed in
//! and enters it; [`PlexTv::check_pin`] (every few seconds) then returns
//! the account's token. No password is typed into this app.

use std::time::Duration;

use crate::http::{ClientInfo, Http, MAX_JSON, Method, PLEX_TV, Token, check, encode, parse};
use crate::model::{Account, HomeUser, HomeUsers, Pin, Resource, SwitchedUser};
use crate::{Error, Result};

/// The plex.tv API, as this app ([`ClientInfo`]).
#[derive(Debug)]
pub struct PlexTv {
    http: Http,
    base: String,
}

impl PlexTv {
    /// plex.tv as `info`.
    #[must_use]
    pub fn new(info: ClientInfo) -> Self {
        Self::with_base(info, PLEX_TV)
    }

    /// The plex.tv API at `base` (tests: a local server).
    #[must_use]
    pub fn with_base(info: ClientInfo, base: &str) -> Self {
        Self {
            http: Http::new(info, Duration::from_secs(10), Duration::from_secs(30)),
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    /// Who this app says it is.
    #[must_use]
    pub fn info(&self) -> &ClientInfo {
        &self.http.info
    }

    fn call(&self, method: Method, path_query: &str, token: Option<&Token>) -> Result<Vec<u8>> {
        let url = format!("{}{path_query}", self.base);
        let r = self
            .http
            .send(method, &url, token, None, MAX_JSON)
            .map_err(crate::http::Fail::into_error)?;
        Ok(check(r)?.body)
    }

    /// A new sign-in code (four characters, valid for about 15 min) for
    /// <https://plex.tv/link>.
    ///
    /// # Errors
    ///
    /// Network errors; [`Error::Protocol`] if plex.tv gave no code.
    pub fn create_pin(&self) -> Result<Pin> {
        let pin: Pin = parse(&self.call(Method::Post, "/api/v2/pins?strong=false", None)?)?;
        if pin.id.is_none() || pin.code.trim().is_empty() {
            return Err(Error::Protocol("plex.tv gave no sign-in code".into()));
        }
        Ok(pin)
    }

    /// The PIN `id` now: its [`Pin::auth_token`] is set once the user
    /// entered the code.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] once the code expired; network errors.
    pub fn check_pin(&self, id: i64) -> Result<Pin> {
        parse(&self.call(Method::Get, &format!("/api/v2/pins/{id}"), None)?)
    }

    /// The account of `token`.
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`] for a revoked token; network errors.
    pub fn account(&self, token: &Token) -> Result<Account> {
        parse(&self.call(Method::Get, "/api/v2/user", Some(token))?)
    }

    /// The users of the account's Plex Home; empty when the account is in
    /// no Home.
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`]; network errors.
    pub fn home_users(&self, token: &Token) -> Result<Vec<HomeUser>> {
        match self.call(Method::Get, "/api/v2/home/users", Some(token)) {
            Ok(body) => Ok(parse::<HomeUsers>(&body)?.into_vec()),
            // Not in a Home: plex.tv answers 404 (or 400 on older APIs).
            Err(Error::NotFound | Error::Status { status: 400, .. }) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Switches to Home user `uuid` (with their `pin` if protected): the
    /// token of that user. `token` is the account's token.
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`] or [`Error::Forbidden`] for a wrong PIN;
    /// network errors.
    pub fn switch_user(&self, token: &Token, uuid: &str, pin: Option<&str>) -> Result<Token> {
        let mut path = format!("/api/v2/home/users/{}/switch", encode(uuid.trim()));
        if let Some(p) = pin.map(str::trim).filter(|p| !p.is_empty()) {
            path.push_str("?pin=");
            path.push_str(&encode(p));
        }
        let user: SwitchedUser = parse(&self.call(Method::Post, &path, Some(token))?)?;
        user.auth_token
            .or(user.authentication_token)
            .filter(|t| !t.is_empty())
            .map(Token::new)
            .ok_or_else(|| Error::Protocol("plex.tv gave no token for the user".into()))
    }

    /// The Plex Media Servers `token` can reach (owned and shared), with
    /// all their connections (local, remote, relay; HTTPS and IPv6
    /// included).
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`]; network errors.
    pub fn servers(&self, token: &Token) -> Result<Vec<Resource>> {
        let all: Vec<Resource> = parse(&self.call(
            Method::Get,
            "/api/v2/resources?includeHttps=1&includeRelay=1&includeIPv6=1",
            Some(token),
        )?)?;
        Ok(all.into_iter().filter(Resource::is_server).collect())
    }
}
