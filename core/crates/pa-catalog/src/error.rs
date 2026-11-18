// SPDX-License-Identifier: MPL-2.0
//! Error type of the catalog.

use pa_library::rusqlite;

/// Why a catalog operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Opening, migrating or querying the library database.
    #[error("library: {0}")]
    Library(#[from] pa_library::Error),
    /// The scan of a source failed as a whole.
    #[error("scan: {0}")]
    Scan(#[from] pa_scanner::ScanError),
    /// A file could not be read.
    #[error("io: {0}")]
    Io(String),
    /// No such source, item or file.
    #[error("not found: {0}")]
    NotFound(String),
    /// An argument is not acceptable.
    #[error("invalid: {0}")]
    Invalid(String),
    /// The active profile may not do this (a wrong PIN, LIB-012).
    #[error("denied: {0}")]
    Denied(String),
    /// Sync: recording or applying an op (UI-019).
    #[error("sync: {0}")]
    Sync(String),
    /// A provider request failed or no provider is set up (META-016
    /// manual search).
    #[error("provider: {0}")]
    Provider(String),
}

impl Error {
    /// Stable code for the app (CORE-009, ADR-0121): the words of
    /// `pa_runtime::error_codes` plus `foreign_database` (the file is not a
    /// library of this app, e.g. a v0 database to import) and
    /// `database_version` (written by a newer or diverged build).
    #[must_use]
    pub fn code(&self) -> &'static str {
        use pa_scanner::ScanError;
        match self {
            Self::Library(e) => match e {
                pa_library::Error::Invalid(_) => "invalid",
                pa_library::Error::Sync(_) => "sync",
                pa_library::Error::ForeignDatabase { .. } => "foreign_database",
                pa_library::Error::ChecksumMismatch { .. }
                | pa_library::Error::UnknownMigration { .. }
                | pa_library::Error::OutOfOrder { .. } => "database_version",
                _ => "database",
            },
            Self::Scan(ScanError::SourceNotFound(_)) => "not_found",
            Self::Scan(ScanError::SourceUnavailable(_)) => "unreachable",
            Self::Scan(_) => "database",
            Self::Io(_) => "io",
            Self::NotFound(_) => "not_found",
            Self::Invalid(_) => "invalid",
            Self::Denied(_) => "denied",
            Self::Sync(_) => "sync",
            Self::Provider(_) => "provider",
        }
    }
}

impl From<pa_sync::Error> for Error {
    fn from(e: pa_sync::Error) -> Self {
        Self::Sync(e.to_string())
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Library(pa_library::Error::Sqlite(e))
    }
}

impl From<pa_vfs::VfsError> for Error {
    fn from(e: pa_vfs::VfsError) -> Self {
        Self::Io(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_a_snake_case_code() {
        let errors = [
            Error::Library(pa_library::Error::Invalid("x")),
            Error::Library(pa_library::Error::ForeignDatabase { legacy_v0: true }),
            Error::Library(pa_library::Error::OutOfOrder { version: 1 }),
            Error::Library(pa_library::Error::Sqlite(rusqlite::Error::InvalidQuery)),
            Error::Scan(pa_scanner::ScanError::SourceNotFound(1)),
            Error::Io("x".into()),
            Error::NotFound("x".into()),
            Error::Invalid("x".into()),
            Error::Denied("x".into()),
            Error::Sync("x".into()),
            Error::Provider("x".into()),
        ];
        for e in errors {
            let code = e.code();
            assert!(
                !code.is_empty() && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                "{e}: {code}"
            );
        }
        assert_eq!(Error::NotFound("x".into()).code(), "not_found");
        assert_eq!(
            Error::Library(pa_library::Error::ForeignDatabase { legacy_v0: true }).code(),
            "foreign_database"
        );
    }
}
