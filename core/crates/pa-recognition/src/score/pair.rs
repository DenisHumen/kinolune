// SPDX-License-Identifier: MPL-2.0
//! Pair features of a (group, candidate) pair and their hand-set weights
//! (META-014, research 03 §4.2 \[5\]).
//!
//! The title, year and kind of a candidate are scored by the title index and
//! [`super`]'s readings; this module adds what the files themselves say:
//!
//! - **duration** — the file's container duration against the candidate's
//!   runtime (a film, or a typical episode of a series): remakes and
//!   same-named films usually differ by more than a few minutes;
//! - **episode range** — every written `SxxEyy` must exist in the
//!   candidate's season list (`Shameless 9x14` is not the British series);
//! - **audio language** — a dub-free original-language track is a weak
//!   hint for the work in that language;
//! - **hash hits** — a confirmed OSHash/ed2k → work mapping (AniDB `FILE`,
//!   an earlier user fix, a crowd dataset) is decisive;
//! - **user rules** — a folder rule naming the work is decisive.
//!
//! Duration, episode range and language are *relative*: per file, the
//! adjustments of all candidates are shifted so the best-fitting one gets 0.
//! They separate candidates (the margin, hence the confidence, grows) but
//! never lift or sink a lone candidate whose metadata is incomplete (an
//! ongoing season not yet listed, a director's cut). Hash hits and rules are
//! absolute.
//!
//! The weights are set by hand on the train and dev splits of corpus v0;
//! the trained, calibrated scorer is META-018.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::title::{TitleRecord, WorkKind};

/// What one file (or a group of files) says about the work.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Evidence {
    /// Container duration in minutes; `None` when not probed or not
    /// comparable (a split part, a disc stream, an extra).
    pub duration_min: Option<f32>,
    /// `(season, episode)` as written (`S09E14`, `Season 2/… - 05`).
    pub written: Vec<(u32, u32)>,
    /// ISO 639-1 codes of the audio tracks.
    pub audio: Vec<String>,
}

/// Features of one (file, candidate) pair (explanation and input of the
/// trained scorer, META-018).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Features {
    /// `|duration − runtime| / runtime`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_error: Option<f32>,
    /// Written episodes found in the candidate's season list.
    #[serde(default)]
    pub episodes_ok: u32,
    /// Written episodes the candidate does not have.
    #[serde(default)]
    pub episodes_bad: u32,
    /// An audio track is in the candidate's original language (`None`:
    /// unknown on either side).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<bool>,
}

/// Hand-set weights.
mod w {
    /// Film within PAL speed-up and trimmed credits of its runtime.
    pub const MOVIE_CLOSE: f32 = 0.05;
    /// Film within 10 %.
    pub const MOVIE_NEAR: f32 = 0.02;
    /// Film 10–20 % off.
    pub const MOVIE_OFF: f32 = -0.03;
    /// Film more than 20 % off: another cut or another film.
    pub const MOVIE_FAR: f32 = -0.10;
    /// Episode within 10 % of the typical runtime. Series runtimes are
    /// nominal slots (a 22-minute sitcom lists 30), so only gross
    /// mismatches count against a series.
    pub const EPISODE_NEAR: f32 = 0.02;
    /// Episode runtime off by more than 45 %.
    pub const EPISODE_FAR: f32 = -0.06;
    /// A written episode the candidate does not have.
    pub const EPISODE_MISSING: f32 = -0.15;
    /// Audio in the original language.
    pub const LANGUAGE: f32 = 0.02;
}

/// A file shorter than this share of a runtime is not compared with it.
const SHORT: f32 = 0.4;

/// Decisive bonus of a hash hit or a user rule for the named work.
pub const DECISIVE: f32 = 0.5;

/// Penalty of a candidate when a hash hit or a user rule names another work.
pub const OVERRULED: f32 = -0.3;

/// ISO 639-2 (container tags) → ISO 639-1 (provider metadata).
pub fn iso639_1(code: &str) -> Option<&'static str> {
    Some(match code.trim().to_ascii_lowercase().as_str() {
        "ja" | "jpn" => "ja",
        "en" | "eng" => "en",
        "ru" | "rus" => "ru",
        "uk" | "ukr" => "uk",
        "fr" | "fre" | "fra" => "fr",
        "de" | "ger" | "deu" => "de",
        "es" | "spa" => "es",
        "it" | "ita" => "it",
        "ko" | "kor" => "ko",
        "zh" | "chi" | "zho" => "zh",
        "pt" | "por" => "pt",
        "pl" | "pol" => "pl",
        "sv" | "swe" => "sv",
        "da" | "dan" => "da",
        "no" | "nor" | "nob" => "no",
        "nl" | "dut" | "nld" => "nl",
        _ => return None,
    })
}

impl Evidence {
    /// Evidence of one file: duration in milliseconds, audio tags as
    /// probed (`jpn`, `eng`), written `(season, episode)` pairs.
    pub fn of_file(duration_ms: Option<u64>, audio: &[String], written: Vec<(u32, u32)>) -> Self {
        let mut langs: Vec<String> = Vec::new();
        for a in audio {
            if let Some(l) = iso639_1(a)
                && !langs.iter().any(|x| x == l)
            {
                langs.push(l.to_owned());
            }
        }
        Self {
            duration_min: duration_ms
                .filter(|&d| d >= 60_000)
                .map(|d| d as f32 / 60_000.0),
            written,
            audio: langs,
        }
    }
}

/// Features of `evidence` against `record`.
pub fn features(evidence: &Evidence, record: &TitleRecord) -> Features {
    // A file far shorter than the runtime is not the feature itself (a
    // trailer, a sample, one part of a split film): no opinion.
    let duration_error = match (evidence.duration_min, record.runtime_min) {
        (Some(d), Some(rt)) if rt > 0 && d >= SHORT * rt as f32 => {
            Some((d - rt as f32).abs() / rt as f32)
        }
        _ => None,
    };
    let (mut ok, mut bad) = (0, 0);
    if record.kind == WorkKind::Series && !record.seasons.is_empty() {
        for (s, e) in &evidence.written {
            match record.seasons.get(s) {
                Some(&count) if *e >= 1 && *e <= count => ok += 1,
                _ => bad += 1,
            }
        }
    }
    let language = match (&record.original_language, evidence.audio.is_empty()) {
        (Some(l), false) => Some(evidence.audio.iter().any(|a| a == l)),
        _ => None,
    };
    Features {
        duration_error,
        episodes_ok: ok,
        episodes_bad: bad,
        language,
    }
}

/// Weighted sum of the relative features (before the per-file shift).
pub fn raw_weight(f: &Features, kind: WorkKind) -> f32 {
    let mut adj = 0.0;
    if let Some(e) = f.duration_error {
        adj += match kind {
            WorkKind::Movie => {
                if e <= 0.05 {
                    w::MOVIE_CLOSE
                } else if e <= 0.10 {
                    w::MOVIE_NEAR
                } else if e <= 0.20 {
                    w::MOVIE_OFF
                } else {
                    w::MOVIE_FAR
                }
            }
            WorkKind::Series => {
                if e <= 0.10 {
                    w::EPISODE_NEAR
                } else if e > 0.45 {
                    w::EPISODE_FAR
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };
    }
    adj += w::EPISODE_MISSING * f.episodes_bad as f32;
    if f.language == Some(true) {
        adj += w::LANGUAGE;
    }
    adj
}

/// Relative adjustments of a file's candidates: [`raw_weight`] of each,
/// shifted so the best one is 0 (`records` → adjustment).
pub fn relative(evidence: &Evidence, records: &[(usize, &TitleRecord)]) -> BTreeMap<usize, f32> {
    let raw: Vec<(usize, f32)> = records
        .iter()
        .map(|(i, r)| (*i, raw_weight(&features(evidence, r), r.kind)))
        .collect();
    let best = raw.iter().map(|(_, a)| *a).fold(f32::MIN, f32::max);
    raw.into_iter().map(|(i, a)| (i, a - best)).collect()
}

/// The ids of `record` name the work `wanted` names: at least one shared
/// namespace, and every shared namespace agrees.
pub fn ids_agree(record: &BTreeMap<String, String>, wanted: &BTreeMap<String, String>) -> bool {
    let mut shared = 0;
    for (ns, v) in wanted {
        if let Some(r) = record.get(ns) {
            if r != v {
                return false;
            }
            shared += 1;
        }
    }
    shared > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie(year: u16, runtime: u32) -> TitleRecord {
        TitleRecord {
            kind: WorkKind::Movie,
            year: Some(year),
            runtime_min: Some(runtime),
            titles: vec!["Dune".into()],
            ..Default::default()
        }
    }

    fn series(seasons: &[(u32, u32)], lang: &str) -> TitleRecord {
        TitleRecord {
            kind: WorkKind::Series,
            seasons: seasons.iter().copied().collect(),
            runtime_min: Some(50),
            original_language: Some(lang.into()),
            titles: vec!["Shameless".into()],
            ..Default::default()
        }
    }

    #[test]
    fn duration_separates_remakes() {
        let (a, b) = (movie(1984, 137), movie(2021, 155));
        let ev = Evidence::of_file(Some(155 * 60_000 + 20_000), &[], Vec::new());
        let adj = relative(&ev, &[(0, &a), (1, &b)]);
        assert_eq!(adj[&1], 0.0);
        assert!(adj[&0] < -0.07, "{adj:?}");
        // No duration: no opinion.
        let adj = relative(&Evidence::default(), &[(0, &a), (1, &b)]);
        assert!(adj.values().all(|&v| v == 0.0));
    }

    #[test]
    fn written_episodes_must_exist() {
        let us = series(&[(1, 12), (9, 14)], "en");
        let uk = series(&[(1, 6), (9, 10)], "en");
        let ev = Evidence::of_file(None, &[], vec![(9, 14)]);
        let f = features(&ev, &uk);
        assert_eq!((f.episodes_ok, f.episodes_bad), (0, 1));
        let adj = relative(&ev, &[(0, &us), (1, &uk)]);
        assert_eq!(adj[&0], 0.0);
        assert!(adj[&1] <= -0.15);
    }

    #[test]
    fn language_is_a_weak_hint() {
        let ja = series(&[], "ja");
        let en = series(&[], "en");
        let ev = Evidence::of_file(None, &["jpn".into(), "eng".into()], Vec::new());
        assert_eq!(features(&ev, &ja).language, Some(true));
        let ev = Evidence::of_file(None, &["rus".into()], Vec::new());
        assert_eq!(features(&ev, &ja).language, Some(false));
        let adj = relative(&ev, &[(0, &ja), (1, &en)]);
        assert!(adj.values().all(|&v| v == 0.0));
        assert_eq!(iso639_1("JPN"), Some("ja"));
        assert_eq!(iso639_1("xx"), None);
    }

    #[test]
    fn ids_agree_needs_a_shared_namespace() {
        let rec: BTreeMap<String, String> = [("anidb".into(), "1".into())].into();
        let same: BTreeMap<String, String> = [("anidb".into(), "1".into())].into();
        let other: BTreeMap<String, String> = [("anidb".into(), "2".into())].into();
        let none: BTreeMap<String, String> = [("tmdb".into(), "tv:1".into())].into();
        assert!(ids_agree(&rec, &same));
        assert!(!ids_agree(&rec, &other));
        assert!(!ids_agree(&rec, &none));
    }
}
