// SPDX-License-Identifier: MPL-2.0
//! Virtual file system of media sources.
//!
//! One interface for every place media lives: a local folder, SMB, NFS,
//! WebDAV, SFTP/FTP (ADR-0001, ADR-0013, `docs/research/02` §1.4 and §4).
//!
//! - [`Vfs`] browses a source: [`list`](Vfs::list), [`stat`](Vfs::stat),
//!   [`open`](Vfs::open).
//! - [`ByteSource`] is an opened file: a fixed [`size`](ByteSource::size)
//!   and positional [`read_at`](ByteSource::read_at). Players, the prober,
//!   the HTTP Range proxy and fingerprinting consume it.
//! - [`VfsPath`] is a `/`-separated UTF-8 path relative to the source root
//!   that cannot escape it.
//! - [`LocalFs`] is the local backend; [`host`] turns stored local paths
//!   (a `file://` root, a `VfsPath` under it) into native host paths.
//! - [`iso`] reads disc images (`.iso`: UDF, ISO 9660) as a read-only
//!   [`Vfs`] (PLAY-017).
//! - [`contract`] is the test suite every backend must pass.
//! - [`VfsWrite`] is the write side of a backend ([`Vfs::as_write`]), and
//!   [`files`] the file management built on it (NET-023): folders,
//!   uploads with resume, move, copy, the source trash, name conflicts;
//!   [`write_contract`] is its test suite.
//!
//! The API is blocking and object-safe; see ADR-0013 for why.
//!
//! ```
//! use pa_vfs::{ByteSource, LocalFs, Vfs, VfsPath};
//!
//! let dir = std::env::temp_dir().join(format!("pa-vfs-doc-{}", std::process::id()));
//! std::fs::create_dir_all(&dir)?;
//! std::fs::write(dir.join("movie.mkv"), b"\x1a\x45\xdf\xa3 EBML")?;
//!
//! let vfs = LocalFs::new(&dir)?;
//! let entries = vfs.list(&VfsPath::root())?;
//! assert_eq!(entries[0].name, "movie.mkv");
//!
//! let src = vfs.open(&entries[0].path)?;
//! let mut magic = [0u8; 4];
//! src.read_exact_at(0, &mut magic)?;
//! assert_eq!(magic, [0x1a, 0x45, 0xdf, 0xa3]);
//! # std::fs::remove_dir_all(&dir)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod contract;
mod error;
pub mod files;
pub mod host;
pub mod iso;
mod local;
mod path;
mod source;
mod vfs;
mod write;
pub mod write_contract;

pub use error::{ErrorKind, Result, VfsError};
pub use local::LocalFs;
pub use path::VfsPath;
pub use source::{ByteSource, SourceReader};
pub use vfs::{DirEntry, EntryKind, Metadata, Vfs};
pub use write::VfsWrite;
