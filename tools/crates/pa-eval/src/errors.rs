// SPDX-License-Identifier: MPL-2.0
//! Errors per category of two reports (META-014 done-when: "on A2, A3 and
//! A8 at least 30 % fewer errors than META-013").
//!
//! An *error* is a scorable unit the recognizer did not get right on its
//! own at the report's auto threshold: not auto (review or manual queue),
//! the wrong work, or the right work with a wrong episode — the episode
//! counts as right when any labelled scheme matches (AniDB for the local
//! scorer, TMDB `SxxEyy` or TVDB absolute for the TMDB matcher; GUIDE §6).
//! Silent errors (auto and wrong) are counted apart: fewer errors must not
//! come from more silent ones.
//!
//! `pa-eval errors --baseline BASE.json --current CUR.json [--categories
//! A2,A3,A8] [--target-pct 30] [--md OUT.md]`

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::metrics::{Decision, ItemOutcome, WorkOutcome};
use crate::report::Report;

/// Errors of one group of units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Errors {
    /// Scorable units.
    pub items: usize,
    /// Units not right on their own (see the module docs).
    pub errors: usize,
    /// Auto and the wrong work.
    pub silent: usize,
}

impl Errors {
    fn add(&mut self, item: &ItemOutcome) {
        if !item.scorable() {
            return;
        }
        self.items += 1;
        let episode_ok = item.episodes.is_empty() || item.episodes.values().any(|&ok| ok);
        let right =
            item.decision == Decision::Auto && item.work == WorkOutcome::Correct && episode_ok;
        if !right {
            self.errors += 1;
        }
        if item.silent_error() {
            self.silent += 1;
        }
    }

    /// `errors / items`.
    pub fn rate(&self) -> f64 {
        if self.items == 0 {
            0.0
        } else {
            self.errors as f64 / self.items as f64
        }
    }
}

/// Errors per category (and `all`).
pub fn per_category(report: &Report) -> BTreeMap<String, Errors> {
    let mut out: BTreeMap<String, Errors> = BTreeMap::new();
    for item in &report.items {
        out.entry("all".to_owned()).or_default().add(item);
        for c in &item.categories {
            out.entry(c.clone()).or_default().add(item);
        }
    }
    out
}

/// Comparison of two reports on some categories.
#[derive(Debug, Clone)]
pub struct ErrorComparison {
    /// Markdown table.
    pub markdown: String,
    /// Categories whose reduction is below the target.
    pub short: Vec<String>,
    /// Categories with more silent errors than the baseline.
    pub more_silent: Vec<String>,
}

/// Relative reduction of errors, percent (`None` without baseline errors).
pub fn reduction(base: &Errors, cur: &Errors) -> Option<f64> {
    (base.errors > 0).then(|| 100.0 * (base.errors as f64 - cur.errors as f64) / base.errors as f64)
}

fn category_order(c: &str) -> (u8, u32) {
    match c.strip_prefix('A').and_then(|n| n.parse().ok()) {
        Some(n) => (0, n),
        None => (1, 0),
    }
}

/// Compares `current` with `baseline` on `categories` (all when empty).
pub fn compare(
    baseline: &Report,
    current: &Report,
    categories: &[String],
    target_pct: f64,
) -> ErrorComparison {
    let (b, c) = (per_category(baseline), per_category(current));
    let mut cats: Vec<String> = if categories.is_empty() {
        b.keys().chain(c.keys()).cloned().collect()
    } else {
        categories.to_vec()
    };
    cats.sort_by_key(|x| category_order(x));
    cats.dedup();
    let mut md = String::new();
    let _ = writeln!(
        md,
        "Ошибки — элементы, которые распознаватель не решил сам верно на пороге авто: \
         проверка или ручная очередь, неверная работа, неверный эпизод (верен, если совпала \
         любая размеченная схема). Тихие — авто и неверная работа.\n"
    );
    let _ = writeln!(
        md,
        "| Категория | Элементов | Было: `{}` (порог {}) | Стало: `{}` (порог {}) | Меньше ошибок | Тихие: было → стало |",
        baseline.recognizer, baseline.auto_threshold, current.recognizer, current.auto_threshold
    );
    let _ = writeln!(md, "|---|---:|---:|---:|---:|---:|");
    let mut short = Vec::new();
    let mut more_silent = Vec::new();
    for cat in &cats {
        let (x, y) = (
            b.get(cat).copied().unwrap_or_default(),
            c.get(cat).copied().unwrap_or_default(),
        );
        let red = reduction(&x, &y);
        if red.is_none_or(|r| r < target_pct) && x.errors > 0 {
            short.push(cat.clone());
        }
        if y.silent > x.silent {
            more_silent.push(cat.clone());
        }
        let _ = writeln!(
            md,
            "| {cat} | {} | {} ({:.2} %) | {} ({:.2} %) | {} | {} → {} |",
            x.items,
            x.errors,
            100.0 * x.rate(),
            y.errors,
            100.0 * y.rate(),
            red.map_or("—".to_owned(), |r| format!("{r:.1} %")),
            x.silent,
            y.silent,
        );
    }
    ErrorComparison {
        markdown: md,
        short,
        more_silent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(cat: &str, decision: Decision, work: WorkOutcome, eps: &[bool]) -> ItemOutcome {
        ItemOutcome {
            case_id: "c".into(),
            path: "p".into(),
            categories: vec![cat.into()],
            kind: Some("anime".into()),
            role: "main".into(),
            decision,
            work,
            confidence: Some(1.0),
            episodes: eps
                .iter()
                .enumerate()
                .map(|(i, &ok)| (format!("s{i}"), ok))
                .collect(),
            parse: BTreeMap::new(),
        }
    }

    #[test]
    fn counts_review_wrong_work_and_wrong_episode() {
        let mut e = Errors::default();
        e.add(&item("A2", Decision::Auto, WorkOutcome::Correct, &[]));
        e.add(&item(
            "A2",
            Decision::Auto,
            WorkOutcome::Correct,
            &[false, true],
        ));
        e.add(&item("A2", Decision::Auto, WorkOutcome::Correct, &[false]));
        e.add(&item("A2", Decision::Review, WorkOutcome::Correct, &[]));
        e.add(&item("A2", Decision::Auto, WorkOutcome::Wrong, &[]));
        e.add(&item("A2", Decision::Auto, WorkOutcome::Unscorable, &[]));
        assert_eq!(
            e,
            Errors {
                items: 5,
                errors: 3,
                silent: 1
            }
        );
        let base = Errors {
            items: 10,
            errors: 10,
            silent: 0,
        };
        assert_eq!(reduction(&base, &e), Some(70.0));
    }
}
