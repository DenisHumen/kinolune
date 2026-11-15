// SPDX-License-Identifier: MPL-2.0
//! Typing on a TV from a phone (UI-027): the `input` message, sans-IO.
//!
//! When a text field takes the focus on a TV, the TV sends an `input`
//! frame with [`InputAction::Request`] to its family. A phone answers with
//! [`InputAction::Open`] (its sheet is up: the TV shows who types), then
//! [`InputAction::Text`] as the user types, and [`InputAction::Submit`] at
//! the end. Either side ends the session with [`InputAction::Cancel`].
//!
//! Like `pb.*`, the frames are ephemeral: relays fan them out and never
//! store them in a journal or an outbox. The typed text never travels in
//! clear: every frame that carries text carries it sealed
//! ([`crate::seal`], the family key) for the context `input:<session>`, so
//! a hub that is not in the family (the user's own server, SYNC-008) only
//! relays it, and a text frame moved into another session does not open.
//! A secret field (a password, a PIN) is marked [`InputFrame::secret`]:
//! apps never log it and never keep it after the session.
//!
//! ```
//! use pa_sync::hlc::Hlc;
//! use pa_sync::input::{self, InputAction, InputFrame};
//! use pa_sync::seal::SealKey;
//!
//! let key = SealKey::new("fam1", 1, &[7; 32]);
//! let hlc = Hlc { wall_ms: 1, counter: 0, device: "phone".into() };
//! let mut frame = InputFrame::new("s1", InputAction::Text, hlc);
//! frame.secret = true;
//! frame.sealed = Some(input::seal_text(&key, "s1", "hunter2").unwrap());
//! frame.validate().unwrap();
//! assert!(!serde_json::to_string(&frame).unwrap().contains("hunter2"));
//! assert_eq!(&*input::open_text(&key, &frame).unwrap().unwrap(), "hunter2");
//! ```

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::Error;
use crate::hlc::{Hlc, is_valid_device_id};
use crate::seal::{self, SealError, SealKey};

/// A request stays open this long unless a frame of the session renews
/// it; relays replay it to devices that connect meanwhile (a phone that
/// comes back from the background).
pub const REQUEST_TTL_MS: u64 = 5 * 60_000;

/// Longest text of a field, in bytes.
pub const MAX_TEXT_LEN: usize = 1024;

/// Longest [`InputFrame::label`], in bytes.
pub const MAX_LABEL_LEN: usize = 160;

fn is_false(v: &bool) -> bool {
    !*v
}

/// What an `input` frame does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputAction {
    /// The TV asks its family for input into a field.
    Request,
    /// A device opened its input sheet for the session.
    Open,
    /// The whole text so far (not a diff: a lost frame does not matter).
    Text,
    /// The user finished typing: the final text.
    Submit,
    /// The session ends (the field lost focus, the user closed the sheet).
    Cancel,
    /// An action of a newer version; ignored.
    #[serde(other)]
    Other,
}

/// What a field is for: the input sheet picks its keyboard by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputField {
    Text,
    Search,
    /// An address (URL, host).
    Address,
    Username,
    Password,
    /// Digits only (a PIN, a pairing code).
    Pin,
    /// A kind of a newer version; treated as text.
    #[serde(other)]
    Other,
}

/// `input`: one frame of a remote input session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputFrame {
    /// Id of the session (the TV picks a fresh UUID per focused field).
    pub session: String,
    pub action: InputAction,
    /// When it was sent; `hlc.device` is the sender.
    pub hlc: Hlc,
    /// The device it is for; `None` for every device of the family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// [`InputAction::Request`]: what the field is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<InputField>,
    /// [`InputAction::Request`]: the label of the field as the TV shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The field hides what is typed: never log or keep its text.
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,
    /// The text, sealed for `input:<session>` ([`seal_text`]); on a
    /// request, what the field holds already (never for a secret field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed: Option<String>,
    /// [`InputAction::Request`]: how long it stays open after `hlc.wall_ms`
    /// (absent: [`REQUEST_TTL_MS`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
}

impl InputFrame {
    /// A frame of `session` with nothing else set.
    pub fn new(session: impl Into<String>, action: InputAction, hlc: Hlc) -> Self {
        Self {
            session: session.into(),
            action,
            hlc,
            to: None,
            field: None,
            label: None,
            secret: false,
            sealed: None,
            ttl_ms: None,
        }
    }

    /// Checks ids, sizes and that text is sealed.
    pub fn validate(&self) -> Result<(), Error> {
        if !is_valid_device_id(&self.session) {
            return Err(Error::Invalid("input session"));
        }
        if !is_valid_device_id(&self.hlc.device) {
            return Err(Error::Invalid("device id"));
        }
        if self.to.as_deref().is_some_and(|to| !is_valid_device_id(to)) {
            return Err(Error::Invalid("device id"));
        }
        if let Some(label) = &self.label
            && (label.len() > MAX_LABEL_LEN || label.chars().any(char::is_control))
        {
            return Err(Error::Invalid("input label"));
        }
        if let Some(sealed) = &self.sealed
            && !seal::is_sealed(sealed)
        {
            return Err(Error::Invalid("input text"));
        }
        if self.action == InputAction::Request && self.field.is_none() {
            return Err(Error::Invalid("input field"));
        }
        // A secret field starts empty: its text never goes out in a request.
        if self.action == InputAction::Request && self.secret && self.sealed.is_some() {
            return Err(Error::Invalid("input text"));
        }
        Ok(())
    }

    /// Whether a request is still open at `now_ms` (Unix ms here).
    pub fn is_current(&self, now_ms: u64) -> bool {
        let ttl = self.ttl_ms.unwrap_or(REQUEST_TTL_MS).min(REQUEST_TTL_MS);
        now_ms < self.hlc.wall_ms.saturating_add(ttl)
    }
}

/// The seal context of a session's text.
pub fn context(session: &str) -> String {
    format!("input:{session}")
}

/// Seals the text of `session`; longer than [`MAX_TEXT_LEN`] fails.
pub fn seal_text(key: &SealKey, session: &str, text: &str) -> Result<String, SealError> {
    if text.len() > MAX_TEXT_LEN {
        return Err(SealError::TooLong);
    }
    seal::seal(key, &context(session), text.as_bytes())
}

/// The text of `frame`, if it carries one.
pub fn open_text(
    key: &SealKey,
    frame: &InputFrame,
) -> Result<Option<Zeroizing<String>>, SealError> {
    let Some(sealed) = &frame.sealed else {
        return Ok(None);
    };
    let bytes = seal::open(key, &context(&frame.session), sealed)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| SealError::Malformed)?;
    if text.len() > MAX_TEXT_LEN {
        return Err(SealError::TooLong);
    }
    Ok(Some(Zeroizing::new(text.to_owned())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hlc(device: &str) -> Hlc {
        Hlc {
            wall_ms: 10_000,
            counter: 0,
            device: device.into(),
        }
    }

    #[test]
    fn text_is_bound_to_its_session() {
        let key = SealKey::new("fam", 1, &[3; 32]);
        let mut f = InputFrame::new("s1", InputAction::Text, hlc("phone"));
        f.sealed = Some(seal_text(&key, "s1", "matrix").unwrap());
        assert_eq!(&*open_text(&key, &f).unwrap().unwrap(), "matrix");
        let moved = InputFrame {
            session: "s2".into(),
            ..f.clone()
        };
        assert_eq!(open_text(&key, &moved), Err(SealError::Open));
        let other = SealKey::new("fam2", 1, &[3; 32]);
        assert_eq!(open_text(&other, &f), Err(SealError::Open));
        let empty = InputFrame::new("s1", InputAction::Cancel, hlc("tv"));
        assert_eq!(open_text(&key, &empty), Ok(None));
        assert_eq!(
            seal_text(&key, "s1", &"x".repeat(MAX_TEXT_LEN + 1)),
            Err(SealError::TooLong)
        );
    }

    #[test]
    fn validation() {
        let key = SealKey::new("fam", 1, &[3; 32]);
        let mut req = InputFrame::new("s1", InputAction::Request, hlc("tv"));
        assert!(req.validate().is_err(), "a request names its field");
        req.field = Some(InputField::Password);
        req.secret = true;
        req.validate().unwrap();
        req.sealed = Some(seal_text(&key, "s1", "old").unwrap());
        assert!(req.validate().is_err(), "a secret field starts empty");
        let mut text = InputFrame::new("s1", InputAction::Text, hlc("phone"));
        text.sealed = Some("hunter2".into());
        assert!(text.validate().is_err(), "text in clear");
        text.sealed = None;
        text.label = Some("a\nb".into());
        assert!(text.validate().is_err());
        text.label = None;
        text.to = Some("bad id!".into());
        assert!(text.validate().is_err());
        assert!(
            InputFrame::new("", InputAction::Cancel, hlc("tv"))
                .validate()
                .is_err()
        );
    }

    #[test]
    fn requests_expire_and_unknown_values_decode() {
        let mut req = InputFrame::new("s1", InputAction::Request, hlc("tv"));
        req.field = Some(InputField::Search);
        assert!(req.is_current(10_000 + REQUEST_TTL_MS - 1));
        assert!(!req.is_current(10_000 + REQUEST_TTL_MS));
        req.ttl_ms = Some(1_000);
        assert!(!req.is_current(11_000));
        let json = r#"{"session":"s","action":"dictate","hlc":{"wall_ms":1,"counter":0,"device":"tv"},"field":"emoji"}"#;
        let f: InputFrame = serde_json::from_str(json).unwrap();
        assert_eq!(f.action, InputAction::Other);
        assert_eq!(f.field, Some(InputField::Other));
    }
}
