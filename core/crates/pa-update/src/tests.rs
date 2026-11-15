// SPDX-License-Identifier: MPL-2.0
//! Every key here is a THROWAWAY TEST KEY generated inside the test run; no
//! key material exists in the repository.

use std::fs;
use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use blake2::{Blake2b512, Digest as _};
use ed25519_dalek::{Signer, SigningKey};

use super::*;

const ASSET: &str = "kinolune-0.1.0-alpha.7-x86_64.AppImage";

/// A throwaway minisign key pair producing what `minisign -S` writes.
struct TestSigner {
    sk: SigningKey,
    key_id: [u8; 8],
}

impl TestSigner {
    fn generate() -> Self {
        let mut seed = [0u8; 32];
        let mut key_id = [0u8; 8];
        getrandom::fill(&mut seed).unwrap();
        getrandom::fill(&mut key_id).unwrap();
        Self {
            sk: SigningKey::from_bytes(&seed),
            key_id,
        }
    }

    fn public_b64(&self) -> String {
        let mut bin = b"Ed".to_vec();
        bin.extend_from_slice(&self.key_id);
        bin.extend_from_slice(&self.sk.verifying_key().to_bytes());
        STANDARD.encode(bin)
    }

    fn sign_file(&self, body: &[u8], file: &str) -> String {
        let hash = Blake2b512::digest(body);
        let signature = self.sk.sign(hash.as_slice()).to_bytes();
        let mut bin = b"ED".to_vec();
        bin.extend_from_slice(&self.key_id);
        bin.extend_from_slice(&signature);
        let trusted = format!("timestamp:1763078400\tfile:{file}\thashed");
        let mut global = signature.to_vec();
        global.extend_from_slice(trusted.as_bytes());
        let global_signature = self.sk.sign(&global).to_bytes();
        format!(
            "untrusted comment: signature from a throwaway test key\n{}\ntrusted comment: {}\n{}\n",
            STANDARD.encode(bin),
            trusted,
            STANDARD.encode(global_signature)
        )
    }

    fn sign(&self, body: &[u8]) -> String {
        self.sign_file(body, SUMS_FILE)
    }
}

/// A fake release: one asset on disk and its `SHA256SUMS`.
struct Release {
    _dir: tempfile::TempDir,
    asset: PathBuf,
    sums: Vec<u8>,
}

fn release(content: &[u8]) -> Release {
    let dir = tempfile::tempdir().unwrap();
    let asset = dir.path().join(ASSET);
    fs::write(&asset, content).unwrap();
    let sums = format!(
        "{}  kinolune-0.1.0-alpha.7-android-x86_64.apk\n{}  {ASSET}\n",
        "ab".repeat(32),
        encode_hex(&sha256_file(&asset).unwrap()),
    );
    Release {
        _dir: dir,
        asset,
        sums: sums.into_bytes(),
    }
}

fn payload() -> Vec<u8> {
    (0..3 * 1024 * 1024 + 17).map(|i| (i % 251) as u8).collect()
}

#[test]
fn intact_download_passes() {
    let r = release(&payload());
    let v = verify_artifact(&r.asset, ASSET, &r.sums, None, None).unwrap();
    assert_eq!(v.signature, Signature::NotRequired);
    assert_eq!(v.sha256.len(), 64);
}

#[test]
fn tampered_download_is_rejected() {
    let r = release(&payload());
    // One flipped byte in the middle of the file.
    let mut bytes = fs::read(&r.asset).unwrap();
    bytes[1_500_000] ^= 0x01;
    fs::write(&r.asset, &bytes).unwrap();
    let err = verify_artifact(&r.asset, ASSET, &r.sums, None, None).unwrap_err();
    assert!(
        matches!(err, Error::Mismatch { ref name, .. } if name == ASSET),
        "{err}"
    );
}

#[test]
fn truncated_or_empty_download_is_rejected() {
    let r = release(&payload());
    let bytes = fs::read(&r.asset).unwrap();
    fs::write(&r.asset, &bytes[..bytes.len() / 2]).unwrap();
    assert!(matches!(
        verify_artifact(&r.asset, ASSET, &r.sums, None, None),
        Err(Error::Mismatch { .. })
    ));
    fs::write(&r.asset, b"").unwrap();
    assert!(matches!(
        verify_artifact(&r.asset, ASSET, &r.sums, None, None),
        Err(Error::Mismatch { .. })
    ));
}

#[test]
fn an_asset_missing_from_the_sums_is_rejected() {
    let r = release(b"x");
    let err = verify_artifact(&r.asset, "kinolune-evil.AppImage", &r.sums, None, None).unwrap_err();
    assert!(matches!(err, Error::NotListed(_)), "{err}");
}

#[test]
fn a_missing_file_is_an_io_error() {
    let r = release(b"x");
    let gone = r.asset.with_extension("gone");
    assert!(matches!(
        verify_artifact(&gone, ASSET, &r.sums, None, None),
        Err(Error::Io(_))
    ));
}

#[test]
fn sums_parse_like_sha256sum_output() {
    let a = "0".repeat(64);
    let b = "F".repeat(64);
    let s = Sha256Sums::parse(format!("{a}  one.apk\r\n\n{b} *two.AppImage\n").as_bytes()).unwrap();
    assert_eq!(s.names().collect::<Vec<_>>(), ["one.apk", "two.AppImage"]);
    assert_eq!(s.get("two.AppImage"), Some(&[0xff; 32]));
    for bad in [
        String::new(),
        "\n\n".into(),
        format!("{a} one.apk"),
        format!("{}  one.apk", "0".repeat(63)),
        format!("{}  one.apk", "g".repeat(64)),
        format!("{a}  "),
        format!("{a}  x\n{b}  x\n"),
    ] {
        assert!(
            matches!(Sha256Sums::parse(bad.as_bytes()), Err(Error::Malformed(_))),
            "{bad:?}"
        );
    }
    assert!(Sha256Sums::parse(&[0xff, 0xfe]).is_err());
    assert!(Sha256Sums::parse(&vec![b'\n'; MAX_SUMS_BYTES + 1]).is_err());
}

#[test]
fn without_a_release_key_the_signature_is_not_checked() {
    // The hook stays a no-op until the owner publishes a key (ADR-NNNN).
    let r = release(b"payload");
    for sig in [None, Some(""), Some("garbage")] {
        let v = verify_artifact(&r.asset, ASSET, &r.sums, sig, None).unwrap();
        assert_eq!(v.signature, Signature::NotRequired);
        let v = verify_artifact(&r.asset, ASSET, &r.sums, sig, Some("  ")).unwrap();
        assert_eq!(v.signature, Signature::NotRequired);
    }
}

#[test]
fn with_a_release_key_a_valid_signature_is_required() {
    let signer = TestSigner::generate();
    let key = signer.public_b64();
    let r = release(&payload());
    let sig = signer.sign(&r.sums);
    let v = verify_artifact(&r.asset, ASSET, &r.sums, Some(&sig), Some(&key)).unwrap();
    assert_eq!(v.signature, Signature::Verified);

    // No signature.
    assert!(matches!(
        verify_artifact(&r.asset, ASSET, &r.sums, None, Some(&key)),
        Err(Error::SignatureMissing)
    ));
    // Signed by somebody else.
    let other = TestSigner::generate().sign(&r.sums);
    assert!(matches!(
        verify_artifact(&r.asset, ASSET, &r.sums, Some(&other), Some(&key)),
        Err(Error::BadSignature(_))
    ));
    // Checksums edited after signing (a forged release pointing at another
    // file): the signature no longer matches.
    let mut forged = r.sums.clone();
    forged[0] = if forged[0] == b'0' { b'1' } else { b'0' };
    assert!(matches!(
        verify_artifact(&r.asset, ASSET, &forged, Some(&sig), Some(&key)),
        Err(Error::BadSignature(_))
    ));
    // A valid signature of another file.
    let wrong_file = signer.sign_file(&r.sums, "remote-config.json");
    assert!(matches!(
        verify_artifact(&r.asset, ASSET, &r.sums, Some(&wrong_file), Some(&key)),
        Err(Error::BadSignature(_))
    ));
    // A good signature does not excuse a tampered file.
    let mut bytes = fs::read(&r.asset).unwrap();
    bytes[10] ^= 0x80;
    fs::write(&r.asset, &bytes).unwrap();
    assert!(matches!(
        verify_artifact(&r.asset, ASSET, &r.sums, Some(&sig), Some(&key)),
        Err(Error::Mismatch { .. })
    ));
}

#[test]
fn a_broken_release_key_fails_closed() {
    let r = release(b"x");
    assert!(matches!(
        verify_artifact(&r.asset, ASSET, &r.sums, None, Some("RWnotakey")),
        Err(Error::BadKey(_))
    ));
}
