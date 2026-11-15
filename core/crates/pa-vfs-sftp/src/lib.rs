// SPDX-License-Identifier: MPL-2.0
//! SFTP backend of [`pa_vfs`] (NET-006).
//!
//! Plays media from any SSH server with the SFTP subsystem: OpenSSH on a
//! Linux box or a NAS (Synology, QNAP, TrueNAS, Unraid), a seedbox, a router.
//! [`SftpFs`] implements [`pa_vfs::Vfs`] on top of `russh` and
//! `russh-sftp` (pure Rust, ring crypto) and passes the shared contract
//! suite against OpenSSH in Docker (`tests/servers/`).
//!
//! ```no_run
//! use pa_vfs::{ByteSource, Vfs, VfsPath};
//! use pa_vfs_sftp::{HostKeyTrust, SftpAuth, SftpConfig, SftpFs};
//!
//! let fs = SftpFs::new(
//!     SftpConfig::new("nas.local", "anna")
//!         .with_root("/volume1/video")
//!         .with_auth(SftpAuth::Password("secret".into()))
//!         .with_host_key(HostKeyTrust::FirstUse),
//! )?;
//! for entry in fs.list(&VfsPath::parse("/Movies")?)? {
//!     println!("{} {:?}", entry.name, entry.metadata.kind);
//! }
//! // The key seen on the first connect: store it with the source and pass
//! // `HostKeyTrust::Pinned` next time.
//! println!("{:?}", fs.host_key());
//! # Ok::<(), pa_vfs::VfsError>(())
//! ```
//!
//! **Host keys** are trusted on first use (TOFU, like `ssh` with
//! `StrictHostKeyChecking=accept-new`): the first connect accepts the key and
//! reports it ([`HostKeyTrust::FirstUseWith`]); the caller stores it and
//! pins it ([`HostKeyTrust::Pinned`]). A different key later is refused
//! before any credential is sent ([`HostKeyChanged`]).
//!
//! **Reads** are positional `SSH_FXP_READ` requests, pipelined (up to 32 in
//! flight) and sized by the server's `limits@openssh.com`; small reads are
//! served from a read-ahead block. Design notes: ADR-NNNN.

mod config;
mod error;
mod fs;
mod session;
mod source;

pub use config::{DEFAULT_READ_AHEAD, HostKey, HostKeyTrust, OnFirstUse, SftpAuth, SftpConfig};
pub use error::{HostKeyChanged, host_key_changed, is_host_key_changed};
pub use fs::SftpFs;
