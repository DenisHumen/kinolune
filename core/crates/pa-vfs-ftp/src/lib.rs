// SPDX-License-Identifier: MPL-2.0
//! FTP and FTPS backend of [`pa_vfs`] (NET-006).
//!
//! Plays media from FTP servers: a NAS (Synology, QNAP, a router with a USB
//! disk), vsftpd, ProFTPD, Pure-FTPd, FileZilla Server, IIS. [`FtpFs`]
//! implements [`pa_vfs::Vfs`] on top of `suppaftp` (blocking, rustls with
//! ring) and passes the shared contract suite against vsftpd in Docker
//! (`tests/servers/`), over plain FTP and explicit and implicit FTPS.
//!
//! ```no_run
//! use pa_vfs::{ByteSource, Vfs, VfsPath};
//! use pa_vfs_ftp::{Credentials, FtpConfig, FtpFs, Security};
//!
//! let fs = FtpFs::new(
//!     FtpConfig::new("nas.local")
//!         .with_security(Security::Explicit)
//!         .with_root("/video")
//!         .with_credentials(Credentials::new("anna", "secret")),
//! )?;
//! for entry in fs.list(&VfsPath::parse("/Movies")?)? {
//!     println!("{} {:?}", entry.name, entry.metadata.kind);
//! }
//! let movie = fs.open(&VfsPath::parse("/Movies/Dune (2021).mkv")?)?;
//! let mut header = [0u8; 4];
//! movie.read_exact_at(0, &mut header)?;
//! # Ok::<(), pa_vfs::VfsError>(())
//! ```
//!
//! **Seeking:** FTP has no positional read. A file streams with `RETR`; a
//! read where the stream already is continues it, any other read aborts
//! the transfer (`ABOR`) and starts a new one at the offset (`REST` +
//! `RETR`). Each opened file keeps up to two positioned transfers, so a
//! player that reads the stream and now and then the index does not
//! restart on every switch. Small reads are served from a read-ahead block.
//!
//! **TLS:** [`Security::Explicit`] (`AUTH TLS`) or [`Security::Implicit`];
//! the control and every data connection are encrypted (`PROT P`). A
//! self-signed NAS certificate is accepted only when pinned
//! ([`TlsTrust::Pinned`], the `sha256:` format of WebDAV and our server).
//! Design notes: ADR-NNNN.

mod config;
mod conn;
mod fs;
mod source;

pub use config::{Credentials, DEFAULT_READ_AHEAD, Fingerprint, FtpConfig, Security, TlsTrust};
pub use fs::FtpFs;
