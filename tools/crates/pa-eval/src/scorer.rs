// SPDX-License-Identifier: MPL-2.0
//! Adapter of the scorer v0 (`pa_recognition::score`, META-026) to the
//! [`Recognizer`] trait: candidates from the local title index, work and
//! episode decision per file, no provider calls. With series layouts
//! (`--anime-lists`, META-015) the episodes come in every order through the
//! numbering module: AniDB canonical, TMDB and TVDB absolute projections.

use pa_recognition::numbering::{Canonical, SeriesBook};
use pa_recognition::score::{self, FileMatch, GroupFile, Signals};
use pa_recognition::title::{TitleIndex, WorkKind};

use crate::candidates::record_ids;
use crate::corpus::{AnidbEpisode, CaseInput, Numbering, SeasonEpisode};
use crate::provider::Provider;
use crate::recognizer::{FilePrediction, Recognizer, WorkGuess, from_product_parse};

/// Scorer v0 over a prebuilt title index.
pub struct Scorer {
    index: TitleIndex,
    scorer: score::Scorer,
    /// `(case sequence number, path, signals)` of every decision, for
    /// `pa-eval run --signals` (feature analysis; not part of the report).
    pub log: Vec<(usize, String, FileMatch)>,
    cases: usize,
}

impl Scorer {
    /// Wraps an index built from local data (anime-offline-database bundle,
    /// cached provider records).
    pub fn new(index: TitleIndex) -> Self {
        Self {
            index,
            scorer: score::Scorer::new(),
            log: Vec::new(),
            cases: 0,
        }
    }

    /// The same with series layouts (META-015), completed with the index's
    /// episode counts and titles.
    pub fn with_series(index: TitleIndex, mut series: SeriesBook) -> Self {
        series.fill_from_index(&index);
        Self {
            scorer: score::Scorer::with_series(series),
            ..Self::new(index)
        }
    }

    fn prediction(&self, path: &str, m: &FileMatch) -> FilePrediction {
        let mut p = FilePrediction {
            path: path.to_owned(),
            confidence: f64::from(m.confidence),
            parse: Some(from_product_parse(
                &pa_recognition::parse::parse(path).best().parsed,
            )),
            ..Default::default()
        };
        let Some(record) = m.record.map(|r| self.index.record(r)) else {
            return p;
        };
        let kind = if record.anime {
            "anime"
        } else if record.kind == WorkKind::Movie {
            "movie"
        } else {
            "show"
        };
        p.work = Some(WorkGuess {
            kind: Some(kind.to_owned()),
            ids: record_ids(record),
        });
        if record.anime && !m.numbering.is_empty() {
            p.episodes = m
                .numbering
                .iter()
                .map(|x| Numbering {
                    anidb: match x.canonical {
                        Canonical::Anidb { kind, number, .. } => Some(AnidbEpisode {
                            kind: kind.as_str().to_owned(),
                            number,
                        }),
                        Canonical::Tmdb(_) => None,
                    },
                    tmdb: x.tmdb.map(|t| SeasonEpisode {
                        season: t.season,
                        episode: t.episode,
                    }),
                    tvdb_absolute: x.tvdb_absolute,
                    ..Default::default()
                })
                .collect();
            return p;
        }
        p.episodes = m
            .episodes
            .iter()
            .enumerate()
            .map(|(i, e)| {
                if record.anime {
                    Numbering {
                        anidb: Some(AnidbEpisode {
                            kind: "regular".to_owned(),
                            number: e.episode,
                        }),
                        tvdb_absolute: e.absolute,
                        tmdb: m
                            .written
                            .get(i)
                            .map(|&(season, episode)| SeasonEpisode { season, episode }),
                        ..Default::default()
                    }
                } else {
                    Numbering {
                        tmdb: e.season.map(|season| SeasonEpisode {
                            season,
                            episode: e.episode,
                        }),
                        ..Default::default()
                    }
                }
            })
            .collect();
        p
    }
}

impl Recognizer for Scorer {
    fn name(&self) -> &str {
        "scorer-v0"
    }

    fn recognize(&mut self, case: &CaseInput, _provider: &mut dyn Provider) -> Vec<FilePrediction> {
        let files: Vec<GroupFile<'_>> = case
            .files
            .iter()
            .map(|f| GroupFile {
                path: &f.path,
                duration_ms: f.duration_ms,
                audio_langs: &f.audio_langs,
                // No confirmed hash → work data exists for the corpus
                // (no crowd dataset, no AniDB FILE answers): hash hits
                // stay out of the offline eval.
                hashes: &[],
            })
            .collect();
        let matches = self.scorer.score_group(&self.index, &files);
        let out = case
            .files
            .iter()
            .zip(&matches)
            .map(|(f, m)| self.prediction(&f.path, m))
            .collect();
        for (f, m) in case.files.iter().zip(matches) {
            self.log.push((self.cases, f.path.clone(), m));
        }
        self.cases += 1;
        out
    }
}

/// One line of the `--signals` dump.
#[derive(serde::Serialize)]
pub struct SignalLine<'a> {
    /// Corpus case id.
    pub case_id: &'a str,
    /// File path.
    pub path: &'a str,
    /// Confidence of the decision.
    pub confidence: f32,
    /// Main title of the chosen record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chosen: Option<&'a str>,
    /// Main title of the runner-up record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_up: Option<&'a str>,
    /// Signals behind it.
    pub signals: &'a Signals,
    /// `(main title, year, score, reading)` of the best readings.
    pub alternatives: Vec<(&'a str, Option<u16>, f32, score::Reading)>,
}

impl Scorer {
    /// Main title of a record.
    pub fn title(&self, record: Option<usize>) -> Option<&str> {
        record.map(|r| self.index.record(r).titles[0].as_str())
    }

    /// Alternatives of a decision with record titles.
    pub fn alternatives<'a>(
        &'a self,
        m: &'a FileMatch,
    ) -> Vec<(&'a str, Option<u16>, f32, score::Reading)> {
        m.alternatives
            .iter()
            .map(|a| {
                let r = self.index.record(a.record);
                (r.titles[0].as_str(), r.year, a.score, a.reading)
            })
            .collect()
    }
}
