// SPDX-License-Identifier: MPL-2.0
//! The cryptography of MEGA's documented protocol: AES-128 in ECB for keys,
//! CBC with a zero IV for node attributes, CTR for file contents; the
//! password key (PBKDF2-HMAC-SHA512 for v2 accounts, the AES-based
//! `prepare_key` for v1); RSA decryption of the session id.

use aes::Aes128;
use aes::cipher::{
    BlockCipherDecrypt, BlockCipherEncrypt, KeyInit, KeyIvInit, StreamCipher, StreamCipherSeek,
};
use base64::Engine;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use num_bigint::BigUint;
use pa_vfs::{ErrorKind, VfsError};

type Ctr = ctr::Ctr128BE<Aes128>;

/// MEGA's base64: URL alphabet, no padding (padding and `+/` tolerated).
const B64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

fn bad(what: &str) -> VfsError {
    VfsError::new(ErrorKind::Other, format!("MEGA: malformed {what}"))
}

/// Encodes bytes in MEGA's base64.
#[must_use]
pub fn b64_encode(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

/// Decodes MEGA's base64.
///
/// # Errors
///
/// Not base64.
pub fn b64_decode(text: &str) -> pa_vfs::Result<Vec<u8>> {
    let t: String = text
        .trim()
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            c => c,
        })
        .filter(|c| *c != ',')
        .collect();
    B64.decode(t.as_bytes()).map_err(|_| bad("base64"))
}

fn cipher(key: &[u8; 16]) -> Aes128 {
    Aes128::new(&(*key).into())
}

/// AES-128 encryption of one block.
#[must_use]
pub fn aes_encrypt_block(key: &[u8; 16], block: &[u8; 16]) -> [u8; 16] {
    let mut b = (*block).into();
    cipher(key).encrypt_block(&mut b);
    b.into()
}

/// AES-128 decryption of one block.
#[must_use]
pub fn aes_decrypt_block(key: &[u8; 16], block: &[u8; 16]) -> [u8; 16] {
    let mut b = (*block).into();
    cipher(key).decrypt_block(&mut b);
    b.into()
}

fn blocks(data: &[u8], what: &str) -> pa_vfs::Result<()> {
    if data.is_empty() || !data.len().is_multiple_of(16) {
        return Err(bad(what));
    }
    Ok(())
}

/// AES-128-ECB decryption (node keys, the master key).
///
/// # Errors
///
/// `data` is not whole blocks.
pub fn ecb_decrypt(key: &[u8; 16], data: &[u8]) -> pa_vfs::Result<Vec<u8>> {
    blocks(data, "key")?;
    let c = cipher(key);
    let mut out = data.to_vec();
    for chunk in out.as_chunks_mut::<16>().0 {
        let mut b = aes::Block::from(*chunk);
        c.decrypt_block(&mut b);
        chunk.copy_from_slice(&b);
    }
    Ok(out)
}

/// AES-128-ECB encryption (tests and the key check of a temporary session).
///
/// # Errors
///
/// `data` is not whole blocks.
pub fn ecb_encrypt(key: &[u8; 16], data: &[u8]) -> pa_vfs::Result<Vec<u8>> {
    blocks(data, "key")?;
    let c = cipher(key);
    let mut out = data.to_vec();
    for chunk in out.as_chunks_mut::<16>().0 {
        let mut b = aes::Block::from(*chunk);
        c.encrypt_block(&mut b);
        chunk.copy_from_slice(&b);
    }
    Ok(out)
}

/// AES-128-CBC decryption with a zero IV (node attributes).
///
/// # Errors
///
/// `data` is not whole blocks.
pub fn cbc_decrypt(key: &[u8; 16], data: &[u8]) -> pa_vfs::Result<Vec<u8>> {
    blocks(data, "attributes")?;
    let c = cipher(key);
    let mut prev = [0u8; 16];
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.as_chunks::<16>().0 {
        let mut b = aes::Block::from(*chunk);
        c.decrypt_block(&mut b);
        out.extend(b.iter().zip(prev).map(|(x, p)| x ^ p));
        prev = *chunk;
    }
    Ok(out)
}

/// AES-128-CBC encryption with a zero IV, zero padding (tests).
#[must_use]
pub fn cbc_encrypt(key: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let c = cipher(key);
    let mut padded = data.to_vec();
    padded.resize(data.len().div_ceil(16).max(1) * 16, 0);
    let mut prev = [0u8; 16];
    for chunk in padded.as_chunks_mut::<16>().0 {
        for (x, p) in chunk.iter_mut().zip(prev) {
            *x ^= p;
        }
        let mut b = aes::Block::from(*chunk);
        c.encrypt_block(&mut b);
        chunk.copy_from_slice(&b);
        prev = *chunk;
    }
    padded
}

/// XORs `data` with the AES-128-CTR keystream of a file at byte `offset`.
/// MEGA's counter block is the 8-byte nonce of the file key followed by
/// the 64-bit big-endian block number (`offset / 16`); an offset inside a
/// block skips into its keystream. Encryption and decryption are the same.
pub fn ctr_xor(key: &[u8; 16], nonce: &[u8; 8], offset: u64, data: &mut [u8]) {
    let mut iv = [0u8; 16];
    iv[..8].copy_from_slice(nonce);
    let mut c = Ctr::new(&(*key).into(), &iv.into());
    c.seek(offset);
    c.apply_keystream(data);
}

/// XORs `data` with the CTR keystream of a full 16-byte counter block
/// `iv` (the general form, for standard test vectors).
pub fn ctr_xor_iv(key: &[u8; 16], iv: &[u8; 16], data: &mut [u8]) {
    let mut c = Ctr::new(&(*key).into(), &(*iv).into());
    c.apply_keystream(data);
}

/// The AES key and CTR nonce of a decrypted 32-byte file key.
#[must_use]
pub fn file_key_parts(k: &[u8; 32]) -> ([u8; 16], [u8; 8]) {
    let mut key = [0u8; 16];
    for i in 0..16 {
        key[i] = k[i] ^ k[i + 16];
    }
    let mut nonce = [0u8; 8];
    nonce.copy_from_slice(&k[16..24]);
    (key, nonce)
}

/// The attribute key of a node key: a file key folds into 16 bytes, a
/// folder key is used as is.
///
/// # Errors
///
/// A key of another length.
pub fn node_aes_key(k: &[u8]) -> pa_vfs::Result<[u8; 16]> {
    match k.len() {
        16 => Ok(k.try_into().map_err(|_| bad("key"))?),
        32 => {
            let k32: [u8; 32] = k.try_into().map_err(|_| bad("key"))?;
            Ok(file_key_parts(&k32).0)
        }
        _ => Err(bad("node key")),
    }
}

/// The JSON of a node's attributes (`MEGA{"n":"name"}` + zero padding).
///
/// # Errors
///
/// Wrong key or damaged attributes.
pub fn decrypt_attributes(key: &[u8; 16], at: &str) -> pa_vfs::Result<serde_json::Value> {
    let plain = cbc_decrypt(key, &b64_decode(at)?)?;
    let text = plain
        .strip_prefix(b"MEGA")
        .ok_or_else(|| bad("attributes (wrong key)"))?;
    let end = text.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
    serde_json::from_slice(&text[..end]).map_err(|_| bad("attributes"))
}

/// Encrypts attributes the way MEGA stores them (tests).
#[must_use]
pub fn encrypt_attributes(key: &[u8; 16], attrs: &serde_json::Value) -> String {
    let mut plain = b"MEGA".to_vec();
    plain.extend(attrs.to_string().bytes());
    b64_encode(&cbc_encrypt(key, &plain))
}

/// Rounds of PBKDF2 of v2 accounts.
pub const PBKDF2_ROUNDS: u32 = 100_000;

/// The password key and the user hash (`uh`) of a v2 account from its
/// password and the salt of `us0`.
#[must_use]
pub fn v2_keys(password: &str, salt: &[u8]) -> ([u8; 16], String) {
    let mut dk = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha512>(password.as_bytes(), salt, PBKDF2_ROUNDS, &mut dk);
    let mut pw = [0u8; 16];
    pw.copy_from_slice(&dk[..16]);
    (pw, b64_encode(&dk[16..]))
}

/// The password key of a v1 account (`prepare_key` of the web client):
/// 65 536 rounds of AES over a fixed block, keyed by the password's
/// 16-byte chunks.
#[must_use]
pub fn v1_password_key(password: &str) -> [u8; 16] {
    let mut bytes = password.as_bytes().to_vec();
    bytes.resize(bytes.len().div_ceil(16).max(1) * 16, 0);
    let keys: Vec<Aes128> = bytes
        .as_chunks::<16>()
        .0
        .iter()
        .map(|c| Aes128::new(&(*c).into()))
        .collect();
    let mut pkey: aes::Block = [
        0x93, 0xC4, 0x67, 0xE3, 0x7D, 0xB0, 0xC7, 0xA4, 0xD1, 0xBE, 0x3F, 0x81, 0x01, 0x52, 0xCB,
        0x56,
    ]
    .into();
    for _ in 0..65_536 {
        for k in &keys {
            k.encrypt_block(&mut pkey);
        }
    }
    pkey.into()
}

/// The user hash of a v1 account (`stringhash` of the lower-case e-mail
/// under the password key).
#[must_use]
pub fn v1_user_hash(email: &str, pw_key: &[u8; 16]) -> String {
    let email = email.trim().to_lowercase();
    let mut h = [0u8; 16];
    for (i, b) in email.bytes().enumerate() {
        h[i % 16] ^= b;
    }
    let c = cipher(pw_key);
    let mut block: aes::Block = h.into();
    for _ in 0..16_384 {
        c.encrypt_block(&mut block);
    }
    let mut out = Vec::with_capacity(8);
    out.extend_from_slice(&block[0..4]);
    out.extend_from_slice(&block[8..12]);
    b64_encode(&out)
}

/// Reads one MPI (2-byte bit length, then the bytes) from `data`.
fn mpi(data: &[u8]) -> pa_vfs::Result<(BigUint, &[u8])> {
    if data.len() < 2 {
        return Err(bad("RSA key"));
    }
    let bits = usize::from(u16::from_be_bytes([data[0], data[1]]));
    let len = bits.div_ceil(8);
    let rest = &data[2..];
    if rest.len() < len {
        return Err(bad("RSA key"));
    }
    Ok((BigUint::from_bytes_be(&rest[..len]), &rest[len..]))
}

/// Writes one MPI (tests).
#[must_use]
pub fn mpi_bytes(n: &BigUint) -> Vec<u8> {
    let bits = u16::try_from(n.bits()).unwrap_or(u16::MAX);
    let mut out = bits.to_be_bytes().to_vec();
    out.extend(n.to_bytes_be());
    out
}

/// The session id of a full login: `csid` decrypted with the account's
/// RSA private key (`privk`, already AES-decrypted: MPIs p, q, d, u).
/// The id is the first 43 bytes of the plaintext.
///
/// The user's own key decrypts the user's own session on the user's
/// device, so the timing of the big-number arithmetic exposes nothing to
/// anyone else.
///
/// # Errors
///
/// A damaged key or `csid`.
pub fn decrypt_csid(privk: &[u8], csid: &str) -> pa_vfs::Result<String> {
    let (p, rest) = mpi(privk)?;
    let (q, rest) = mpi(rest)?;
    let (d, _) = mpi(rest)?;
    let raw = b64_decode(csid)?;
    let (c, _) = mpi(&raw)?;
    let n = &p * &q;
    if n.bits() == 0 || d.bits() == 0 {
        return Err(bad("RSA key"));
    }
    let m = c.modpow(&d, &n).to_bytes_be();
    if m.len() < 43 {
        return Err(bad("session"));
    }
    Ok(b64_encode(&m[..43]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn k16(s: &str) -> [u8; 16] {
        hex(s).try_into().unwrap()
    }

    /// NIST SP 800-38A F.5.1: AES-128-CTR.
    #[test]
    fn ctr_nist_vector() {
        let key = k16("2b7e151628aed2a6abf7158809cf4f3c");
        let iv = k16("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff");
        let mut data = hex(
            "6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51\
             30c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710",
        );
        ctr_xor_iv(&key, &iv, &mut data);
        assert_eq!(
            data,
            hex(
                "874d6191b620e3261bef6864990db6ce9806f66b7970fdff8617187bb9fffdff\
                 5ae4df3edbd5d35e5b4f09020db03eab1e031dda2fbe03d1792170a0f3009cee"
            )
        );
    }

    /// NIST SP 800-38A F.1.1 / F.1.2: AES-128-ECB.
    #[test]
    fn ecb_nist_vector() {
        let key = k16("2b7e151628aed2a6abf7158809cf4f3c");
        let plain = hex("6bc1bee22e409f96e93d7e117393172a");
        let cipher = hex("3ad77bb40d7a3660a89ecaf32466ef97");
        assert_eq!(ecb_encrypt(&key, &plain).unwrap(), cipher);
        assert_eq!(ecb_decrypt(&key, &cipher).unwrap(), plain);
    }

    /// A MEGA counter starts at zero at offset 0: block `n` of the file
    /// uses counter block `nonce || n`, and any offset decrypts the same
    /// bytes as decrypting from the start.
    #[test]
    fn mega_ctr_at_any_offset() {
        let key = k16("000102030405060708090a0b0c0d0e0f");
        let nonce: [u8; 8] = hex("a0a1a2a3a4a5a6a7").try_into().unwrap();
        let plain: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 256) as u8).collect();
        let mut whole = plain.clone();
        ctr_xor(&key, &nonce, 0, &mut whole);
        // Counter block 3 = nonce || 00..03.
        let mut iv = [0u8; 16];
        iv[..8].copy_from_slice(&nonce);
        iv[15] = 3;
        let mut block3 = plain[48..64].to_vec();
        ctr_xor_iv(&key, &iv, &mut block3);
        assert_eq!(block3, whole[48..64]);
        for (off, len) in [(0, 1), (5, 30), (16, 16), (17, 200), (999, 1), (321, 679)] {
            let mut part = whole[off..off + len].to_vec();
            ctr_xor(&key, &nonce, off as u64, &mut part);
            assert_eq!(part, plain[off..off + len], "offset {off}");
        }
    }

    #[test]
    fn attributes_round_trip() {
        let key = k16("00112233445566778899aabbccddeeff");
        let a = serde_json::json!({"n": "Фильм (2021).mkv"});
        let enc = encrypt_attributes(&key, &a);
        assert_eq!(decrypt_attributes(&key, &enc).unwrap(), a);
        let other = k16("ffffffffffffffffffffffffffffffff");
        assert!(decrypt_attributes(&other, &enc).is_err());
    }

    /// PBKDF2-HMAC-SHA512, "password"/"salt", 1 round, first 32 bytes
    /// (published test vector), checked through the same primitive.
    #[test]
    fn pbkdf2_sha512_vector() {
        let mut dk = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<sha2::Sha512>(b"password", b"salt", 1, &mut dk);
        assert_eq!(
            dk.to_vec(),
            hex("867f70cf1ade02cff3752599a3a53dc4af34c7a669815ae5d513554e1c8cf252")
        );
    }

    #[test]
    fn file_key_folds() {
        let mut k = [0u8; 32];
        k[0] = 0xff;
        k[16] = 0x0f;
        k[17] = 0xaa;
        let (key, nonce) = file_key_parts(&k);
        assert_eq!(key[0], 0xf0);
        assert_eq!(key[1], 0xaa);
        assert_eq!(nonce[1], 0xaa);
    }

    #[test]
    fn base64_variants() {
        assert_eq!(b64_decode("-_8").unwrap(), vec![0xfb, 0xff]);
        assert_eq!(b64_decode("+/8=").unwrap(), vec![0xfb, 0xff]);
        assert_eq!(b64_encode(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn csid_is_decrypted_with_the_private_key() {
        // Two Mersenne primes, 2^521 − 1 and 2^607 − 1; e = 65537 is
        // coprime to both p − 1 and q − 1 (the order of 2 mod 65537 is 32).
        let one = BigUint::from(1u32);
        let p = (&one << 521u32) - &one;
        let q = (&one << 607u32) - &one;
        let n = &p * &q;
        let phi = (&p - &one) * (&q - &one);
        let e = BigUint::from(65_537u32);
        let d = e.modinv(&phi).unwrap();
        let session: Vec<u8> = (1..=64u8).collect();
        let c = BigUint::from_bytes_be(&session).modpow(&e, &n);
        let mut privk = mpi_bytes(&p);
        privk.extend(mpi_bytes(&q));
        privk.extend(mpi_bytes(&d));
        privk.extend(mpi_bytes(&one)); // u, unused
        let csid = b64_encode(&mpi_bytes(&c));
        assert_eq!(
            decrypt_csid(&privk, &csid).unwrap(),
            b64_encode(&session[..43])
        );
        assert!(decrypt_csid(&privk[..10], &csid).is_err());
    }
}
