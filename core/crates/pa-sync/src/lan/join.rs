// SPDX-License-Identifier: MPL-2.0
//! Joining the family over the LAN without a code (SYNC-015, ADR-0159 in
//! `docs/adr/0159-family-join-lan.md`), sans-IO.
//!
//! A new device sees the family's devices on mDNS (`_alpha-sync._tcp`,
//! TXT `fam`) and asks to join: it announces a [`JoinAsk`] as a
//! `_alpha-pair._tcp` service with the TXT key [`TXT_JOIN`] (the family it
//! asks for), its device id, name, kind, its signing key and a nonce.
//! Every trusted device of that family shows the request with the device's
//! name, kind and [`short_code`]; the new device shows the same short code.
//! One "Allow" on any trusted device opens an ordinary pairing invitation
//! (`pa_pairing`, ADR-0026) for that device only (TXT [`TXT_FOR`]) with the
//! code [`JoinAsk::code`], and the new device joins it.
//!
//! The code is derived from public data, so here SPAKE2 is no secret: what
//! authenticates both sides is the device keys that each side signs the
//! pairing transcript with. The member admits only the key it showed
//! (pinned from the request the user allowed): a stranger on the LAN that
//! races to the invitation cannot sign as that key and gets `pair.denied`.
//! The new device keeps only a family with the id it asked for: the id is
//! the hash of the founder's key, and a roster of that id must be signed
//! by the founder's chain, so nobody can push another family onto it.
//! The short code ties the request on the member's screen to the device in
//! front of the user (32 bits of a hash of the device's key and the family
//! id: a lookalike request needs ~2^32 key generations within the
//! request's two minutes).
//!
//! ```
//! use pa_pairing::{DeviceIdentity, DeviceKind};
//! use pa_sync::lan::join::JoinAsk;
//!
//! let tv = DeviceIdentity::generate("ktv").unwrap();
//! let ask = JoinAsk::new(&tv, "TV", DeviceKind::Tv, "0123456789abcdef0123456789abcdef").unwrap();
//! let txt = ask.to_txt();
//! let seen = JoinAsk::from_txt(|k| txt.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())).unwrap();
//! assert_eq!(seen, ask);
//! assert_eq!(seen.short_code(), ask.short_code());
//! assert_eq!(seen.code().expose(), ask.code().expose());
//! ```

use pa_pairing::pair::PairingCode;
use pa_pairing::{DeviceIdentity, DeviceKind, check_name};
use sha2::{Digest, Sha256};

/// TXT key of a join request: the family id it asks for.
pub const TXT_JOIN: &str = "join";
/// TXT key of an invitation opened for one requesting device: its id.
pub const TXT_FOR: &str = "for";
/// TXT key: the requesting device's signing key (hex).
pub const TXT_KEY: &str = "pk";
/// TXT key: the request's nonce (hex).
pub const TXT_NONCE: &str = "n";
/// TXT key: the requesting device's kind.
pub const TXT_KIND: &str = "kind";
/// TXT key of a sync peer: its family id.
pub const TXT_FAMILY: &str = "fam";

/// How long a request stays shown (on the member, from when it was first
/// seen) and announced (on the new device).
pub const ASK_TTL_MS: u64 = 2 * 60 * 1000;
/// A declined device is not shown again for this long.
pub const DECLINE_COOLDOWN_MS: u64 = 10 * 60 * 1000;
/// Most requests a member shows at once; more are ignored (a flood).
pub const MAX_PENDING: usize = 4;
/// Port in the request's SRV record: nothing listens there (the new device
/// dials the member), DNS-SD only needs one. 9 is the discard port.
pub const ANNOUNCE_PORT: u16 = 9;

/// Why a request in a TXT record is not taken.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JoinAskError {
    #[error("join request without {0}")]
    Missing(&'static str),
    #[error("join request with a bad {0}")]
    Invalid(&'static str),
}

/// A new device's request to join a family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinAsk {
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// Its signing key: the member admits only this key.
    pub key: [u8; 32],
    /// The family it asks for.
    pub family_id: String,
    /// Fresh per request: a new request gets a new code.
    pub nonce: [u8; 16],
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn is_family_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn kind_of(text: &str) -> DeviceKind {
    match text {
        "phone" => DeviceKind::Phone,
        "tablet" => DeviceKind::Tablet,
        "tv" => DeviceKind::Tv,
        "desktop" => DeviceKind::Desktop,
        "server" => DeviceKind::Server,
        _ => DeviceKind::Other,
    }
}

fn digest(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for part in parts {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part);
    }
    h.finalize().into()
}

/// The short code both screens show: `XXXX-XXXX`, 32 bits of a hash of the
/// device's signing key and the family id.
pub fn short_code(key: &[u8; 32], family_id: &str) -> String {
    let h = digest(&[b"kinolune.join.short.v1", key, family_id.as_bytes()]);
    let hex = to_hex(&h[..4]).to_ascii_uppercase();
    format!("{}-{}", &hex[..4], &hex[4..])
}

impl JoinAsk {
    /// A fresh request of `identity` to join `family_id`.
    pub fn new(
        identity: &DeviceIdentity,
        name: &str,
        kind: DeviceKind,
        family_id: &str,
    ) -> Result<Self, JoinAskError> {
        check_name(name).map_err(|_| JoinAskError::Invalid("name"))?;
        if !is_family_id(family_id) {
            return Err(JoinAskError::Invalid("family"));
        }
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| JoinAskError::Invalid("nonce"))?;
        Ok(Self {
            device_id: identity.device_id().to_owned(),
            name: name.to_owned(),
            kind,
            key: identity.public().sign,
            family_id: family_id.to_owned(),
            nonce,
        })
    }

    /// TXT entries of the request's announcement.
    pub fn to_txt(&self) -> Vec<(String, String)> {
        vec![
            ("txtvers".into(), "1".into()),
            ("id".into(), self.device_id.clone()),
            ("name".into(), self.name.clone()),
            (TXT_KIND.into(), self.kind.as_str().into()),
            (TXT_JOIN.into(), self.family_id.clone()),
            (TXT_KEY.into(), to_hex(&self.key)),
            (TXT_NONCE.into(), to_hex(&self.nonce)),
        ]
    }

    /// A request from a TXT record (`get` looks a key up).
    pub fn from_txt<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Result<Self, JoinAskError> {
        let need = |k: &'static str| {
            get(k)
                .filter(|v| !v.is_empty())
                .ok_or(JoinAskError::Missing(k))
        };
        let device_id = need("id")?;
        if !pa_pairing::is_valid_device_id(device_id) {
            return Err(JoinAskError::Invalid("id"));
        }
        let name = need("name")?;
        check_name(name).map_err(|_| JoinAskError::Invalid("name"))?;
        let family_id = need(TXT_JOIN)?;
        if !is_family_id(family_id) {
            return Err(JoinAskError::Invalid("family"));
        }
        Ok(Self {
            device_id: device_id.to_owned(),
            name: name.to_owned(),
            kind: get(TXT_KIND).map_or(DeviceKind::Other, kind_of),
            key: from_hex(need(TXT_KEY)?).ok_or(JoinAskError::Invalid("key"))?,
            family_id: family_id.to_owned(),
            nonce: from_hex(need(TXT_NONCE)?).ok_or(JoinAskError::Invalid("nonce"))?,
        })
    }

    /// The short code to compare on both screens.
    pub fn short_code(&self) -> String {
        short_code(&self.key, &self.family_id)
    }

    /// The pairing code of the invitation for this request. Not secret
    /// (see the module docs): the keys authenticate, not the code.
    pub fn code(&self) -> PairingCode {
        let h = digest(&[
            b"kinolune.join.code.v1",
            &self.key,
            self.family_id.as_bytes(),
            &self.nonce,
        ]);
        PairingCode::parse(&to_hex(&h[..16])).expect("32 hex characters are a pairing code")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAM: &str = "00112233445566778899aabbccddeeff";

    fn txt_of(ask: &JoinAsk) -> Vec<(String, String)> {
        ask.to_txt()
    }

    fn parse(txt: &[(String, String)]) -> Result<JoinAsk, JoinAskError> {
        JoinAsk::from_txt(|k| txt.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()))
    }

    #[test]
    fn request_round_trips_and_codes_match() {
        let tv = DeviceIdentity::generate("ktv").unwrap();
        let ask = JoinAsk::new(&tv, "Living room TV", DeviceKind::Tv, FAM).unwrap();
        let back = parse(&txt_of(&ask)).unwrap();
        assert_eq!(back, ask);
        assert_eq!(back.code().expose(), ask.code().expose());
        let code = ask.short_code();
        assert_eq!(code.len(), 9);
        assert_eq!(&code[4..5], "-");
        // Another request of the same device: same short code, new code.
        let again = JoinAsk::new(&tv, "Living room TV", DeviceKind::Tv, FAM).unwrap();
        assert_eq!(again.short_code(), code);
        assert_ne!(again.code().expose(), ask.code().expose());
        // Another device or another family: another short code.
        let pc = DeviceIdentity::generate("kpc").unwrap();
        let other = JoinAsk::new(&pc, "PC", DeviceKind::Desktop, FAM).unwrap();
        assert_ne!(other.short_code(), code);
        assert_ne!(
            short_code(&ask.key, "ffeeddccbbaa99887766554433221100"),
            code
        );
    }

    #[test]
    fn bad_requests_are_refused() {
        let tv = DeviceIdentity::generate("ktv").unwrap();
        assert!(JoinAsk::new(&tv, "TV", DeviceKind::Tv, "not-a-family").is_err());
        assert!(JoinAsk::new(&tv, "", DeviceKind::Tv, FAM).is_err());
        let ask = JoinAsk::new(&tv, "TV", DeviceKind::Tv, FAM).unwrap();
        for (key, value) in [
            ("pk", "zz"),
            ("n", "00"),
            ("join", "ABC"),
            ("name", ""),
            ("id", "bad id with spaces"),
        ] {
            let txt: Vec<(String, String)> = txt_of(&ask)
                .into_iter()
                .map(|(k, v)| {
                    if k == key {
                        (k, value.to_owned())
                    } else {
                        (k, v)
                    }
                })
                .collect();
            assert!(parse(&txt).is_err(), "{key}");
        }
        let unknown_kind: Vec<(String, String)> = txt_of(&ask)
            .into_iter()
            .map(|(k, v)| {
                if k == "kind" {
                    (k, "toaster".into())
                } else {
                    (k, v)
                }
            })
            .collect();
        assert_eq!(parse(&unknown_kind).unwrap().kind, DeviceKind::Other);
    }
}
