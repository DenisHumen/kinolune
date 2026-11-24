// SPDX-License-Identifier: MPL-2.0
//! Regression cases from the owner's reports (`corpus/regression`), run
//! through the META-013 library matcher on their own synthetic TMDB.
//!
//! BUG-011: a film named by the title before its subtitle is decided alone
//! when it is the only candidate of the year; a shared head stays with the
//! user, and an exact title still wins over a subtitled one.

use pa_eval::Recognizer;
use pa_eval::corpus::Corpus;
use pa_eval::tmdb::{SyntheticTmdb, Tmdb};
use pa_recognition::title::TitleRecord;

const DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../recognition-eval/corpus/regression"
);

/// The `auto` threshold of the metadata run (`pa_catalog::metadata::AUTO`).
const AUTO: f64 = 0.92;

fn load(name: &str) -> (Corpus, SyntheticTmdb) {
    let path = format!("{DIR}/{name}.jsonl");
    let text = std::fs::read_to_string(&path).unwrap();
    let corpus = Corpus::load(&[(path, text)], true).unwrap();
    let records: Vec<TitleRecord> = std::fs::read_to_string(format!("{DIR}/{name}-tmdb.jsonl"))
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    (corpus, SyntheticTmdb::new(&records))
}

#[test]
fn bug_011_title_before_the_subtitle() {
    let (corpus, mut tmdb) = load("bug-011");
    assert_eq!(corpus.cases.len(), 4);
    for case in &corpus.cases {
        let out = Tmdb::default().recognize(&case.input, &mut tmdb);
        assert_eq!(out.len(), 1, "{}", case.id);
        let p = &out[0];
        let got = p.work.as_ref().and_then(|w| w.ids.tmdb.clone());
        let truth = case.labels.works[0].ids.tmdb.clone();
        if case.id == "bug011-shared-head-tie" {
            // Two films share the head and the year: never decided alone.
            assert!(
                p.confidence < AUTO,
                "{}: {got:?} at {}",
                case.id,
                p.confidence
            );
        } else {
            assert_eq!(got, truth, "{}", case.id);
            assert!(p.confidence >= AUTO, "{}: {}", case.id, p.confidence);
        }
    }
}
