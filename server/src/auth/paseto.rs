// SPDX-License-Identifier: MPL-2.0
//! PASETO `v4.public` tokens: Ed25519 signatures over the pre-authentication
//! encoding (PAE) of the header, message, footer and implicit assertion.
//!
//! The protocol is small enough to implement on `ed25519-dalek` (already in
//! the workspace for device keys) instead of pulling in a PASETO crate; the
//! official test vectors (`paseto-standard/test-vectors`, v4) check it.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};

/// Header of every token of this version and purpose.
pub const HEADER: &str = "v4.public.";

const SIG_LEN: usize = 64;

/// A token that is malformed or whose signature does not verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invalid;

/// Little-endian 64-bit length with the top bit cleared (PAE `LE64`).
fn le64(n: usize) -> [u8; 8] {
    ((n as u64) & (u64::MAX >> 1)).to_le_bytes()
}

/// Pre-authentication encoding of `pieces`.
fn pae(pieces: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + pieces.iter().map(|p| 8 + p.len()).sum::<usize>());
    out.extend_from_slice(&le64(pieces.len()));
    for piece in pieces {
        out.extend_from_slice(&le64(piece.len()));
        out.extend_from_slice(piece);
    }
    out
}

/// Signs `message` with an optional `footer` (sent in clear, signed) and an
/// `implicit` assertion (signed, not sent).
pub fn sign(key: &SigningKey, message: &[u8], footer: &[u8], implicit: &[u8]) -> String {
    let m2 = pae(&[HEADER.as_bytes(), message, footer, implicit]);
    let sig = key.sign(&m2);
    let mut body = Vec::with_capacity(message.len() + SIG_LEN);
    body.extend_from_slice(message);
    body.extend_from_slice(&sig.to_bytes());
    let mut token = String::from(HEADER);
    token.push_str(&URL_SAFE_NO_PAD.encode(body));
    if !footer.is_empty() {
        token.push('.');
        token.push_str(&URL_SAFE_NO_PAD.encode(footer));
    }
    token
}

/// Splits a token into the decoded body and footer, without verifying.
fn split(token: &str) -> Result<(Vec<u8>, Vec<u8>), Invalid> {
    let rest = token.strip_prefix(HEADER).ok_or(Invalid)?;
    let (body, footer) = match rest.split_once('.') {
        // An empty footer is encoded without the dot.
        Some((_, "")) => return Err(Invalid),
        Some((body, footer)) => (body, URL_SAFE_NO_PAD.decode(footer).map_err(|_| Invalid)?),
        None => (rest, Vec::new()),
    };
    let body = URL_SAFE_NO_PAD.decode(body).map_err(|_| Invalid)?;
    if body.len() < SIG_LEN {
        return Err(Invalid);
    }
    Ok((body, footer))
}

/// The footer of a token, **unverified** (to pick the key by its id).
pub fn footer(token: &str) -> Result<Vec<u8>, Invalid> {
    split(token).map(|(_, footer)| footer)
}

/// Verifies `token` with `key` and the `implicit` assertion; returns the
/// message and the footer.
pub fn verify(
    key: &VerifyingKey,
    token: &str,
    implicit: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), Invalid> {
    let (mut body, footer) = split(token)?;
    let sig: [u8; SIG_LEN] = body[body.len() - SIG_LEN..]
        .try_into()
        .map_err(|_| Invalid)?;
    body.truncate(body.len() - SIG_LEN);
    let m2 = pae(&[HEADER.as_bytes(), &body, &footer, implicit]);
    key.verify_strict(&m2, &Signature::from_bytes(&sig))
        .map_err(|_| Invalid)?;
    Ok((body, footer))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex<const N: usize>(s: &str) -> [u8; N] {
        let bytes: Vec<u8> = (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect();
        bytes.try_into().unwrap()
    }

    const SEED: &str = "b4cbfb43df4ce210727d953e4a713307fa19bb7d9f85041438d9e11b942a3774";
    const PUBLIC: &str = "1eb9dbbbbc047c03fd70604e0071f0987e16b28b757225c11f00415d0e20b1a2";
    const PAYLOAD: &str =
        r#"{"data":"this is a signed message","exp":"2022-01-01T00:00:00+00:00"}"#;
    const FOOTER: &str = r#"{"kid":"zVhMiPBP9fRf2snEcT7gFTioeA9COcNy9DfgL1W60haN"}"#;

    /// `paseto-standard/test-vectors` v4.json: 4-S-1, 4-S-2, 4-S-3.
    const VECTORS: [(&str, &str, &str); 3] = [
        (
            "",
            "",
            "v4.public.eyJkYXRhIjoidGhpcyBpcyBhIHNpZ25lZCBtZXNzYWdlIiwiZXhwIjoiMjAyMi0wMS0wMVQwMDowMDowMCswMDowMCJ9bg_XBBzds8lTZShVlwwKSgeKpLT3yukTw6JUz3W4h_ExsQV-P0V54zemZDcAxFaSeef1QlXEFtkqxT1ciiQEDA",
        ),
        (
            FOOTER,
            "",
            "v4.public.eyJkYXRhIjoidGhpcyBpcyBhIHNpZ25lZCBtZXNzYWdlIiwiZXhwIjoiMjAyMi0wMS0wMVQwMDowMDowMCswMDowMCJ9v3Jt8mx_TdM2ceTGoqwrh4yDFn0XsHvvV_D0DtwQxVrJEBMl0F2caAdgnpKlt4p7xBnx1HcO-SPo8FPp214HDw.eyJraWQiOiJ6VmhNaVBCUDlmUmYyc25FY1Q3Z0ZUaW9lQTlDT2NOeTlEZmdMMVc2MGhhTiJ9",
        ),
        (
            FOOTER,
            r#"{"test-vector":"4-S-3"}"#,
            "v4.public.eyJkYXRhIjoidGhpcyBpcyBhIHNpZ25lZCBtZXNzYWdlIiwiZXhwIjoiMjAyMi0wMS0wMVQwMDowMDowMCswMDowMCJ9NPWciuD3d0o5eXJXG5pJy-DiVEoyPYWs1YSTwWHNJq6DZD3je5gf-0M4JR9ipdUSJbIovzmBECeaWmaqcaP0DQ.eyJraWQiOiJ6VmhNaVBCUDlmUmYyc25FY1Q3Z0ZUaW9lQTlDT2NOeTlEZmdMMVc2MGhhTiJ9",
        ),
    ];

    #[test]
    fn official_vectors() {
        let key = SigningKey::from_bytes(&unhex::<32>(SEED));
        let public = VerifyingKey::from_bytes(&unhex::<32>(PUBLIC)).unwrap();
        assert_eq!(key.verifying_key(), public);
        for (footer, implicit, token) in VECTORS {
            assert_eq!(
                sign(&key, PAYLOAD.as_bytes(), footer.as_bytes(), implicit.as_bytes()),
                token
            );
            let (message, got_footer) = verify(&public, token, implicit.as_bytes()).unwrap();
            assert_eq!(message, PAYLOAD.as_bytes());
            assert_eq!(got_footer, footer.as_bytes());
            assert_eq!(super::footer(token).unwrap(), footer.as_bytes());
        }
    }

    #[test]
    fn rejects_tampering() {
        let key = SigningKey::from_bytes(&unhex::<32>(SEED));
        let public = key.verifying_key();
        let token = sign(&key, b"{\"a\":1}", b"{\"kid\":\"x\"}", b"");
        assert!(verify(&public, &token, b"").is_ok());
        // Wrong implicit assertion, other key, changed footer, other purpose.
        assert_eq!(verify(&public, &token, b"other"), Err(Invalid));
        let other = SigningKey::from_bytes(&[7; 32]).verifying_key();
        assert_eq!(verify(&other, &token, b""), Err(Invalid));
        let (body, _) = token.rsplit_once('.').unwrap();
        let swapped = format!("{body}.{}", URL_SAFE_NO_PAD.encode(b"{\"kid\":\"y\"}"));
        assert_eq!(verify(&public, &swapped, b""), Err(Invalid));
        assert_eq!(verify(&public, &format!("{body}."), b""), Err(Invalid));
        let local = token.replacen("v4.public.", "v4.local.", 1);
        assert_eq!(verify(&public, &local, b""), Err(Invalid));
        // A flipped bit in the message.
        let mut chars: Vec<char> = token.chars().collect();
        let i = HEADER.len() + 2;
        chars[i] = if chars[i] == 'A' { 'B' } else { 'A' };
        let flipped: String = chars.into_iter().collect();
        assert_eq!(verify(&public, &flipped, b""), Err(Invalid));
        assert_eq!(verify(&public, "v4.public.", b""), Err(Invalid));
        assert_eq!(verify(&public, "garbage", b""), Err(Invalid));
    }
}
