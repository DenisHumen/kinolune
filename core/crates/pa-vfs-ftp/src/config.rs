// SPDX-License-Identifier: MPL-2.0
//! Source configuration.

use std::fmt;
use std::time::Duration;

pub use pa_discovery::Fingerprint;

/// User name and password of an FTP source.
///
/// `Debug` never prints the password. The source manager keeps it in the
/// platform secret store and builds this value only to connect.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    /// User name.
    pub username: String,
    /// Password.
    pub password: String,
}

impl Credentials {
    /// Credentials from a user name and a password.
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }

    /// The conventional anonymous login.
    #[must_use]
    pub fn anonymous() -> Self {
        Self::new("anonymous", "anonymous@")
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Whether and how the connection is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Security {
    /// Plain FTP: the password and the data travel in clear text.
    #[default]
    Plain,
    /// Explicit FTPS (`AUTH TLS` on the usual port 21, `ftpes://`): the
    /// control and every data connection are encrypted.
    Explicit,
    /// Implicit FTPS (TLS from the first byte, port 990, `ftps://`).
    Implicit,
}

impl Security {
    /// The usual port: 990 for implicit FTPS, 21 otherwise.
    #[must_use]
    pub fn default_port(self) -> u16 {
        match self {
            Self::Implicit => 990,
            _ => 21,
        }
    }
}

/// Which server certificates FTPS accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TlsTrust {
    /// Certificates signed by a CA of the Mozilla root store (default).
    #[default]
    System,
    /// Exactly the certificate with this SHA-256 fingerprint; host name,
    /// issuer and validity dates are not checked (self-signed NAS).
    Pinned(Fingerprint),
}

/// Default read-ahead block: big enough that a player reading 32–64 KiB at
/// a time does not wait on every call.
pub const DEFAULT_READ_AHEAD: usize = 256 * 1024;

/// How to reach one FTP source.
#[derive(Debug, Clone)]
pub struct FtpConfig {
    /// Host name or IP address (IPv6 without brackets).
    pub host: String,
    /// TCP port of the control connection.
    pub port: u16,
    /// Plain FTP or FTPS.
    pub security: Security,
    /// Which certificates to accept (FTPS only).
    pub tls: TlsTrust,
    /// Folder on the server that is the source root: an absolute path, or
    /// empty for the folder the server puts the user in after login.
    pub root: String,
    /// `None`: the anonymous login.
    pub credentials: Option<Credentials>,
    /// Connect and read timeout.
    pub timeout: Duration,
    /// Smaller reads are served from a block of this size; `0` disables it.
    pub read_ahead: usize,
    /// Idle control connections kept for listings (each opened file has
    /// its own while it reads).
    pub max_idle: usize,
}

impl FtpConfig {
    /// A plain FTP source at `host:21`, anonymous, the login folder.
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 21,
            security: Security::Plain,
            tls: TlsTrust::System,
            root: String::new(),
            credentials: None,
            timeout: Duration::from_secs(20),
            read_ahead: DEFAULT_READ_AHEAD,
            max_idle: 2,
        }
    }

    /// Sets the port.
    #[must_use]
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the security; does not change the port.
    #[must_use]
    pub fn with_security(mut self, security: Security) -> Self {
        self.security = security;
        self
    }

    /// Sets the certificate trust.
    #[must_use]
    pub fn with_tls(mut self, tls: TlsTrust) -> Self {
        self.tls = tls;
        self
    }

    /// Sets the root folder.
    #[must_use]
    pub fn with_root(mut self, root: impl Into<String>) -> Self {
        self.root = root.into();
        self
    }

    /// Sets the login.
    #[must_use]
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Sets the timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets the read-ahead block size.
    #[must_use]
    pub fn with_read_ahead(mut self, bytes: usize) -> Self {
        self.read_ahead = bytes;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_password() {
        let c = FtpConfig::new("nas").with_credentials(Credentials::new("anna", "hunter2"));
        assert!(!format!("{c:?}").contains("hunter2"));
        assert_eq!(Security::Implicit.default_port(), 990);
        assert_eq!(Security::Explicit.default_port(), 21);
    }
}
