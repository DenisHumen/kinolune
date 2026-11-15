// SPDX-License-Identifier: MPL-2.0
//! Control connections: connecting, TLS, login, error mapping and the pool.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use pa_vfs::{ErrorKind, Result, VfsError};
use rustls::ClientConfig;
use suppaftp::types::FileType;
use suppaftp::{FtpError, FtpResult, RustlsConnector, RustlsFtpStream, Status, TlsConnector};

use crate::config::{Credentials, FtpConfig, Security, TlsTrust};

/// The TLS stream type of suppaftp's rustls connector.
pub(crate) type Tls = <RustlsConnector as TlsConnector>::Stream;

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sets read and write timeouts on every socket before its TLS handshake:
/// the control connection of implicit FTPS and every data connection.
#[derive(Debug)]
struct Connector {
    inner: RustlsConnector,
    timeout: Duration,
}

impl TlsConnector for Connector {
    type Stream = Tls;

    fn connect(&self, domain: &str, stream: TcpStream) -> FtpResult<Self::Stream> {
        set_timeouts(&stream, self.timeout);
        self.inner.connect(domain, stream)
    }
}

fn set_timeouts(s: &TcpStream, timeout: Duration) {
    let _ = s.set_read_timeout(Some(timeout));
    let _ = s.set_write_timeout(Some(timeout));
    let _ = s.set_nodelay(true);
}

/// `host:port` for messages.
pub(crate) fn place(cfg: &FtpConfig) -> String {
    if cfg.host.contains(':') {
        format!("[{}]:{}", cfg.host, cfg.port)
    } else {
        format!("{}:{}", cfg.host, cfg.port)
    }
}

/// Maps a suppaftp error; `context` is the path or the server.
pub(crate) fn ftp_error(context: &str, e: FtpError) -> VfsError {
    match e {
        FtpError::ConnectionError(io) => {
            // rustls reports a refused certificate as an I/O error.
            match io.get_ref().and_then(|e| e.downcast_ref::<rustls::Error>()) {
                Some(tls) => {
                    let m = format!("{context}: TLS: {tls}");
                    VfsError::new(ErrorKind::Other, m).with_source(io)
                }
                None => VfsError::from_io(io, context),
            }
        }
        FtpError::SecureError(m) => VfsError::new(ErrorKind::Other, format!("{context}: TLS: {m}")),
        FtpError::UnexpectedResponse(r) => {
            let code = r.status.code();
            let kind = match code {
                530 | 332 => ErrorKind::AuthenticationFailed,
                550 | 450 | 553 => ErrorKind::NotFound,
                421 | 425 | 426 => ErrorKind::Unavailable,
                500 | 501 | 502 | 504 | 534 => ErrorKind::Unsupported,
                _ => ErrorKind::Other,
            };
            let text = r.as_string().unwrap_or_default();
            let text = text.trim();
            VfsError::new(kind, format!("{context}: {code} {text}"))
        }
        other => VfsError::new(ErrorKind::Other, format!("{context}: {other}")),
    }
}

/// One logged-in control connection in binary mode.
pub(crate) struct Conn {
    pub(crate) ftp: RustlsFtpStream,
    /// The server supports `MLST`/`MLSD` (RFC 3659).
    pub(crate) mlst: bool,
}

/// The TLS client config of a source.
fn tls_config(trust: TlsTrust) -> Result<Arc<ClientConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = match trust {
        TlsTrust::Pinned(pin) => pa_discovery::PinnedVerifier::client_config(pin)
            .map_err(|e| VfsError::new(ErrorKind::Other, format!("TLS config: {e}")))?,
        TlsTrust::System => {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| VfsError::new(ErrorKind::Other, format!("TLS config: {e}")))?
                .with_root_certificates(roots)
                .with_no_client_auth()
        }
    };
    // RFC 7301 name of FTP; vsftpd and others refuse other protocols.
    config.alpn_protocols = vec![b"ftp".to_vec()];
    Ok(Arc::new(config))
}

fn connect(cfg: &FtpConfig, tls: Option<&Arc<ClientConfig>>) -> Result<Conn> {
    let at = place(cfg);
    let err = |e| ftp_error(&at, e);
    let addr: SocketAddr = (cfg.host.as_str(), cfg.port)
        .to_socket_addrs()
        .map_err(|e| VfsError::from_io(e, &at))?
        .next()
        .ok_or_else(|| VfsError::new(ErrorKind::Unavailable, format!("{at}: no address")))?;
    let timeout = cfg.timeout;
    // A session cache of this control connection only: servers that demand
    // TLS session reuse on data connections (vsftpd `require_ssl_reuse`)
    // refuse a data connection that resumes another control connection's
    // session, which a cache shared by the whole pool would offer.
    let tls = tls.map(|c| {
        let mut own = (**c).clone();
        own.resumption = rustls::client::Resumption::in_memory_sessions(32);
        Arc::new(own)
    });
    let connector = || {
        tls.as_ref().map(|c| Connector {
            inner: RustlsConnector::from(Arc::clone(c)),
            timeout,
        })
    };
    let ftp = match (cfg.security, connector()) {
        (Security::Implicit, Some(c)) => {
            let mut ftp =
                RustlsFtpStream::connect_secure_implicit(addr, c, &cfg.host).map_err(err)?;
            // `into_secure` does this for explicit FTPS: protect the data
            // connections too (RFC 4217 §9).
            ftp.custom_command("PBSZ 0", &[Status::CommandOk])
                .map_err(err)?;
            ftp.custom_command("PROT P", &[Status::CommandOk])
                .map_err(err)?;
            ftp
        }
        _ => {
            let tcp = TcpStream::connect_timeout(&addr, timeout)
                .map_err(|e| VfsError::from_io(e, &at))?;
            set_timeouts(&tcp, timeout);
            RustlsFtpStream::connect_with_stream(tcp).map_err(err)?
        }
    };
    // Passive mode; data connections go to the control connection's
    // address (servers behind NAT or in containers announce a private one).
    let mut ftp = ftp.passive_stream_builder(move |a: SocketAddr| {
        let s = TcpStream::connect_timeout(&a, timeout).map_err(FtpError::ConnectionError)?;
        set_timeouts(&s, timeout);
        Ok(s)
    });
    ftp.set_passive_nat_workaround(true);
    if let (Security::Explicit, Some(c)) = (cfg.security, connector()) {
        ftp = ftp.into_secure(c, &cfg.host).map_err(err)?;
    }
    let login = cfg
        .credentials
        .clone()
        .unwrap_or_else(Credentials::anonymous);
    if let Err(e) = ftp.login(login.username.as_str(), login.password.as_str()) {
        let e = ftp_error(&at, e);
        return Err(if e.kind() == ErrorKind::AuthenticationFailed {
            VfsError::new(
                ErrorKind::AuthenticationFailed,
                format!("{at}: the server refused the login of {:?}", login.username),
            )
        } else {
            e
        });
    }
    // Names in UTF-8 (RFC 2640); servers that always use it say no.
    let _ = ftp.custom_command("OPTS UTF8 ON", &[Status::CommandOk]);
    ftp.transfer_type(FileType::Binary).map_err(err)?;
    let mlst = ftp
        .feat()
        .map(|f| f.keys().any(|k| k.eq_ignore_ascii_case("MLST")))
        .unwrap_or(false);
    Ok(Conn { ftp, mlst })
}

/// Idle control connections of one source, the TLS config and the root.
pub(crate) struct Pool {
    pub(crate) cfg: FtpConfig,
    tls: Option<Arc<ClientConfig>>,
    idle: Mutex<Vec<Conn>>,
    root: OnceLock<String>,
}

impl Pool {
    pub(crate) fn new(cfg: FtpConfig) -> Result<Self> {
        let tls = match cfg.security {
            Security::Plain => None,
            _ => Some(tls_config(cfg.tls)?),
        };
        Ok(Self {
            cfg,
            tls,
            idle: Mutex::new(Vec::new()),
            root: OnceLock::new(),
        })
    }

    /// A new connection (not from the pool).
    pub(crate) fn connect(&self) -> Result<Conn> {
        let mut conn = connect(&self.cfg, self.tls.as_ref())?;
        if self.root.get().is_none() {
            // An absolute root as is; a relative one (or none) under the
            // login folder, as in RFC 1738 and curl.
            let raw = self.cfg.root.trim().trim_end_matches('/');
            let root = if raw.starts_with('/') {
                raw.to_owned()
            } else {
                let home = conn
                    .ftp
                    .pwd()
                    .map_err(|e| ftp_error(&place(&self.cfg), e))?;
                let home = home.trim_end_matches('/');
                if raw.is_empty() {
                    home.to_owned()
                } else {
                    format!("{home}/{raw}")
                }
            };
            let root = root.trim_end_matches('/');
            let root = if root.is_empty() || root.starts_with('/') {
                root.to_owned()
            } else {
                format!("/{root}")
            };
            let _ = self.root.set(root);
        }
        Ok(conn)
    }

    /// The source root on the server, without a trailing `/` (empty for
    /// the server root). Known after the first connect.
    pub(crate) fn root(&self) -> &str {
        self.root.get().map_or("", String::as_str)
    }

    /// Hands a connection back for reuse.
    pub(crate) fn put(&self, conn: Conn) {
        let mut idle = lock(&self.idle);
        if idle.len() < self.cfg.max_idle.max(1) {
            idle.push(conn);
        }
    }

    /// Runs `op` on an idle or new connection. A pooled connection that
    /// fails with a connection error (the server dropped it while idle) is
    /// replaced by a fresh one once.
    pub(crate) fn with<T>(&self, mut op: impl FnMut(&Self, &mut Conn) -> Result<T>) -> Result<T> {
        let pooled = lock(&self.idle).pop();
        let reused = pooled.is_some();
        let mut conn = match pooled {
            Some(c) => c,
            None => self.connect()?,
        };
        match op(self, &mut conn) {
            Err(e) if reused && e.kind().is_transient() => {
                drop(conn);
                let mut conn = self.connect()?;
                let r = op(self, &mut conn);
                self.recycle(conn, &r);
                r
            }
            r => {
                self.recycle(conn, &r);
                r
            }
        }
    }

    fn recycle<T>(&self, conn: Conn, r: &Result<T>) {
        if r.as_ref().err().is_none_or(|e| !e.kind().is_transient()) {
            self.put(conn);
        }
    }
}
