// SPDX-License-Identifier: MPL-2.0
//! The E2E container of secrets that travel in sync ops (SYNC-010).
//!
//! A password of a synced source never goes into an op in clear text. When
//! the user allows it, the op carries it sealed with NaCl `crypto_secretbox`
//! (XSalsa20-Poly1305, RustCrypto `crypto_secretbox`, byte-compatible with
//! libsodium's `crypto_secretbox_easy`). Only members of the device family
//! can open it: the key comes from the family key of the pairing
//! (`pa-pairing`, ADR-0026), which hubs, the outbox and journals never hold.
//!
//! - Key: HKDF-SHA256 over the 256-bit family key, salt
//!   `kinolune/sync/seal/v1`, info = family id, epoch of the family key and
//!   a *context* (e.g. `source:<sync id>`). A container moved into another
//!   op (another source) or another family does not open.
//! - Container: `sb1.<epoch>.<base64url(nonce ‖ box)>`, a 24-byte random
//!   nonce per seal. The epoch says which family key sealed it; after a key
//!   rotation (a revoked device) old containers no longer open and the
//!   sender seals again with the new key.
//!
//! ```
//! use pa_sync::seal::{self, SealKey};
//!
//! let key = SealKey::new("fam1", 1, &[7; 32]);
//! let sealed = seal::seal(&key, "source:abc", b"hunter2").unwrap();
//! assert!(seal::is_sealed(&sealed));
//! assert!(!sealed.contains("hunter2"));
//! assert_eq!(&*seal::open(&key, "source:abc", &sealed).unwrap(), b"hunter2");
//! assert!(seal::open(&key, "source:other", &sealed).is_err());
//! ```

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use crypto_secretbox::aead::{Aead, KeyInit};
use crypto_secretbox::{Nonce, XSalsa20Poly1305};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

/// Prefix of a sealed container.
pub const PREFIX: &str = "sb1.";
/// Largest secret that can be sealed, in bytes.
pub const MAX_SECRET_LEN: usize = 4096;
/// Longest container accepted by [`open`] and [`is_sealed`].
pub const MAX_SEALED_LEN: usize = 6 * 1024;

const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const SALT: &[u8] = b"kinolune/sync/seal/v1";

/// Why a container cannot be made or opened. Never carries the secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SealError {
    /// Not a container of this format.
    #[error("not a sealed secret")]
    Malformed,
    /// Sealed with another epoch of the family key.
    #[error("sealed with family key epoch {found}, this device has {have}")]
    Epoch { found: u32, have: u32 },
    /// Wrong key, wrong context or tampered with.
    #[error("sealed secret does not open")]
    Open,
    /// The secret is longer than [`MAX_SECRET_LEN`].
    #[error("secret too long")]
    TooLong,
    /// The system random source failed.
    #[error("random: {0}")]
    Random(String),
}

/// The family's sealing key: family id, epoch and the 256-bit family key
/// (`pa_pairing::FamilyKey`). `Debug` does not print the key.
#[derive(Clone)]
pub struct SealKey {
    family: String,
    epoch: u32,
    ikm: Zeroizing<[u8; 32]>,
}

impl fmt::Debug for SealKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SealKey")
            .field("family", &self.family)
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

impl PartialEq for SealKey {
    fn eq(&self, other: &Self) -> bool {
        self.family == other.family && self.epoch == other.epoch && *self.ikm == *other.ikm
    }
}

impl SealKey {
    /// The key of family `family` at `epoch` with the family key `key`.
    pub fn new(family: impl Into<String>, epoch: u32, key: &[u8; 32]) -> Self {
        Self {
            family: family.into(),
            epoch,
            ikm: Zeroizing::new(*key),
        }
    }

    /// Epoch of the family key.
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// Family id.
    pub fn family(&self) -> &str {
        &self.family
    }

    fn cipher(&self, context: &str) -> XSalsa20Poly1305 {
        let mut info = Vec::with_capacity(self.family.len() + context.len() + 8);
        info.extend_from_slice(self.family.as_bytes());
        info.push(0x1f);
        info.extend_from_slice(&self.epoch.to_be_bytes());
        info.push(0x1f);
        info.extend_from_slice(context.as_bytes());
        let mut okm = Zeroizing::new([0u8; 32]);
        Hkdf::<Sha256>::new(Some(SALT), self.ikm.as_ref())
            .expand(&info, okm.as_mut())
            .expect("32 bytes is a valid HKDF-SHA256 length");
        XSalsa20Poly1305::new_from_slice(okm.as_ref()).expect("32-byte key")
    }
}

/// Seals `secret` for `context` (e.g. `source:<sync id>`).
pub fn seal(key: &SealKey, context: &str, secret: &[u8]) -> Result<String, SealError> {
    if secret.len() > MAX_SECRET_LEN {
        return Err(SealError::TooLong);
    }
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|e| SealError::Random(e.to_string()))?;
    let boxed = key
        .cipher(context)
        .encrypt(Nonce::from_slice(&nonce), secret)
        .map_err(|_| SealError::Open)?;
    let mut raw = Vec::with_capacity(NONCE_LEN + boxed.len());
    raw.extend_from_slice(&nonce);
    raw.extend_from_slice(&boxed);
    Ok(format!(
        "{PREFIX}{}.{}",
        key.epoch,
        URL_SAFE_NO_PAD.encode(raw)
    ))
}

/// Epoch and raw bytes (nonce ‖ box) of a container.
fn parse(sealed: &str) -> Result<(u32, Vec<u8>), SealError> {
    if sealed.len() > MAX_SEALED_LEN {
        return Err(SealError::Malformed);
    }
    let rest = sealed.strip_prefix(PREFIX).ok_or(SealError::Malformed)?;
    let (epoch, data) = rest.split_once('.').ok_or(SealError::Malformed)?;
    if epoch.is_empty() || !epoch.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SealError::Malformed);
    }
    let epoch: u32 = epoch.parse().map_err(|_| SealError::Malformed)?;
    let raw = URL_SAFE_NO_PAD
        .decode(data)
        .map_err(|_| SealError::Malformed)?;
    if raw.len() < NONCE_LEN + TAG_LEN {
        return Err(SealError::Malformed);
    }
    Ok((epoch, raw))
}

/// Whether `text` has the form of a container (it is not opened).
pub fn is_sealed(text: &str) -> bool {
    parse(text).is_ok()
}

/// Epoch of the family key that sealed a container.
pub fn epoch_of(sealed: &str) -> Option<u32> {
    parse(sealed).ok().map(|(e, _)| e)
}

/// Opens a container sealed for `context` with `key`.
pub fn open(key: &SealKey, context: &str, sealed: &str) -> Result<Zeroizing<Vec<u8>>, SealError> {
    let (epoch, raw) = parse(sealed)?;
    if epoch != key.epoch {
        return Err(SealError::Epoch {
            found: epoch,
            have: key.epoch,
        });
    }
    let (nonce, boxed) = raw.split_at(NONCE_LEN);
    key.cipher(context)
        .decrypt(Nonce::from_slice(nonce), boxed)
        .map(Zeroizing::new)
        .map_err(|_| SealError::Open)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_binding() {
        let key = SealKey::new("fam", 3, &[9; 32]);
        let a = seal(&key, "source:a", b"pw").unwrap();
        let b = seal(&key, "source:a", b"pw").unwrap();
        assert_ne!(a, b, "a fresh nonce per seal");
        assert_eq!(epoch_of(&a), Some(3));
        assert_eq!(&*open(&key, "source:a", &a).unwrap(), b"pw");
        assert_eq!(open(&key, "source:b", &a), Err(SealError::Open));
        let other_family = SealKey::new("fam2", 3, &[9; 32]);
        assert_eq!(open(&other_family, "source:a", &a), Err(SealError::Open));
        let other_key = SealKey::new("fam", 3, &[8; 32]);
        assert_eq!(open(&other_key, "source:a", &a), Err(SealError::Open));
        let next = SealKey::new("fam", 4, &[9; 32]);
        assert_eq!(
            open(&next, "source:a", &a),
            Err(SealError::Epoch { found: 3, have: 4 })
        );
    }

    #[test]
    fn tampering_and_garbage_are_refused() {
        let key = SealKey::new("fam", 1, &[1; 32]);
        let sealed = seal(&key, "c", b"secret").unwrap();
        let mut raw = parse(&sealed).unwrap().1;
        let last = raw.len() - 1;
        raw[last] ^= 1;
        let tampered = format!("{PREFIX}1.{}", URL_SAFE_NO_PAD.encode(raw));
        assert_eq!(open(&key, "c", &tampered), Err(SealError::Open));
        for bad in ["", "sb1.", "sb1.x.AAAA", "sb1.1.AAAA", "plain", "sb1.1.!!"] {
            assert!(!is_sealed(bad), "{bad}");
        }
        assert_eq!(
            seal(&key, "c", &[0; MAX_SECRET_LEN + 1]),
            Err(SealError::TooLong)
        );
        assert!(!format!("{key:?}").contains("ikm"));
    }

    #[test]
    fn empty_secret_seals() {
        let key = SealKey::new("fam", 1, &[2; 32]);
        let sealed = seal(&key, "c", b"").unwrap();
        assert!(open(&key, "c", &sealed).unwrap().is_empty());
    }
}
