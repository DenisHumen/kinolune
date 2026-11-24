// SPDX-License-Identifier: MPL-2.0
//! The chat JSON of TwitchDownloader (06 §2.5): what the online replay
//! produces and what is read back from a file next to a downloaded VOD.
//!
//! Only the fields the replay needs are typed; everything else (the
//! `FileInfo`, `embeddedData`, unknown fields of a comment) is kept as is,
//! so a file read and written again stays a valid TwitchDownloader file.
//! Ids are strings here; TwitchDownloader writes some of them as numbers
//! (`streamer.id`), both are read.

use std::io::Read;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// A chat file could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplayFileError {
    /// Not JSON, or not the TwitchDownloader shape.
    #[error("not a TwitchDownloader chat file: {0}")]
    Format(String),
    /// A gzip file that does not unpack.
    #[error("cannot unpack the chat file: {0}")]
    Gzip(String),
}

/// A whole chat file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChatReplayFile {
    /// TwitchDownloader's file version and dates.
    #[serde(rename = "FileInfo", default, skip_serializing_if = "Option::is_none")]
    pub file_info: Option<Value>,
    /// The channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streamer: Option<Streamer>,
    /// The VOD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoInfo>,
    /// Messages, in file order (normally by offset).
    #[serde(default)]
    pub comments: Vec<Comment>,
    /// Emote and badge images embedded by TwitchDownloader; not used.
    #[serde(
        rename = "embeddedData",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub embedded_data: Option<Value>,
}

/// `streamer`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Streamer {
    /// Display name.
    #[serde(default)]
    pub name: String,
    /// Broadcaster id (TwitchDownloader writes a number).
    #[serde(default, deserialize_with = "lenient_id")]
    pub id: String,
    /// Login, when the writer knows it (our recorder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// Other fields.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `video`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct VideoInfo {
    /// VOD id.
    #[serde(default, deserialize_with = "lenient_opt_id")]
    pub id: Option<String>,
    /// Title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// First second of the chat (a part of the VOD may be saved).
    #[serde(default)]
    pub start: f64,
    /// Last second of the chat.
    #[serde(default)]
    pub end: f64,
    /// VOD length in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,
    /// Other fields (`chapters`, `created_at`, `viewCount`, …).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One message.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Comment {
    /// Message id.
    #[serde(rename = "_id", default, deserialize_with = "lenient_id")]
    pub id: String,
    /// When it was sent (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Broadcaster id.
    #[serde(
        default,
        deserialize_with = "lenient_opt_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub channel_id: Option<String>,
    /// `video`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// VOD id.
    #[serde(
        default,
        deserialize_with = "lenient_opt_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub content_id: Option<String>,
    /// Seconds from the start of the VOD (whole seconds from GQL).
    #[serde(default)]
    pub content_offset_seconds: f64,
    /// Author.
    #[serde(default)]
    pub commenter: Commenter,
    /// Text, emotes, badges, colour.
    #[serde(default)]
    pub message: Message,
    /// Other fields.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `commenter`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Commenter {
    /// Display name.
    #[serde(default)]
    pub display_name: String,
    /// User id.
    #[serde(rename = "_id", default, deserialize_with = "lenient_id")]
    pub id: String,
    /// Login.
    #[serde(default)]
    pub name: String,
    /// Other fields (`logo`, `bio`, dates).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `message`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Message {
    /// Whole text.
    #[serde(default)]
    pub body: String,
    /// Text and emote parts in order.
    #[serde(default)]
    pub fragments: Vec<Fragment>,
    /// Badges of the author.
    #[serde(default)]
    pub user_badges: Vec<UserBadge>,
    /// `#RRGGBB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_color: Option<String>,
    /// Bits cheered (TwitchDownloader: `bits_spent`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bits_spent: Option<u32>,
    /// Other fields (`emoticons`, `user_notice_params`, …).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Text or a Twitch emote.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Fragment {
    /// Text (the emote code for an emote).
    #[serde(default)]
    pub text: String,
    /// A Twitch emote.
    #[serde(default)]
    pub emoticon: Option<Emoticon>,
}

/// `emoticon` of a fragment.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Emoticon {
    /// Twitch emote id.
    #[serde(default, deserialize_with = "lenient_id")]
    pub emoticon_id: String,
}

/// A badge: set and version.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UserBadge {
    /// Set id (`subscriber`, `moderator`, …).
    #[serde(rename = "_id", default, deserialize_with = "lenient_id")]
    pub id: String,
    /// Version within the set.
    #[serde(default, deserialize_with = "lenient_id")]
    pub version: String,
}

fn id_of(v: Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn lenient_id<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(id_of(Value::deserialize(d)?).unwrap_or_default())
}

fn lenient_opt_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(id_of(Value::deserialize(d)?))
}

impl Comment {
    /// The offset in milliseconds (negative and non-finite ones are 0).
    pub fn offset_ms(&self) -> u64 {
        let s = self.content_offset_seconds;
        if s.is_finite() && s > 0.0 {
            // Seconds of a VOD are far below 2^53 ms.
            (s * 1000.0).round() as u64
        } else {
            0
        }
    }

    /// Removed by a moderator (or by the author). TwitchDownloader keeps
    /// no such comments; other writers mark them in one of these ways:
    /// `state: "deleted"` (the old v5 API), `deleted`/`is_deleted: true`
    /// on the comment or on its message. Never shown.
    pub fn is_deleted(&self) -> bool {
        fn flagged(m: &Map<String, Value>) -> bool {
            ["deleted", "is_deleted", "isDeleted"]
                .iter()
                .any(|k| m.get(*k).and_then(Value::as_bool) == Some(true))
        }
        self.extra
            .get("state")
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("deleted"))
            || flagged(&self.extra)
            || flagged(&self.message.extra)
    }

    /// The text: `body`, else the fragments joined.
    pub fn text(&self) -> String {
        if !self.message.body.is_empty() || self.message.fragments.is_empty() {
            return self.message.body.clone();
        }
        self.message
            .fragments
            .iter()
            .map(|f| f.text.as_str())
            .collect()
    }
}

impl ChatReplayFile {
    /// Parses a file: JSON, or gzip-packed JSON (`.chat.json.gz`).
    pub fn parse(bytes: &[u8]) -> Result<Self, ReplayFileError> {
        if bytes.starts_with(&[0x1f, 0x8b]) {
            let mut raw = Vec::new();
            flate2::read::GzDecoder::new(bytes)
                .read_to_end(&mut raw)
                .map_err(|e| ReplayFileError::Gzip(e.to_string()))?;
            return Self::parse_json(&raw);
        }
        Self::parse_json(bytes)
    }

    fn parse_json(bytes: &[u8]) -> Result<Self, ReplayFileError> {
        let value: Value =
            serde_json::from_slice(bytes).map_err(|e| ReplayFileError::Format(e.to_string()))?;
        if !value.get("comments").is_some_and(Value::is_array) {
            return Err(ReplayFileError::Format("no comments array".into()));
        }
        serde_json::from_value(value).map_err(|e| ReplayFileError::Format(e.to_string()))
    }

    /// The file as JSON (what TwitchDownloader reads).
    pub fn to_json(&self) -> Vec<u8> {
        // Plain data with string keys: serialising cannot fail.
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Broadcaster id from `streamer`, else from the first comment.
    pub fn channel_id(&self) -> Option<String> {
        self.streamer
            .as_ref()
            .map(|s| s.id.clone())
            .filter(|id| !id.is_empty())
            .or_else(|| self.comments.iter().find_map(|c| c.channel_id.clone()))
    }

    /// Channel name for the header: login, else display name.
    pub fn channel_name(&self) -> String {
        self.streamer
            .as_ref()
            .map(|s| {
                s.login
                    .clone()
                    .filter(|l| !l.is_empty())
                    .unwrap_or_else(|| s.name.clone())
            })
            .unwrap_or_default()
    }
}

/// Paths where a chat file of the media file `media` may be, in the order
/// they are tried: our sidecars (`<stem>.chat.json.gz`, `.chat.json`,
/// TWI-017/TWI-018) and TwitchDownloader's usual names (`<stem>.json`,
/// `<stem>_chat.json`, `<stem>-chat.json`).
pub fn sidecar_candidates(media: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Some(stem) = media.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let dir = media.parent().unwrap_or_else(|| std::path::Path::new(""));
    [
        ".chat.json.gz",
        ".chat.json",
        ".json.gz",
        ".json",
        "_chat.json",
        "-chat.json",
    ]
    .iter()
    .map(|suffix| dir.join(format!("{stem}{suffix}")))
    .filter(|p| p != media)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_strings_are_ids() {
        let f = ChatReplayFile::parse(
            br#"{"streamer":{"name":"A","id":123},"comments":[
                {"_id":"x","content_offset_seconds":1.5,"commenter":{"_id":7,"name":"u","display_name":"U"},
                 "message":{"body":"hi","fragments":[{"text":"hi","emoticon":null}],"user_badges":[{"_id":"subscriber","version":12}]}}]}"#,
        )
        .unwrap();
        assert_eq!(f.channel_id().as_deref(), Some("123"));
        let c = &f.comments[0];
        assert_eq!(c.commenter.id, "7");
        assert_eq!(c.offset_ms(), 1500);
        assert_eq!(c.message.user_badges[0].version, "12");
        assert!(!c.is_deleted());
    }

    #[test]
    fn deleted_marks() {
        for extra in [
            r#""state":"deleted""#,
            r#""deleted":true"#,
            r#""is_deleted":true"#,
        ] {
            let json = format!(r#"{{"comments":[{{"_id":"a",{extra}}}]}}"#);
            assert!(ChatReplayFile::parse(json.as_bytes()).unwrap().comments[0].is_deleted());
        }
        let msg = br#"{"comments":[{"_id":"a","message":{"body":"x","isDeleted":true}}]}"#;
        assert!(ChatReplayFile::parse(msg).unwrap().comments[0].is_deleted());
    }

    #[test]
    fn not_a_chat_file() {
        assert!(ChatReplayFile::parse(b"{}").is_err());
        assert!(ChatReplayFile::parse(b"nope").is_err());
        assert!(ChatReplayFile::parse(&[0x1f, 0x8b, 0, 0]).is_err());
    }

    #[test]
    fn sidecars_of_a_vod() {
        let names: Vec<String> = sidecar_candidates(std::path::Path::new("/v/123 - title.mkv"))
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names[0], "123 - title.chat.json.gz");
        assert!(names.contains(&"123 - title.json".to_owned()));
    }
}
