// SPDX-License-Identifier: MPL-2.0
//! Which trailer the movie or show page plays (UI-036, ADR-0100).
//!
//! Sources, in order:
//!
//! 1. the user's own file next to the film (`-trailer`, a `Trailers/`
//!    folder: the extras of LIB-005/LIB-007);
//! 2. the trailer of the user's media server (Jellyfin, Emby);
//! 3. a TMDB `videos` entry: YouTube or Vimeo, played only by the site's
//!    official embedded player; nothing is downloaded or extracted.
//!
//! A higher source wins outright, and the lower ones are not even asked:
//! [`choose`] takes the TMDB lookup as a closure and calls it only when
//! there is neither an own file nor a server trailer, and only when
//! internet trailers are on. So a film with its own trailer never sends a
//! request to TMDB, and none goes to YouTube before a page is opened.
//!
//! Among TMDB videos ([`pick_online`]): a trailer or teaser beats a clip
//! (a clip is a scene, not a trailer); then the language: the UI language,
//! the original language, English, anything else; then the type
//! (trailer, teaser, clip); then the official channel; then YouTube over
//! Vimeo, the bigger and the newer video.
//!
//! This crate is pure: TMDB is asked through `pa-metadata`, the server
//! through `pa-sources`.

use serde::Deserialize;

/// Where an online video plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Site {
    /// `youtube-nocookie.com` IFrame player.
    YouTube,
    /// `player.vimeo.com` embed.
    Vimeo,
}

impl Site {
    /// `youtube` / `vimeo`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::YouTube => "youtube",
            Self::Vimeo => "vimeo",
        }
    }

    fn parse(site: &str) -> Option<Self> {
        match site.trim().to_ascii_lowercase().as_str() {
            "youtube" => Some(Self::YouTube),
            "vimeo" => Some(Self::Vimeo),
            _ => None,
        }
    }
}

/// The kinds of TMDB videos a page may play, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VideoType {
    Trailer,
    Teaser,
    Clip,
}

impl VideoType {
    /// `trailer` / `teaser` / `clip`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trailer => "trailer",
            Self::Teaser => "teaser",
            Self::Clip => "clip",
        }
    }

    fn parse(kind: &str) -> Option<Self> {
        match kind.trim().to_ascii_lowercase().as_str() {
            "trailer" => Some(Self::Trailer),
            "teaser" => Some(Self::Teaser),
            "clip" => Some(Self::Clip),
            _ => None,
        }
    }
}

/// One entry of TMDB `/{movie|tv}/{id}/videos`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct TmdbVideo {
    /// Language (ISO 639-1), if TMDB knows it.
    #[serde(default)]
    pub iso_639_1: Option<String>,
    /// Video id at the site.
    #[serde(default)]
    pub key: String,
    /// `YouTube`, `Vimeo`.
    #[serde(default)]
    pub site: String,
    /// `Trailer`, `Teaser`, `Clip`, `Featurette`, …
    #[serde(rename = "type", default)]
    pub video_type: String,
    /// Published by the rights holder's channel.
    #[serde(default)]
    pub official: bool,
    #[serde(default)]
    pub name: String,
    /// Height: 360, 720, 1080…
    #[serde(default)]
    pub size: Option<u32>,
    /// RFC 3339.
    #[serde(default)]
    pub published_at: Option<String>,
}

#[derive(Deserialize)]
struct VideosPage {
    #[serde(default)]
    results: Vec<TmdbVideo>,
}

/// The `results` of a TMDB `videos` response.
///
/// # Errors
///
/// The body is not a TMDB `videos` object.
pub fn parse_tmdb_videos(body: &str) -> Result<Vec<TmdbVideo>, serde_json::Error> {
    serde_json::from_str::<VideosPage>(body).map(|p| p.results)
}

/// Primary language subtag, lower case: `ru-RU` → `ru`, `pt_BR` → `pt`.
fn primary(tag: &str) -> Option<String> {
    let p = tag
        .trim()
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    (p.len() >= 2 && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_lowercase())).then_some(p)
}

/// The languages a video is wanted in, best first: the UI language, the
/// original language of the title, English.
pub fn preferred_languages(ui_language: &str, original_language: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    for tag in [Some(ui_language), original_language, Some("en")]
        .into_iter()
        .flatten()
    {
        if let Some(p) = primary(tag)
            && !out.contains(&p)
        {
            out.push(p);
        }
    }
    out
}

/// TMDB `include_video_language`: the preferred languages and videos
/// without a language (`null`).
pub fn include_video_language(languages: &[String]) -> String {
    let mut parts: Vec<&str> = languages.iter().map(String::as_str).collect();
    parts.push("null");
    parts.join(",")
}

/// A video id is safe to put into an embed URL: YouTube ids are 11
/// characters of `[A-Za-z0-9_-]`, Vimeo ids are digits.
fn valid_key(site: Site, key: &str) -> bool {
    match site {
        Site::YouTube => {
            key.len() == 11
                && key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        }
        Site::Vimeo => {
            !key.is_empty() && key.len() <= 20 && key.bytes().all(|b| b.is_ascii_digit())
        }
    }
}

/// A TMDB video to play with the site's official player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnlineTrailer {
    pub site: Site,
    /// Validated video id at the site.
    pub key: String,
    pub name: String,
    /// ISO 639-1, if known.
    pub language: Option<String>,
    pub video_type: VideoType,
    pub official: bool,
}

impl OnlineTrailer {
    /// The page of the video at its site: what opens the site's own app
    /// where no embedded player is possible (tvOS, desktops without a
    /// WebView).
    pub fn watch_url(&self) -> String {
        match self.site {
            Site::YouTube => format!("https://www.youtube.com/watch?v={}", self.key),
            Site::Vimeo => format!("https://vimeo.com/{}", self.key),
        }
    }
}

/// The best TMDB video for `languages` (from [`preferred_languages`]);
/// `None` if there is no trailer, teaser or clip on YouTube or Vimeo.
pub fn pick_online(videos: &[TmdbVideo], languages: &[String]) -> Option<OnlineTrailer> {
    let rank_of = |lang: Option<&str>| {
        lang.and_then(primary)
            .and_then(|l| languages.iter().position(|p| *p == l))
            .unwrap_or(languages.len())
    };
    videos
        .iter()
        .filter_map(|v| {
            let site = Site::parse(&v.site)?;
            let video_type = VideoType::parse(&v.video_type)?;
            let key = v.key.trim();
            valid_key(site, key).then_some((v, site, video_type, key))
        })
        .min_by_key(|(v, site, video_type, _)| {
            (
                *video_type == VideoType::Clip,
                rank_of(v.iso_639_1.as_deref()),
                *video_type,
                !v.official,
                *site == Site::Vimeo,
                std::cmp::Reverse(v.size.unwrap_or(0)),
                std::cmp::Reverse(v.published_at.clone().unwrap_or_default()),
            )
        })
        .map(|(v, site, video_type, key)| OnlineTrailer {
            site,
            key: key.to_owned(),
            name: v.name.trim().to_owned(),
            language: v.iso_639_1.as_deref().and_then(primary),
            video_type,
            official: v.official,
        })
}

/// The trailer among the extras of an item: the index of the first
/// `trailer`, else of the first `teaser` (the LIB-007 extra types).
pub fn pick_own<S: AsRef<str>>(extra_types: &[S]) -> Option<usize> {
    let find = |t: &str| extra_types.iter().position(|e| e.as_ref() == t);
    find("trailer").or_else(|| find("teaser"))
}

/// What a page plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// The extra at this index of the item's extras.
    Own(usize),
    /// A stream URL of the media server.
    Server(String),
    /// A TMDB video.
    Online(OnlineTrailer),
}

/// The trailer of a page: the own file, else the server's trailer, else —
/// only with `internet` on — what `online` finds. `online` is not called
/// when a higher source exists or `internet` is off.
pub fn choose<S: AsRef<str>>(
    extra_types: &[S],
    server: Option<String>,
    internet: bool,
    online: impl FnOnce() -> Option<OnlineTrailer>,
) -> Option<Choice> {
    if let Some(i) = pick_own(extra_types) {
        return Some(Choice::Own(i));
    }
    if let Some(url) = server.filter(|u| !u.trim().is_empty()) {
        return Some(Choice::Server(url));
    }
    if internet {
        return online().map(Choice::Online);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(lang: Option<&str>, kind: &str, official: bool, key: &str) -> TmdbVideo {
        TmdbVideo {
            iso_639_1: lang.map(str::to_owned),
            key: key.to_owned(),
            site: "YouTube".into(),
            video_type: kind.into(),
            official,
            name: format!("{kind} {}", lang.unwrap_or("-")),
            size: Some(1080),
            published_at: None,
        }
    }

    fn langs(ui: &str, original: Option<&str>) -> Vec<String> {
        preferred_languages(ui, original)
    }

    #[test]
    fn languages_are_ui_then_original_then_english() {
        assert_eq!(langs("ru-RU", Some("ja")), ["ru", "ja", "en"]);
        assert_eq!(langs("en-US", Some("en")), ["en"]);
        assert_eq!(langs("pt_BR", None), ["pt", "en"]);
        assert_eq!(langs("", Some("fr")), ["fr", "en"]);
        assert_eq!(
            include_video_language(&langs("ru", Some("ja"))),
            "ru,ja,en,null"
        );
    }

    #[test]
    fn ui_language_wins_over_original_and_english() {
        let v = [
            video(Some("en"), "Trailer", true, "enTrailer01"),
            video(Some("ja"), "Trailer", true, "jaTrailer01"),
            video(Some("ru"), "Trailer", true, "ruTrailer01"),
        ];
        let got = pick_online(&v, &langs("ru-RU", Some("ja"))).unwrap();
        assert_eq!(got.key, "ruTrailer01");
        assert_eq!(got.language.as_deref(), Some("ru"));
        // No Russian: the original language, then English.
        let got = pick_online(&v[..2], &langs("ru-RU", Some("ja"))).unwrap();
        assert_eq!(got.key, "jaTrailer01");
        let got = pick_online(&v[..1], &langs("ru-RU", Some("ja"))).unwrap();
        assert_eq!(got.key, "enTrailer01");
    }

    #[test]
    fn any_language_beats_nothing() {
        let v = [video(Some("de"), "Trailer", true, "deTrailer01")];
        assert_eq!(
            pick_online(&v, &langs("ru", Some("ja"))).unwrap().key,
            "deTrailer01"
        );
    }

    #[test]
    fn type_order_is_trailer_teaser_clip() {
        let v = [
            video(Some("en"), "Clip", true, "enClip00001"),
            video(Some("en"), "Teaser", true, "enTeaser001"),
            video(Some("en"), "Trailer", true, "enTrailer01"),
            video(Some("en"), "Featurette", true, "enFeature01"),
        ];
        let l = langs("en", None);
        assert_eq!(pick_online(&v, &l).unwrap().key, "enTrailer01");
        assert_eq!(pick_online(&v[..2], &l).unwrap().key, "enTeaser001");
        let clip = pick_online(&v[..1], &l).unwrap();
        assert_eq!(clip.video_type, VideoType::Clip);
        // A featurette alone is not a trailer.
        assert_eq!(pick_online(&v[3..], &l), None);
    }

    #[test]
    fn a_teaser_in_english_beats_a_clip_in_the_ui_language() {
        let v = [
            video(Some("ru"), "Clip", true, "ruClip00001"),
            video(Some("en"), "Teaser", true, "enTeaser001"),
        ];
        assert_eq!(
            pick_online(&v, &langs("ru", None)).unwrap().key,
            "enTeaser001"
        );
    }

    #[test]
    fn language_beats_type_and_official_beats_unofficial() {
        let v = [
            video(Some("en"), "Trailer", true, "enTrailer01"),
            video(Some("ru"), "Teaser", false, "ruTeaserFan"),
            video(Some("ru"), "Teaser", true, "ruTeaserOff"),
        ];
        assert_eq!(
            pick_online(&v, &langs("ru", None)).unwrap().key,
            "ruTeaserOff"
        );
    }

    #[test]
    fn youtube_beats_vimeo_and_bad_keys_are_skipped() {
        let mut vimeo = video(Some("en"), "Trailer", true, "123456789");
        vimeo.site = "Vimeo".into();
        let yt = video(Some("en"), "Trailer", true, "abcdefghijk");
        let l = langs("en", None);
        let got = pick_online(&[vimeo.clone(), yt], &l).unwrap();
        assert_eq!(got.site, Site::YouTube);
        let got = pick_online(std::slice::from_ref(&vimeo), &l).unwrap();
        assert_eq!(got.site, Site::Vimeo);
        assert_eq!(got.watch_url(), "https://vimeo.com/123456789");
        // Ids that could break out of an embed URL are refused.
        let bad = [
            video(Some("en"), "Trailer", true, "abc\"><scrip"),
            video(Some("en"), "Trailer", true, "short"),
            TmdbVideo {
                site: "Dailymotion".into(),
                ..video(Some("en"), "Trailer", true, "abcdefghijk")
            },
        ];
        assert_eq!(pick_online(&bad, &l), None);
    }

    #[test]
    fn bigger_and_newer_win_a_tie() {
        let mut small = video(Some("en"), "Trailer", true, "small000001");
        small.size = Some(360);
        let mut old = video(Some("en"), "Trailer", true, "old00000001");
        old.published_at = Some("2019-01-01T00:00:00.000Z".into());
        let mut new = video(Some("en"), "Trailer", true, "new00000001");
        new.published_at = Some("2020-01-01T00:00:00.000Z".into());
        let l = langs("en", None);
        assert_eq!(
            pick_online(&[small.clone(), old.clone(), new], &l)
                .unwrap()
                .key,
            "new00000001"
        );
        assert_eq!(pick_online(&[small, old], &l).unwrap().key, "old00000001");
    }

    #[test]
    fn parses_a_tmdb_response() {
        let body = r#"{"id":603,"results":[
          {"iso_639_1":"en","iso_3166_1":"US","name":"Official Trailer",
           "key":"vKQi3bBA1y8","site":"YouTube","size":1080,"type":"Trailer",
           "official":true,"published_at":"2021-12-09T17:00:00.000Z","id":"x"},
          {"iso_639_1":null,"name":"No language","key":"m8e-FF8MsqU",
           "site":"YouTube","type":"Teaser"}]}"#;
        let v = parse_tmdb_videos(body).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].iso_639_1, None);
        assert!(!v[1].official);
        let got = pick_online(&v, &langs("ru", Some("en"))).unwrap();
        assert_eq!(got.key, "vKQi3bBA1y8");
        assert_eq!(got.name, "Official Trailer");
        assert_eq!(
            got.watch_url(),
            "https://www.youtube.com/watch?v=vKQi3bBA1y8"
        );
        assert!(parse_tmdb_videos("\"videos\"").is_err());
        assert_eq!(parse_tmdb_videos("{}").unwrap(), []);
    }

    #[test]
    fn own_trailer_then_teaser() {
        assert_eq!(pick_own(&["featurette", "teaser", "trailer"]), Some(2));
        assert_eq!(pick_own(&["featurette", "teaser"]), Some(1));
        assert_eq!(pick_own(&["featurette", "interview"]), None);
        assert_eq!(pick_own::<&str>(&[]), None);
    }

    fn online() -> Option<OnlineTrailer> {
        pick_online(
            &[video(Some("en"), "Trailer", true, "enTrailer01")],
            &langs("en", None),
        )
    }

    #[test]
    fn own_file_wins_and_tmdb_is_not_asked() {
        let mut asked = false;
        let got = choose(
            &["behind_the_scenes", "trailer"],
            Some("http://server/Videos/1/stream".into()),
            true,
            || {
                asked = true;
                online()
            },
        );
        assert_eq!(got, Some(Choice::Own(1)));
        assert!(!asked, "an own trailer must not cause a TMDB request");
    }

    #[test]
    fn server_trailer_comes_before_tmdb() {
        let mut asked = false;
        let got = choose::<&str>(&[], Some("http://server/t".into()), true, || {
            asked = true;
            online()
        });
        assert_eq!(got, Some(Choice::Server("http://server/t".into())));
        assert!(!asked);
        // A blank server URL is none.
        let got = choose::<&str>(&[], Some("  ".into()), true, online);
        assert!(matches!(got, Some(Choice::Online(_))));
    }

    #[test]
    fn tmdb_last_and_only_with_internet_on() {
        let got = choose(&["featurette"], None, true, online);
        assert!(matches!(got, Some(Choice::Online(t)) if t.key == "enTrailer01"));
        let mut asked = false;
        let got = choose::<&str>(&[], None, false, || {
            asked = true;
            online()
        });
        assert_eq!(got, None);
        assert!(!asked, "internet trailers off: no request at all");
        // Own files still play with internet trailers off.
        assert_eq!(
            choose(&["teaser"], None, false, online),
            Some(Choice::Own(0))
        );
    }
}
