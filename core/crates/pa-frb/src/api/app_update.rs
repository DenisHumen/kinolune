// SPDX-License-Identifier: MPL-2.0
//! In-app updates (PKG-018, `pa-update`): the check of a downloaded release
//! asset against the release's `SHA256SUMS` and, once the owner publishes a
//! release key in the signed remote config, its minisign signature.
//!
//! Dart downloads, installs and talks to GitHub; the core only says whether
//! a file on disk is the one the release lists.

use pa_update as up;

/// Why a download was refused; Dart receives it as an exception and
/// deletes the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateVerifyError {
    pub kind: UpdateVerifyErrorKind,
    /// `kind` as a stable code (CORE-009): `malformed`, `not_listed`,
    /// `mismatch`, `signature_missing`, `bad_signature`, `bad_key`, `io`.
    pub code: String,
    /// Human-readable detail for logs (no secrets: public files only).
    pub message: String,
}

/// What [`UpdateVerifyError`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateVerifyErrorKind {
    /// `SHA256SUMS` is not `sha256sum` output.
    Malformed,
    /// The asset is not listed in `SHA256SUMS`.
    NotListed,
    /// The file hashes to something else (corrupt, truncated, replaced).
    Mismatch,
    /// A release key is configured but the release is not signed.
    SignatureMissing,
    /// The signature does not verify with the release key.
    BadSignature,
    /// The configured release key does not parse.
    BadKey,
    /// The download cannot be read.
    Io,
}

impl From<up::Error> for UpdateVerifyError {
    fn from(e: up::Error) -> Self {
        let kind = match &e {
            up::Error::Malformed(_) => UpdateVerifyErrorKind::Malformed,
            up::Error::NotListed(_) => UpdateVerifyErrorKind::NotListed,
            up::Error::Mismatch { .. } => UpdateVerifyErrorKind::Mismatch,
            up::Error::SignatureMissing => UpdateVerifyErrorKind::SignatureMissing,
            up::Error::BadSignature(_) => UpdateVerifyErrorKind::BadSignature,
            up::Error::BadKey(_) => UpdateVerifyErrorKind::BadKey,
            up::Error::Io(_) => UpdateVerifyErrorKind::Io,
        };
        Self {
            kind,
            code: update_code(kind).into(),
            message: e.to_string(),
        }
    }
}

fn update_code(kind: UpdateVerifyErrorKind) -> &'static str {
    use UpdateVerifyErrorKind as K;
    match kind {
        K::Malformed => "malformed",
        K::NotListed => "not_listed",
        K::Mismatch => "mismatch",
        K::SignatureMissing => "signature_missing",
        K::BadSignature => "bad_signature",
        K::BadKey => "bad_key",
        K::Io => "io",
    }
}

/// A download that passed [`update_verify_artifact`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateVerified {
    /// Lowercase hex SHA-256 of the file.
    pub sha256: String,
    /// The checksum file was signed with the release key (false: no key
    /// is configured yet, checksums only).
    pub signed: bool,
}

/// Checks the file at `path`, downloaded as the release asset
/// `asset_name`, against `sha256sums` (the bytes of the release's
/// `SHA256SUMS`). With `release_key` (from
/// `CoreRemoteConfig::release_public_key`) `signature` — the text of
/// `SHA256SUMS.minisig` — must sign the checksums; without one it is
/// ignored. Hashes the whole file: call it off the UI isolate (FRB runs it
/// on its thread pool).
pub fn update_verify_artifact(
    path: String,
    asset_name: String,
    sha256sums: Vec<u8>,
    signature: Option<String>,
    release_key: Option<String>,
) -> Result<UpdateVerified, UpdateVerifyError> {
    let v = up::verify_artifact(
        std::path::Path::new(&path),
        &asset_name,
        &sha256sums,
        signature.as_deref(),
        release_key.as_deref(),
    )?;
    Ok(UpdateVerified {
        sha256: v.sha256,
        signed: v.signature == up::Signature::Verified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::remote_config::CoreRemoteConfig;

    #[test]
    fn errors_keep_their_kind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.apk");
        std::fs::write(&path, b"tampered").unwrap();
        let sums = format!("{}  a.apk\n", "0".repeat(64)).into_bytes();
        let err = update_verify_artifact(
            path.to_string_lossy().into_owned(),
            "a.apk".into(),
            sums.clone(),
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(err.kind, UpdateVerifyErrorKind::Mismatch);
        let err = update_verify_artifact(
            path.to_string_lossy().into_owned(),
            "b.apk".into(),
            sums,
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(err.kind, UpdateVerifyErrorKind::NotListed);
    }

    #[test]
    fn release_key_comes_from_the_config_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let config = CoreRemoteConfig::open(
            dir.path().to_string_lossy().into_owned(),
            "0.1.0".into(),
            "linux".into(),
        );
        // A fresh client runs on the built-in copy.
        assert_eq!(
            config.release_public_key().as_deref(),
            pa_remote_config::builtin().release_public_key()
        );
    }
}
