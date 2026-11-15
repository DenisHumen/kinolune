// SPDX-License-Identifier: MPL-2.0
//! The parts of the Jellyfin DTOs (OpenAPI schema of 10.10) this client
//! reads. Field names are the server's (PascalCase); unknown fields are
//! ignored and `null` reads as the default, so newer servers stay
//! compatible.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};

/// `null` → `T::default()`.
fn nullable<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// 100-ns ticks of the API → milliseconds.
#[must_use]
pub fn ticks_to_ms(ticks: i64) -> i64 {
    ticks / 10_000
}

/// Milliseconds → 100-ns ticks of the API.
#[must_use]
pub fn ms_to_ticks(ms: i64) -> i64 {
    ms.saturating_mul(10_000)
}

/// `GET /System/Info/Public`: no sign-in needed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PublicSystemInfo {
    /// Server id: the same on every address of one server.
    #[serde(deserialize_with = "nullable")]
    pub id: String,
    /// Display name of the server.
    #[serde(deserialize_with = "nullable")]
    pub server_name: String,
    /// Server version (`10.10.7`).
    #[serde(deserialize_with = "nullable")]
    pub version: String,
    /// `Jellyfin Server`.
    #[serde(deserialize_with = "nullable")]
    pub product_name: String,
    /// The LAN address the server reports, a candidate secondary address.
    pub local_address: Option<String>,
    /// Whether the startup wizard is done.
    pub startup_wizard_completed: Option<bool>,
}

/// A user (`UserDto`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct User {
    /// User id.
    #[serde(deserialize_with = "nullable")]
    pub id: String,
    /// User name.
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    /// Server id.
    pub server_id: Option<String>,
    /// Whether signing in needs a password.
    pub has_password: bool,
    /// Tag of the avatar, if any.
    pub primary_image_tag: Option<String>,
}

/// Answer of a sign-in (`AuthenticationResult`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub(crate) struct AuthenticationResult {
    #[serde(deserialize_with = "nullable")]
    pub(crate) user: User,
    #[serde(deserialize_with = "nullable")]
    pub(crate) access_token: String,
    #[serde(deserialize_with = "nullable")]
    pub(crate) server_id: String,
}

/// State of a Quick Connect request (`QuickConnectResult`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct QuickConnectState {
    /// Whether a signed-in user approved the code.
    pub authenticated: bool,
    /// Secret of this request; trade it for a token once approved. Not
    /// shown to the user.
    #[serde(deserialize_with = "nullable")]
    pub secret: String,
    /// The code the user types into a signed-in client.
    #[serde(deserialize_with = "nullable")]
    pub code: String,
}

/// Per-user state of an item (`UserItemDataDto`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserData {
    /// Resume position, ticks.
    pub playback_position_ticks: i64,
    /// Times played.
    pub play_count: i64,
    /// Favorite of this user.
    pub is_favorite: bool,
    /// Watched.
    pub played: bool,
    /// Last play, ISO 8601.
    pub last_played_date: Option<String>,
}

impl UserData {
    /// Resume position in ms.
    #[must_use]
    pub fn position_ms(&self) -> i64 {
        ticks_to_ms(self.playback_position_ticks)
    }
}

/// One stream of a media source (`MediaStream`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct MediaStream {
    /// `Video`, `Audio`, `Subtitle`, …
    #[serde(rename = "Type", deserialize_with = "nullable")]
    pub kind: String,
    /// Codec (`h264`, `aac`, `subrip`).
    pub codec: Option<String>,
    /// Index inside the source.
    pub index: i64,
    /// Language code.
    pub language: Option<String>,
    /// Video width.
    pub width: Option<i64>,
    /// Video height.
    pub height: Option<i64>,
    /// Audio channels.
    pub channels: Option<i64>,
    /// Default track.
    pub is_default: bool,
    /// Forced subtitles.
    pub is_forced: bool,
    /// A file next to the media.
    pub is_external: bool,
}

/// One playable version of an item (`MediaSourceInfo`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct MediaSource {
    /// Media source id (the item id for a single version).
    #[serde(deserialize_with = "nullable")]
    pub id: String,
    /// Path on the server.
    pub path: Option<String>,
    /// Version name.
    pub name: Option<String>,
    /// Container, possibly a list (`mov,mp4,m4a,3gp,3g2,mj2`).
    pub container: Option<String>,
    /// File size in bytes.
    pub size: Option<i64>,
    /// Total bitrate.
    pub bitrate: Option<i64>,
    /// Duration, ticks.
    pub run_time_ticks: Option<i64>,
    /// Streams.
    #[serde(deserialize_with = "nullable")]
    pub media_streams: Vec<MediaStream>,
    /// The device can play the file as is.
    pub supports_direct_play: bool,
    /// The server can remux it without re-encoding.
    pub supports_direct_stream: bool,
    /// The server can transcode it.
    pub supports_transcoding: bool,
    /// Path and query of the transcoded (or remuxed) stream, when the
    /// device profile needs one.
    pub transcoding_url: Option<String>,
    /// `hls` or `http`.
    pub transcoding_sub_protocol: Option<String>,
}

/// Trickplay tiles of one width (`TrickplayInfoDto`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct TrickplayInfo {
    /// Tile width, px.
    pub width: u32,
    /// Tile height, px.
    pub height: u32,
    /// Tiles per row of a sheet.
    pub tile_width: u32,
    /// Rows of a sheet.
    pub tile_height: u32,
    /// Thumbnails in total.
    pub thumbnail_count: u32,
    /// Time between thumbnails, ms.
    pub interval: u32,
    /// Bandwidth of the tiles, bit/s.
    pub bandwidth: u32,
}

impl TrickplayInfo {
    /// Number of sheet images.
    #[must_use]
    pub fn sheets(&self) -> u32 {
        let per = self.tile_width.max(1) * self.tile_height.max(1);
        self.thumbnail_count.div_ceil(per)
    }
}

/// An item (`BaseItemDto`): a library, a movie, a show, an episode, a
/// track, …
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Item {
    /// Item id.
    #[serde(deserialize_with = "nullable")]
    pub id: String,
    /// Display name.
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    /// `Movie`, `Series`, `Season`, `Episode`, `Audio`, `MusicAlbum`,
    /// `Video`, `CollectionFolder`, …
    #[serde(rename = "Type", deserialize_with = "nullable")]
    pub kind: String,
    /// Libraries: `movies`, `tvshows`, `music`, `homevideos`, …
    pub collection_type: Option<String>,
    /// `Video`, `Audio`, …
    pub media_type: Option<String>,
    /// Release year.
    pub production_year: Option<i32>,
    /// Episode number; track number.
    pub index_number: Option<i32>,
    /// Last episode of a multi-episode file.
    pub index_number_end: Option<i32>,
    /// Season number; disc number.
    pub parent_index_number: Option<i32>,
    /// Episodes: the show.
    pub series_name: Option<String>,
    /// Episodes: the show id.
    pub series_id: Option<String>,
    /// Episodes: the season id.
    pub season_id: Option<String>,
    /// Parent item.
    pub parent_id: Option<String>,
    /// Tracks: album.
    pub album: Option<String>,
    /// Tracks: album id.
    pub album_id: Option<String>,
    /// Tracks: album artist.
    pub album_artist: Option<String>,
    /// Tracks: artists.
    #[serde(deserialize_with = "nullable")]
    pub artists: Vec<String>,
    /// Tracks: tag of the album cover.
    pub album_primary_image_tag: Option<String>,
    /// Path on the server.
    pub path: Option<String>,
    /// Container.
    pub container: Option<String>,
    /// Duration, ticks.
    pub run_time_ticks: Option<i64>,
    /// When the server added it, ISO 8601.
    pub date_created: Option<String>,
    /// External ids: `Tmdb`, `Imdb`, `Tvdb`, …
    #[serde(deserialize_with = "nullable")]
    pub provider_ids: BTreeMap<String, Option<String>>,
    /// Versions.
    #[serde(deserialize_with = "nullable")]
    pub media_sources: Vec<MediaSource>,
    /// State of the signed-in user.
    pub user_data: Option<UserData>,
    /// Image type → tag (`Primary`, `Logo`, `Thumb`).
    #[serde(deserialize_with = "nullable")]
    pub image_tags: BTreeMap<String, String>,
    /// Tags of the backdrops.
    #[serde(deserialize_with = "nullable")]
    pub backdrop_image_tags: Vec<String>,
    /// Trickplay: media source id → tile width → info.
    #[serde(deserialize_with = "nullable")]
    pub trickplay: BTreeMap<String, BTreeMap<String, TrickplayInfo>>,
    /// Whether it is a folder.
    pub is_folder: bool,
}

impl Item {
    /// External id of `provider` (`tmdb`, `imdb`, `tvdb`; any case), if
    /// set.
    #[must_use]
    pub fn provider_id(&self, provider: &str) -> Option<&str> {
        self.provider_ids
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(provider))
            .and_then(|(_, v)| v.as_deref())
            .filter(|v| !v.trim().is_empty())
    }

    /// Whether the user has a resume position.
    #[must_use]
    pub fn is_resumable(&self) -> bool {
        self.user_data
            .as_ref()
            .is_some_and(|u| u.playback_position_ticks > 0)
    }
}

/// A page of items (`BaseItemDtoQueryResult`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ItemPage {
    /// Items of the page.
    #[serde(deserialize_with = "nullable")]
    pub items: Vec<Item>,
    /// Items matching in total.
    pub total_record_count: i64,
}

/// Kind of a media segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SegmentKind {
    /// Opening.
    Intro,
    /// Credits.
    Outro,
    /// "Previously on".
    Recap,
    /// Next-episode preview.
    Preview,
    /// Advert break.
    Commercial,
    /// Anything else.
    Unknown,
}

/// A media segment (`MediaSegmentDto`, 10.10+).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// What it is.
    pub kind: SegmentKind,
    /// Start, ms.
    pub start_ms: i64,
    /// End, ms.
    pub end_ms: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub(crate) struct RawSegment {
    #[serde(rename = "Type", deserialize_with = "nullable")]
    pub(crate) kind: String,
    pub(crate) start_ticks: i64,
    pub(crate) end_ticks: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub(crate) struct RawSegments {
    #[serde(deserialize_with = "nullable")]
    pub(crate) items: Vec<RawSegment>,
}

impl From<&RawSegment> for Segment {
    fn from(s: &RawSegment) -> Self {
        let kind = match s.kind.as_str() {
            "Intro" => SegmentKind::Intro,
            "Outro" => SegmentKind::Outro,
            "Recap" => SegmentKind::Recap,
            "Preview" => SegmentKind::Preview,
            "Commercial" => SegmentKind::Commercial,
            _ => SegmentKind::Unknown,
        };
        Self {
            kind,
            start_ms: ticks_to_ms(s.start_ticks),
            end_ms: ticks_to_ms(s.end_ticks),
        }
    }
}

/// `2026-11-15T05:30:09.2700348Z` → Unix ms; `None` if malformed. Offsets
/// other than `Z` are applied.
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
    let (y2, m2) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
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
    fn parses_dates() {
        assert_eq!(parse_date_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_date_ms("2026-11-15T05:30:09.2700348Z"),
            Some(1_794_720_609_270)
        );
        assert_eq!(parse_date_ms("2000-03-01T01:00:00+01:00"), Some(951_868_800_000));
        assert_eq!(parse_date_ms("garbage"), None);
    }

    #[test]
    fn reads_items_with_nulls_and_unknown_fields() {
        let json = r#"{"Items":[{"Id":"a","Name":"M","Type":"Movie","ChannelId":null,
            "ProviderIds":{"Tmdb":"603","Imdb":"tt0133093","Tvdb":null},
            "MediaSources":[{"Id":"a","Size":10,"Container":"mkv","MediaStreams":null}],
            "UserData":{"PlaybackPositionTicks":120000000,"IsFavorite":true,"Played":false},
            "ImageTags":{"Primary":"t"},"BackdropImageTags":null,"Unknown":1,
            "Trickplay":{"a":{"320":{"Width":320,"Height":180,"TileWidth":10,"TileHeight":10,
            "ThumbnailCount":101,"Interval":10000,"Bandwidth":1}}}}],"TotalRecordCount":1}"#;
        let page: ItemPage = serde_json::from_str(json).unwrap();
        let m = &page.items[0];
        assert_eq!(m.provider_id("tmdb"), Some("603"));
        assert_eq!(m.provider_id("TVDB"), None);
        assert_eq!(m.user_data.as_ref().unwrap().position_ms(), 12_000);
        assert!(m.is_resumable());
        assert_eq!(m.trickplay["a"]["320"].sheets(), 2);
        assert!(m.backdrop_image_tags.is_empty());
    }

    #[test]
    fn reads_segments() {
        let json = r#"{"Items":[{"Id":"s","ItemId":"e","Type":"Intro","StartTicks":0,"EndTicks":900000000},
            {"Type":"Outro","StartTicks":12000000000,"EndTicks":13000000000},{"Type":"Weird"}]}"#;
        let raw: RawSegments = serde_json::from_str(json).unwrap();
        let s: Vec<Segment> = raw.items.iter().map(Segment::from).collect();
        assert_eq!(s[0].kind, SegmentKind::Intro);
        assert_eq!(s[0].end_ms, 90_000);
        assert_eq!(s[1].kind, SegmentKind::Outro);
        assert_eq!(s[2].kind, SegmentKind::Unknown);
    }
}
