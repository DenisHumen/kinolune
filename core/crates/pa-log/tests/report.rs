// SPDX-License-Identifier: MPL-2.0
//! CORE-008 "done when": the report holds no user paths or tokens, and
//! nothing is saved without the opt-in.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pa_log::{Diagnostics, Redactor, ReportInfo, Rotation, read_stored};

const HOME: &str = "/home/alice";
const WIN_HOME: &str = r"C:\Users\Alice";

/// Every secret the lines below carry; none may reach the report.
const SECRETS: &[&str] = &[
    "/home/alice",
    "/home/bob",
    "alice",
    "Alice",
    "hunter2",
    "jf-access-0123",
    "6f1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e",
    "q8xzb3twitchoauthtoken5k2m9",
    "twitchrefresh77abc",
    "plexTok3n9876",
    "subsonicpass1",
    "e10adc3949ba59abbe56e057f20f883e",
    "c19b2dsalt",
    "deadbeefsig42",
    "9e107d9d372bb6826bd81d3542a419d6",
    "eyJhbGciOiJIUzI1NiJ9",
    "tmdbsecret00",
    "cookieValue123",
    "sessionId42abc",
];

fn lines() -> Vec<String> {
    vec![
        format!("scan {HOME}/Videos/Holiday 2025.mkv"),
        format!(r"open {WIN_HOME}\Videos\film.mkv"),
        "watching /home/bob/Movies/x.mkv".into(),
        "smb://alice:hunter2@nas.local/media/a.mkv".into(),
        "GET https://jf.local/Items?api_key=jf-access-0123&Fields=Overview".into(),
        "proxy http://127.0.0.1:40123/6f1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e/stream".into(),
        "twitch Authorization: OAuth q8xzb3twitchoauthtoken5k2m9".into(),
        "irc PASS oauth:q8xzb3twitchoauthtoken5k2m9".into(),
        r#"refresh {"refresh_token": "twitchrefresh77abc", "expires_in": 3600}"#.into(),
        "plex https://plex.local/library?X-Plex-Token=plexTok3n9876".into(),
        "subsonic /rest/ping?u=alice&p=subsonicpass1&v=1.16".into(),
        "subsonic /rest/ping?u=alice&t=e10adc3949ba59abbe56e057f20f883e&s=c19b2dsalt".into(),
        "usher https://usher.ttvnw.net/vod/1.m3u8?sig=deadbeefsig42&token=%7B%7D".into(),
        "tmdb key 9e107d9d372bb6826bd81d3542a419d6".into(),
        "jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig".into(),
        "password=hunter2 cookie: cookieValue123 session=sessionId42abc".into(),
        "client_secret: tmdbsecret00".into(),
        "file:///home/alice/Music/a.flac".into(),
        "http://127.0.0.1:40123/x/smb%3A%2F%2Falice%3Ahunter2%40nas%2F%2Fhome%2Falice".into(),
    ]
}

fn open(dir: &Path) -> Arc<Diagnostics> {
    Arc::new(Diagnostics::open(
        dir.join("diagnostics"),
        Redactor::with_homes([HOME, WIN_HOME]),
        Rotation::default(),
    ))
}

fn all_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}

fn info() -> ReportInfo {
    ReportInfo {
        versions: vec![
            ("app".into(), "0.1.0 (full)".into()),
            ("core".into(), "0.1.0".into()),
            (
                "data_dir".into(),
                format!("{HOME}/.local/share/app.kinolune"),
            ),
        ],
        config: vec![
            ("ui_locale".into(), "ru".into()),
            ("tmdb_api_key".into(), "tmdbsecret00".into()),
            ("twitch_token".into(), "q8xzb3twitchoauthtoken5k2m9".into()),
            ("last_folder".into(), format!("{HOME}/Videos")),
            ("profiles_pin".into(), "1234".into()),
        ],
        attachments: vec![(
            "playback.txt".into(),
            format!("Source: {HOME}/Videos/film.mkv\nDropped frames: 2"),
        )],
    }
}

#[test]
fn report_has_no_user_paths_or_tokens() {
    let tmp = tempfile::tempdir().unwrap();
    let d = open(tmp.path());
    d.set_enabled(true).unwrap();
    for l in lines() {
        d.log("info", "test", &l);
    }
    tracing::subscriber::with_default(d.subscriber(), || {
        for l in lines() {
            tracing::warn!(target: "pa_test", "{l}");
        }
    });
    d.record_crash("dart", &lines().join("\n"), &lines().join("\n"));
    let out = tmp.path().join("out/report.zip");
    let summary = d.export_report(&out, &info()).unwrap();
    assert_eq!(summary.path, out);
    let bytes = std::fs::read(&out).unwrap();
    let entries = read_stored(&bytes).unwrap();
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&"versions.txt") && names.contains(&"config.txt"),
        "{names:?}"
    );
    assert!(names.iter().any(|n| n.starts_with("logs/")), "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("crashes/")), "{names:?}");
    assert_eq!(summary.files as usize, entries.len());

    // The raw zip (names and data, stored uncompressed) has none of them.
    let raw = String::from_utf8_lossy(&bytes);
    for s in SECRETS {
        assert!(!raw.contains(s), "report leaks {s:?}");
    }
    // The useful parts survived.
    let text = |name: &str| {
        entries
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, d)| String::from_utf8_lossy(d).into_owned())
            .unwrap()
    };
    let log = text("logs/kinolune.log");
    assert!(log.contains("scan ~/Videos/Holiday 2025.mkv"), "{log}");
    assert!(log.contains("nas.local/media/a.mkv"), "{log}");
    assert!(log.contains("pa_test"), "{log}");
    let config = text("config.txt");
    assert!(config.contains("ui_locale: ru"), "{config}");
    assert!(config.contains("last_folder: ~/Videos"), "{config}");
    assert!(config.contains("profiles_pin: <redacted>"), "{config}");
    let playback = text("extra/playback.txt");
    assert!(playback.contains("Source: ~/Videos/film.mkv"), "{playback}");
    assert!(playback.contains("Dropped frames: 2"), "{playback}");

    // The issue link carries no secret either, and is a GitHub link.
    let url = d.issue_url(
        "https://github.com/DenisHumen/kinolune",
        &info().versions,
        "kinolune-diagnostics.zip",
    );
    assert!(url.starts_with("https://github.com/DenisHumen/kinolune/issues/new?"));
    let decoded = url
        .replace("%2F", "/")
        .replace("%3A", ":")
        .replace("%20", " ");
    for s in SECRETS {
        assert!(
            !decoded.contains(s) && !url.contains(s),
            "link leaks {s:?}: {url}"
        );
    }
}

#[test]
fn nothing_is_saved_without_opt_in() {
    let tmp = tempfile::tempdir().unwrap();
    let d = open(tmp.path());
    assert!(!d.enabled());
    for l in lines() {
        d.log("error", "test", &l);
    }
    tracing::subscriber::with_default(d.subscriber(), || {
        tracing::error!(target: "pa_test", "core error {HOME}");
    });
    assert!(d.record_crash("dart", "boom", "#0").is_none());
    assert!(
        all_files(tmp.path()).is_empty(),
        "{:?}",
        all_files(tmp.path())
    );
    assert_eq!(d.status(), pa_log::Status::default());

    // Export still works, with versions and config only, and writes
    // nothing but the chosen file.
    let out = tmp.path().join("report.zip");
    d.export_report(&out, &info()).unwrap();
    assert_eq!(all_files(tmp.path()), vec![out.clone()]);
    let entries = read_stored(&std::fs::read(&out).unwrap()).unwrap();
    assert!(
        entries
            .iter()
            .all(|(n, _)| !n.starts_with("logs/") && !n.starts_with("crashes/"))
    );

    // Opting in and out again leaves nothing behind.
    std::fs::remove_file(&out).unwrap();
    d.set_enabled(true).unwrap();
    d.log("info", "test", "kept for a moment");
    d.set_enabled(false).unwrap();
    d.log("info", "test", "dropped");
    assert!(
        all_files(tmp.path()).is_empty(),
        "{:?}",
        all_files(tmp.path())
    );
}
