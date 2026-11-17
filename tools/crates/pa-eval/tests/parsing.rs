// SPDX-License-Identifier: MPL-2.0
//! META-024: the «Парсинг» report on `examples.jsonl` gives the numbers of
//! `pa-recognition`'s `corpus_examples` test (same rules, same floors) and
//! is byte-reproducible.

use pa_eval::parsing::score;

const EXAMPLES: &str = include_str!("../../../recognition-eval/examples/examples.jsonl");

fn docs() -> Vec<(String, String)> {
    vec![("examples.jsonl".to_owned(), EXAMPLES.to_owned())]
}

#[test]
fn parse_report_matches_the_parser_corpus_test() {
    let r = score(&docs()).expect("scored");
    let o = &r.overall;
    assert!(o.files >= 70, "examples were not loaded: {}", o.files);
    // The regression floors of `corpus_examples`.
    assert!(o.field_accuracy.unwrap() >= 0.95, "{o:?}");
    assert!(o.nbest_accuracy.unwrap() >= 0.90, "{o:?}");
    assert!(o.files_top1 <= o.files_nbest && o.files_nbest <= o.files);
    let labelled: usize = r.per_field.values().map(|f| f.labelled).sum();
    assert_eq!(labelled, o.fields);
    // Fields the typed label does not know are scored too.
    assert!(r.per_field.contains_key("alt_titles"));
    assert!(r.per_field.contains_key("resolution"));
    assert!(!r.per_category.is_empty());
}

#[test]
fn parse_report_is_reproducible_and_has_the_section() {
    let a = score(&docs()).unwrap();
    let b = score(&docs()).unwrap();
    assert_eq!(a.to_json(), b.to_json());
    let md = a.to_markdown();
    assert!(md.starts_with("## Парсинг\n"));
    assert!(md.contains("### По полям"));
    assert!(!md.contains("Anime/"), "no paths in the report");
}
