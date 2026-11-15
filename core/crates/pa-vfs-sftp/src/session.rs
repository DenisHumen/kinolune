// SPDX-License-Identifier: MPL-2.0
//! One SSH connection with an SFTP channel: the runtime that drives it, host
//! key checking, authentication and error mapping.

use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use pa_vfs::{ErrorKind, Result, VfsError};
use russh::MethodKind;
use russh::client::{self, AuthResult, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::RawSftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::StatusCode;

use crate::config::{HostKey, SftpAuth, SftpConfig};
use crate::error::HostKeyChanged;

/// Largest read request: OpenSSH serves up to 255 KiB, others 32–64 KiB.
const MAX_READ_LEN: u64 = 128 * 1024;
/// Read request size when the server does not tell its limits.
const DEFAULT_READ_LEN: u32 = 32 * 1024;

/// The process-wide runtime of all SFTP sources. Never dropped, so a source
/// can be dropped from any thread, async or not.
fn runtime() -> Result<&'static tokio::runtime::Runtime> {
    static RT: OnceLock<std::result::Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("pa-vfs-sftp")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| VfsError::new(ErrorKind::Other, format!("SFTP runtime: {e}")))
}

/// Runs `fut` on the SFTP runtime and blocks the calling thread until it
/// finishes. Safe to call from inside another tokio runtime (the caller's
/// thread only waits on a channel).
pub(crate) fn block<T: Send + 'static>(
    fut: impl Future<Output = Result<T>> + Send + 'static,
) -> Result<T> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    runtime()?.spawn(async move {
        let _ = tx.send(fut.await);
    });
    rx.recv()
        .unwrap_or_else(|_| Err(VfsError::new(ErrorKind::Other, "SFTP task stopped")))
}

/// Starts `fut` on the SFTP runtime without waiting (closing handles).
pub(crate) fn spawn(fut: impl Future<Output = ()> + Send + 'static) {
    if let Ok(rt) = runtime() {
        rt.spawn(fut);
    }
}

/// The host key of a server key or certificate.
fn host_key_of(key: &PublicKeyOrCertificate) -> HostKey {
    let data = match key {
        PublicKeyOrCertificate::PublicKey { key, .. } => key.key_data().clone(),
        PublicKeyOrCertificate::Certificate(cert) => cert.public_key().clone(),
    };
    HostKey::new(
        data.algorithm().as_str(),
        data.fingerprint(HashAlg::Sha256).to_string(),
    )
}

/// russh callbacks: checks the host key against the pin, if any.
struct Client {
    expected: Option<HostKey>,
    seen: Arc<Mutex<Option<HostKey>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let key = host_key_of(server_public_key);
        // The fingerprint identifies the key; the algorithm name is for
        // people (an RSA key reads as `ssh-rsa` or `rsa-sha2-*`).
        let ok = self
            .expected
            .as_ref()
            .is_none_or(|e| e.fingerprint() == key.fingerprint());
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(key);
        Ok(ok)
    }
}

/// A connected, authenticated SFTP session.
pub(crate) struct Session {
    /// Keeps the SSH connection open.
    _ssh: client::Handle<Client>,
    pub(crate) sftp: RawSftpSession,
    /// Size of one read request.
    pub(crate) read_len: u32,
    /// Absolute path of the source root on the server.
    pub(crate) root: String,
}

impl Session {
    /// Path on the server of `path` inside the source.
    pub(crate) fn remote(&self, path: &pa_vfs::VfsPath) -> String {
        if path.is_root() {
            self.root.clone()
        } else if self.root == "/" {
            path.as_str().to_owned()
        } else {
            format!("{}{}", self.root, path.as_str())
        }
    }
}

fn place(cfg: &SftpConfig) -> String {
    if cfg.host.contains(':') {
        format!("[{}]:{}", cfg.host, cfg.port)
    } else {
        format!("{}:{}", cfg.host, cfg.port)
    }
}

/// Maps a russh error during connect or authentication.
pub(crate) fn ssh_error(cfg: &SftpConfig, e: russh::Error) -> VfsError {
    use russh::Error as E;
    let at = place(cfg);
    let e = match e {
        E::IO(io) => return VfsError::from_io(io, &at),
        other => other,
    };
    let kind = match &e {
        E::ConnectionTimeout | E::KeepaliveTimeout | E::InactivityTimeout => ErrorKind::TimedOut,
        E::Disconnect | E::HUP | E::SendError | E::RecvError => ErrorKind::Unavailable,
        E::NoAuthMethod | E::NotAuthenticated => ErrorKind::AuthenticationFailed,
        E::NoCommonAlgo { .. } | E::UnknownAlgo => ErrorKind::Unsupported,
        _ => ErrorKind::Other,
    };
    VfsError::new(kind, format!("{at}: {e}"))
}

/// Maps an SFTP error on `path`.
pub(crate) fn sftp_error(path: &str, e: SftpError) -> VfsError {
    let (kind, message) = match &e {
        SftpError::Status(s) => (
            match s.status_code {
                StatusCode::NoSuchFile => ErrorKind::NotFound,
                StatusCode::PermissionDenied => ErrorKind::PermissionDenied,
                StatusCode::OpUnsupported => ErrorKind::Unsupported,
                StatusCode::NoConnection | StatusCode::ConnectionLost => ErrorKind::Unavailable,
                _ => ErrorKind::Other,
            },
            if s.error_message.is_empty() {
                s.status_code.to_string()
            } else {
                s.error_message.clone()
            },
        ),
        SftpError::Timeout => (ErrorKind::TimedOut, e.to_string()),
        SftpError::IO(_) => (ErrorKind::Unavailable, e.to_string()),
        SftpError::UnexpectedBehavior(m) if m.contains("closed") || m.contains("Error") => {
            (ErrorKind::Unavailable, e.to_string())
        }
        _ => (ErrorKind::Other, e.to_string()),
    };
    VfsError::new(kind, format!("{path}: {message}"))
}

/// `true` for the end-of-file status of a read or readdir.
pub(crate) fn is_eof(e: &SftpError) -> bool {
    matches!(e, SftpError::Status(s) if s.status_code == StatusCode::Eof)
}

/// Connects, checks the host key against `expected` (any key if `None`),
/// authenticates and opens the SFTP channel. Returns the session and the
/// server's host key.
pub(crate) async fn connect(
    cfg: Arc<SftpConfig>,
    expected: Option<HostKey>,
) -> Result<(Session, HostKey)> {
    let seen = Arc::new(Mutex::new(None));
    let handler = Client {
        expected: expected.clone(),
        seen: Arc::clone(&seen),
    };
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        nodelay: true,
        ..client::Config::default()
    });
    let at = place(&cfg);
    let connected = tokio::time::timeout(
        cfg.timeout,
        client::connect(config, (cfg.host.as_str(), cfg.port), handler),
    )
    .await
    .map_err(|_| VfsError::new(ErrorKind::TimedOut, format!("{at}: connect timed out")))?;
    let seen_key = || seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let mut ssh = match connected {
        Ok(h) => h,
        Err(russh::Error::UnknownKey) => {
            return Err(match (expected, seen_key()) {
                (Some(expected), Some(actual)) => HostKeyChanged {
                    host: at,
                    expected,
                    actual,
                }
                .into_error(),
                _ => VfsError::new(ErrorKind::AuthenticationFailed, format!("{at}: host key refused")),
            });
        }
        Err(e) => return Err(ssh_error(&cfg, e)),
    };
    let key = seen_key()
        .ok_or_else(|| VfsError::new(ErrorKind::Other, format!("{at}: no host key")))?;

    tokio::time::timeout(cfg.timeout, authenticate(&mut ssh, &cfg))
        .await
        .map_err(|_| VfsError::new(ErrorKind::TimedOut, format!("{at}: login timed out")))??;

    let channel = ssh
        .channel_open_session()
        .await
        .map_err(|e| ssh_error(&cfg, e))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| ssh_error(&cfg, e))?;
    let sftp = RawSftpSession::new(channel.into_stream());
    sftp.set_timeout(cfg.timeout.as_secs().max(1));
    let version = sftp
        .init()
        .await
        .map_err(|e| sftp_error(&at, e))
        .map_err(|e| VfsError::new(ErrorKind::Unsupported, format!("no SFTP subsystem: {e}")))?;
    let read_len = if version.extensions.contains_key("limits@openssh.com") {
        match sftp.limits().await {
            Ok(l) if l.max_read_len > 0 => u32::try_from(l.max_read_len.min(MAX_READ_LEN))
                .unwrap_or(DEFAULT_READ_LEN),
            _ => DEFAULT_READ_LEN,
        }
    } else {
        DEFAULT_READ_LEN
    };
    let root = resolve_root(&sftp, &cfg.root, &at).await?;
    Ok((
        Session {
            _ssh: ssh,
            sftp,
            read_len,
            root,
        },
        key,
    ))
}

/// The absolute root folder: `""`, `~` and `~/…` are under the home folder.
async fn resolve_root(sftp: &RawSftpSession, root: &str, at: &str) -> Result<String> {
    let root = root.trim();
    let rest = match root {
        "" | "~" => "",
        r if r.starts_with("~/") => &r[1..],
        r => {
            let r = r.trim_end_matches('/');
            return Ok(if r.is_empty() {
                "/".to_owned()
            } else if r.starts_with('/') {
                r.to_owned()
            } else {
                format!("/{r}")
            });
        }
    };
    let home = sftp
        .realpath(".")
        .await
        .map_err(|e| sftp_error(at, e))?
        .files
        .into_iter()
        .next()
        .map(|f| f.filename)
        .ok_or_else(|| VfsError::new(ErrorKind::Other, format!("{at}: no home folder")))?;
    let home = home.trim_end_matches('/');
    let rest = rest.trim_end_matches('/');
    Ok(match (home.is_empty(), rest.is_empty()) {
        (true, true) => "/".to_owned(),
        (_, true) => home.to_owned(),
        _ => format!("{home}{rest}"),
    })
}

async fn authenticate(ssh: &mut client::Handle<Client>, cfg: &SftpConfig) -> Result<()> {
    let user = cfg.username.clone();
    let err = |e| ssh_error(cfg, e);
    let ok = match &cfg.auth {
        SftpAuth::None => ssh.authenticate_none(user).await.map_err(err)?.success(),
        SftpAuth::Password(password) => {
            match ssh
                .authenticate_password(user.clone(), password.clone())
                .await
                .map_err(err)?
            {
                AuthResult::Success => true,
                AuthResult::Failure {
                    remaining_methods, ..
                } if remaining_methods.contains(&MethodKind::KeyboardInteractive) => {
                    keyboard_interactive(ssh, cfg, user, password).await?
                }
                AuthResult::Failure { .. } => false,
            }
        }
        SftpAuth::PrivateKey { key, passphrase } => {
            let key = russh::keys::decode_secret_key(key, passphrase.as_deref()).map_err(|e| {
                VfsError::new(
                    ErrorKind::AuthenticationFailed,
                    format!("cannot read the private key: {e}"),
                )
            })?;
            let hash = ssh.best_supported_rsa_hash().await.map_err(err)?.flatten();
            ssh.authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map_err(err)?
                .success()
        }
    };
    if ok {
        Ok(())
    } else {
        Err(VfsError::new(
            ErrorKind::AuthenticationFailed,
            format!(
                "{}: the server refused the login of {:?}",
                place(cfg),
                cfg.username
            ),
        ))
    }
}

/// Answers every keyboard-interactive prompt with the password.
async fn keyboard_interactive(
    ssh: &mut client::Handle<Client>,
    cfg: &SftpConfig,
    user: String,
    password: &str,
) -> Result<bool> {
    let err = |e| ssh_error(cfg, e);
    let mut reply = ssh
        .authenticate_keyboard_interactive_start(user, None)
        .await
        .map_err(err)?;
    // A server asks once or twice; a loop guard against endless prompts.
    for _ in 0..4 {
        match reply {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let answers = prompts.iter().map(|_| password.to_owned()).collect();
                reply = ssh
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(err)?;
            }
        }
    }
    Ok(false)
}
