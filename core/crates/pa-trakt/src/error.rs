// SPDX-License-Identifier: MPL-2.0
//! Errors of the Trakt client. Texts never contain tokens or URLs with
//! secrets.

/// What went wrong.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// No client id in this build and none entered by the user.
    #[error("no Trakt client id")]
    NotConfigured,
    /// Not signed in to Trakt (or the sign-in was revoked).
    #[error("not signed in to Trakt")]
    SignedOut,
    /// The server could not be reached.
    #[error("Trakt unreachable: {0}")]
    Unreachable(String),
    /// The request timed out.
    #[error("Trakt timed out")]
    TimedOut,
    /// Too many requests; wait `retry_after_secs` before the next one.
    #[error("Trakt rate limit, retry after {retry_after_secs} s")]
    RateLimited { retry_after_secs: u64 },
    /// The access token was refused (`401`).
    #[error("Trakt refused the token")]
    Unauthorized,
    /// Any other unexpected status.
    #[error("Trakt answered HTTP {0}")]
    Http(u16),
    /// The answer did not parse.
    #[error("Trakt answer not understood: {0}")]
    Parse(String),
    /// The secret store failed.
    #[error("secret store: {0}")]
    Storage(String),
    /// The library failed (sync).
    #[error("library: {0}")]
    Library(String),
}

/// Result of the Trakt client.
pub type Result<T> = std::result::Result<T, Error>;

impl From<pa_secrets::SecretError> for Error {
    fn from(e: pa_secrets::SecretError) -> Self {
        Self::Storage(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Parse(e.to_string())
    }
}
