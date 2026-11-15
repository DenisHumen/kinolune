// SPDX-License-Identifier: MPL-2.0
//! TWI-019 "done when": importing a TwitchDownloader folder matches the
//! files by video id and pulls in the chapters. Also yt-dlp and streamlink
//! archives, our own downloads and recordings, deduplication and files
//! without an id. Made-up files only: no Twitch, no FFmpeg.

#![cfg(all(pa_twitch_archive, pa_twitch_native))]

use std::fs;
use std::io::Write;
use std::path::Path;

use pa_library::twitch::{self as lib, AssetKind, SegmentOwner};
use pa_model::MediaKey;
use pa_twitch_archive::import::{FoundKind, Origin, scan};
use pa_twitch_archive::library::{self, UserLookup, import_dir};
use serde_json::json;

const NOW: i64 = 1_800_000_000_000;

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn write_json(path: &Path, v: &serde_json::Value) {
    write(path, &serde_json::to_vec_pretty(v).unwrap());
}

fn write_gz(path: &Path, v: &serde_json::Value) {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&serde_json::to_vec(v).unwrap()).unwrap();
    write(path, &gz.finish().unwrap());
}

/// A TwitchDownloader chat JSON (the format of TwitchDownloaderCLI
/// `chatdownload`): streamer, video with chapters, comments.
fn tdl_chat(video_id: &str, comments: usize) -> serde_json::Value {
    json!({
        "FileInfo": {"Version": {"Major": 1, "Minor": 4, "Patch": 0}},
        "streamer": {"name": "MadeUpStreamer", "id": 4040},
        "video": {
            "title": "Practice run: all bosses",
            "description": null,
            "id": video_id,
            "created_at": "2026-11-12T18:00:00Z",
            "start": 0,
            "end": 10800,
            "length": 10800,
            "viewCount": 12,
            "game": "Elden Ring",
            "chapters": [
                {"id": "c1", "startMilliseconds": 0, "lengthMilliseconds": 3_600_000,
                 "_type": "GAME_CHANGE", "description": "Elden Ring", "subDescription": "",
                 "thumbnailUrl": "", "gameId": "512953", "gameDisplayName": "Elden Ring",
                 "gameBoxArtUrl": ""},
                {"id": "c2", "startMilliseconds": 3_600_000, "lengthMilliseconds": 7_200_000,
                 "_type": "GAME_CHANGE", "description": "Hades II", "subDescription": "",
                 "thumbnailUrl": "", "gameId": "1159690097", "gameDisplayName": "Hades II",
                 "gameBoxArtUrl": ""}
            ]
        },
        "comments": (0..comments).map(|i| json!({
            "_id": format!("m{i}"),
            "created_at": "2026-11-12T18:00:00Z",
            "channel_id": "4040",
            "content_type": "video",
            "content_id": video_id,
            "content_offset_seconds": 10 + i * 60,
            "commenter": {"display_name": "Viewer", "_id": "9", "name": "viewer"},
            "message": {"body": "hi", "fragments": [{"text": "hi", "emoticon": null}]}
        })).collect::<Vec<_>>(),
        "embeddedData": null
    })
}

struct NoLookup;
impl UserLookup for NoLookup {
    fn user_by_login(&self, _: &str) -> Option<(String, String)> {
        None
    }
}

struct FakeHelix;
impl UserLookup for FakeHelix {
    fn user_by_login(&self, login: &str) -> Option<(String, String)> {
        (login == "ytstreamer").then(|| ("5050".to_owned(), "YtStreamer".to_owned()))
    }
}

#[test]
fn twitch_downloader_folder_matches_by_video_id_and_pulls_chapters() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("TwitchDownloader");
    // TwitchDownloader's default names do not share a stem: the chat file
    // is matched by the video id in the video's name.
    write(&root.join("[2026-11-12] MadeUpStreamer - 777000111.mp4"), b"video");
    write_json(&root.join("MadeUpStreamer_777000111_chat.json"), &tdl_chat("777000111", 5));
    // Another VOD in the same folder, chat gzipped.
    write(&root.join("777000222.mp4"), b"video two");
    write_gz(&root.join("777000222.json.gz"), &tdl_chat("777000222", 2));

    let (found, skipped) = scan(&root);
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(found.len(), 2);
    let first = found
        .iter()
        .find(|f| f.video_id.as_deref() == Some("777000111"))
        .unwrap();
    assert_eq!(first.origin, Origin::TwitchDownloader);
    assert_eq!(first.kind, FoundKind::Vod);
    assert_eq!(first.user_id.as_deref(), Some("4040"));
    assert_eq!(first.chapters.len(), 2);
    assert_eq!(first.chapters[1].game_id.as_deref(), Some("1159690097"));
    assert_eq!(first.chat.as_ref().unwrap().message_count, 5);

    let mut conn = pa_library::open_in_memory().unwrap();
    let report = import_dir(&mut conn, &root, "TwitchDownloader", "import", None, NOW).unwrap();
    assert_eq!(report.added.len(), 2, "{report:?}");
    assert_eq!(report.chapters, 4);

    // The VOD, its chapters with category names, the saved file and chat.
    let video = lib::video(&conn, "777000111").unwrap().unwrap();
    assert_eq!(video.user_id, "4040");
    assert_eq!(video.title, "Practice run: all bosses");
    assert_eq!(video.duration_ms, 10_800_000);
    let segs = lib::segments(&conn, &SegmentOwner::Video("777000111".into())).unwrap();
    assert_eq!(segs.len(), 2);
    assert_eq!(segs[0].game_id, "512953");
    assert_eq!((segs[1].start_ms, segs[1].end_ms), (3_600_000, Some(10_800_000)));
    assert_eq!(segs[1].source, "import");
    assert_eq!(lib::category(&conn, "1159690097").unwrap().unwrap().name, "Hades II");
    let assets = lib::assets_for_key(
        &conn,
        &MediaKey::TwitchVod {
            video_id: "777000111".into(),
        },
    )
    .unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].kind, AssetKind::Vod);
    assert_eq!(assets[0].origin, "import");
    let chat = lib::chat_log(&conn, assets[0].id).unwrap().unwrap();
    assert_eq!(chat.path, "MadeUpStreamer_777000111_chat.json");
    assert_eq!(chat.message_count, 5);
    assert_eq!((chat.first_offset_ms, chat.last_offset_ms), (Some(10_000), Some(250_000)));
    assert_eq!(lib::streamer(&conn, "4040").unwrap().unwrap().login, "madeupstreamer");

    // The library view lists both, the folder is not a media source.
    let entries = library::entries(&conn, Some(AssetKind::Vod)).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|e| e.has_chat && e.chapters == 2));
    assert!(entries[0].path.is_file());
    let kinds: Vec<String> = conn
        .prepare("SELECT kind FROM sources")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(kinds, vec![library::SOURCE_KIND.to_owned()]);

    // Importing again adds nothing; a copy of the same VOD elsewhere is a
    // duplicate.
    let again = import_dir(&mut conn, &root, "TwitchDownloader", "import", None, NOW).unwrap();
    assert!(again.added.is_empty());
    assert_eq!(again.known, 2);
    let copy = dir.path().join("copy");
    write(&copy.join("777000111.mp4"), b"video");
    write_json(&copy.join("777000111.json"), &tdl_chat("777000111", 5));
    let dup = import_dir(&mut conn, &copy, "copy", "import", None, NOW).unwrap();
    assert_eq!(dup.duplicates.len(), 1, "{dup:?}");
    assert!(library::saved_asset(&conn, FoundKind::Vod, "777000111", None).unwrap().is_some());
}

#[test]
fn yt_dlp_and_streamlink_archives() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("archive");
    // yt-dlp: `%(title)s [%(id)s].%(ext)s` and `--write-info-json`.
    let name = "Boss rush [v888000111]";
    write(&root.join(format!("{name}.mp4")), b"vod");
    write_json(
        &root.join(format!("{name}.info.json")),
        &json!({
            "id": "v888000111", "title": "Boss rush", "extractor": "twitch:vod",
            "extractor_key": "TwitchVod", "webpage_url": "https://www.twitch.tv/videos/888000111",
            "uploader": "YtStreamer", "uploader_id": "ytstreamer", "timestamp": 1_763_000_000,
            "duration": 5400.0, "format_id": "1080p60",
            "chapters": [
                {"start_time": 0.0, "end_time": 1800.0, "title": "Just Chatting"},
                {"start_time": 1800.0, "end_time": 5400.0, "title": "Elden Ring"}
            ]
        }),
    );
    // yt-dlp clip.
    write(&root.join("clips/Funny moment [FunnySlug-x1].mp4"), b"clip");
    write_json(
        &root.join("clips/Funny moment [FunnySlug-x1].info.json"),
        &json!({
            "id": "123456", "display_id": "FunnySlug-x1", "title": "Funny moment",
            "extractor": "twitch:clips", "webpage_url": "https://clips.twitch.tv/FunnySlug-x1",
            "uploader": "YtStreamer", "uploader_id": "ytstreamer", "duration": 30.0
        }),
    );
    // streamlink `--json` dump next to a recording.
    write(&root.join("live/ytstreamer 2026-11-13.ts"), b"live");
    write_json(
        &root.join("live/ytstreamer 2026-11-13.json"),
        &json!({
            "plugin": "twitch",
            "metadata": {"id": "51234567890", "author": "YtStreamer",
                         "category": "Elden Ring", "title": "Late night"},
            "streams": {}
        }),
    );
    // Unknown file: skipped.
    write(&root.join("holiday.mkv"), b"x");

    let mut conn = pa_library::open_in_memory().unwrap();
    // A known category lets yt-dlp's chapter names become chapters.
    lib::upsert_category(
        &conn,
        &lib::Category {
            game_id: "512953".into(),
            name: "Elden Ring".into(),
            box_art_url: None,
            igdb_id: None,
        },
        NOW,
    )
    .unwrap();

    // Without Helix the login stays unresolved.
    let report = import_dir(&mut conn, &root, "archive", "import", Some(&NoLookup), NOW).unwrap();
    assert_eq!(report.no_user.len(), 3, "{report:?}");
    assert_eq!(report.skipped.len(), 1);
    assert!(report.skipped[0].path.ends_with("holiday.mkv"));

    let report = import_dir(&mut conn, &root, "archive", "import", Some(&FakeHelix), NOW).unwrap();
    assert_eq!(report.added.len(), 3, "{report:?}");
    assert_eq!(report.chapters, 1, "only the chapter with a known category");
    let by_kind = |k| library::entries(&conn, Some(k)).unwrap();
    let vods = by_kind(AssetKind::Vod);
    assert_eq!(vods[0].video_id.as_deref(), Some("888000111"));
    assert_eq!(vods[0].user_id, "5050");
    assert_eq!(vods[0].duration_ms, Some(5_400_000));
    let clips = by_kind(AssetKind::Clip);
    assert_eq!(clips[0].clip_slug.as_deref(), Some("FunnySlug-x1"));
    let recs = by_kind(AssetKind::Recording);
    assert_eq!(recs[0].stream_id.as_deref(), Some("51234567890"));
    assert_eq!(recs[0].title, "Late night");
    assert_eq!(recs[0].display_name.as_deref(), Some("YtStreamer"));
}

#[test]
fn own_downloads_and_recordings_by_info_json_and_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("twitch");
    let vod = "made_up_streamer [4040]/2026/2026-11-12 Elden Ring — Run [v777000333]";
    let base = "2026-11-12 Elden Ring — Run [v777000333]";
    write(&root.join(vod).join(format!("{base}.mkv")), b"mkv");
    write_json(
        &root.join(vod).join(format!("{base}.info.json")),
        &json!({
            "format": "kinolune.twitch.vod/1",
            "video": {"id": "777000333", "user_id": "4040", "user_login": "made_up_streamer",
                      "user_name": "Made Up", "title": "Run", "created_at": "2026-11-12T18:00:00Z",
                      "type": "archive", "duration": "1h0m0s"},
            "chapters": [{"start_ms": 0, "end_ms": 3_600_000, "title": "Elden Ring"}],
            "variant": {"quality": "1080p60"},
            "file": {"name": format!("{base}.mkv"), "duration_ms": 3_600_000},
        }),
    );
    write_gz(
        &root.join(vod).join(format!("{base}.chat.json.gz")),
        &json!({"video": {"id": "777000333"}, "comments": [{"content_offset_seconds": 1}]}),
    );
    let rec = "made_up_streamer [4040]/2026/2026-11-13 LIVE Chat — Late [s90001]";
    let rbase = "2026-11-13 LIVE Chat — Late [s90001]";
    write(&root.join(rec).join(format!("{rbase}.mkv")), b"mkv");
    write_json(
        &root.join(rec).join(format!("{rbase}.info.json")),
        &json!({
            "format": "kinolune.twitch.live/1",
            "stream": {"stream_id": "90001", "user_id": "4040", "login": "made_up_streamer",
                       "display_name": "Made Up", "title": "Late", "started_at_ms": NOW},
            "ads": [{"start_ms": 0, "end_ms": 30_000}],
            "file": {"name": format!("{rbase}.mkv"), "duration_ms": 7_200_000},
        }),
    );
    // The recording's chat: a chat JSON whose video id is the stream id
    // must not turn the recording into a VOD.
    write_gz(
        &root.join(rec).join(format!("{rbase}.chat.json.gz")),
        &json!({"video": {"id": "90001"}, "comments": []}),
    );
    // A file renamed by hand, no sidecar: found by `[v…]` in the name, its
    // channel unknown without a login.
    write(&root.join("loose/Some title [v777000444].mkv"), b"mkv");
    // `.tmp` of the storage target is never imported.
    write(&root.join(".tmp/job-1/out/x [v1].mkv"), b"tmp");

    let mut conn = pa_library::open_in_memory().unwrap();
    let report = import_dir(&mut conn, &root, "Twitch", "desktop", None, NOW).unwrap();
    assert_eq!(report.added.len(), 2, "{report:?}");
    assert_eq!(report.no_user.len(), 1);
    let recs = library::entries(&conn, Some(AssetKind::Recording)).unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].stream_id.as_deref(), Some("90001"));
    assert_eq!(recs[0].video_id, None);
    assert!(recs[0].has_ads_marked && recs[0].has_chat);
    assert_eq!(recs[0].recorded_at_ms, Some(NOW));
    let vods = library::entries(&conn, Some(AssetKind::Vod)).unwrap();
    assert_eq!(vods[0].login.as_deref(), Some("made_up_streamer"));
    assert_eq!(vods[0].origin, "desktop");

    // Gone from disk (retention moved it to the trash): not listed.
    fs::remove_file(root.join(rec).join(format!("{rbase}.mkv"))).unwrap();
    assert!(library::entries(&conn, Some(AssetKind::Recording)).unwrap().is_empty());
}
