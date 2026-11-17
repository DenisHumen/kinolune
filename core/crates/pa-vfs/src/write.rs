// SPDX-License-Identifier: MPL-2.0
//! The write side of a source (NET-023): the primitive operations a backend
//! implements. The user-facing operations (upload with resume, move, copy,
//! trash, name conflicts) are built on them in [`files`](crate::files).

use std::io::Read;

use crate::error::{ErrorKind, Result, VfsError};
use crate::path::VfsPath;
use crate::vfs::Vfs;

/// Primitive writes of a backend that can write.
///
/// A backend exposes it through [`Vfs::as_write`]. The methods are small
/// and literal, one protocol request each where the protocol allows; the
/// safety rules (trash by default, no work on the root, no recursion
/// through links, `.part` files) live in [`files`](crate::files), so every
/// backend gets them the same way. Like [`Vfs`], every path is relative to
/// the source root and cannot leave it.
///
/// Errors use the common [`ErrorKind`]s: a read-only share answers
/// [`PermissionDenied`](ErrorKind::PermissionDenied), an existing target
/// [`AlreadyExists`](ErrorKind::AlreadyExists) (where the protocol tells),
/// a missing parent [`NotFound`](ErrorKind::NotFound).
pub trait VfsWrite: Vfs {
    /// Creates one directory; the parent must exist.
    fn create_dir(&self, path: &VfsPath) -> Result<()>;

    /// Writes `len` bytes read from `data` into the file `path`.
    ///
    /// - `offset == 0`: creates the file or truncates an existing one.
    /// - `offset > 0`: appends to an existing file whose size is exactly
    ///   `offset` (resuming an upload). A backend that cannot append
    ///   answers [`Unsupported`](ErrorKind::Unsupported) and the caller
    ///   starts again from 0.
    ///
    /// Returns the number of bytes written. If `data` fails or ends early,
    /// the error is returned and what was written so far may stay on the
    /// source: that is what an upload resumes from.
    fn write_file(&self, path: &VfsPath, offset: u64, data: &mut dyn Read, len: u64)
    -> Result<u64>;

    /// Renames or moves `from` to `to` on the server, in one step.
    ///
    /// `to` must not exist (callers check first; backends that can refuse
    /// atomically do). [`Unsupported`](ErrorKind::Unsupported) means the
    /// server cannot do it in one step (for example across file systems);
    /// the caller then copies and deletes.
    fn rename(&self, from: &VfsPath, to: &VfsPath) -> Result<()>;

    /// Copies a file or a whole directory on the server, without the data
    /// passing through this device (WebDAV `COPY`, `copy_file_range`).
    /// Default: [`Unsupported`](ErrorKind::Unsupported), and the caller
    /// copies the bytes itself.
    fn server_copy(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        let _ = (from, to);
        Err(VfsError::new(
            ErrorKind::Unsupported,
            "server-side copy is not supported",
        ))
    }

    /// Deletes one file (or one symbolic link, not its target).
    fn remove_file(&self, path: &VfsPath) -> Result<()>;

    /// Deletes one empty directory.
    fn remove_dir(&self, path: &VfsPath) -> Result<()>;

    /// `true` if `path` itself is a symbolic link. Listings follow links,
    /// so a recursive delete asks this before entering a directory: it
    /// removes the link and never what it points to (possibly outside the
    /// source root). Default: `false`, for protocols without links.
    fn is_symlink(&self, path: &VfsPath) -> Result<bool> {
        let _ = path;
        Ok(false)
    }
}
