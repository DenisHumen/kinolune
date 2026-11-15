// SPDX-License-Identifier: MPL-2.0
//! Verification of in-app update downloads (PKG-018, ADR-NNNN).
//!
//! A GitHub release of the app carries `SHA256SUMS` (`sha256sum` output for
//! every asset, `release-app.yml`) and, once the owner signs releases,
//! `SHA256SUMS.minisig`. Before the app installs a downloaded asset it calls
//! [`verify_artifact`]:
//!
//! 1. **Signature hook.** When the signed remote config carries a release
//!    key (`updates.release_public_key`, `pa-remote-config`), the checksum
//!    file must be signed with it (minisign, prehashed, `file:SHA256SUMS` in
//!    the trusted comment). Without a key the step is skipped: the checksums
//!    then only catch corrupt and truncated downloads, not a forged release.
//! 2. **Checksum.** The asset must be listed in `SHA256SUMS` and the file on
//!    disk must hash to that value.
//!
//! Anything else — a missing entry, a different hash, a missing or bad
//! signature while a key is set — is an [`Error`], and the app deletes the
//! download.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use pa_remote_config::TrustedKey;
use sha2::{Digest, Sha256};

/// Name of the checksum file in a release (and in the signature's trusted
/// comment).
pub const SUMS_FILE: &str = "SHA256SUMS";

/// Name of its detached minisign signature.
pub const SIGNATURE_FILE: &str = "SHA256SUMS.minisig";

/// Largest checksum file accepted (a release has a dozen lines).
pub const MAX_SUMS_BYTES: usize = 64 * 1024;

/// Why a download was refused.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `SHA256SUMS` is not `sha256sum` output.
    #[error("SHA256SUMS is malformed: {0}")]
    Malformed(String),
    /// The asset has no line in `SHA256SUMS`.
    #[error("{0} is not listed in SHA256SUMS")]
    NotListed(String),
    /// The file on disk hashes to something else: corrupt, truncated or
    /// replaced.
    #[error("checksum mismatch for {name}: expected {expected}, got {actual}")]
    Mismatch {
        name: String,
        expected: String,
        actual: String,
    },
    /// A release key is configured but the release has no signature.
    #[error("the release is not signed (SHA256SUMS.minisig is missing)")]
    SignatureMissing,
    /// The signature does not verify with the release key.
    #[error("bad release signature: {0}")]
    BadSignature(String),
    /// The configured release key does not parse.
    #[error("bad release key: {0}")]
    BadKey(String),
    /// The download cannot be read.
    #[error("cannot read the download: {0}")]
    Io(#[from] io::Error),
}

/// Whether the checksum file was signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signature {
    /// No release key is configured yet: checksums only.
    NotRequired,
    /// Signed with the configured release key.
    Verified,
}

/// A verified download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// Lowercase hex SHA-256 of the file.
    pub sha256: String,
    pub signature: Signature,
}

/// The parsed `SHA256SUMS`: asset name → digest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sha256Sums(BTreeMap<String, [u8; 32]>);

impl Sha256Sums {
    /// Parses `sha256sum` output: `<64 hex>  <name>` (text mode) or
    /// `<64 hex> *<name>` (binary mode) per line; blank lines are skipped.
    /// A name listed twice with different digests is malformed.
    pub fn parse(text: &[u8]) -> Result<Self, Error> {
        if text.len() > MAX_SUMS_BYTES {
            return Err(Error::Malformed(format!(
                "larger than {MAX_SUMS_BYTES} bytes"
            )));
        }
        let text = std::str::from_utf8(text).map_err(|_| Error::Malformed("not UTF-8".into()))?;
        let mut sums = BTreeMap::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() {
                continue;
            }
            let bad = || Error::Malformed(format!("line {}", n + 1));
            let (hex, rest) = line.split_at_checked(64).ok_or_else(bad)?;
            let name = rest
                .strip_prefix("  ")
                .or_else(|| rest.strip_prefix(" *"))
                .filter(|name| !name.is_empty())
                .ok_or_else(bad)?;
            let digest = decode_hex(hex).ok_or_else(bad)?;
            if let Some(old) = sums.insert(name.to_owned(), digest)
                && old != digest
            {
                return Err(Error::Malformed(format!("{name} is listed twice")));
            }
        }
        if sums.is_empty() {
            return Err(Error::Malformed("no entries".into()));
        }
        Ok(Self(sums))
    }

    /// The digest of `name`, if listed.
    pub fn get(&self, name: &str) -> Option<&[u8; 32]> {
        self.0.get(name)
    }

    /// Listed asset names, sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }
}

/// The signature hook: with a `release_key`, `signature` (the text of
/// `SHA256SUMS.minisig`) must sign `sums` with it; without one there is
/// nothing to check.
pub fn check_signature(
    sums: &[u8],
    signature: Option<&str>,
    release_key: Option<&str>,
) -> Result<Signature, Error> {
    let Some(key) = release_key.map(str::trim).filter(|k| !k.is_empty()) else {
        return Ok(Signature::NotRequired);
    };
    let key = TrustedKey::from_base64(key).map_err(|e| Error::BadKey(e.to_string()))?;
    let signature = signature
        .filter(|s| !s.trim().is_empty())
        .ok_or(Error::SignatureMissing)?;
    key.verify(SUMS_FILE, sums, signature)
        .map_err(|e| Error::BadSignature(e.to_string()))?;
    Ok(Signature::Verified)
}

/// Checks the download at `path` of the release asset `asset_name`:
/// first the signature of `sums` ([`check_signature`]), then that the file
/// hashes to the asset's line in `sums`.
pub fn verify_artifact(
    path: &Path,
    asset_name: &str,
    sums: &[u8],
    signature: Option<&str>,
    release_key: Option<&str>,
) -> Result<Verified, Error> {
    let signed = check_signature(sums, signature, release_key)?;
    let parsed = Sha256Sums::parse(sums)?;
    let expected = parsed
        .get(asset_name)
        .ok_or_else(|| Error::NotListed(asset_name.to_owned()))?;
    let actual = sha256_file(path)?;
    if &actual != expected {
        return Err(Error::Mismatch {
            name: asset_name.to_owned(),
            expected: encode_hex(expected),
            actual: encode_hex(&actual),
        });
    }
    Ok(Verified {
        sha256: encode_hex(&actual),
        signature: signed,
    })
}

/// SHA-256 of a file, read in 1 MiB chunks.
pub fn sha256_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

fn decode_hex(hex: &str) -> Option<[u8; 32]> {
    let bytes = hex.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Some(out)
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests;
