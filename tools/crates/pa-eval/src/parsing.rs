// SPDX-License-Identifier: MPL-2.0
//! «Парсинг» metric (META-024, research 03 §6.2): how well
//! `pa_recognition::parse` reads the fields written in a file name, scored
//! against the `parse` blocks of the corpus labels.
//!
//! The rules are those of `pa-recognition`'s `corpus_examples` test, so the
//! numbers match it on `examples.jsonl`: only fields present in a label are
//! compared (extra output fields are not penalized), strings
//! case-insensitively, lists as sets, numbers by value. A file counts for
//! top-1 when every labelled field of the first hypothesis matches, and for
//! N-best when any hypothesis matches all of them.
//!
//! The corpus is read as raw JSON on purpose: labels may carry fields the
//! typed [`crate::corpus::ParsedName`] does not know (`alt_titles`,
//! `resolution`, `source`, …), and they all count.
//!
//! Reports are byte-reproducible (sorted maps, rounded ratios) and carry no
//! paths.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use pa_recognition::parse::parse;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::metrics::{ratio, round6};

/// Labelled and correct counts of one field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldCount {
    pub labelled: usize,
    pub correct: usize,
}

/// Parse accuracy over a set of files.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParseCounts {
    pub files: usize,
    pub files_top1: usize,
    pub files_nbest: usize,
    pub fields: usize,
    pub fields_correct: usize,
    pub field_accuracy: Option<f64>,
    pub top1_accuracy: Option<f64>,
    pub nbest_accuracy: Option<f64>,
}

impl ParseCounts {
    fn finish(&mut self) {
        let r = |a, b| ratio(a, b).map(round6);
        self.field_accuracy = r(self.fields_correct, self.fields);
        self.top1_accuracy = r(self.files_top1, self.files);
        self.nbest_accuracy = r(self.files_nbest, self.files);
    }
}

/// The «Парсинг» report of a corpus.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParseReport {
    /// Source file names (no directories).
    pub sources: Vec<String>,
    pub overall: ParseCounts,
    pub per_field: BTreeMap<String, FieldCount>,
    pub per_category: BTreeMap<String, ParseCounts>,
}

fn norm(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(s.to_lowercase()),
        Value::Array(a) => {
            let mut items: Vec<String> = a.iter().map(|x| norm(x).to_string()).collect();
            items.sort();
            items.dedup();
            Value::Array(items.into_iter().map(Value::String).collect())
        }
        Value::Number(n) => Value::String(n.as_f64().unwrap_or(f64::NAN).to_string()),
        other => other.clone(),
    }
}

fn matches(expected: &Value, got: Option<&Value>) -> bool {
    got.is_some_and(|g| norm(expected) == norm(g))
}

/// Scores the parser on every labelled file of the JSONL documents
/// `(name, text)`.
pub fn score(docs: &[(String, String)]) -> Result<ParseReport, String> {
    let mut report = ParseReport {
        sources: docs
            .iter()
            .map(|(n, _)| n.rsplit(['/', '\\']).next().unwrap_or(n).to_owned())
            .collect(),
        ..Default::default()
    };
    for (name, text) in docs {
        for (idx, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let rec: Value =
                serde_json::from_str(line).map_err(|e| format!("{name}:{}: {e}", idx + 1))?;
            let categories: Vec<String> = rec["categories"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let Some(files) = rec["labels"]["files"].as_array() else {
                continue;
            };
            for f in files {
                let Some(expected) = f.get("parse").and_then(Value::as_object) else {
                    continue;
                };
                let Some(path) = f["path"].as_str() else {
                    continue;
                };
                let hyps: Vec<Value> = parse(path)
                    .hypotheses
                    .iter()
                    .map(|h| serde_json::to_value(&h.parsed).unwrap_or(Value::Null))
                    .collect();
                let first = hyps.first();
                let mut file = ParseCounts {
                    files: 1,
                    ..Default::default()
                };
                let mut all_ok = true;
                for (k, v) in expected {
                    let ok = matches(v, first.and_then(|h| h.get(k)));
                    file.fields += 1;
                    let e = report.per_field.entry(k.clone()).or_default();
                    e.labelled += 1;
                    if ok {
                        file.fields_correct += 1;
                        e.correct += 1;
                    } else {
                        all_ok = false;
                    }
                }
                file.files_top1 = usize::from(all_ok);
                file.files_nbest = usize::from(
                    hyps.iter()
                        .any(|h| expected.iter().all(|(k, v)| matches(v, h.get(k)))),
                );
                add(&mut report.overall, &file);
                for c in &categories {
                    add(report.per_category.entry(c.clone()).or_default(), &file);
                }
            }
        }
    }
    report.overall.finish();
    for c in report.per_category.values_mut() {
        c.finish();
    }
    Ok(report)
}

fn add(into: &mut ParseCounts, file: &ParseCounts) {
    into.files += file.files;
    into.files_top1 += file.files_top1;
    into.files_nbest += file.files_nbest;
    into.fields += file.fields;
    into.fields_correct += file.fields_correct;
}

fn pct(x: Option<f64>) -> String {
    x.map_or_else(|| "—".to_owned(), |v| format!("{:.1} %", v * 100.0))
}

impl ParseReport {
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).unwrap_or_default();
        s.push('\n');
        s
    }

    /// The «Парсинг» section (Markdown).
    pub fn to_markdown(&self) -> String {
        let mut o = String::new();
        let _ = writeln!(o, "## Парсинг\n");
        let _ = writeln!(
            o,
            "Корпус: {} · файлов с разметкой `parse`: {}\n",
            self.sources.join(", "),
            self.overall.files
        );
        let _ = writeln!(o, "| Группа | Файлов | Поля | Файл top-1 | Файл N-best |");
        let _ = writeln!(o, "|---|---:|---:|---:|---:|");
        let row = |o: &mut String, name: &str, c: &ParseCounts| {
            let _ = writeln!(
                o,
                "| {name} | {} | {} | {} | {} |",
                c.files,
                pct(c.field_accuracy),
                pct(c.top1_accuracy),
                pct(c.nbest_accuracy)
            );
        };
        row(&mut o, "всё", &self.overall);
        for (k, c) in &self.per_category {
            row(&mut o, k, c);
        }
        let _ = writeln!(o, "\n### По полям\n");
        let _ = writeln!(o, "| Поле | Верно | Размечено | Точность |");
        let _ = writeln!(o, "|---|---:|---:|---:|");
        for (k, f) in &self.per_field {
            let _ = writeln!(
                o,
                "| `{k}` | {} | {} | {} |",
                f.correct,
                f.labelled,
                pct(ratio(f.correct, f.labelled))
            );
        }
        o
    }
}
