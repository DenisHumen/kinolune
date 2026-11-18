// SPDX-License-Identifier: MPL-2.0
//! The DTOs of the Subsonic JSON API. They read only the fields we use,
//! ignore the rest, treat `null` and missing fields as defaults and accept
//! ids and numbers as JSON numbers or strings (servers differ).

use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// A JSON string or number as text; anything else is empty.
fn text<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s,
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    })
}

fn opt_text<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let s = text(d)?;
    Ok((!s.trim().is_empty()).then_some(s))
}

/// A JSON number or numeric string; anything else is `None`.
fn opt_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<f64>().ok().map(|f| f as i64),
        _ => None,
    })
}

fn flag<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Bool(b) => b,
        Value::String(s) => s == "true",
        _ => false,
    })
}

/// A list that may be missing, `null`, or (in some servers' JSON made
/// from XML) a single object.
fn list<'de, D: Deserializer<'de>, T: serde::de::DeserializeOwned>(
    d: D,
) -> Result<Vec<T>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(a) => a
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
        v @ Value::Object(_) => serde_json::from_value(v).ok().into_iter().collect(),
        _ => Vec::new(),
    })
}

/// What [`Client::ping`](crate::Client::ping) learns about the server.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerInfo {
    /// Subsonic API version the server speaks (`1.16.1`).
    #[serde(deserialize_with = "text")]
    pub version: String,
    /// Server software (`navidrome`, `gonic`, …), OpenSubsonic only.
    #[serde(rename = "type", deserialize_with = "text")]
    pub server_type: String,
    /// Version of the server software, OpenSubsonic only.
    #[serde(deserialize_with = "text")]
    pub server_version: String,
    /// Whether the server supports OpenSubsonic.
    #[serde(deserialize_with = "flag")]
    pub open_subsonic: bool,
}

/// An OpenSubsonic extension the server supports.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct Extension {
    /// Name (`apiKeyAuthentication`, `transcodeOffset`, …).
    #[serde(deserialize_with = "text")]
    pub name: String,
    /// Supported versions of it.
    #[serde(deserialize_with = "list")]
    pub versions: Vec<i64>,
}

/// An artist (`ArtistID3`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Artist {
    /// Artist id.
    #[serde(deserialize_with = "text")]
    pub id: String,
    /// Name.
    #[serde(deserialize_with = "text")]
    pub name: String,
    /// Number of albums.
    #[serde(deserialize_with = "opt_i64")]
    pub album_count: Option<i64>,
    /// Cover art id.
    #[serde(deserialize_with = "opt_text")]
    pub cover_art: Option<String>,
    /// Albums (only in `getArtist`).
    #[serde(rename = "album", deserialize_with = "list")]
    pub albums: Vec<Album>,
}

/// An album (`AlbumID3`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Album {
    /// Album id.
    #[serde(deserialize_with = "text")]
    pub id: String,
    /// Name.
    #[serde(deserialize_with = "text")]
    pub name: String,
    /// Title, which some servers send in place of `name`.
    #[serde(deserialize_with = "opt_text")]
    pub title: Option<String>,
    /// Album artist as shown.
    #[serde(deserialize_with = "opt_text")]
    pub artist: Option<String>,
    /// Album artist id.
    #[serde(deserialize_with = "opt_text")]
    pub artist_id: Option<String>,
    /// Cover art id.
    #[serde(deserialize_with = "opt_text")]
    pub cover_art: Option<String>,
    /// Number of songs.
    #[serde(deserialize_with = "opt_i64")]
    pub song_count: Option<i64>,
    /// Total length, seconds.
    #[serde(deserialize_with = "opt_i64")]
    pub duration: Option<i64>,
    /// Year.
    #[serde(deserialize_with = "opt_i64")]
    pub year: Option<i64>,
    /// Genre.
    #[serde(deserialize_with = "opt_text")]
    pub genre: Option<String>,
    /// When it was added (ISO 8601).
    #[serde(deserialize_with = "opt_text")]
    pub created: Option<String>,
    /// Songs (only in `getAlbum`).
    #[serde(rename = "song", deserialize_with = "list")]
    pub songs: Vec<Song>,
}

/// A playlist (`getPlaylists`, `getPlaylist`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Playlist {
    /// Playlist id.
    #[serde(deserialize_with = "text")]
    pub id: String,
    /// Name.
    #[serde(deserialize_with = "text")]
    pub name: String,
    /// Number of songs.
    #[serde(deserialize_with = "opt_i64")]
    pub song_count: Option<i64>,
    /// Last change (ISO 8601).
    #[serde(deserialize_with = "opt_text")]
    pub changed: Option<String>,
    /// Songs in order (only in `getPlaylist`).
    #[serde(rename = "entry", deserialize_with = "list")]
    pub entries: Vec<Song>,
}

impl Album {
    /// The name, or the title of servers that send that instead.
    #[must_use]
    pub fn display_name(&self) -> &str {
        if self.name.trim().is_empty() {
            self.title.as_deref().unwrap_or("")
        } else {
            &self.name
        }
    }
}

/// A song (`Child` of type music).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Song {
    /// Song id.
    #[serde(deserialize_with = "text")]
    pub id: String,
    /// Title.
    #[serde(deserialize_with = "text")]
    pub title: String,
    /// Album name.
    #[serde(deserialize_with = "opt_text")]
    pub album: Option<String>,
    /// Album id.
    #[serde(deserialize_with = "opt_text")]
    pub album_id: Option<String>,
    /// Track artist.
    #[serde(deserialize_with = "opt_text")]
    pub artist: Option<String>,
    /// Track artist id.
    #[serde(deserialize_with = "opt_text")]
    pub artist_id: Option<String>,
    /// Album artist as shown (OpenSubsonic `displayAlbumArtist`).
    #[serde(deserialize_with = "opt_text")]
    pub display_album_artist: Option<String>,
    /// Track number.
    #[serde(deserialize_with = "opt_i64")]
    pub track: Option<i64>,
    /// Disc number.
    #[serde(deserialize_with = "opt_i64")]
    pub disc_number: Option<i64>,
    /// Year.
    #[serde(deserialize_with = "opt_i64")]
    pub year: Option<i64>,
    /// Genre.
    #[serde(deserialize_with = "opt_text")]
    pub genre: Option<String>,
    /// Cover art id.
    #[serde(deserialize_with = "opt_text")]
    pub cover_art: Option<String>,
    /// Size of the original file, bytes.
    #[serde(deserialize_with = "opt_i64")]
    pub size: Option<i64>,
    /// MIME type of the original file.
    #[serde(deserialize_with = "opt_text")]
    pub content_type: Option<String>,
    /// Extension of the original file (`flac`, `mp3`).
    #[serde(deserialize_with = "opt_text")]
    pub suffix: Option<String>,
    /// Length, seconds.
    #[serde(deserialize_with = "opt_i64")]
    pub duration: Option<i64>,
    /// Bit rate, kbit/s.
    #[serde(deserialize_with = "opt_i64")]
    pub bit_rate: Option<i64>,
    /// Path of the file on the server (relative to its music folder).
    #[serde(deserialize_with = "opt_text")]
    pub path: Option<String>,
    /// When it was added (ISO 8601).
    #[serde(deserialize_with = "opt_text")]
    pub created: Option<String>,
    /// How often the user played it.
    #[serde(deserialize_with = "opt_i64")]
    pub play_count: Option<i64>,
    /// When the user last played it (OpenSubsonic, ISO 8601).
    #[serde(deserialize_with = "opt_text")]
    pub played: Option<String>,
    /// MusicBrainz recording id.
    #[serde(deserialize_with = "opt_text")]
    pub music_brainz_id: Option<String>,
    /// A folder rather than a file (browsing by folders).
    #[serde(deserialize_with = "flag")]
    pub is_dir: bool,
    /// `music`, `podcast`, `audiobook`, `video`.
    #[serde(rename = "type", deserialize_with = "opt_text")]
    pub media_type: Option<String>,
}

impl Song {
    /// Length in milliseconds.
    #[must_use]
    pub fn duration_ms(&self) -> Option<i64> {
        self.duration.map(|s| s * 1000)
    }

    /// A music file (not a folder, a video or a podcast episode).
    #[must_use]
    pub fn is_music(&self) -> bool {
        !self.is_dir && !matches!(self.media_type.as_deref(), Some("video" | "podcast"))
    }
}

/// Unix milliseconds of an ISO 8601 date-time (`2021-04-01T10:20:30Z`,
/// with optional fraction and offset).
#[must_use]
pub fn parse_date_ms(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| text.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut rest = &text[19..];
    let mut ms = 0;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits: String = frac.chars().take_while(char::is_ascii_digit).collect();
        rest = &frac[digits.len()..];
        let three: String = digits.chars().chain("000".chars()).take(3).collect();
        ms = three.parse::<i64>().ok()?;
    }
    let offset_min = match rest {
        "" | "Z" | "z" => 0,
        o if o.len() == 6 && (o.starts_with('+') || o.starts_with('-')) => {
            let oh: i64 = o[1..3].parse().ok()?;
            let om: i64 = o[4..6].parse().ok()?;
            let v = oh * 60 + om;
            if o.starts_with('-') { -v } else { v }
        }
        _ => return None,
    };
    // Days from civil (H. Hinnant).
    let (y2, m2) = if mo <= 2 {
        (y - 1, mo + 9)
    } else {
        (y, mo - 3)
    };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + s - offset_min * 60;
    Some(secs * 1000 + ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lenient_songs() {
        let s: Song = serde_json::from_str(
            r#"{"id":42,"title":"Song","album":"LP","albumId":"al-1","artist":"Band",
                "track":"3","discNumber":2,"year":2021,"size":123456,"suffix":"flac",
                "duration":187,"created":"2024-01-02T03:04:05.678Z","isDir":false,
                "type":"music","genre":null,"coverArt":"","extra":{"x":1}}"#,
        )
        .unwrap();
        assert_eq!(s.id, "42");
        assert_eq!(
            (s.track, s.disc_number, s.year),
            (Some(3), Some(2), Some(2021))
        );
        assert_eq!(s.duration_ms(), Some(187_000));
        assert_eq!(s.genre, None);
        assert_eq!(s.cover_art, None);
        assert!(s.is_music());
        let a: Album =
            serde_json::from_str(r#"{"id":"1","title":"Old","song":{"id":"9"}}"#).unwrap();
        assert_eq!(a.display_name(), "Old");
        assert_eq!(a.songs.len(), 1);
        let p: Playlist = serde_json::from_str(
            r#"{"id":7,"name":"Road","songCount":"2","entry":[{"id":"1"},{"id":"2"}]}"#,
        )
        .unwrap();
        assert_eq!((p.id.as_str(), p.song_count, p.entries.len()), ("7", Some(2), 2));
    }

    #[test]
    fn parses_dates() {
        assert_eq!(parse_date_ms("1970-01-01T00:00:01Z"), Some(1000));
        assert_eq!(
            parse_date_ms("2024-01-02T03:04:05.678912+02:00"),
            parse_date_ms("2024-01-02T01:04:05.678Z")
        );
        assert_eq!(parse_date_ms("2024-01-02"), None);
    }
}
