// SPDX-License-Identifier: MPL-2.0
//! Credentials of the app and the user's tokens.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Placeholder values from `.env.example` that mean "none".
const PLACEHOLDERS: &[&str] = &["changeme", "CHANGEME", "your-key-here"];

fn real(raw: &str) -> Option<String> {
    let v = raw.trim();
    (!v.is_empty() && !PLACEHOLDERS.contains(&v)).then(|| v.to_owned())
}

/// The app's Trakt credentials: the `client_id` (public, sent as
/// `trakt-api-key`) and, optionally, the `client_secret`. Trakt marks the
/// secret deprecated for user sign-in and optional in the device flow; it
/// is sent only when configured. `Debug` hides the secret.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    client_id: String,
    client_secret: Option<String>,
}

impl Credentials {
    /// Credentials with `client_id` (`None` for an empty or placeholder
    /// id) and an optional secret.
    pub fn new(client_id: &str, client_secret: Option<&str>) -> Option<Self> {
        Some(Self {
            client_id: real(client_id)?,
            client_secret: client_secret.and_then(real),
        })
    }

    /// The project's credentials compiled into this build:
    /// `PA_TRAKT_CLIENT_ID` (or `CARGO_PA_TRAKT_CLIENT_ID` from the Flutter
    /// build hook) and `PA_TRAKT_CLIENT_SECRET` /
    /// `CARGO_PA_TRAKT_CLIENT_SECRET`. Builds from source have none.
    pub fn from_build() -> Option<Self> {
        let id = option_env!("PA_TRAKT_CLIENT_ID").or(option_env!("CARGO_PA_TRAKT_CLIENT_ID"))?;
        let secret =
            option_env!("PA_TRAKT_CLIENT_SECRET").or(option_env!("CARGO_PA_TRAKT_CLIENT_SECRET"));
        Self::new(id, secret)
    }

    /// The user's own client id and secret (advanced settings) if set,
    /// else the build's.
    pub fn choose(user_id: Option<&str>, user_secret: Option<&str>) -> Option<Self> {
        user_id
            .and_then(|id| Self::new(id, user_secret))
            .or_else(Self::from_build)
    }

    /// The client id.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The client secret, if any.
    pub fn client_secret(&self) -> Option<&str> {
        self.client_secret.as_deref()
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "***"))
            .finish()
    }
}

/// Codes of a device sign-in (`POST /oauth/device/code`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCode {
    /// Polled with; never shown.
    pub device_code: String,
    /// Shown to the user (about 8 characters).
    pub user_code: String,
    /// Where the user enters the code (`https://trakt.tv/activate`).
    pub verification_url: String,
    /// Seconds until the codes expire.
    pub expires_in: u64,
    /// Seconds between polls.
    pub interval: u64,
}

impl DeviceCode {
    /// The activation page with the code filled in, for the QR code.
    pub fn qr_url(&self) -> String {
        format!(
            "{}/{}",
            self.verification_url.trim_end_matches('/'),
            self.user_code
        )
    }
}

/// The user's tokens, kept in the secret store as JSON. `Debug` hides
/// them.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    /// Single-use: each refresh returns a new one.
    pub refresh_token: String,
    /// When the access token expires (Unix ms).
    pub expires_at_ms: i64,
    /// The Trakt user name, for the settings screen.
    #[serde(default)]
    pub username: Option<String>,
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens")
            .field("expires_at_ms", &self.expires_at_ms)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// The token answer of `/oauth/token` and `/oauth/device/token`.
#[derive(Deserialize)]
pub(crate) struct TokenAnswer {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    pub(crate) expires_in: i64,
    pub(crate) created_at: i64,
}

impl TokenAnswer {
    pub(crate) fn into_tokens(self) -> Tokens {
        Tokens {
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            expires_at_ms: (self.created_at + self.expires_in) * 1000,
            username: None,
        }
    }
}

/// One poll of a device sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    /// The user has not confirmed yet (`400`).
    Pending,
    /// Polling too fast (`429`): wait longer.
    SlowDown,
    /// Confirmed: the user's tokens (`200`).
    Approved(Tokens),
    /// The codes expired (`410`): start again.
    Expired,
    /// The user refused (`418`).
    Denied,
    /// The code was already used (`409`).
    AlreadyUsed,
    /// Unknown device code (`404`).
    InvalidCode,
}
