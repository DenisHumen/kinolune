// SPDX-License-Identifier: MPL-2.0
//! Setting up a new device from a phone (UI-046), sans-IO.
//!
//! The new device (a TV, a computer, a second phone) shows a QR code
//! `kinolune://setup?v=1&c=<6 digits>[&n=<its name>]` and the same code as
//! digits for phones without a camera. A phone of the family scans it (or
//! the user types the code) and opens an ordinary pairing invitation
//! (`pa_pairing`, ADR-0026) **with that code**, announced on mDNS with the
//! TXT key `setup`. The new device looks for such invitations and joins
//! with its code: SPAKE2 proves that both sides know it, so nobody else on
//! the LAN can push a family onto the TV or take its place. Roles stay as
//! in UI-019: the phone, which has the family, invites; the new device,
//! which has nothing, joins.
//!
//! Right after `pair.welcome` the phone sends one more frame on the same
//! connection, [`SetupFrame`]: the setup bundle (sources with their
//! passwords, settings) sealed with the family key the welcome just
//! delivered ([`crate::seal::seal_large`], context `setup:<new device
//! id>`). The pairing TLS does not authenticate anybody, so nothing in the
//! frame is readable without the family key; a bundle moved to another
//! device does not open. The bundle never enters an op, the outbox, a
//! journal or a log; the receiver opens it once and drops it.
//!
//! ```
//! use pa_sync::seal::SealKey;
//! use pa_sync::setup::{self, SetupRequest};
//!
//! let request = SetupRequest::new("123456", Some("Living room TV")).unwrap();
//! let uri = request.to_uri();
//! assert_eq!(SetupRequest::parse(&uri).unwrap(), request);
//! assert_eq!(SetupRequest::parse("123 456").unwrap().code(), "123456");
//!
//! let key = SealKey::new("fam1", 1, &[7; 32]);
//! let frame = setup::seal_frame(&key, "ktv", br#"{"password":"hunter2"}"#).unwrap();
//! assert!(!frame.contains("hunter2"));
//! let opened = setup::open_frame(&key, "ktv", &frame).unwrap();
//! assert_eq!(&*opened, br#"{"password":"hunter2"}"#);
//! assert!(setup::open_frame(&key, "kother", &frame).is_err());
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};
/// Re-exported: the buffer the bundle travels in.
pub use zeroize::Zeroizing;

use crate::seal::{self, SealError, SealKey};

/// Prefix of the QR code a new device shows.
pub const SETUP_PREFIX: &str = "kinolune://setup?";

/// Version of the QR code and of the frame.
pub const SETUP_VERSION: u32 = 1;

/// TXT key that marks a pairing announcement as a setup invitation.
pub const TXT_SETUP: &str = "setup";

/// `t` of the frame with the bundle.
pub const FRAME_TYPE: &str = "setup.bundle";

/// Longest device name carried in the QR code, in bytes.
pub const MAX_NAME_LEN: usize = 64;

/// Why a setup code or frame is not accepted. Never carries a secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SetupError {
    /// Not a setup QR code or not a 6-digit code.
    #[error("not a setup code")]
    Invalid,
    /// A QR code of a newer version.
    #[error("setup code of another version")]
    Version,
    /// Not a frame with a bundle.
    #[error("not a setup frame")]
    Frame,
    /// The bundle does not open (another family, another device, damaged).
    #[error(transparent)]
    Seal(#[from] SealError),
}

/// What the new device shows: its one-time code and, for the phone's
/// screen, its name. `Debug` hides the code.
#[derive(Clone, PartialEq, Eq)]
pub struct SetupRequest {
    code: Zeroizing<String>,
    name: Option<String>,
}

impl fmt::Debug for SetupRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetupRequest")
            .field("code", &"***")
            .field("name", &self.name)
            .finish()
    }
}

/// The 6 digits of what a person typed: spaces and dashes are ignored.
fn digits(text: &str) -> Option<String> {
    let code: String = text
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '\u{a0}'))
        .collect();
    (code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit())).then_some(code)
}

fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode_component(s: &str) -> Option<String> {
    let raw = s.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Cuts `name` to [`MAX_NAME_LEN`] bytes on a character boundary.
fn short_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut end = name.len().min(MAX_NAME_LEN);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    Some(name[..end].to_owned())
}

impl SetupRequest {
    /// A request with `code` (6 digits) and the device's `name`.
    pub fn new(code: &str, name: Option<&str>) -> Result<Self, SetupError> {
        Ok(Self {
            code: Zeroizing::new(digits(code).ok_or(SetupError::Invalid)?),
            name: name.and_then(short_name),
        })
    }

    /// The 6 digits. Do not log them.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The new device's name, if the QR code had one.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The text of the QR code.
    pub fn to_uri(&self) -> String {
        let mut uri = format!("{SETUP_PREFIX}v={SETUP_VERSION}&c={}", &*self.code);
        if let Some(name) = &self.name {
            uri.push_str("&n=");
            uri.push_str(&encode_component(name));
        }
        uri
    }

    /// A scanned QR code or the 6 digits a person typed. Unknown
    /// parameters are ignored.
    pub fn parse(input: &str) -> Result<Self, SetupError> {
        let input = input.trim();
        let Some(query) = input.strip_prefix(SETUP_PREFIX) else {
            return Self::new(input, None);
        };
        let (mut v, mut code, mut name) = (None, None, None);
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            match key {
                "v" => v = Some(value),
                "c" => code = Some(value),
                "n" => name = decode_component(value),
                _ => {}
            }
        }
        if v != Some("1") {
            return Err(SetupError::Version);
        }
        Self::new(code.ok_or(SetupError::Invalid)?, name.as_deref())
    }

    /// Whether `text` is a setup QR code (not a pairing one).
    pub fn is_setup_uri(text: &str) -> bool {
        text.trim().starts_with(SETUP_PREFIX)
    }
}

/// The E2E context of the bundle for the new device `device_id`.
pub fn context(device_id: &str) -> String {
    format!("setup:{device_id}")
}

/// The frame after `pair.welcome`: `{"t":"setup.bundle","sealed":"sb1…"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupFrame {
    pub t: String,
    pub v: u32,
    /// The bundle sealed with the family key ([`seal::seal_large`]).
    pub sealed: String,
}

/// Seals `bundle` for the new device `device_id` and wraps it in a frame.
pub fn seal_frame(key: &SealKey, device_id: &str, bundle: &[u8]) -> Result<String, SetupError> {
    let sealed = seal::seal_large(key, &context(device_id), bundle)?;
    serde_json::to_string(&SetupFrame {
        t: FRAME_TYPE.to_owned(),
        v: SETUP_VERSION,
        sealed,
    })
    .map_err(|_| SetupError::Frame)
}

/// Opens a frame of [`seal_frame`] on the new device `device_id`.
pub fn open_frame(
    key: &SealKey,
    device_id: &str,
    frame: &str,
) -> Result<Zeroizing<Vec<u8>>, SetupError> {
    let frame: SetupFrame = serde_json::from_str(frame).map_err(|_| SetupError::Frame)?;
    if frame.t != FRAME_TYPE {
        return Err(SetupError::Frame);
    }
    if frame.v != SETUP_VERSION {
        return Err(SetupError::Version);
    }
    Ok(seal::open_large(key, &context(device_id), &frame.sealed)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_parse_from_qr_and_typing() {
        let r = SetupRequest::new("042 917", Some("ТВ в гостиной")).unwrap();
        assert_eq!(r.code(), "042917");
        let uri = r.to_uri();
        assert!(uri.starts_with(SETUP_PREFIX));
        assert!(SetupRequest::is_setup_uri(&uri));
        assert!(!SetupRequest::is_setup_uri("projectalpha://pair?v=1"));
        let back = SetupRequest::parse(&uri).unwrap();
        assert_eq!(back.name(), Some("ТВ в гостиной"));
        assert_eq!(back.code(), "042917");
        assert_eq!(SetupRequest::parse("042-917").unwrap().code(), "042917");
        for bad in ["", "12345", "1234567", "abcdef", "kinolune://setup?v=1"] {
            assert_eq!(SetupRequest::parse(bad), Err(SetupError::Invalid), "{bad}");
        }
        assert_eq!(
            SetupRequest::parse("kinolune://setup?v=2&c=123456"),
            Err(SetupError::Version)
        );
        assert!(!format!("{r:?}").contains("042917"));
    }

    #[test]
    fn long_names_are_cut_on_a_char_boundary() {
        let name = "ж".repeat(MAX_NAME_LEN);
        let r = SetupRequest::new("123456", Some(&name)).unwrap();
        assert!(r.name().unwrap().len() <= MAX_NAME_LEN);
        assert_eq!(SetupRequest::parse(&r.to_uri()).unwrap(), r);
    }

    #[test]
    fn frames_open_only_for_their_device_and_family() {
        let key = SealKey::new("fam", 2, &[5; 32]);
        let frame = seal_frame(&key, "ktv", b"secret bundle").unwrap();
        assert!(!frame.contains("secret"));
        assert_eq!(&*open_frame(&key, "ktv", &frame).unwrap(), b"secret bundle");
        assert!(open_frame(&key, "kphone", &frame).is_err());
        let other = SealKey::new("fam2", 2, &[5; 32]);
        assert!(open_frame(&other, "ktv", &frame).is_err());
        assert_eq!(
            open_frame(&key, "ktv", r#"{"t":"pair.hello","v":1,"sealed":""}"#),
            Err(SetupError::Frame)
        );
        assert_eq!(open_frame(&key, "ktv", "not json"), Err(SetupError::Frame));
    }
}
