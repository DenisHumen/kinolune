// SPDX-License-Identifier: MPL-2.0
//! MUS-019 done-when: a corpus of 107 audiobook structures as people and
//! release groups lay them out (`tests/fixtures/audiobooks/corpus.json`:
//! real books and narrators, no personal paths) is recognized on at least
//! 90 %, with silent errors on at most 1 %.
//!
//! Each structure is written to a library the way the scanner leaves it
//! (tags with the path fallback, the audiobook content kind, chapters of
//! single files), then [`Catalog::fetch_book_metadata`] asks Open Library
//! through recorded answers (`openlibrary.jsonl`, no network) and
//! [`Catalog::audiobooks`] gives the books.
//!
//! A book is **recognized** when its title, author, narrator, series,
//! number in the series and year are the expected ones, and the Open
//! Library work shown for it is the expected one (or none, when Open
//! Library does not have the book under that name). It is a **silent
//! error** when something is wrong and the book is not marked for review,
//! or when a wrong work is shown.
//!
//! Re-recording (needs network, one request per second):
//! `KINOLUNE_RECORD_OPENLIBRARY=1 cargo test -p pa-catalog --test
//! audiobook_corpus -- --nocapture`. Work descriptions are replaced by a
//! placeholder before the cassette is written: they are often publishers'
//! texts, which the repository must not carry.

use std::path::PathBuf;
use std::sync::Arc;

use pa_catalog::book_metadata::{BookMetadataOptions, BookProviders};
use pa_catalog::pa_metadata::cache::MemoryCache;
use pa_catalog::pa_metadata::clock::{Clock, ManualClock, SystemClock};
use pa_catalog::pa_metadata::http::{HttpTransport, UreqTransport};
use pa_catalog::pa_metadata::openlibrary::{OpenLibraryClient, OpenLibraryConfig};
use pa_catalog::pa_metadata::rate::RateLimit;
use pa_catalog::pa_metadata::replay::{RecordingTransport, ReplayTransport};
use pa_catalog::{Audiobook, CancelToken, Catalog};
use pa_library::music::{self, AudioInfo, TrackTags};
use pa_library::rusqlite::{Connection, params};
use serde_json::{Value, json};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audiobooks")
}

const RECORD: &str = "KINOLUNE_RECORD_OPENLIBRARY";

/// The cassette as committed: work descriptions replaced by a placeholder.
fn scrub(jsonl: &str) -> String {
    let mut out = String::new();
    for line in jsonl.lines() {
        let mut v: Value = serde_json::from_str(line).expect("recorded line");
        let path = v["request"]["path"].as_str().unwrap_or_default().to_owned();
        if path.starts_with("/works/") {
            let body = &mut v["response"]["body"];
            if body.is_object() {
                let has = body.get("description").is_some();
                let keep = json!({
                    "key": body["key"],
                    "title": body["title"],
                    "covers": body.get("covers").cloned().unwrap_or(json!([])),
                });
                *body = keep;
                if has {
                    body["description"] =
                        json!("Описание работы из Open Library (в фикстуре заменено).");
                }
            }
        }
        out.push_str(&v.to_string());
        out.push('\n');
    }
    out
}

/// Files of one corpus entry: relative path, tags, chapters (single files).
fn files_of(e: &Value) -> Vec<(String, u32)> {
    if let Some(f) = e["file"].as_str() {
        return vec![(f.to_owned(), e["chapters"].as_u64().unwrap_or(5) as u32)];
    }
    let dir = e["dir"].as_str().expect("dir or file");
    if let Some(names) = e["names"].as_array() {
        return names
            .iter()
            .map(|n| (format!("{dir}/{}", n.as_str().unwrap()), 0))
            .collect();
    }
    let parts = e["parts"].as_u64().unwrap_or(3);
    (1..=parts)
        .map(|i| (format!("{dir}/{i:02}.mp3"), 0))
        .collect()
}

fn tags_of(e: &Value, path: &str, part: usize) -> TrackTags {
    let t = &e["tags"];
    let s = |k: &str| t[k].as_str().map(str::to_owned);
    let list = |k: &str| s(k).into_iter().collect::<Vec<_>>();
    let tags = TrackTags {
        title: None,
        artists: list("artist"),
        album_artists: list("album_artist"),
        album: s("album"),
        composers: list("composer"),
        genres: list("genre"),
        track_number: t.is_object().then_some(part as u32 + 1),
        year: t["year"].as_u64().map(|y| y as u16),
        audio: AudioInfo {
            codec: Some("mp3".into()),
            duration_ms: Some(1_800_000),
            ..AudioInfo::default()
        },
        ..TrackTags::default()
    };
    pa_scanner::with_path_fallback(tags, path)
}

fn embedded(e: &Value, chapters: u32) -> String {
    let list: Vec<Value> = (0..chapters)
        .map(|i| {
            json!([
                i64::from(i) * 600_000,
                i64::from(i + 1) * 600_000,
                format!("Глава {}", i + 1)
            ])
        })
        .collect();
    let mut v = json!({"tags": {}, "chapters": chapters, "chapter_list": list});
    if let Some(b) = e["book"].as_object() {
        v["book"] = Value::Object(b.clone());
    }
    v.to_string()
}

fn build(db: &Connection, corpus: &[Value]) {
    db.execute(
        "INSERT INTO sources (id, kind, name, uri, created_at_ms) VALUES (1, 'local', 'Books', 'file:///b', 0)",
        [],
    )
    .unwrap();
    let mut id = 0i64;
    let mut tags = std::collections::HashMap::new();
    for e in corpus {
        for (part, (rel, chapters)) in files_of(e).into_iter().enumerate() {
            id += 1;
            let path = format!("/{rel}");
            db.execute(
                "INSERT INTO files (id, source_id, path, size_bytes, added_at_ms, embedded_tags_json,
                                    fingerprint)
                 VALUES (?1, 1, ?2, 10, 0, ?3, ?4)",
                params![id, path, embedded(e, chapters), format!("{id:016x}")],
            )
            .unwrap();
            db.execute(
                "INSERT INTO file_content_kinds (file_id, kind, detected, version, score, reasons_json,
                                                 classified_at_ms)
                 VALUES (?1, 'audiobook', 'audiobook', 1, 1.0, '[]', 0)",
                [id],
            )
            .unwrap();
            tags.insert(path.clone(), tags_of(e, &path, part));
        }
    }
    let mut albums = Vec::new();
    for file in music::stale_files(db, 1).unwrap() {
        let put = music::put_track(db, &file, &tags[&file.path], 1).unwrap();
        albums.extend(put.album_id);
    }
    albums.sort_unstable();
    albums.dedup();
    for a in albums {
        music::refresh_album(db, a, 1).unwrap();
    }
}

fn same(a: Option<&str>, b: Option<&str>) -> bool {
    let f = |s: Option<&str>| {
        s.map(pa_catalog::book_metadata::fold)
            .filter(|s| !s.is_empty())
    };
    f(a) == f(b)
}

/// What went wrong with a book, if anything.
fn mismatches(b: &Audiobook, x: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let text = |k: &str| x[k].as_str();
    for (k, got) in [
        ("title", Some(b.title.as_str())),
        ("author", b.author.as_deref()),
        ("narrator", b.narrator.as_deref()),
        ("series", b.series.as_deref()),
    ] {
        if !same(got, text(k)) {
            out.push(format!("{k}: {got:?} ≠ {:?}", text(k)));
        }
    }
    if b.series_number != x["number"].as_f64() {
        out.push(format!(
            "number: {:?} ≠ {:?}",
            b.series_number,
            x["number"].as_f64()
        ));
    }
    if let Some(y) = x["year"].as_i64()
        && b.year != Some(y)
    {
        out.push(format!("year: {:?} ≠ {y}", b.year));
    }
    if x["review"].as_bool().unwrap_or(false) != b.needs_review && b.open_library_id.is_none() {
        // Only a wrong flag on an otherwise right book matters here.
        if x["review"].as_bool().unwrap_or(false) {
            out.push("not marked for review".into());
        }
    }
    out
}

#[test]
fn corpus_of_audiobook_structures_is_recognized() {
    let corpus: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(fixtures().join("corpus.json")).unwrap())
            .unwrap();
    assert!(corpus.len() >= 100, "{} structures", corpus.len());

    let dir = tempfile::tempdir().unwrap();
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    build(
        &Connection::open(dir.path().join("library.db")).unwrap(),
        &corpus,
    );

    let record = std::env::var(RECORD).is_ok_and(|v| v == "1");
    let cassette = fixtures().join("openlibrary.jsonl");
    let recorder = record.then(|| {
        Arc::new(RecordingTransport::new(UreqTransport::new(
            pa_catalog::pa_metadata::USER_AGENT,
        )))
    });
    let transport: Arc<dyn HttpTransport> = match &recorder {
        Some(r) => r.clone(),
        None => Arc::new(
            ReplayTransport::from_jsonl(&std::fs::read_to_string(&cassette).unwrap()).unwrap(),
        ),
    };
    let mut config = OpenLibraryConfig::default();
    if !record {
        config.rate_limit = RateLimit {
            burst: 10_000,
            per_second: 10_000.0,
        };
    }
    // Recording keeps to Open Library's one request per second for real.
    let clock: Arc<dyn Clock> = if record {
        Arc::new(SystemClock::default())
    } else {
        Arc::new(ManualClock::at_unix(1_800_000_000))
    };
    let client = OpenLibraryClient::new(config, transport, Arc::new(MemoryCache::default()), clock);
    let providers = BookProviders {
        open_library: Some(Arc::new(client)),
    };
    let report = cat
        .fetch_book_metadata(
            &providers,
            &BookMetadataOptions::default(),
            &CancelToken::new(),
            &mut |_, _| {},
        )
        .unwrap();
    if let Some(r) = &recorder {
        std::fs::write(&cassette, scrub(&r.to_jsonl())).unwrap();
    }
    assert_eq!(report.failed, 0, "{report:?}");

    let books = cat.audiobooks().unwrap();
    assert_eq!(books.len(), corpus.len(), "one book per structure");
    let (mut ok, mut uncertain, mut silent) = (0, 0, Vec::new());
    for e in &corpus {
        let rel = e["file"].as_str().or(e["dir"].as_str()).unwrap();
        let b = books
            .iter()
            .find(|b| b.rel_path == rel)
            .unwrap_or_else(|| panic!("no book at {rel}"));
        let x = &e["expect"];
        let mut wrong = mismatches(b, x);
        let want = x["ol"].as_str();
        let got = b.open_library_id.as_deref();
        let wrong_work = got.is_some() && got != want;
        if got != want {
            wrong.push(format!("work: {got:?} ≠ {want:?}"));
        }
        println!(
            "{} {rel}\n    → {:?} / {:?} / {:?} / {:?} {:?} / {:?} / work {:?}{}",
            if wrong.is_empty() { "ok " } else { "ERR" },
            b.title,
            b.author,
            b.narrator,
            b.series,
            b.series_number,
            b.year,
            got,
            if b.needs_review { " / review" } else { "" }
        );
        for w in &wrong {
            println!("    ! {w}");
        }
        if wrong.is_empty() {
            ok += 1;
        } else if wrong_work || !b.needs_review {
            silent.push(rel.to_owned());
        } else {
            uncertain += 1;
        }
    }
    let n = corpus.len();
    println!(
        "recognized {ok}/{n} ({:.1} %), uncertain {uncertain}, silent errors {} ({:.1} %): {silent:?}",
        100.0 * ok as f64 / n as f64,
        silent.len(),
        100.0 * silent.len() as f64 / n as f64
    );
    println!("{report:?}");
    assert!(ok * 100 >= n * 90, "recognized {ok} of {n}");
    assert!(silent.len() * 100 <= n, "silent errors: {silent:?}");
}

#[test]
fn nothing_is_asked_without_an_author_and_answers_are_reused() {
    let corpus: Vec<Value> = vec![
        json!({"dir": "Аудиокниги/Сказки на ночь", "parts": 2, "expect": {}}),
        json!({"dir": "Audiobooks/Frank Herbert/Dune", "parts": 2, "expect": {}}),
        json!({"dir": "Other/Frank Herbert - Dune", "parts": 2, "expect": {}}),
    ];
    let dir = tempfile::tempdir().unwrap();
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    build(
        &Connection::open(dir.path().join("library.db")).unwrap(),
        &corpus,
    );
    let replay = Arc::new(
        ReplayTransport::from_jsonl(
            &std::fs::read_to_string(fixtures().join("openlibrary.jsonl")).unwrap(),
        )
        .unwrap(),
    );
    let client = OpenLibraryClient::new(
        OpenLibraryConfig {
            rate_limit: RateLimit {
                burst: 100,
                per_second: 100.0,
            },
            ..OpenLibraryConfig::default()
        },
        replay.clone(),
        Arc::new(MemoryCache::default()),
        Arc::new(ManualClock::at_unix(1_800_000_000)),
    );
    let providers = BookProviders {
        open_library: Some(Arc::new(client)),
    };
    let run = || {
        cat.fetch_book_metadata(
            &providers,
            &BookMetadataOptions::default(),
            &CancelToken::new(),
            &mut |_, _| {},
        )
        .unwrap()
    };
    let first = run();
    // Two copies of one book are one question; the book without an
    // author is not asked about at all.
    assert_eq!(first.books, 1, "{first:?}");
    assert_eq!(first.matched, 1, "{first:?}");
    let asked = replay.requests();
    assert!(
        asked
            .iter()
            .all(|r| !r.query.iter().any(|(_, v)| v.contains("Сказки"))),
        "{asked:?}"
    );
    // Only the title and the author leave the device.
    for r in &asked {
        for (k, _) in &r.query {
            assert!(
                ["title", "author", "fields", "limit"].contains(&k.as_str()),
                "{k}"
            );
        }
    }
    // A second run asks nothing: the answer is fresh.
    let second = run();
    assert_eq!(second.books, 0, "{second:?}");
    assert_eq!(replay.requests().len(), asked.len());

    let books = cat.audiobooks().unwrap();
    let dune: Vec<&Audiobook> = books.iter().filter(|b| b.title == "Dune").collect();
    assert_eq!(dune.len(), 2);
    for b in dune {
        assert!(b.open_library_id.is_some());
        assert!(
            b.cover_url
                .as_deref()
                .is_some_and(|u| u.starts_with("https://covers.openlibrary.org/b/id/"))
        );
        assert!(b.description.is_some());
        assert!(!b.needs_review);
    }
    let tales = books.iter().find(|b| b.title == "Сказки на ночь").unwrap();
    assert!(tales.needs_review);
    assert!(tales.description.is_none());
}
