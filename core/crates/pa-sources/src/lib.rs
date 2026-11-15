// SPDX-License-Identifier: MPL-2.0
//! Source manager of the project_alpha core (NET-011).
//!
//! A *source* is a place the library reads media from: a local folder, an
//! SMB share, a WebDAV collection, an SFTP or FTP folder (NET-006) or a
//! Jellyfin or Emby server (NET-014,
//! NET-015, whose libraries read as a file tree, `pa_jellyfin::JellyfinFs`;
//! its secret is the access token of the sign-in, see [`JellyfinAccount`]
//! and [`QuickConnect`]). [`SourceManager`] keeps sources in the
//! `sources` table of the library database (`pa-library`) and their
//! passwords in the OS secret store (`pa-secrets`):
//!
//! - **CRUD:** [`SourceManager::create`], [`get`](SourceManager::get),
//!   [`list`](SourceManager::list), [`update`](SourceManager::update),
//!   [`delete`](SourceManager::delete).
//! - **Settings:** kind, address (a URI *without* credentials), user name,
//!   backend [`SourceOptions`], "scan into library" and auto-scan toggles.
//! - **Status** ([`SourceStatus`]): online, last check, last scan, last
//!   error, last speed test.
//! - **Connection:** [`SourceManager::connect`] builds the
//!   [`Vfs`](pa_vfs::Vfs) backend of the source kind;
//!   [`check`](SourceManager::check) and
//!   [`speed_test`](SourceManager::speed_test) use it and record the status.
//!
//! - **Sync** ([`sync`], SYNC-010): a network source can opt in to sync to
//!   the paired devices ([`SourceSync`]); the password travels only sealed
//!   with the family key, and lands in the receiver's secret store.
//!
//! The password never reaches SQLite or logs: the row holds only
//! `credential_ref`, the key of the secret; addresses with `user:pass@`
//! are refused; `Debug` of every type hides the password; error texts saved
//! in the status or returned to the caller go through
//! [`pa_secrets::redact`] and have the password itself masked.
//!
//! ```
//! use std::sync::Arc;
//! use pa_secrets::{MemoryStore, SecretString};
//! use pa_sources::{NewSource, SourceKind, SourceManager};
//!
//! let manager = SourceManager::new(
//!     pa_library::open_in_memory().unwrap(),
//!     Arc::new(MemoryStore::new()),
//! );
//! let nas = manager
//!     .create(
//!         NewSource::new(SourceKind::Smb, "NAS", "smb://nas.local/media/Movies")
//!             .with_login("anna", SecretString::new("hunter2")),
//!     )
//!     .unwrap();
//! assert!(nas.has_password);
//! assert_eq!(manager.list().unwrap().len(), 1);
//! ```

mod address;
mod connect;
mod jellyfin;
mod manager;
mod model;
pub mod sync;

pub use address::{ServerAddress, SmbAddress, file_uri_to_path, path_to_file_uri};

/// Normalizes the address of a source of `kind` the way
/// [`SourceManager::create`] stores it (a URI without credentials).
pub fn normalize_address(kind: SourceKind, address: &str) -> Result<String> {
    address::normalize(kind, address)
}
pub use connect::{SpeedReport, SpeedTestOptions};
pub use jellyfin::{JellyfinAccount, JellyfinPlayback, QuickConnect, ReportEvent};
pub use manager::SourceManager;
pub use model::{
    JellyfinOptions, NewSource, PasswordChange, SmbEncryption, SmbVersion, Source, SourceKind,
    SourceOptions, SourceStatus, SourceSync, SourceUpdate, SpeedSample,
};
/// The Jellyfin and Emby client (NET-014, NET-015), for callers of the
/// media server helpers.
pub use pa_jellyfin;

/// Error of the source manager. No variant carries a password.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No source with this id.
    #[error("source {0} not found")]
    NotFound(i64),
    /// An argument is not acceptable (bad address, unknown option value).
    #[error("invalid source: {0}")]
    Invalid(String),
    /// The library database failed.
    #[error("library: {0}")]
    Library(#[from] pa_library::Error),
    /// The secret store failed (locked keychain, no storage).
    #[error("secret store: {0}")]
    Secrets(#[from] pa_secrets::SecretError),
    /// A Jellyfin or Emby server refused or could not be reached while signing in
    /// (there is no source yet). `message` has secrets masked.
    #[error("jellyfin: {kind}: {message}")]
    Server {
        /// What went wrong, as a `pa-vfs` kind.
        kind: pa_vfs::ErrorKind,
        /// Message with secrets masked.
        message: String,
    },
    /// The source cannot be reached or read. `message` has secrets masked.
    #[error("source {id}: {kind}: {message}")]
    Source {
        /// Source id.
        id: i64,
        /// What went wrong, as the backend classified it.
        kind: pa_vfs::ErrorKind,
        /// Backend message with secrets masked.
        message: String,
    },
}

impl From<pa_library::rusqlite::Error> for Error {
    fn from(e: pa_library::rusqlite::Error) -> Self {
        Self::Library(pa_library::Error::Sqlite(e))
    }
}

impl Error {
    /// Kind of a [`Error::Source`] failure, `None` for other errors.
    pub fn source_kind(&self) -> Option<pa_vfs::ErrorKind> {
        match self {
            Self::Source { kind, .. } | Self::Server { kind, .. } => Some(*kind),
            _ => None,
        }
    }
}

/// Shorthand for results of this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Masks secrets in a text that may end up in the database or a log: the
/// known password of the source, then [`pa_secrets::redact`] patterns.
pub(crate) fn scrub(text: &str, password: Option<&pa_secrets::SecretString>) -> String {
    let text = match password {
        Some(p) if !p.is_empty() => text.replace(p.expose(), "***"),
        _ => text.to_owned(),
    };
    pa_secrets::redact(&text)
}

static SSH_RSA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Whether SFTP sources accept RSA host keys and RSA user keys (default
/// `true`). The `rsa` crate has an unfixed timing side channel
/// (RUSTSEC-2023-0071); the app's "RSA keys" setting turns them off for
/// every source manager of the process. Remove with the deny.toml
/// exception once a fixed `rsa` release ships.
pub fn set_ssh_rsa_allowed(allowed: bool) {
    SSH_RSA.store(allowed, std::sync::atomic::Ordering::Relaxed);
}

/// See [`set_ssh_rsa_allowed`].
pub fn ssh_rsa_allowed() -> bool {
    SSH_RSA.load(std::sync::atomic::Ordering::Relaxed)
}

/// Last line of an SFTP secret that holds a private key with a passphrase.
const PASSPHRASE_LINE: &str = "Passphrase: ";

/// The secret of an SFTP source that logs in with a private key (OpenSSH,
/// PEM or PuTTY format), optionally encrypted with `passphrase`. Store it
/// as the source's password; a secret that is not a key is a password.
pub fn ssh_key_secret(key: &str, passphrase: Option<&str>) -> pa_secrets::SecretString {
    let key = key.trim();
    pa_secrets::SecretString::new(match passphrase.filter(|p| !p.is_empty()) {
        Some(p) => format!("{key}\n{PASSPHRASE_LINE}{p}"),
        None => format!("{key}\n"),
    })
}

/// `true` if an SFTP secret is a private key rather than a password.
pub fn is_ssh_key_secret(secret: &str) -> bool {
    let s = secret.trim_start();
    s.starts_with("-----BEGIN ") || s.starts_with("PuTTY-User-Key-File-")
}

/// The SFTP login of a stored secret (see [`ssh_key_secret`]).
pub(crate) fn ssh_auth(secret: &str) -> pa_vfs_sftp::SftpAuth {
    if !is_ssh_key_secret(secret) {
        return pa_vfs_sftp::SftpAuth::Password(secret.to_owned());
    }
    let body = secret.trim_end();
    match body.rsplit_once('\n') {
        Some((key, last)) if last.starts_with(PASSPHRASE_LINE) => {
            pa_vfs_sftp::SftpAuth::PrivateKey {
                key: format!("{}\n", key.trim_end()),
                passphrase: Some(last[PASSPHRASE_LINE.len()..].to_owned()),
            }
        }
        _ => pa_vfs_sftp::SftpAuth::PrivateKey {
            key: format!("{body}\n"),
            passphrase: None,
        },
    }
}

/// Wall clock in Unix milliseconds.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pa_vfs_sftp::SftpAuth;

    const KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----";

    #[test]
    fn ssh_secrets_round_trip() {
        assert_eq!(ssh_auth("hunter2"), SftpAuth::Password("hunter2".into()));
        let plain = ssh_key_secret(KEY, None);
        assert!(is_ssh_key_secret(plain.expose()));
        assert_eq!(
            ssh_auth(plain.expose()),
            SftpAuth::PrivateKey {
                key: format!("{KEY}\n"),
                passphrase: None
            }
        );
        let locked = ssh_key_secret(&format!("\n{KEY}\n\n"), Some("p a s s"));
        assert_eq!(
            ssh_auth(locked.expose()),
            SftpAuth::PrivateKey {
                key: format!("{KEY}\n"),
                passphrase: Some("p a s s".into())
            }
        );
    }
}
