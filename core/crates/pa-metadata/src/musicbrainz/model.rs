// SPDX-License-Identifier: MPL-2.0
//! MusicBrainz Web Service v2 JSON shapes (`fmt=json`), only the fields
//! Kinolune reads. Unknown fields are ignored; missing ones default.

use serde::Deserialize;

/// One name of an artist credit (`artist-credit[]`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ArtistCredit {
    /// Credited name (may differ from the artist's name).
    pub name: String,
    /// Text after this name (`" & "`, `" feat. "`).
    pub joinphrase: String,
    /// The artist.
    pub artist: ArtistRef,
}

/// An artist as embedded in credits and search results.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ArtistRef {
    /// Artist MBID.
    pub id: String,
    /// Name.
    pub name: String,
    /// Sort name (`Beatles, The`).
    #[serde(rename = "sort-name")]
    pub sort_name: String,
    /// Disambiguation comment.
    pub disambiguation: String,
}

/// The whole credit as one string: names joined by their join phrases.
pub fn credit_string(credits: &[ArtistCredit]) -> String {
    credits
        .iter()
        .map(|c| {
            let name = if c.name.is_empty() {
                &c.artist.name
            } else {
                &c.name
            };
            format!("{name}{}", c.joinphrase)
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// A genre with its vote count.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct GenreVote {
    /// Lower-case MusicBrainz genre name (`alternative rock`).
    pub name: String,
    /// Votes.
    pub count: i64,
}

/// Genre names, most votes first (ties by name), without zero-vote ones
/// when any has votes.
pub fn top_genres(genres: &[GenreVote], max: usize) -> Vec<String> {
    let mut g: Vec<&GenreVote> = genres.iter().filter(|g| !g.name.is_empty()).collect();
    if g.iter().any(|g| g.count > 0) {
        g.retain(|g| g.count > 0);
    }
    g.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    g.into_iter().take(max).map(|g| g.name.clone()).collect()
}

/// A release group as embedded in a release or looked up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ReleaseGroup {
    /// Release-group MBID.
    pub id: String,
    /// Title.
    pub title: String,
    /// `Album`, `Single`, `EP`, `Broadcast`, `Other`.
    #[serde(rename = "primary-type")]
    pub primary_type: Option<String>,
    /// `Compilation`, `Live`, `Soundtrack`…
    #[serde(rename = "secondary-types")]
    pub secondary_types: Vec<String>,
    /// Date of the earliest release (`1997`, `1997-05-21`).
    #[serde(rename = "first-release-date")]
    pub first_release_date: Option<String>,
    /// Credits (lookups with `artist-credits`).
    #[serde(rename = "artist-credit")]
    pub artist_credit: Vec<ArtistCredit>,
    /// Genres (lookups with `genres`).
    pub genres: Vec<GenreVote>,
}

/// A track of a medium (`media[].tracks[]`, with `recordings`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Track {
    /// Position on the medium, from 1.
    pub position: u32,
    /// Title.
    pub title: String,
    /// Length in milliseconds.
    pub length: Option<u64>,
    /// The recording.
    pub recording: RecordingRef,
}

/// A recording as embedded in a track.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct RecordingRef {
    /// Recording MBID.
    pub id: String,
    /// Length in milliseconds.
    pub length: Option<u64>,
}

/// One medium (disc) of a release.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Medium {
    /// Disc number, from 1.
    pub position: u32,
    /// `CD`, `Digital Media`, `12" Vinyl`…
    pub format: Option<String>,
    /// Tracks on this medium.
    #[serde(rename = "track-count")]
    pub track_count: u32,
    /// Tracks (lookups with `recordings`).
    pub tracks: Vec<Track>,
}

/// Label of a release (`label-info[]`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct LabelInfo {
    /// Catalogue number.
    #[serde(rename = "catalog-number")]
    pub catalog_number: Option<String>,
    /// The label.
    pub label: Option<LabelRef>,
}

/// A label.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct LabelRef {
    /// Label MBID.
    pub id: String,
    /// Name.
    pub name: String,
}

/// A release: a lookup (`/release/{id}`) or a search result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Release {
    /// Release MBID.
    pub id: String,
    /// Search score, 0–100 (search results only).
    pub score: Option<u32>,
    /// Title.
    pub title: String,
    /// `Official`, `Promotion`, `Bootleg`…
    pub status: Option<String>,
    /// Release date (`1997`, `1997-05-21`).
    pub date: Option<String>,
    /// Release country (ISO 3166-1).
    pub country: Option<String>,
    /// Disambiguation comment (`deluxe edition`).
    pub disambiguation: String,
    /// Tracks on all media (search results).
    #[serde(rename = "track-count")]
    pub track_count: Option<u32>,
    /// Credits.
    #[serde(rename = "artist-credit")]
    pub artist_credit: Vec<ArtistCredit>,
    /// Release group.
    #[serde(rename = "release-group")]
    pub release_group: Option<ReleaseGroup>,
    /// Media (discs).
    pub media: Vec<Medium>,
    /// Labels.
    #[serde(rename = "label-info")]
    pub label_info: Vec<LabelInfo>,
    /// Genres of the release itself (lookups with `genres`).
    pub genres: Vec<GenreVote>,
}

impl Release {
    /// Tracks on all media: the media counts, else `track-count`.
    pub fn total_tracks(&self) -> u32 {
        let media: u32 = self.media.iter().map(|m| m.track_count).sum();
        if media > 0 {
            media
        } else {
            self.track_count.unwrap_or(0)
        }
    }

    /// First label name, if any.
    pub fn label(&self) -> Option<&str> {
        self.label_info
            .iter()
            .filter_map(|l| l.label.as_ref())
            .map(|l| l.name.as_str())
            .find(|n| !n.is_empty())
    }
}

/// `/release?query=` response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ReleaseSearch {
    /// Matches, best first.
    pub releases: Vec<Release>,
}

/// Begin and end of an artist.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct LifeSpan {
    /// `1985`, `1985-03`, `1985-03-01`.
    pub begin: Option<String>,
    /// End date, if ended.
    pub end: Option<String>,
    /// Whether the artist ended (dissolved, died).
    pub ended: Option<bool>,
}

/// An artist: a lookup (`/artist/{id}`) or a search result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Artist {
    /// Artist MBID.
    pub id: String,
    /// Search score, 0–100 (search results only).
    pub score: Option<u32>,
    /// Name.
    pub name: String,
    /// Sort name.
    #[serde(rename = "sort-name")]
    pub sort_name: String,
    /// `Person`, `Group`, `Orchestra`, `Choir`, `Character`, `Other`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Country (ISO 3166-1).
    pub country: Option<String>,
    /// Disambiguation comment.
    pub disambiguation: String,
    /// Begin and end.
    #[serde(rename = "life-span")]
    pub life_span: LifeSpan,
    /// Genres (lookups with `genres`).
    pub genres: Vec<GenreVote>,
}

/// `/artist?query=` response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ArtistSearch {
    /// Matches, best first.
    pub artists: Vec<Artist>,
}

/// Lowercase UUID, or `None` if `s` is not an MBID.
pub fn mbid(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    let ok = s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        });
    ok.then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_lookup_parses() {
        let json = r#"{"id":"11111111-2222-4333-8444-555555555555","title":"Синтетика",
            "date":"2001-05-04","country":"XW","status":"Official","disambiguation":"",
            "artist-credit":[{"name":"Тест","joinphrase":" & ","artist":{"id":"a","name":"Тест","sort-name":"Тест"}},
                             {"name":"Другой","joinphrase":"","artist":{"id":"b","name":"Другой","sort-name":"Другой"}}],
            "release-group":{"id":"rg","title":"Синтетика","primary-type":"Album","secondary-types":["Live"],
                             "first-release-date":"2000","genres":[{"name":"rock","count":3}]},
            "media":[{"position":1,"format":"CD","track-count":2,
                      "tracks":[{"position":1,"title":"A","length":1000,"recording":{"id":"r1","length":1000}},
                                {"position":2,"title":"B","length":null,"recording":{"id":"r2"}}]}],
            "label-info":[{"catalog-number":"X1","label":{"id":"l","name":"Лейбл"}}],
            "unknown":{"x":1}}"#;
        let r: Release = serde_json::from_str(json).unwrap();
        assert_eq!(r.total_tracks(), 2);
        assert_eq!(r.label(), Some("Лейбл"));
        assert_eq!(credit_string(&r.artist_credit), "Тест & Другой");
        let rg = r.release_group.unwrap();
        assert_eq!(rg.primary_type.as_deref(), Some("Album"));
        assert_eq!(top_genres(&rg.genres, 5), ["rock"]);
        assert_eq!(r.media[0].tracks[1].length, None);
    }

    #[test]
    fn genres_sorted_by_votes() {
        let g = |name: &str, count| GenreVote {
            name: name.into(),
            count,
        };
        let v = [g("b", 1), g("a", 5), g("c", 0), g("d", 5)];
        assert_eq!(top_genres(&v, 2), ["a", "d"]);
        assert_eq!(top_genres(&v, 10), ["a", "d", "b"]);
        assert_eq!(top_genres(&[g("x", 0)], 3), ["x"]);
    }

    #[test]
    fn mbid_checks_shape() {
        assert_eq!(
            mbid(" 11111111-2222-4333-8444-55555555555A ").as_deref(),
            Some("11111111-2222-4333-8444-55555555555a")
        );
        assert_eq!(mbid("not-an-id"), None);
        assert_eq!(mbid("11111111x2222-4333-8444-555555555555"), None);
    }
}
