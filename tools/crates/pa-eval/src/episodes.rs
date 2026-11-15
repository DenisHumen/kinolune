// SPDX-License-Identifier: MPL-2.0
//! Episode accuracy per category and numbering scheme (META-015 done-when:
//! "episode accuracy on A1–A4 and A16 ≥ 97 % in both schemes").
//!
//! For a category and a scheme, over the scorable units whose label has
//! that scheme for every episode:
//!
//! - **accuracy** = right / auto: of the units the recognizer decided on
//!   its own at the report's auto threshold, the share with the right work
//!   and the same episode positions in that scheme (the report's
//!   "precision" column of the episode table);
//! - **recall** = right / labelled: the same over all labelled units, so
//!   review and manual decisions count against it.
//!
//! `pa-eval episodes --report R.json [--baseline B.json] [--categories
//! A1,A2,A3,A4,A16] [--schemes anidb,tmdb] [--target-pct 97] [--md OUT.md]`
//! exits with 1 when an accuracy is below the target.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::metrics::Decision;
use crate::report::Report;

/// Counts of one category and scheme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Units labelled in the scheme.
    pub labelled: usize,
    /// Of them, decided automatically.
    pub auto: usize,
    /// Of them, right (auto, right work, same positions).
    pub right: usize,
}

impl Counts {
    /// `right / auto`, percent.
    #[must_use]
    pub fn accuracy(&self) -> Option<f64> {
        (self.auto > 0).then(|| 100.0 * self.right as f64 / self.auto as f64)
    }

    /// `right / labelled`, percent.
    #[must_use]
    pub fn recall(&self) -> Option<f64> {
        (self.labelled > 0).then(|| 100.0 * self.right as f64 / self.labelled as f64)
    }
}

/// `(category, scheme)` → counts, plus `all` over the categories asked for.
#[must_use]
pub fn per_category(
    report: &Report,
    categories: &[String],
    schemes: &[String],
) -> BTreeMap<(String, String), Counts> {
    let mut out: BTreeMap<(String, String), Counts> = BTreeMap::new();
    for item in &report.items {
        if !item.scorable() {
            continue;
        }
        let cats: Vec<&String> = item
            .categories
            .iter()
            .filter(|c| categories.contains(c))
            .collect();
        if cats.is_empty() {
            continue;
        }
        for (scheme, &ok) in &item.episodes {
            if !schemes.contains(scheme) {
                continue;
            }
            for cat in cats.iter().map(|c| c.as_str()).chain(["all"]) {
                let c = out.entry((cat.to_owned(), scheme.clone())).or_default();
                c.labelled += 1;
                c.auto += usize::from(item.decision == Decision::Auto);
                c.right += usize::from(ok);
            }
        }
    }
    out
}

fn pct(v: Option<f64>) -> String {
    v.map_or("—".to_owned(), |v| format!("{v:.2} %"))
}

fn order(c: &str) -> (u8, u32) {
    match c.strip_prefix('A').and_then(|n| n.parse().ok()) {
        Some(n) => (0, n),
        None => (1, 0),
    }
}

/// Markdown table of `current` (and `baseline`, when given) and the
/// `(category, scheme)` pairs below `target_pct`.
#[must_use]
pub fn table(
    current: &Report,
    baseline: Option<&Report>,
    categories: &[String],
    schemes: &[String],
    target_pct: f64,
) -> (String, Vec<String>) {
    let cur = per_category(current, categories, schemes);
    let base = baseline.map(|b| per_category(b, categories, schemes));
    let mut keys: Vec<&(String, String)> = cur.keys().collect();
    keys.sort_by_key(|(c, s)| (order(c), s.clone()));
    let mut md = String::new();
    let _ = writeln!(
        md,
        "Точность эпизода — верных среди решённых автоматически (порог {}), полнота — верных среди \
         размеченных. Верно: авто, верная работа и те же позиции в схеме.\n",
        current.auto_threshold
    );
    let head = if baseline.is_some() {
        "| Категория | Схема | Размечено | Было: точность | Стало: авто | Стало: верно | Стало: точность | Стало: полнота |\n|---|---|---:|---:|---:|---:|---:|---:|"
    } else {
        "| Категория | Схема | Размечено | Авто | Верно | Точность | Полнота |\n|---|---|---:|---:|---:|---:|---:|"
    };
    let _ = writeln!(md, "{head}");
    let mut short = Vec::new();
    for key in keys {
        let c = cur[key];
        if c.accuracy().is_none_or(|a| a < target_pct) {
            short.push(format!("{}/{}", key.0, key.1));
        }
        match &base {
            Some(b) => {
                let old = b.get(key).copied().unwrap_or_default();
                let _ = writeln!(
                    md,
                    "| {} | {} | {} | {} | {} | {} | {} | {} |",
                    key.0,
                    key.1,
                    c.labelled,
                    pct(old.accuracy()),
                    c.auto,
                    c.right,
                    pct(c.accuracy()),
                    pct(c.recall())
                );
            }
            None => {
                let _ = writeln!(
                    md,
                    "| {} | {} | {} | {} | {} | {} | {} |",
                    key.0,
                    key.1,
                    c.labelled,
                    c.auto,
                    c.right,
                    pct(c.accuracy()),
                    pct(c.recall())
                );
            }
        }
    }
    (md, short)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{ItemOutcome, WorkOutcome};

    fn item(decision: Decision, work: WorkOutcome, eps: &[(&str, bool)]) -> ItemOutcome {
        ItemOutcome {
            case_id: "c".into(),
            path: "p".into(),
            categories: vec!["A1".into(), "A9".into()],
            kind: Some("anime".into()),
            role: "main".into(),
            decision,
            work,
            confidence: Some(1.0),
            episodes: eps.iter().map(|(s, ok)| ((*s).to_owned(), *ok)).collect(),
            parse: BTreeMap::new(),
        }
    }

    #[test]
    fn accuracy_is_over_auto_and_recall_over_labelled() {
        let report = Report {
            report_version: 1,
            recognizer: "x".into(),
            auto_threshold: 0.92,
            corpus: Default::default(),
            overall: Default::default(),
            per_category: Default::default(),
            per_kind: Default::default(),
            kinds: Vec::new(),
            performance: Default::default(),
            items: vec![
                item(
                    Decision::Auto,
                    WorkOutcome::Correct,
                    &[("anidb", true), ("tmdb", false)],
                ),
                item(Decision::Auto, WorkOutcome::Correct, &[("anidb", true)]),
                item(Decision::Review, WorkOutcome::Correct, &[("anidb", false)]),
                item(Decision::Auto, WorkOutcome::Unscorable, &[("anidb", false)]),
            ],
        };
        let cats = vec!["A1".to_owned()];
        let schemes = vec!["anidb".to_owned(), "tmdb".to_owned()];
        let m = per_category(&report, &cats, &schemes);
        let a = m[&("A1".to_owned(), "anidb".to_owned())];
        assert_eq!((a.labelled, a.auto, a.right), (3, 2, 2));
        assert_eq!(a.accuracy(), Some(100.0));
        let t = m[&("A1".to_owned(), "tmdb".to_owned())];
        assert_eq!(t.accuracy(), Some(0.0));
        let (_, short) = table(&report, None, &cats, &schemes, 97.0);
        assert_eq!(short, ["A1/tmdb", "all/tmdb"]);
    }
}
