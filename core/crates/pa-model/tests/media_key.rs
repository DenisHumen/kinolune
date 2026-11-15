// SPDX-License-Identifier: MPL-2.0
//! `MediaKey` parses and prints without loss.

use pa_model::{MAX_KEY_LEN, MediaKey, ParseKeyError};

const VALID: &[&str] = &[
    "file:0123456789abcdef",
    "file:8e245d9679d31e1225d9a5c9e8c7b8a9",
    "tmdb:movie:603",
    "tmdb:episode:1399:1:1",
    "tmdb:episode:1399:0:12",
    "mb:recording:b1a9c0e9-d987-4042-ae91-78d6a3267d69",
    "twitch:vod:2189457123",
    "twitch:rec:41375541868",
    "twitch:clip:AwkwardHelplessSalamanderSwiftRage",
    "twitch:clip:Funny-Clip_slug-9",
    "anidb:episode:23:1",
    "ed2k:31d6cfe0d16ae931b73c59d7e0c089c0:9728000",
];

#[test]
fn valid_keys_round_trip_through_text() {
    for s in VALID {
        let key: MediaKey = s.parse().unwrap_or_else(|e| panic!("{s}: {e}"));
        assert_eq!(key.to_string(), *s);
        assert!(key.is_valid());
        assert_eq!(key.to_string().parse::<MediaKey>(), Ok(key));
    }
}

#[test]
fn valid_keys_round_trip_through_serde() {
    for s in VALID {
        let key: MediaKey = s.parse().unwrap();
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, format!("\"{s}\""));
        assert_eq!(serde_json::from_str::<MediaKey>(&json).unwrap(), key);
    }
}

#[test]
fn parsed_fields() {
    assert_eq!(
        "tmdb:episode:1399:2:10".parse(),
        Ok(MediaKey::TmdbEpisode {
            show_id: 1399,
            season: 2,
            episode: 10
        })
    );
    assert_eq!(
        "twitch:vod:42".parse(),
        Ok(MediaKey::TwitchVod {
            video_id: "42".into()
        })
    );
    assert!(MediaKey::TwitchClip { slug: "x".into() }.is_twitch());
    assert!(!MediaKey::TmdbMovie { id: 1 }.is_twitch());
    assert_eq!(
        "anidb:episode:23:5".parse(),
        Ok(MediaKey::AnidbEpisode {
            aid: 23,
            episode: 5
        })
    );
}

#[test]
fn newer_kinds_are_not_v1_wire_keys() {
    for s in VALID {
        let key: MediaKey = s.parse().unwrap();
        let newer = s.starts_with("anidb:") || s.starts_with("ed2k:");
        assert_eq!(key.is_v1_wire(), !newer, "{s}");
        let content = s.starts_with("file:") || s.starts_with("ed2k:");
        assert_eq!(key.is_content(), content, "{s}");
    }
}

#[test]
fn non_canonical_and_malformed_keys_are_rejected() {
    use ParseKeyError::*;
    let cases: &[(&str, ParseKeyError)] = &[
        ("", BadLength),
        ("file", WrongArity),
        ("smb://nas/movie.mkv", UnknownKind),
        ("tmdb:tv:1399", UnknownKind),
        ("TWITCH:vod:1", UnknownKind),
        ("file:", InvalidPart),
        ("file:ABCDEF", InvalidPart),
        ("file:xyz", InvalidPart),
        ("file:ab:cd", WrongArity),
        ("tmdb:movie:0", InvalidPart),
        ("tmdb:movie:0603", InvalidPart),
        ("tmdb:movie:+603", InvalidPart),
        ("tmdb:movie:18446744073709551616", InvalidPart),
        ("tmdb:movie:603:1", WrongArity),
        ("tmdb:episode:1399:1", WrongArity),
        ("tmdb:episode:1399:01:1", InvalidPart),
        ("tmdb:episode:1399:1:4294967296", InvalidPart),
        ("tmdb:episode:0:1:1", InvalidPart),
        (
            "mb:recording:B1A9C0E9-D987-4042-AE91-78D6A3267D69",
            InvalidPart,
        ),
        ("mb:recording:b1a9c0e9d9874042ae9178d6a3267d69", InvalidPart),
        ("twitch:vod:", InvalidPart),
        ("twitch:vod:v123", InvalidPart),
        ("twitch:vod:0", InvalidPart),
        ("twitch:rec:007", InvalidPart),
        ("twitch:clip:has space", InvalidPart),
        ("twitch:clip:a/b", InvalidPart),
        ("twitch:clip:a:b", WrongArity),
        ("twitch:live:123", UnknownKind),
        ("anidb:episode:23", WrongArity),
        ("anidb:episode:0:1", InvalidPart),
        ("anidb:episode:23:0", InvalidPart),
        ("anidb:episode:23:S1", InvalidPart),
        ("anidb:anime:23", UnknownKind),
        ("ed2k:31d6cfe0d16ae931b73c59d7e0c089c0", WrongArity),
        ("ed2k:31D6CFE0D16AE931B73C59D7E0C089C0:1", InvalidPart),
        ("ed2k:31d6cfe0:1", InvalidPart),
        ("ed2k:31d6cfe0d16ae931b73c59d7e0c089c0:0", InvalidPart),
    ];
    for (s, err) in cases {
        assert_eq!(s.parse::<MediaKey>(), Err(*err), "{s:?}");
    }
    let long = format!("file:{}", "a".repeat(MAX_KEY_LEN));
    assert_eq!(long.parse::<MediaKey>(), Err(BadLength));
    assert!(serde_json::from_str::<MediaKey>("\"tmdb:movie:0\"").is_err());
}

#[test]
fn hand_built_invalid_key_is_detected() {
    let bad = MediaKey::File {
        fingerprint: "NOT-HEX".into(),
    };
    assert!(!bad.is_valid());
}

#[test]
fn generated_numeric_keys_round_trip() {
    let numbers = [1u64, 9, 10, 99, 100, 65_535, u32::MAX as u64, u64::MAX];
    for &a in &numbers {
        let movie = MediaKey::TmdbMovie { id: a };
        assert_eq!(movie.to_string().parse::<MediaKey>(), Ok(movie));
        for &b in &[0u32, 1, 10, u32::MAX] {
            let ep = MediaKey::TmdbEpisode {
                show_id: a,
                season: b,
                episode: b,
            };
            assert_eq!(ep.to_string().parse::<MediaKey>(), Ok(ep));
        }
        let vod = MediaKey::TwitchVod {
            video_id: a.to_string(),
        };
        assert!(vod.is_valid());
    }
}
