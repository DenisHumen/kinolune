// SPDX-License-Identifier: MPL-2.0
//! LIB-025: the content kind of a synthetic corpus of paths and tags
//! (`tests/data/content_corpus.jsonl`, made by `gen_content_corpus.py`):
//! movies, shows, anime, albums, audiobooks, phone and camera clips and
//! mixed folders, all invented. Files are decided per folder, as the
//! catalog does, with no input from the user.
//!
//! Done when (ROADMAP LIB-025): at least 95 % right, and fewer than 1 %
//! silent «movie ↔ own video» mistakes.
//!
//! `cargo test -p pa-recognition --test content_corpus -- --nocapture`
//! prints the confusion table and every mistake.

use std::collections::BTreeMap;

use pa_recognition::content::{
    AudioFile, ContentKind, VideoFile, classify_audio, classify_videos, folder_of,
};
use serde_json::Value;

const CORPUS: &str = include_str!("data/content_corpus.jsonl");

const AUDIO: &[&str] = &[
    "mp3", "m4a", "m4b", "flac", "opus", "ogg", "wav", "aac", "ape", "wv", "aax",
];

struct Row {
    path: String,
    truth: ContentKind,
    duration_ms: Option<u64>,
    tags: Vec<(String, String)>,
    genres: Vec<String>,
    texts: Vec<String>,
    chapters: u32,
}

fn rows() -> Vec<Row> {
    CORPUS
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: Value = serde_json::from_str(l).expect("valid JSON line");
            let strings = |k: &str| -> Vec<String> {
                v[k].as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            Row {
                path: v["path"].as_str().unwrap().to_owned(),
                truth: ContentKind::parse(v["kind"].as_str().unwrap()).expect("known kind"),
                duration_ms: v["duration_ms"].as_u64(),
                tags: v["tags"]
                    .as_object()
                    .map(|o| {
                        o.iter()
                            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned()))
                            .collect()
                    })
                    .unwrap_or_default(),
                genres: strings("genres"),
                texts: strings("texts"),
                chapters: v["chapters"]
                    .as_u64()
                    .and_then(|c| u32::try_from(c).ok())
                    .unwrap_or(0),
            }
        })
        .collect()
}

fn is_audio(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    AUDIO.contains(&ext.as_str())
}

/// Every row with what the classifier said.
fn decide(rows: &[Row]) -> Vec<ContentKind> {
    let mut out = vec![ContentKind::Video; rows.len()];
    let mut groups: BTreeMap<(&str, bool), Vec<usize>> = BTreeMap::new();
    for (i, r) in rows.iter().enumerate() {
        groups
            .entry((folder_of(&r.path), is_audio(&r.path)))
            .or_default()
            .push(i);
    }
    for ((_, audio), idx) in groups {
        if audio {
            let files: Vec<AudioFile<'_>> = idx
                .iter()
                .map(|&i| AudioFile {
                    path: &rows[i].path,
                    duration_ms: rows[i].duration_ms,
                    chapters: rows[i].chapters,
                    genres: rows[i].genres.iter().map(String::as_str).collect(),
                    texts: rows[i].texts.iter().map(String::as_str).collect(),
                })
                .collect();
            for (v, &i) in classify_audio(&files).into_iter().zip(&idx) {
                out[i] = v.kind;
            }
        } else {
            let files: Vec<VideoFile<'_>> = idx
                .iter()
                .map(|&i| VideoFile {
                    path: &rows[i].path,
                    duration_ms: rows[i].duration_ms,
                    tags: rows[i]
                        .tags
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect(),
                })
                .collect();
            for (v, &i) in classify_videos(&files).into_iter().zip(&idx) {
                out[i] = v.kind;
            }
        }
    }
    out
}

#[test]
fn corpus_is_classified_without_the_user() {
    let rows = rows();
    assert!(rows.len() >= 300, "corpus has {} files", rows.len());
    let got = decide(&rows);
    let mut confusion: BTreeMap<(ContentKind, ContentKind), usize> = BTreeMap::new();
    for (r, g) in rows.iter().zip(&got) {
        *confusion.entry((r.truth, *g)).or_default() += 1;
        if r.truth != *g {
            println!("MISS {:>9} → {:<9} {}", r.truth.as_str(), g.as_str(), r.path);
        }
    }
    let total = rows.len();
    let right: usize = confusion
        .iter()
        .filter(|((t, g), _)| t == g)
        .map(|(_, n)| n)
        .sum();
    let film_own = confusion
        .iter()
        .filter(|((t, g), _)| {
            matches!(
                (t, g),
                (ContentKind::Video, ContentKind::OwnVideo)
                    | (ContentKind::OwnVideo, ContentKind::Video)
            )
        })
        .map(|(_, n)| n)
        .sum::<usize>();
    #[allow(clippy::cast_precision_loss)]
    let pct = |n: usize| n as f64 * 100.0 / total as f64;
    println!("files: {total}");
    for ((t, g), n) in &confusion {
        println!("  {:>9} → {:<9} {n}", t.as_str(), g.as_str());
    }
    println!(
        "right: {right} ({:.1} %), movie ↔ own video: {film_own} ({:.2} %)",
        pct(right),
        pct(film_own)
    );
    assert!(pct(right) >= 95.0, "accuracy {:.1} % < 95 %", pct(right));
    assert!(
        pct(film_own) < 1.0,
        "movie ↔ own video {:.2} % ≥ 1 %",
        pct(film_own)
    );
}

#[test]
fn corpus_covers_every_kind() {
    let rows = rows();
    for kind in [
        ContentKind::Video,
        ContentKind::OwnVideo,
        ContentKind::Music,
        ContentKind::Audiobook,
    ] {
        let n = rows.iter().filter(|r| r.truth == kind).count();
        assert!(n >= 50, "{} has {n} files", kind.as_str());
    }
}
