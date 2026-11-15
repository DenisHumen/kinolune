// SPDX-License-Identifier: MPL-2.0
//! The host key mismatch error.

use std::error::Error as StdError;
use std::fmt;

use pa_vfs::{ErrorKind, VfsError};

use crate::config::HostKey;

/// The server shows a different host key than the one pinned for the
/// source. Found in the `source()` chain of a [`VfsError`] of kind
/// [`ErrorKind::AuthenticationFailed`]; see [`is_host_key_changed`].
///
/// Either the server was reinstalled (its keys regenerated) or someone sits
/// between the device and the server. The connection is refused before any
/// password or key is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyChanged {
    /// `host:port`.
    pub host: String,
    /// The pinned key.
    pub expected: HostKey,
    /// The key the server showed.
    pub actual: HostKey,
}

impl HostKeyChanged {
    pub(crate) fn into_error(self) -> VfsError {
        VfsError::new(ErrorKind::AuthenticationFailed, self.to_string()).with_source(self)
    }
}

impl fmt::Display for HostKeyChanged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: the SSH host key changed (was {}, now {}). Nothing was sent. \
             If the server was reinstalled, clear the stored host key in the \
             source settings; otherwise someone may be intercepting the connection",
            self.host, self.expected, self.actual
        )
    }
}

impl StdError for HostKeyChanged {}

/// The [`HostKeyChanged`] behind `err`, if that is why it failed.
#[must_use]
pub fn host_key_changed(err: &VfsError) -> Option<&HostKeyChanged> {
    err.source().and_then(|s| s.downcast_ref::<HostKeyChanged>())
}

/// `true` if `err` is a refused connection because the host key changed.
#[must_use]
pub fn is_host_key_changed(err: &VfsError) -> bool {
    host_key_changed(err).is_some()
}
