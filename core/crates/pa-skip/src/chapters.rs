// SPDX-License-Identifier: MPL-2.0
//! Segments from chapter names: "Opening", "OP", "Intro", "Credits",
//! "ED", "Recap", "Preview" and their Russian equivalents. Fansub and
//! Blu-ray releases often mark them; generic names ("Chapter 3") say
//! nothing and are ignored.

use pa_library::skip::SkipKind;

/// A chapter of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterMark {
    pub start_ms: u64,
    pub end_ms: u64,
    pub title: Option<String>,
}

/// Longest segment of each kind a chapter may give.
fn max_ms(kind: SkipKind) -> u64 {
    match kind {
        SkipKind::Intro | SkipKind::Recap | SkipKind::Preview => 5 * 60_000,
        SkipKind::Credits => 15 * 60_000,
    }
}

/// Phrases (lowercase words) a chapter title starts with, per kind.
const PHRASES: &[(SkipKind, &[&str])] = &[
    (
        SkipKind::Intro,
        &[
            "op",
            "opening",
            "intro",
            "introduction",
            "title sequence",
            "main title",
            "main titles",
            "theme song",
            "opening credits",
            "opening titles",
            "вступление",
            "заставка",
            "опенинг",
            "интро",
            "начальные титры",
        ],
    ),
    (
        SkipKind::Credits,
        &[
            "ed",
            "ending",
            "end credits",
            "end titles",
            "closing credits",
            "credits",
            "outro",
            "титры",
            "финальные титры",
            "конечные титры",
            "эндинг",
        ],
    ),
    (
        SkipKind::Recap,
        &["recap", "previously", "previously on", "ранее", "в предыдущих сериях"],
    ),
    (
        SkipKind::Preview,
        &[
            "preview",
            "next episode",
            "next time",
            "превью",
            "анонс",
            "в следующей серии",
        ],
    ),
];

/// Words of a title: lowercase letters and digits, digits stripped from
/// the end of a word ("OP2" → "op").
fn words(title: &str) -> Vec<String> {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let w = w.to_lowercase();
            let trimmed = w.trim_end_matches(|c: char| c.is_ascii_digit());
            if trimmed.is_empty() {
                w
            } else {
                trimmed.to_owned()
            }
        })
        .collect()
}

/// What a chapter title says the chapter is.
#[must_use]
pub fn kind_of(title: &str) -> Option<SkipKind> {
    let w = words(title);
    if w.is_empty() {
        return None;
    }
    // The longest phrase the title starts with decides ("ending credits"
    // is credits, "opening credits" is the intro).
    let mut best: Option<(usize, SkipKind)> = None;
    for (kind, phrases) in PHRASES {
        for p in *phrases {
            let pw: Vec<&str> = p.split(' ').collect();
            if w.len() >= pw.len()
                && w.iter().zip(&pw).all(|(a, b)| a == b)
                && best.is_none_or(|(n, _)| pw.len() > n)
            {
                best = Some((pw.len(), *kind));
            }
        }
    }
    best.map(|(_, k)| k)
}

/// Segments the chapters of a file mark: one per kind (the first chapter
/// of that kind, merged with directly following ones of the same kind).
#[must_use]
pub fn segments(chapters: &[ChapterMark]) -> Vec<(SkipKind, u64, u64)> {
    let mut out: Vec<(SkipKind, u64, u64)> = Vec::new();
    let mut last: Option<SkipKind> = None;
    for c in chapters {
        let kind = c.title.as_deref().and_then(kind_of);
        match kind {
            Some(k) if c.end_ms > c.start_ms => {
                if last == Some(k)
                    && let Some(seg) = out.iter_mut().rev().find(|s| s.0 == k)
                    && seg.2.abs_diff(c.start_ms) <= 1_000
                {
                    seg.2 = c.end_ms;
                } else if !out.iter().any(|s| s.0 == k) {
                    out.push((k, c.start_ms, c.end_ms));
                }
                last = Some(k);
            }
            _ => last = None,
        }
    }
    out.retain(|(k, s, e)| e - s <= max_ms(*k));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(kind_of("Opening"), Some(SkipKind::Intro));
        assert_eq!(kind_of("OP"), Some(SkipKind::Intro));
        assert_eq!(kind_of("OP2 - \"Song\""), Some(SkipKind::Intro));
        assert_eq!(kind_of("Opening Credits"), Some(SkipKind::Intro));
        assert_eq!(kind_of("ED"), Some(SkipKind::Credits));
        assert_eq!(kind_of("Ending"), Some(SkipKind::Credits));
        assert_eq!(kind_of("End Credits"), Some(SkipKind::Credits));
        assert_eq!(kind_of("Credits"), Some(SkipKind::Credits));
        assert_eq!(kind_of("Заставка"), Some(SkipKind::Intro));
        assert_eq!(kind_of("Титры"), Some(SkipKind::Credits));
        assert_eq!(kind_of("Previously on..."), Some(SkipKind::Recap));
        assert_eq!(kind_of("Preview"), Some(SkipKind::Preview));
        assert_eq!(kind_of("Chapter 3"), None);
        assert_eq!(kind_of("Part A"), None);
        assert_eq!(kind_of("Opera house"), None);
        assert_eq!(kind_of("Education"), None);
        assert_eq!(kind_of(""), None);
    }

    #[test]
    fn segments_from_chapters() {
        let ch = |s: u64, e: u64, t: &str| ChapterMark {
            start_ms: s,
            end_ms: e,
            title: Some(t.to_owned()),
        };
        let chapters = [
            ch(0, 90_000, "Prologue"),
            ch(90_000, 180_000, "Opening"),
            ch(180_000, 600_000, "Part A"),
            ch(600_000, 1_300_000, "Part B"),
            ch(1_300_000, 1_390_000, "Ending"),
            ch(1_390_000, 1_420_000, "Preview"),
        ];
        assert_eq!(
            segments(&chapters),
            [
                (SkipKind::Intro, 90_000, 180_000),
                (SkipKind::Credits, 1_300_000, 1_390_000),
                (SkipKind::Preview, 1_390_000, 1_420_000),
            ]
        );
        // Two consecutive credit chapters merge; a 20-minute "intro" is not one.
        let chapters = [
            ch(0, 1_200_000, "Intro"),
            ch(1_200_000, 1_260_000, "Credits"),
            ch(1_260_000, 1_300_000, "End credits"),
        ];
        assert_eq!(segments(&chapters), [(SkipKind::Credits, 1_200_000, 1_300_000)]);
    }
}
