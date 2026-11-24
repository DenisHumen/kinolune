// SPDX-License-Identifier: MPL-2.0
//! Cloud storages as media sources (NET-017, ADR-0144).
//!
//! Every cloud is a read-only [`Vfs`](pa_vfs::Vfs): listing, `stat` and
//! files opened as [`ByteSource`](pa_vfs::ByteSource)s read with byte
//! ranges, so players seek without downloading the whole file. Playback goes
//! through the loopback proxy like any other source (LIB-018, ADR-0038).
//!
//! - **MEGA** ([`mega`]): a small client of MEGA's documented JSON API
//!   (`cs` requests), no native SDK. Files are encrypted end to end
//!   (AES-128-CTR per file, keys under the account's master key);
//!   [`mega::MegaFs`] decrypts any byte range at its offset. Sign-in with
//!   e-mail and password (and a 2FA code); what is kept is the session
//!   ([`mega::Session`]), never the password. Public folder and file links
//!   (the key is in the link) need no account.
//! - **Google Drive** ([`gdrive`]), **OneDrive** ([`onedrive`]),
//!   **Dropbox** ([`dropbox`]): OAuth 2.0 with PKCE as a public client
//!   ([`oauth`]): no client secret in the code, a loopback redirect
//!   (RFC 8252 §7.3), read-only scopes, the refresh token in the secret
//!   store; a rotated refresh token is handed back through
//!   [`TokenSink`].
//!
//! [`CloudFs`] turns any [`Drive`] (a tree of nodes with ids) into a
//! `Vfs`: paths are resolved through cached listings, duplicate names in
//! a folder (allowed by Google Drive and MEGA) get a ` (2)` suffix.
//!
//! Errors never carry a token, a password, a session id or a download link.

mod drive;
pub mod dropbox;
pub mod gdrive;
mod http;
pub mod mega;
pub mod oauth;
pub mod onedrive;
mod range;
mod time;

pub use drive::{CloudFs, Drive, Node};
pub use http::USER_AGENT;
pub use oauth::{Provider, TokenSink, TokenSource, Tokens};
pub use range::{RangeFetch, RangeFile};

/// The cloud of a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cloud {
    /// MEGA (account or public link).
    Mega,
    /// Google Drive.
    GoogleDrive,
    /// Microsoft OneDrive (personal and work or school accounts).
    OneDrive,
    /// Dropbox.
    Dropbox,
}

impl Cloud {
    /// Short id: `mega`, `gdrive`, `onedrive`, `dropbox`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mega => "mega",
            Self::GoogleDrive => "gdrive",
            Self::OneDrive => "onedrive",
            Self::Dropbox => "dropbox",
        }
    }

    /// Display name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Mega => "MEGA",
            Self::GoogleDrive => "Google Drive",
            Self::OneDrive => "OneDrive",
            Self::Dropbox => "Dropbox",
        }
    }

    /// The OAuth provider of a cloud that signs in with OAuth.
    #[must_use]
    pub fn oauth(self) -> Option<Provider> {
        match self {
            Self::Mega => None,
            Self::GoogleDrive => Some(Provider::google()),
            Self::OneDrive => Some(Provider::microsoft()),
            Self::Dropbox => Some(Provider::dropbox()),
        }
    }
}

impl std::str::FromStr for Cloud {
    type Err = pa_vfs::VfsError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mega" => Ok(Self::Mega),
            "gdrive" => Ok(Self::GoogleDrive),
            "onedrive" => Ok(Self::OneDrive),
            "dropbox" => Ok(Self::Dropbox),
            other => Err(pa_vfs::VfsError::new(
                pa_vfs::ErrorKind::Unsupported,
                format!("unknown cloud {other:?}"),
            )),
        }
    }
}
