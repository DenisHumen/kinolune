// SPDX-License-Identifier: MPL-2.0
//! Errors of the runtime and of the settings store.

use std::io;
use std::path::PathBuf;

use crate::paths::Platform;

/// Error of the runtime lifecycle and of directory setup.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RuntimeError {
    /// [`global::init`](crate::global::init) was called while a runtime is running.
    #[error("the core runtime is already initialized")]
    AlreadyInitialized,
    /// The global runtime was used before `init` or after `shutdown`.
    #[error("the core runtime is not initialized")]
    NotInitialized,
    /// The runtime instance was shut down; start a new one.
    #[error("the core runtime is shut down")]
    ShutDown,
    /// The platform has no default data directories (Android): the app must
    /// pass them in [`RuntimeConfig`](crate::RuntimeConfig) or through
    /// [`PlatformHost::dirs`](crate::PlatformHost::dirs).
    #[error("data directories for {platform} must be provided by the app")]
    DirsRequired {
        /// Platform the runtime was started for.
        platform: Platform,
    },
    /// A directory passed by the app is not absolute.
    #[error("data directory must be an absolute path")]
    RelativeDir {
        /// The rejected path.
        path: PathBuf,
    },
    /// A data directory could not be created.
    #[error("cannot create a data directory: {source}")]
    CreateDir {
        /// Directory that could not be created.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// The tokio runtime could not be built.
    #[error("cannot start the async runtime: {0}")]
    AsyncRuntime(#[source] io::Error),
    /// The settings store failed.
    #[error(transparent)]
    Settings(#[from] SettingsError),
}

/// Error of the [`Settings`](crate::Settings) store.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SettingsError {
    /// The key is empty, too long or has characters outside `[a-z0-9._-]`.
    #[error("invalid settings key {0:?}")]
    InvalidKey(String),
    /// The store was closed by [`Runtime::shutdown`](crate::Runtime::shutdown).
    #[error("the settings store is closed")]
    Closed,
    /// The database was written by a newer core; downgrades are refused so
    /// that the newer data is not damaged.
    #[error("settings schema version {found} is newer than supported {supported}")]
    SchemaTooNew {
        /// `user_version` found in the database.
        found: i64,
        /// Latest schema version this build knows.
        supported: i64,
    },
    /// The import document is not a settings export of a supported version.
    #[error("not a supported settings export: {0}")]
    InvalidExport(String),
    /// A value could not be converted from or to JSON.
    #[error("settings value is not valid JSON for the requested type: {0}")]
    Json(#[from] serde_json::Error),
    /// SQLite failed (I/O, corruption, locking).
    #[error("settings database error: {0}")]
    Database(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl From<rusqlite::Error> for SettingsError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Database(Box::new(err))
    }
}

impl crate::ErrorCode for RuntimeError {
    fn code(&self) -> &'static str {
        use crate::error_codes as common;
        match self {
            Self::AlreadyInitialized => "already_initialized",
            Self::NotInitialized | Self::ShutDown => common::NOT_INITIALIZED,
            Self::DirsRequired { .. } | Self::RelativeDir { .. } => common::INVALID,
            Self::CreateDir { .. } => common::IO,
            Self::AsyncRuntime(_) => common::INTERNAL,
            Self::Settings(e) => e.code(),
        }
    }
}

impl crate::ErrorCode for SettingsError {
    fn code(&self) -> &'static str {
        use crate::error_codes as common;
        match self {
            Self::InvalidKey(_) | Self::InvalidExport(_) | Self::Json(_) => common::INVALID,
            Self::Closed => common::NOT_INITIALIZED,
            Self::SchemaTooNew { .. } => "schema_too_new",
            Self::Database(_) => common::DATABASE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ErrorCode, is_valid_code};

    #[test]
    fn every_error_has_a_valid_code() {
        let errors = [
            RuntimeError::AlreadyInitialized,
            RuntimeError::NotInitialized,
            RuntimeError::ShutDown,
            RuntimeError::DirsRequired {
                platform: Platform::Android,
            },
            RuntimeError::RelativeDir {
                path: PathBuf::from("x"),
            },
            RuntimeError::CreateDir {
                path: PathBuf::from("/x"),
                source: io::Error::other("x"),
            },
            RuntimeError::AsyncRuntime(io::Error::other("x")),
            RuntimeError::Settings(SettingsError::InvalidKey("A".into())),
            RuntimeError::Settings(SettingsError::Closed),
            RuntimeError::Settings(SettingsError::SchemaTooNew {
                found: 9,
                supported: 1,
            }),
            RuntimeError::Settings(SettingsError::InvalidExport("x".into())),
            RuntimeError::Settings(SettingsError::Database(Box::new(io::Error::other("x")))),
        ];
        for e in errors {
            assert!(is_valid_code(e.code()), "{e}: {}", e.code());
        }
    }
}
