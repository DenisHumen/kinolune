// SPDX-License-Identifier: MPL-2.0
//! Music continues on another device (MUS-015): the `music` message,
//! sans-IO.
//!
//! «Продолжить на…» sends [`MusicAction::Push`] to one device of the
//! family: the whole queue, the current track and the playhead, sealed
//! with the family key ([`crate::seal`]) for the context
//! `music:<session>`. The target starts the queue at the same position and
//! the sender pauses. «Забрать сюда» sends [`MusicAction::Pull`] to a
//! device: if music is loaded there, it answers with a push to the asker
//! and pauses.
//!
//! Like `pb.*` and `input`, the frames are ephemeral: relays fan them out
//! once per stamp and never store them; nothing is replayed to devices that
//! connect later (a push to a device that is gone is simply lost, the
//! sender keeps playing). The queue itself is opaque here: the app writes
//! and reads its JSON (`v: 1`, see `docs/api/sync-protocol.md` §7.5).
//!
//! ```
//! use pa_sync::hlc::Hlc;
//! use pa_sync::music::{self, MusicAction, MusicFrame};
//! use pa_sync::seal::SealKey;
//!
//! let key = SealKey::new("fam1", 1, &[7; 32]);
//! let hlc = Hlc { wall_ms: 1, counter: 0, device: "phone".into() };
//! let mut frame = MusicFrame::new("s1", MusicAction::Push, "tv", hlc);
//! frame.sealed = Some(music::seal_queue(&key, "s1", r#"{"v":1}"#).unwrap());
//! frame.validate().unwrap();
//! assert_eq!(&*music::open_queue(&key, &frame).unwrap().unwrap(), r#"{"v":1}"#);
//! ```

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::Error;
use crate::hlc::{Hlc, is_valid_device_id};
use crate::seal::{self, SealError, SealKey};

/// Largest queue payload (the app's JSON), in bytes. Sealed and base64url
/// it stays well below `proto::MAX_FRAME_LEN`; the app trims a longer
/// queue around the current track before it sends.
pub const MAX_QUEUE_LEN: usize = 128 * 1024;

/// What a `music` frame does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicAction {
    /// Here is my queue: play it from the playhead (I pause).
    Push,
    /// Hand me your queue (answered with a push, if music is loaded).
    Pull,
    /// An action of a newer version; ignored.
    #[serde(other)]
    Other,
}

/// `music`: one frame of a music handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MusicFrame {
    /// Id of the handoff (a fresh UUID per push or pull; a push that
    /// answers a pull keeps the pull's id).
    pub session: String,
    pub action: MusicAction,
    /// When it was sent; `hlc.device` is the sender.
    pub hlc: Hlc,
    /// The one device it is for.
    pub to: String,
    /// [`MusicAction::Push`]: the queue, sealed for `music:<session>`
    /// ([`seal_queue`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed: Option<String>,
}

impl MusicFrame {
    /// A frame of `session` for `to` with nothing sealed.
    pub fn new(
        session: impl Into<String>,
        action: MusicAction,
        to: impl Into<String>,
        hlc: Hlc,
    ) -> Self {
        Self {
            session: session.into(),
            action,
            hlc,
            to: to.into(),
            sealed: None,
        }
    }

    /// Checks ids, that a push carries a sealed queue and a pull none.
    pub fn validate(&self) -> Result<(), Error> {
        if !is_valid_device_id(&self.session) {
            return Err(Error::Invalid("music session"));
        }
        if !is_valid_device_id(&self.hlc.device) || !is_valid_device_id(&self.to) {
            return Err(Error::Invalid("device id"));
        }
        match (self.action, &self.sealed) {
            (MusicAction::Push, Some(s)) if seal::is_sealed_up_to(s, MAX_QUEUE_LEN) => Ok(()),
            (MusicAction::Push, _) => Err(Error::Invalid("music queue")),
            (MusicAction::Pull, Some(_)) => Err(Error::Invalid("music queue")),
            _ => Ok(()),
        }
    }
}

/// The seal context of a handoff's queue.
pub fn context(session: &str) -> String {
    format!("music:{session}")
}

/// Seals the queue JSON of `session`; longer than [`MAX_QUEUE_LEN`] fails.
pub fn seal_queue(key: &SealKey, session: &str, queue: &str) -> Result<String, SealError> {
    seal::seal_up_to(key, &context(session), queue.as_bytes(), MAX_QUEUE_LEN)
}

/// The queue JSON of `frame`, if it carries one.
pub fn open_queue(
    key: &SealKey,
    frame: &MusicFrame,
) -> Result<Option<Zeroizing<String>>, SealError> {
    let Some(sealed) = &frame.sealed else {
        return Ok(None);
    };
    let bytes = seal::open_up_to(key, &context(&frame.session), sealed, MAX_QUEUE_LEN)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| SealError::Malformed)?;
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
    fn queue_is_bound_to_its_session_and_family() {
        let key = SealKey::new("fam", 1, &[3; 32]);
        let mut f = MusicFrame::new("s1", MusicAction::Push, "tv", hlc("phone"));
        f.sealed = Some(seal_queue(&key, "s1", "{\"v\":1}").unwrap());
        f.validate().unwrap();
        assert_eq!(&*open_queue(&key, &f).unwrap().unwrap(), "{\"v\":1}");
        let moved = MusicFrame {
            session: "s2".into(),
            ..f.clone()
        };
        assert_eq!(open_queue(&key, &moved), Err(SealError::Open));
        let other = SealKey::new("fam2", 1, &[3; 32]);
        assert_eq!(open_queue(&other, &f), Err(SealError::Open));
        let pull = MusicFrame::new("s1", MusicAction::Pull, "phone", hlc("tv"));
        assert_eq!(open_queue(&key, &pull), Ok(None));
    }

    #[test]
    fn large_queues_fit_and_longer_ones_fail() {
        let key = SealKey::new("fam", 1, &[3; 32]);
        let queue = format!("{{\"v\":1,\"x\":\"{}\"}}", "a".repeat(MAX_QUEUE_LEN - 20));
        let mut f = MusicFrame::new("s1", MusicAction::Push, "tv", hlc("phone"));
        f.sealed = Some(seal_queue(&key, "s1", &queue).unwrap());
        f.validate().unwrap();
        let raw = crate::proto::encode(&crate::proto::Message::Music(f)).unwrap();
        assert!(raw.len() < crate::proto::MAX_FRAME_LEN);
        let too_long = "a".repeat(MAX_QUEUE_LEN + 1);
        assert_eq!(
            seal_queue(&key, "s1", &too_long),
            Err(SealError::TooLong)
        );
    }

    #[test]
    fn validation() {
        let mut push = MusicFrame::new("s1", MusicAction::Push, "tv", hlc("phone"));
        assert!(push.validate().is_err(), "a push carries the queue");
        push.sealed = Some("{\"v\":1}".into());
        assert!(push.validate().is_err(), "queue in clear");
        let mut pull = MusicFrame::new("s1", MusicAction::Pull, "tv", hlc("phone"));
        pull.validate().unwrap();
        pull.sealed = push.sealed.clone();
        assert!(pull.validate().is_err());
        assert!(
            MusicFrame::new("s1", MusicAction::Pull, "bad id!", hlc("phone"))
                .validate()
                .is_err()
        );
        assert!(
            MusicFrame::new("", MusicAction::Pull, "tv", hlc("phone"))
                .validate()
                .is_err()
        );
    }

    #[test]
    fn unknown_actions_decode() {
        let json = r#"{"session":"s","action":"cast","hlc":{"wall_ms":1,"counter":0,"device":"tv"},"to":"phone"}"#;
        let f: MusicFrame = serde_json::from_str(json).unwrap();
        assert_eq!(f.action, MusicAction::Other);
        f.validate().unwrap();
    }
}
