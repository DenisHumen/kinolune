// SPDX-License-Identifier: MPL-2.0
//! minisign signature check against the key compiled into the app.

use minisign_verify::{PublicKey, Signature};

use crate::Error;

/// Marker the production key starts with while the owner has not generated
/// the real key yet (docs/dev/remote-config.md).
pub const PLACEHOLDER_MARKER: &str = "PLACEHOLDER";

/// The minisign public key (base64 line of `minisign.pub`) that signs
/// `config/*.json` in the public repo.
///
/// The owner's config key (generated 2026-11-15; the secret half is kept
/// offline by the owner). A value starting with [`PLACEHOLDER_MARKER`] means
/// "no key": [`TrustedKey::production`] fails, the client never trusts a
/// downloaded or cached document and runs on the built-in copy only.
/// `just core remote-config-release-check` fails on a placeholder.
pub const PRODUCTION_PUBLIC_KEY: &str = "RWR3z2UnJ3Q9XyS+nLTf9pSBfKHFm2neWHvVHtLtGuYR5XLE83RT044j";

/// Is [`PRODUCTION_PUBLIC_KEY`] still the placeholder?
pub fn production_key_is_placeholder() -> bool {
    PRODUCTION_PUBLIC_KEY.starts_with(PLACEHOLDER_MARKER)
}

/// A minisign public key the client trusts.
#[derive(Debug, Clone)]
pub struct TrustedKey(PublicKey);

impl TrustedKey {
    /// The key compiled into the app. `Err(Error::NoTrustedKey)` while it is
    /// the placeholder.
    pub fn production() -> Result<Self, Error> {
        if production_key_is_placeholder() {
            return Err(Error::NoTrustedKey);
        }
        Self::from_base64(PRODUCTION_PUBLIC_KEY)
    }

    /// From the base64 line of a `minisign.pub` file.
    pub fn from_base64(b64: &str) -> Result<Self, Error> {
        if b64.trim().starts_with(PLACEHOLDER_MARKER) {
            return Err(Error::NoTrustedKey);
        }
        PublicKey::from_base64(b64.trim())
            .map(Self)
            .map_err(|e| Error::BadKey(e.to_string()))
    }

    /// From the whole text of a `minisign.pub` file (comment + key line).
    pub fn decode(text: &str) -> Result<Self, Error> {
        PublicKey::decode(text)
            .map(Self)
            .map_err(|e| Error::BadKey(e.to_string()))
    }

    /// Checks that `signature` (text of a `.minisig` file) signs `body` with
    /// this key, and that its trusted comment names `file_name`.
    ///
    /// Only prehashed (`ED`) signatures — the default of minisign ≥ 0.6 —
    /// are accepted. The file-name check (`file:<name>` in the trusted
    /// comment, which `minisign -S` writes by default) stops a validly
    /// signed file from being served in place of another one.
    pub fn verify(&self, file_name: &str, body: &[u8], signature: &str) -> Result<(), Error> {
        let sig = Signature::decode(signature).map_err(|e| Error::BadSignature(e.to_string()))?;
        self.0
            .verify(body, &sig, false)
            .map_err(|e| Error::BadSignature(e.to_string()))?;
        let expected = format!("file:{file_name}");
        if !sig
            .trusted_comment()
            .split('\t')
            .any(|field| field == expected)
        {
            return Err(Error::BadSignature(format!(
                "trusted comment does not name {file_name}"
            )));
        }
        Ok(())
    }
}
