// SPDX-License-Identifier: MPL-2.0
//! The signed-in user: tokens in the secret store, refreshed before they
//! expire and once more on a `401`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pa_secrets::{SecretKey, SecretStore, SecretString};

use crate::auth::{DeviceCode, Poll, Tokens};
use crate::client::{Client, ScrobbleAction};
use crate::model::{Listed, Rated, Target, Watched};
use crate::{Error, Result};

/// Our id of the one Trakt account per device: the secret store key is
/// `trakt.tokens:main` (no user name in a key, it is stored in the clear).
pub const ACCOUNT_ID: &str = "main";

/// Refresh an access token this long before it expires (Trakt: 7 days).
const REFRESH_AHEAD_MS: i64 = 24 * 3600 * 1000;

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

fn system_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// A Trakt account on this device.
pub struct Session {
    client: Client,
    store: Arc<dyn SecretStore>,
    key: SecretKey,
    tokens: Mutex<Option<Tokens>>,
    clock: Clock,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("client", &self.client)
            .field("signed_in", &self.username())
            .finish_non_exhaustive()
    }
}

impl Session {
    /// A session over `client` with tokens in `store`; a stored sign-in is
    /// restored (no network). A store that fails to read means "signed
    /// out".
    pub fn open(client: Client, store: Arc<dyn SecretStore>) -> Result<Self> {
        let key = SecretKey::trakt_tokens(ACCOUNT_ID)?;
        let tokens = store
            .get(&key)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str::<Tokens>(s.expose()).ok());
        Ok(Self {
            client,
            store,
            key,
            tokens: Mutex::new(tokens),
            clock: Arc::new(system_ms),
        })
    }

    /// The same session with a custom clock (tests).
    #[must_use]
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    fn lock(&self) -> MutexGuard<'_, Option<Tokens>> {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn save(&self, tokens: Option<Tokens>) -> Result<()> {
        match &tokens {
            Some(t) => {
                let json = serde_json::to_string(t)?;
                self.store.set(&self.key, &SecretString::new(json))?;
            }
            None => {
                self.store.delete(&self.key)?;
            }
        }
        *self.lock() = tokens;
        Ok(())
    }

    /// The client.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Whether someone is signed in.
    pub fn is_signed_in(&self) -> bool {
        self.lock().is_some()
    }

    /// The signed-in user's Trakt name, if any (no network).
    pub fn username(&self) -> Option<String> {
        self.lock().as_ref().and_then(|t| t.username.clone())
    }

    /// Starts a device sign-in: show `user_code`, the verification URL and
    /// a QR code of [`DeviceCode::qr_url`], then [`poll`](Self::poll) every
    /// `interval` seconds.
    pub fn begin(&self) -> Result<DeviceCode> {
        self.client.device_code()
    }

    /// Polls a device sign-in once. On [`Poll::Approved`] the tokens (with
    /// the user name) are stored and the session is signed in.
    pub fn poll(&self, code: &DeviceCode) -> Result<Poll> {
        let poll = self.client.poll_device(code)?;
        if let Poll::Approved(tokens) = &poll {
            let mut tokens = tokens.clone();
            tokens.username = self.client.username(&tokens.access_token).ok();
            self.save(Some(tokens.clone()))?;
            return Ok(Poll::Approved(tokens));
        }
        Ok(poll)
    }

    /// Signs out: revokes the token (best effort) and forgets it.
    pub fn sign_out(&self) -> Result<()> {
        let tokens = self.lock().clone();
        if let Some(t) = tokens {
            let _ = self.client.revoke(&t.access_token);
        }
        self.save(None)
    }

    /// Refreshes the tokens; a refused refresh signs out.
    fn refresh(&self, current: &Tokens) -> Result<Tokens> {
        match self.client.refresh(&current.refresh_token) {
            Ok(mut fresh) => {
                fresh.username.clone_from(&current.username);
                self.save(Some(fresh.clone()))?;
                Ok(fresh)
            }
            Err(Error::SignedOut) => {
                self.save(None)?;
                Err(Error::SignedOut)
            }
            Err(e) => Err(e),
        }
    }

    /// A valid access token, refreshed when it expires within a day.
    fn access(&self) -> Result<Tokens> {
        let tokens = self.lock().clone().ok_or(Error::SignedOut)?;
        if tokens.expires_at_ms - (self.clock)() < REFRESH_AHEAD_MS {
            return self.refresh(&tokens);
        }
        Ok(tokens)
    }

    /// Runs an authorized call; a `401` refreshes the token and tries once
    /// more.
    fn call<T>(&self, op: impl Fn(&Client, &str) -> Result<T>) -> Result<T> {
        let tokens = self.access()?;
        match op(&self.client, &tokens.access_token) {
            Err(Error::Unauthorized) => {
                let fresh = self.refresh(&tokens)?;
                op(&self.client, &fresh.access_token)
            }
            other => other,
        }
    }

    /// Scrobbles `target` at `progress` percent.
    pub fn scrobble(&self, action: ScrobbleAction, target: &Target, progress: f64) -> Result<()> {
        self.call(|c, t| c.scrobble(t, action, target, progress))
    }

    /// Every watched movie and episode.
    pub fn watched(&self) -> Result<Vec<Watched>> {
        self.call(Client::watched)
    }

    /// Adds plays to the history.
    pub fn add_history(&self, items: &[Watched]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        self.call(|c, t| c.add_history(t, items))
    }

    /// Removes every play of `targets`.
    pub fn remove_history(&self, targets: &[Target]) -> Result<()> {
        if targets.is_empty() {
            return Ok(());
        }
        self.call(|c, t| c.remove_history(t, targets))
    }

    /// The watchlist.
    pub fn watchlist(&self) -> Result<Vec<Listed>> {
        self.call(Client::watchlist)
    }

    /// Adds to or removes from the watchlist.
    pub fn set_watchlisted(&self, target: &Target, listed: bool) -> Result<()> {
        self.call(|c, t| {
            if listed {
                c.watchlist_add(t, target)
            } else {
                c.watchlist_remove(t, target)
            }
        })
    }

    /// Ratings of movies and shows.
    pub fn ratings(&self) -> Result<Vec<Rated>> {
        self.call(Client::ratings)
    }

    /// Adds or replaces ratings.
    pub fn add_ratings(&self, items: &[Rated]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        self.call(|c, t| c.add_ratings(t, items))
    }

    /// Removes ratings.
    pub fn remove_ratings(&self, targets: &[Target]) -> Result<()> {
        if targets.is_empty() {
            return Ok(());
        }
        self.call(|c, t| c.remove_ratings(t, targets))
    }
}
