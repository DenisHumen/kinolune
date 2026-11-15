// SPDX-License-Identifier: MPL-2.0
//! Source configuration: address, login and host key trust.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use pa_vfs::{ErrorKind, VfsError};

/// How the client proves who it is.
///
/// `Debug` never prints the password, the key or its passphrase. The source
/// manager keeps them in the platform secret store and builds this value
/// only to connect.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum SftpAuth {
    /// `none` authentication (servers that let anyone in).
    #[default]
    None,
    /// Password; also answers keyboard-interactive prompts, which many NAS
    /// use instead of plain password authentication.
    Password(String),
    /// A private key in OpenSSH, PEM (PKCS#1/PKCS#8) or PuTTY format.
    PrivateKey {
        /// The key file contents.
        key: String,
        /// Passphrase of an encrypted key.
        passphrase: Option<String>,
    },
}

impl fmt::Debug for SftpAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::None => "None",
            Self::Password(_) => "Password(<redacted>)",
            Self::PrivateKey { .. } => "PrivateKey(<redacted>)",
        })
    }
}

/// The identity of an SSH server: key algorithm and SHA-256 fingerprint,
/// written like OpenSSH prints it (`ssh-ed25519 SHA256:<base64>`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostKey {
    algorithm: String,
    fingerprint: String,
}

impl HostKey {
    /// From an algorithm name (`ssh-ed25519`, `ecdsa-sha2-nistp256`,
    /// `rsa-sha2-512`, …) and a `SHA256:<base64>` fingerprint.
    pub fn new(algorithm: impl Into<String>, fingerprint: impl Into<String>) -> Self {
        Self {
            algorithm: algorithm.into(),
            fingerprint: fingerprint.into(),
        }
    }

    /// Key algorithm, e.g. `ssh-ed25519`.
    #[must_use]
    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }

    /// `SHA256:<base64>` as printed by `ssh-keygen -l`.
    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

impl fmt::Display for HostKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.algorithm, self.fingerprint)
    }
}

impl FromStr for HostKey {
    type Err = VfsError;

    /// Parses `<algorithm> SHA256:<base64>`.
    fn from_str(s: &str) -> Result<Self, VfsError> {
        let bad = || {
            VfsError::new(
                ErrorKind::Other,
                format!("not an SSH host key fingerprint (`ssh-ed25519 SHA256:…`): {s:?}"),
            )
        };
        let (algorithm, fingerprint) = s.trim().split_once(' ').ok_or_else(bad)?;
        let fingerprint = fingerprint.trim();
        let b64 = fingerprint.strip_prefix("SHA256:").ok_or_else(bad)?;
        let ok = |c: char| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=');
        if algorithm.is_empty() || b64.is_empty() || !b64.chars().all(ok) {
            return Err(bad());
        }
        Ok(Self::new(algorithm, fingerprint.trim_end_matches('=')))
    }
}

/// Called with the server's key the first time a [`HostKeyTrust::FirstUse`]
/// source connects; the caller stores it and pins it from then on.
pub type OnFirstUse = Arc<dyn Fn(&HostKey) + Send + Sync>;

/// Which server host keys are accepted.
#[derive(Clone, Default)]
pub enum HostKeyTrust {
    /// Trust on first use: accept whatever key the server shows the first
    /// time, report it to the callback and pin it for the life of this
    /// [`SftpFs`](crate::SftpFs) value. The default.
    #[default]
    FirstUse,
    /// Like [`FirstUse`](Self::FirstUse), with a callback that stores the key.
    FirstUseWith(OnFirstUse),
    /// Only this key. A different key fails with
    /// [`ErrorKind::AuthenticationFailed`] and a [`HostKeyChanged`](crate::HostKeyChanged)
    /// in the error chain.
    Pinned(HostKey),
}

impl fmt::Debug for HostKeyTrust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FirstUse => f.write_str("FirstUse"),
            Self::FirstUseWith(_) => f.write_str("FirstUseWith(..)"),
            Self::Pinned(k) => write!(f, "Pinned({k})"),
        }
    }
}

/// Default read-ahead block: 16 pipelined 32 KiB requests, one round trip.
pub const DEFAULT_READ_AHEAD: usize = 512 * 1024;

/// How to reach one SFTP source.
#[derive(Debug, Clone)]
pub struct SftpConfig {
    /// Host name or IP address (IPv6 without brackets).
    pub host: String,
    /// TCP port (22).
    pub port: u16,
    /// Folder on the server that is the source root: an absolute path,
    /// `~` or `~/…` (the user's home), or empty for the home folder.
    pub root: String,
    /// Login name.
    pub username: String,
    /// How to authenticate.
    pub auth: SftpAuth,
    /// Which host keys to accept.
    pub host_key: HostKeyTrust,
    /// Connect and per-request timeout.
    pub timeout: Duration,
    /// Smaller reads are served from a block of this size fetched ahead;
    /// `0` disables read-ahead.
    pub read_ahead: usize,
    /// Accept RSA host keys and RSA user keys (default `true`). The `rsa`
    /// crate has an unfixed timing side channel (RUSTSEC-2023-0071); with
    /// `false` both are refused with [`ErrorKind::Unsupported`].
    pub allow_rsa: bool,
}

impl SftpConfig {
    /// A source at `host:22`, the home folder of `username`, no
    /// authentication, host key trusted on first use.
    pub fn new(host: impl Into<String>, username: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 22,
            root: String::new(),
            username: username.into(),
            auth: SftpAuth::None,
            host_key: HostKeyTrust::FirstUse,
            timeout: Duration::from_secs(20),
            read_ahead: DEFAULT_READ_AHEAD,
            allow_rsa: true,
        }
    }

    /// Accepts or refuses RSA host and user keys.
    #[must_use]
    pub fn with_rsa(mut self, allow: bool) -> Self {
        self.allow_rsa = allow;
        self
    }

    /// Sets the port.
    #[must_use]
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the root folder.
    #[must_use]
    pub fn with_root(mut self, root: impl Into<String>) -> Self {
        self.root = root.into();
        self
    }

    /// Sets the authentication.
    #[must_use]
    pub fn with_auth(mut self, auth: SftpAuth) -> Self {
        self.auth = auth;
        self
    }

    /// Sets the host key trust.
    #[must_use]
    pub fn with_host_key(mut self, trust: HostKeyTrust) -> Self {
        self.host_key = trust;
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
    fn host_key_round_trips() {
        let k: HostKey = "ssh-ed25519 SHA256:AbC+/9xyz=".parse().unwrap();
        assert_eq!(k.algorithm(), "ssh-ed25519");
        assert_eq!(k.fingerprint(), "SHA256:AbC+/9xyz");
        assert_eq!(k.to_string(), "ssh-ed25519 SHA256:AbC+/9xyz");
        for bad in ["", "ssh-ed25519", "ssh-ed25519 MD5:aa", "x SHA256:a b"] {
            assert!(bad.parse::<HostKey>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn debug_hides_secrets() {
        let a = SftpAuth::Password("hunter2".into());
        let k = SftpAuth::PrivateKey {
            key: "-----BEGIN OPENSSH PRIVATE KEY-----".into(),
            passphrase: Some("pp".into()),
        };
        let cfg = SftpConfig::new("nas", "anna").with_auth(a.clone());
        for s in [format!("{a:?}"), format!("{k:?}"), format!("{cfg:?}")] {
            assert!(!s.contains("hunter2") && !s.contains("BEGIN") && !s.contains("pp)"));
        }
    }
}
