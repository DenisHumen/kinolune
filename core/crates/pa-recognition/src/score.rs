// SPDX-License-Identifier: MPL-2.0
//! Scorer v0: picks the work (and, for anime, the franchise entry and the
//! episode inside it) for every file of a group from the local title index —
//! stage \[5\] of research 03 §4.2 with hand-set weights (META-026; the
//! trained and calibrated scorer is META-018).
//!
//! For each file the parser's N-best hypotheses build a [`TitleQuery`], the
//! [`TitleIndex`] returns the top candidates, and every candidate is turned
//! into one or more *readings*:
//!
//! - movies and western series: the candidate itself, adjusted for kind
//!   (episode markers vs a movie record, a date vs an anime record) and year;
//! - anime series with an episode number: the *franchise* of the candidate —
//!   AniDB-backed TV/ONA records sharing its title base key, ordered by year
//!   and cut at long gaps (remakes) — lets the written number be read inside
//!   the matched entry (`Clannad - After Story - 05`), as an absolute number
//!   across the franchise (`Boku no Hero Academia - 133` → 6th season,
//!   ep. 20; entries titled `(Episodes 202-252)` say it directly), through an
//!   explicit season (`Haikyuu!! S04E15` → the entry titled `Season 4`,
//!   overflowing into its second cour) or as a number running on from the
//!   matched entry (`Mugen Ressha-hen - 12` → `Yuukaku-hen` ep. 5).
//!
//! Readings are scored, merged per record, and the confidence of a file comes
//! from the best score and its margin over the best *different* record: two
//! equally good works (remakes without a year, same-name series) give a low
//! confidence instead of a silent error. Extras and files without a title of
//! their own (`sample.mkv`, `Extras/Trailer 2.mkv`) take the decision of
//! their folder when every confident file there agrees.
//!
//! Everything is deterministic and offline: no provider calls, no clocks.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

pub mod pair;

use crate::parse::{EpisodeKind, MediaHint, ParsedName, parse};
use crate::title::normalize::TitleKey;
use crate::title::{Candidate, MatchTier, TitleIndex, TitleQuery, TitleRecord, WorkKind};

/// Candidates taken from the index per file.
pub const TOP_K: usize = 10;

/// Candidates searched for franchise entries named after an arc
/// (`Kimetsu no Yaiba: Hashira Geiko-hen` ranks low for `Kimetsu no Yaiba`).
const POOL_K: usize = 30;

/// Consecutive franchise entries further apart than this many years are
/// different series (remake, reboot).
const REMAKE_GAP_YEARS: u16 = 8;

/// One input file of a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GroupFile<'a> {
    /// Path relative to the library root, `/`-separated.
    pub path: &'a str,
    /// Container duration, if probed.
    pub duration_ms: Option<u64>,
    /// Audio track languages as probed (`jpn`, `eng`).
    pub audio_langs: &'a [String],
    /// Content hashes of the file (OSHash, ed2k; lowercase hex), looked up
    /// in [`Hints::hashes`].
    pub hashes: &'a [String],
}

/// A user rule: every file under `prefix` is the work named by `ids`
/// (a folder fix, research 03 §4.6; META-017 creates them from fixes).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserRule {
    /// Folder (or file) path prefix, `/`-separated, relative to the root;
    /// empty matches every file.
    pub prefix: String,
    /// Ids of the work, record notation (`anidb` → `9980`, `tmdb` →
    /// `tv:1429`).
    pub ids: BTreeMap<String, String>,
}

/// Evidence a caller knows beyond the files (META-014): user rules and
/// confirmed hash → work mappings. Both are decisive pair features.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hints {
    /// User rules; the longest matching prefix wins.
    pub rules: Vec<UserRule>,
    /// Content hash → ids of the work it was confirmed as (AniDB `FILE`
    /// for an ed2k, an earlier user fix, a crowd dataset).
    pub hashes: HashMap<String, BTreeMap<String, String>>,
}

impl Hints {
    /// Ids a rule or a hash hit names for `file`; a rule wins.
    fn named(&self, file: &GroupFile<'_>) -> Option<&BTreeMap<String, String>> {
        let rule = self
            .rules
            .iter()
            .filter(|r| {
                let prefix = r.prefix.trim_end_matches('/');
                prefix.is_empty()
                    || file.path == prefix
                    || file
                        .path
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
            .max_by_key(|r| r.prefix.trim_end_matches('/').len())
            .map(|r| &r.ids);
        rule.or_else(|| file.hashes.iter().find_map(|h| self.hashes.get(h)))
    }
}

/// An episode position inside the chosen record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EpisodeRef {
    /// Season as written (western series, `SxxEyy`); `None` for anime, whose
    /// records are single entries (AniDB-style).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    /// Regular episode number inside the record (anime) or season (series).
    pub episode: u32,
    /// Absolute number across the franchise, when every earlier entry has a
    /// known episode count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absolute: Option<u32>,
}

/// How the decision was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reading {
    /// No episode number involved (movies, series without numbers).
    #[default]
    Title,
    /// Number inside the matched entry.
    Direct,
    /// Absolute number across the franchise.
    Absolute,
    /// Explicit season → franchise entry.
    Season,
    /// Number running on from the matched entry into the next ones.
    Overflow,
    /// Taken from the folder's other files.
    Inherited,
}

/// Signals behind a decision (explanation, and features for the trained
/// scorer of META-018).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Signals {
    /// Score of the chosen reading.
    pub top: f32,
    /// Margin over the best reading of a different record (`top` when there
    /// is none).
    pub margin: f32,
    /// Best different record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_up: Option<usize>,
    /// How the best title matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<MatchTier>,
    /// How the decision was reached.
    pub reading: Reading,
    /// The name carries a year.
    pub year: bool,
    /// The name carries episode markers.
    pub episodic: bool,
    /// The franchise of the chosen record has a remake with the same title.
    pub remake: bool,
    /// Candidates returned by the index.
    pub candidates: usize,
    /// Pair features of the chosen record (duration, episode range,
    /// language; META-014).
    #[serde(default)]
    pub pair: pair::Features,
    /// A user rule or a hash hit named the work.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hinted: bool,
    /// The decision was taken for the file's series group as a whole
    /// (META-014).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub joint: bool,
}

/// Decision for one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileMatch {
    /// Chosen record ([`TitleIndex::record`]); `None` — abstain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<usize>,
    /// Confidence in `[0, 1]` that `record` is the right work (hand-set
    /// mapping of the signals, not calibrated — META-018).
    pub confidence: f32,
    /// Episodes contained in the file, inside `record`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub episodes: Vec<EpisodeRef>,
    /// Why.
    pub signals: Signals,
    /// `(season, episode)` exactly as written (`S04E15`, `Season 4/… - 15`):
    /// the TMDB order of most releases, whatever record the file maps to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub written: Vec<(u32, u32)>,
    /// Best readings per record, best first (the decision included), for
    /// explanations and the review UI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Alternative>,
}

/// A scored reading of the file as one record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alternative {
    /// Record ([`TitleIndex::record`]).
    pub record: usize,
    /// Score of the reading.
    pub score: f32,
    /// How the number was read.
    pub reading: Reading,
    /// Episodes inside the record.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub episodes: Vec<EpisodeRef>,
}

/// Alternatives kept per file.
const ALTERNATIVES: usize = 5;

/// Per-record facts derived from its titles, computed once.
#[derive(Debug, Default)]
struct RecordInfo {
    /// Base keys of all titles.
    bases: Vec<String>,
    /// `(base, season, part)` of every title claiming a season (`Season 2`,
    /// `2nd Season`, `S2 Part 2`).
    seasons: Vec<(String, u32, u32)>,
    /// Absolute ranges written in titles: `Gintama (Episodes 202-252)`.
    ranges: Vec<(u32, u32)>,
    /// Lower-cased titles, for literal matches.
    lower: Vec<String>,
}

impl RecordInfo {
    fn of(rec: &TitleRecord) -> Self {
        let mut info = Self::default();
        for t in &rec.titles {
            let key = TitleKey::new(t);
            if !info.bases.contains(&key.base) {
                info.bases.push(key.base.clone());
            }
            if let Some(s) = key.season {
                info.seasons
                    .push((key.base.clone(), s, key.part.unwrap_or(1)));
            }
            if let Some(r) = episode_range(t) {
                info.ranges.push(r);
            }
            info.lower.push(t.trim().to_lowercase());
        }
        info
    }

    /// Titles claiming season `s` (any part when `part` is `None`) under
    /// one of `bases` (any base when empty): `sxf s2` or `Vigilante: …
    /// 2nd Season` do not speak for `Spy x Family` or `My Hero Academia`.
    fn claims(&self, s: u32, part: Option<u32>, bases: &[String]) -> u32 {
        self.seasons
            .iter()
            .filter(|(b, ss, pp)| {
                *ss == s && part.is_none_or(|p| p == *pp) && (bases.is_empty() || bases.contains(b))
            })
            .count() as u32
    }
}

/// `… (Episodes 202-252)` (also `Episoden`, `Episodi`, `Episodios`).
fn episode_range(title: &str) -> Option<(u32, u32)> {
    let lower = title.to_lowercase();
    let start = lower.find("(episod")?;
    let rest = &lower[start..];
    let close = rest.find(')')?;
    let inner = &rest[..close];
    let digits = inner.find(|c: char| c.is_ascii_digit())?;
    let (a, b) = inner[digits..].split_once('-')?;
    let a: u32 = a.trim().parse().ok()?;
    let b: u32 = b.trim().parse().ok()?;
    (a >= 1 && b >= a).then_some((a, b))
}

/// The scorer with its per-record cache. One instance can serve many groups
/// of the same index.
#[derive(Debug, Default)]
pub struct Scorer {
    info: HashMap<usize, RecordInfo>,
}

/// One reading of a file as a record.
#[derive(Debug, Clone)]
struct Hyp {
    record: usize,
    /// First entry of the franchise segment the reading runs through (the
    /// record itself outside anime franchises): files of one group read
    /// through the same head belong to one series.
    head: usize,
    score: f32,
    episodes: Vec<EpisodeRef>,
    reading: Reading,
    tier: MatchTier,
    remake: bool,
    /// Pair features of the record against the file.
    pair: pair::Features,
}

/// What the file name says about numbers.
struct Numbers {
    /// Whole episode numbers as written (absolute or within a season).
    values: Vec<u32>,
    season: Option<u32>,
    part: Option<u32>,
    /// The name has episode markers of any kind (numbers, `.5`, dates, kinds).
    episodic: bool,
}

impl Numbers {
    fn of(p: &ParsedName) -> Self {
        let source = if p.absolute.is_empty() {
            &p.episodes
        } else {
            &p.absolute
        };
        let values: Vec<u32> = source
            .iter()
            .filter(|n| n.fraction.is_none())
            .map(|n| n.number)
            .collect();
        let key = p.title.as_deref().map(TitleKey::new);
        Self {
            episodic: !source.is_empty() || p.date.is_some() || p.episode_kind.is_some(),
            values,
            season: p.season.or(key.as_ref().and_then(|k| k.season)),
            part: key.and_then(|k| k.part),
        }
    }
}

/// A franchise: positions of records in the index, in broadcast order.
struct Franchise {
    chain: Vec<usize>,
    /// Another series with the same title exists outside this chain.
    remake: bool,
    /// Episode count of a much later entry of the chain that also carries
    /// the written title (`Kino no Tabi (2017)`): a bare title with a number
    /// up to it may mean either series.
    soft_remake: Option<u32>,
}

/// Scores a whole group (one folder tree) with a fresh cache.
pub fn score_group(index: &TitleIndex, files: &[GroupFile<'_>]) -> Vec<FileMatch> {
    Scorer::default().score_group(index, files)
}

impl Scorer {
    /// A scorer with an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Scores a whole group (one folder tree, one case) at once.
    pub fn score_group(&mut self, index: &TitleIndex, files: &[GroupFile<'_>]) -> Vec<FileMatch> {
        self.score_group_with(index, files, &Hints::default())
    }

    /// Scores a whole group with the caller's user rules and hash hits:
    /// every file on its own (readings and pair features), then every
    /// series of the group at once ([`joint`]: one series per folder and
    /// title, episodes unique and consecutive), then extras and files
    /// without a title from their folder.
    pub fn score_group_with(
        &mut self,
        index: &TitleIndex,
        files: &[GroupFile<'_>],
        hints: &Hints,
    ) -> Vec<FileMatch> {
        let parsed: Vec<ParsedName> = files
            .iter()
            .map(|f| parse(f.path).best().parsed.clone())
            .collect();
        let mut out = Vec::with_capacity(files.len());
        let mut ranked = Vec::with_capacity(files.len());
        for f in files {
            let (m, hyps) = self.score_one(index, f, hints);
            out.push(m);
            ranked.push(hyps);
        }
        joint(files, &parsed, &ranked, &mut out);
        inherit_from_folder(files, &parsed, &mut out);
        out
    }

    fn info(&mut self, index: &TitleIndex, r: usize) -> &RecordInfo {
        self.info
            .entry(r)
            .or_insert_with(|| RecordInfo::of(index.record(r)))
    }

    /// Scores one file on its own (no group context, no probe data).
    pub fn score_file(&mut self, index: &TitleIndex, path: &str) -> FileMatch {
        let file = GroupFile {
            path,
            ..Default::default()
        };
        self.score_one(index, &file, &Hints::default()).0
    }

    /// Scores one file: the decision and every record's best reading, best
    /// first (for [`joint`]).
    fn score_one(
        &mut self,
        index: &TitleIndex,
        file: &GroupFile<'_>,
        hints: &Hints,
    ) -> (FileMatch, Vec<Hyp>) {
        let parsed = parse(file.path);
        let best = &parsed.best().parsed;
        let numbers = Numbers::of(best);
        let mut signals = Signals {
            year: best.year.is_some(),
            episodic: numbers.episodic,
            ..Default::default()
        };
        if best.title.is_none() {
            return (
                FileMatch {
                    signals,
                    ..Default::default()
                },
                Vec::new(),
            );
        }
        let query = TitleQuery::from_parse(&parsed);
        let pool = index.search(&query, POOL_K);
        let candidates = &pool[..pool.len().min(TOP_K)];
        signals.candidates = candidates.len();

        let written: Vec<(u32, u32)> = match numbers.season {
            Some(season) if best.absolute.is_empty() => {
                numbers.values.iter().map(|&e| (season, e)).collect()
            }
            _ => Vec::new(),
        };
        // A split part or an extra is not the work's runtime.
        let duration = file
            .duration_ms
            .filter(|_| best.part.is_none() && best.extra.is_none());
        let evidence = pair::Evidence::of_file(duration, file.audio_langs, written.clone());
        let records: Vec<(usize, &TitleRecord)> = candidates
            .iter()
            .map(|c| (c.record, index.record(c.record)))
            .collect();
        let relative = pair::relative(&evidence, &records);

        let mut hyps = Vec::new();
        for c in candidates {
            let adj = relative.get(&c.record).copied().unwrap_or(0.0);
            self.interpret(index, c, &pool, best, &numbers, adj, &mut hyps);
        }
        let named = hints.named(file);
        if let Some(ids) = named {
            hint(index, ids, &numbers, &mut hyps);
        }
        let mut per_record: BTreeMap<usize, Hyp> = BTreeMap::new();
        for h in hyps {
            match per_record.get(&h.record) {
                Some(old) if old.score >= h.score => {}
                _ => {
                    per_record.insert(h.record, h);
                }
            }
        }
        let mut ranked: Vec<Hyp> = per_record.into_values().collect();
        ranked.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.record.cmp(&b.record)));
        for h in &mut ranked {
            h.pair = pair::features(&evidence, index.record(h.record));
        }
        let Some(top) = ranked.first() else {
            return (
                FileMatch {
                    signals,
                    ..Default::default()
                },
                ranked,
            );
        };
        signals.top = top.score;
        signals.margin = top.score - ranked.get(1).map_or(0.0, |h| h.score);
        signals.runner_up = ranked.get(1).map(|h| h.record);
        signals.tier = Some(top.tier);
        signals.reading = top.reading;
        signals.remake = top.remake;
        signals.pair = top.pair;
        signals.hinted =
            named.is_some_and(|ids| pair::ids_agree(&index.record(top.record).ids, ids));
        let alternatives = alternatives(&ranked);
        let m = FileMatch {
            record: Some(top.record),
            confidence: confidence(&signals, best),
            episodes: top.episodes.clone(),
            signals,
            written,
            alternatives,
        };
        (m, ranked)
    }

    /// Turns one candidate into readings.
    fn interpret(
        &mut self,
        index: &TitleIndex,
        c: &Candidate,
        all: &[Candidate],
        best: &ParsedName,
        numbers: &Numbers,
        relative: f32,
        out: &mut Vec<Hyp>,
    ) {
        let rec = index.record(c.record);
        let mut base = c.score + agreement(best, numbers, rec) + relative;
        // The written title says more than the matched one (`Fairy Tail
        // Final Series` → `Fairy Tail`, matched on the base key only).
        if c.tier == MatchTier::Base
            && let Some(q) = best.title.as_deref()
        {
            let (qk, mk) = (TitleKey::new(q), TitleKey::new(&rec.titles[c.title]));
            if qk.loose != qk.base && mk.loose == mk.base {
                base -= 0.05;
            }
        }
        let anime_series = rec.anime && rec.kind == WorkKind::Series;
        if !anime_series || numbers.values.is_empty() {
            let episodes = match (rec.anime, rec.kind, numbers.season) {
                (false, WorkKind::Series, Some(season)) => numbers
                    .values
                    .iter()
                    .map(|&e| EpisodeRef {
                        season: Some(season),
                        episode: e,
                        absolute: None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            base += self.literal(index, c, best);
            out.push(Hyp {
                record: c.record,
                head: c.record,
                score: base,
                episodes,
                reading: Reading::Title,
                tier: c.tier,
                remake: false,
                pair: pair::Features::default(),
            });
            return;
        }

        let fr = self.franchise(index, c, all, best);
        if !fr.remake {
            base += self.literal(index, c, best);
        }
        let chain = &fr.chain;
        let pos = chain.iter().position(|&r| r == c.record);
        let first = numbers.values[0];
        let last = *numbers.values.iter().max().unwrap_or(&first);
        let head = chain.first().copied().unwrap_or(c.record);
        let mut push = |record, score, episodes, reading| {
            out.push(Hyp {
                record,
                head,
                score,
                episodes,
                reading,
                tier: c.tier,
                remake: fr.remake
                    || (reading == Reading::Direct && fr.soft_remake.is_some_and(|n| last <= n)),
                pair: pair::Features::default(),
            })
        };

        // Direct: the number is inside the matched entry.
        let own = rec.episodes.filter(|&n| n > 0);
        if own.is_none_or(|n| last <= n) {
            let mut prior = 0.0;
            // `Title - 05` matched through a qualified title (`Title (2017)`,
            // `Title 2`) of a later entry: the bare title means the first one.
            let matched = TitleKey::new(&rec.titles[c.title]);
            if c.tier != MatchTier::Exact && (matched.season.is_some() || matched.year.is_some()) {
                prior -= 0.04;
            }
            // A later entry reached through a bare synonym (`Dorohedoro` for
            // `Dorohedoro Season 2`): a bare title means the first entry.
            if pos.is_some_and(|p| p > 0) && c.title != 0 && matched.season.is_none() {
                prior -= 0.05;
            }
            if let Some(s) = numbers.season
                && chain.len() > 1
                && self.info(index, c.record).claims(s, None, &[]) == 0
                && pos.is_none_or(|p| p + 1 != s as usize)
            {
                prior -= 0.12;
            }
            let eps = numbers
                .values
                .iter()
                .map(|&e| EpisodeRef {
                    season: None,
                    episode: e,
                    absolute: pos.and_then(|p| offset(index, chain, p)).map(|o| o + e),
                })
                .collect();
            push(c.record, base + prior, eps, Reading::Direct);
        } else if pos.is_none() || chain.len() < 2 {
            // Number past the end and no franchise to run into.
            push(c.record, base - 0.15, Vec::new(), Reading::Direct);
        }
        let Some(pos) = pos else { return };
        if chain.len() < 2 {
            return;
        }
        let place = |from: usize| place(index, chain, from, &numbers.values);

        // Absolute across the franchise.
        if numbers.season.is_none_or(|s| s <= 1) {
            // A title that states the range (`Gintama (Episodes 202-252)`).
            let mut ranged = false;
            for &r in chain {
                let hit = self
                    .info(index, r)
                    .ranges
                    .iter()
                    .find(|(a, b)| *a <= first && first <= *b)
                    .copied();
                if let Some((a, _)) = hit
                    && a > 1
                {
                    ranged = true;
                    let eps = numbers
                        .values
                        .iter()
                        .map(|&n| EpisodeRef {
                            season: None,
                            episode: n + 1 - a.min(n),
                            absolute: Some(n),
                        })
                        .collect();
                    push(r, base + 0.02, eps, Reading::Absolute);
                }
            }
            let head_eps = index.record(chain[0]).episodes.unwrap_or(0);
            if !ranged
                && first > head_eps
                && let Some((r, eps)) = place(0)
            {
                let prior = if pos == 0 { 0.0 } else { -0.02 };
                push(r, base + prior, eps, Reading::Absolute);
            }
        }
        // Explicit season → the first entry whose titles claim it most
        // (`Season 2`, `2nd Season`; a later cour continues the season), or
        // the n-th entry when none does. Season 1 starts the franchise.
        if let Some(s) = numbers.season
            && s >= 1
        {
            let part = numbers.part.unwrap_or(1);
            let mut bases = self.info(index, chain[0]).bases.clone();
            if let Some(q) = best.title.as_deref() {
                bases.push(TitleKey::new(q).base);
            }
            let claims: Vec<u32> = chain
                .iter()
                .map(|&r| self.info(index, r).claims(s, Some(part), &bases))
                .collect();
            let max = claims.iter().copied().max().unwrap_or(0);
            if max > 0 && (s > 1 || part > 1) {
                let start = claims.iter().position(|&n| n == max).unwrap_or(0);
                // A single title claiming the season is weak evidence
                // (synonym lists carry stray `Season 4`s).
                let strength = if max == 1 { -0.06 } else { 0.0 };
                for (p, &n) in claims.iter().enumerate() {
                    if n > 0
                        && let Some((r, eps)) = place(p)
                    {
                        let prior = if p == start {
                            0.0
                        } else {
                            0.10 * (n as f32 / max as f32 - 1.0) - 0.03
                        };
                        push(r, base + 0.01 + strength + prior, eps, Reading::Season);
                    }
                }
            } else if (s as usize) <= chain.len()
                && let Some((r, eps)) = place(s as usize - 1)
            {
                push(r, base - 0.02, eps, Reading::Season);
            }
        }
        // Number running on from a later entry named after itself
        // (`Mugen Ressha-hen - 12`), not reached through the franchise title.
        let specific = !self
            .info(index, chain[0])
            .bases
            .contains(&TitleKey::new(&rec.titles[c.title]).base);
        if pos > 0
            && specific
            && own.is_some_and(|n| last > n)
            && let Some((r, eps)) = place(pos)
        {
            push(r, base - 0.04, eps, Reading::Overflow);
        }
    }

    /// Bonus for a literal (case-insensitive, unfolded) match of the written
    /// title: `Gintama'` is the record titled `Gintama'`, not `Gintama`,
    /// although both fold to the same key.
    fn literal(&mut self, index: &TitleIndex, c: &Candidate, best: &ParsedName) -> f32 {
        let Some(title) = best.title.as_deref() else {
            return 0.0;
        };
        let want = title.trim().to_lowercase();
        let info = self.info(index, c.record);
        if info.lower.first() == Some(&want) {
            0.08
        } else if info.lower.contains(&want) {
            0.01
        } else {
            0.0
        }
    }

    /// Franchise of an anime candidate.
    fn franchise(
        &mut self,
        index: &TitleIndex,
        c: &Candidate,
        all: &[Candidate],
        best: &ParsedName,
    ) -> Franchise {
        let rec = index.record(c.record);
        let anidb = rec.ids.contains_key("anidb");
        let query_base = best.title.as_deref().map(|t| TitleKey::new(t).base);
        let mut bases: Vec<String> = vec![
            TitleKey::new(&rec.titles[c.title]).base,
            TitleKey::new(&rec.titles[0]).base,
        ];
        bases.extend(query_base.clone());
        bases.sort();
        bases.dedup();
        let mut members: Vec<usize> = bases
            .iter()
            .flat_map(|b| index.records_with_base(b))
            .collect();
        // Entries named after an arc (`Kimetsu no Yaiba: Yuukaku-hen`) share
        // only a prefix with the franchise title.
        if let Some(q) = &query_base {
            for o in all {
                let main = &self.info(index, o.record).bases;
                if main.iter().any(|b| shares_prefix(q, b)) {
                    members.push(o.record);
                }
            }
        }
        members.push(c.record);
        members.retain(|&r| {
            let x = index.record(r);
            x.anime && x.kind == WorkKind::Series && (!anidb || x.ids.contains_key("anidb"))
        });
        members.sort_by_key(|&r| {
            let x = index.record(r);
            let aid = x
                .ids
                .get("anidb")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(u64::MAX);
            (x.year.unwrap_or(u16::MAX), aid, r)
        });
        members.dedup();
        // Cut at a restart: after a long gap, an entry whose main title is
        // the segment head's title again (`Fruits Basket 1st Season` (2019)
        // after `Fruits Basket` (2001), `Hunter x Hunter (2011)`) is a
        // remake; `Bleach: Sennen Kessen-hen` (2022) continues `Bleach`.
        let mut segments: Vec<Vec<usize>> = Vec::new();
        let mut last_year: Option<u16> = None;
        // Entries that follow a long gap without a season claim and carry
        // the written title (`Kino no Tabi (2017)`): a small number under
        // the bare title may belong to either series.
        let mut remake_like: Vec<usize> = Vec::new();
        for r in members {
            let year = index.record(r).year;
            let gap = matches!((last_year, year), (Some(a), Some(b)) if b.saturating_sub(a) >= REMAKE_GAP_YEARS);
            let restart = match segments.last() {
                Some(seg) if gap => {
                    let head = TitleKey::new(&index.record(seg[0]).titles[0]).base;
                    TitleKey::new(&index.record(r).titles[0]).base == head
                }
                _ => false,
            };
            if gap && !restart {
                let info = self.info(index, r);
                if let Some(q) = &query_base
                    && info.bases.contains(q)
                    && !info.seasons.iter().any(|(b, s, _)| *s >= 2 && b == q)
                {
                    remake_like.push(r);
                }
            }
            if restart || segments.is_empty() {
                segments.push(Vec::new());
            }
            segments.last_mut().expect("pushed").push(r);
            if year.is_some() {
                last_year = year;
            }
        }
        // A remake competes only if it carries the written title too.
        let remake = segments.len() > 1
            && query_base.as_ref().is_some_and(|q| {
                segments
                    .iter()
                    .filter(|seg| !seg.contains(&c.record))
                    .flatten()
                    .any(|&r| self.info(index, r).bases.contains(q))
            });
        let chain = segments
            .into_iter()
            .find(|s| s.contains(&c.record))
            .unwrap_or_else(|| vec![c.record]);
        let soft_remake = remake_like
            .iter()
            .filter(|r| chain.contains(r))
            .filter_map(|&r| index.record(r).episodes)
            .max();
        Franchise {
            chain,
            remake,
            soft_remake,
        }
    }
}

/// `entry` continues the written title `query`: it starts with all words of
/// `query` (`gintama` → `gintama porori hen`), or both share at least three
/// leading words (`kimetsu no yaiba mugen ressha hen` → `kimetsu no yaiba
/// yuukaku hen`). A shorter parent title (`dragon ball` for `dragon ball
/// super`) does not.
fn shares_prefix(query: &str, entry: &str) -> bool {
    let (wq, we): (Vec<&str>, Vec<&str>) = (query.split(' ').collect(), entry.split(' ').collect());
    let common = wq.iter().zip(&we).take_while(|(x, y)| x == y).count();
    common >= 3 || (common == wq.len() && common > 0)
}

/// Places written numbers counted from the start of entry `from`.
fn place(
    index: &TitleIndex,
    chain: &[usize],
    from: usize,
    values: &[u32],
) -> Option<(usize, Vec<EpisodeRef>)> {
    let mut target = None;
    let mut eps = Vec::new();
    for &n in values {
        let (p, e) = locate(index, chain, from, n)?;
        if *target.get_or_insert(p) != p {
            // A multi-episode file across two entries: keep the first.
            continue;
        }
        eps.push(EpisodeRef {
            season: None,
            episode: e,
            absolute: offset(index, chain, p).map(|o| o + e),
        });
    }
    target.map(|p| (chain[p], eps))
}

/// Hand-set mapping of the signals to `[0, 1]`.
fn confidence(s: &Signals, best: &ParsedName) -> f32 {
    let quality = ((s.top - 0.70) / 0.30).clamp(0.0, 1.0);
    // Margins of 0.075 and more separate cleanly on train/dev; a literal
    // title bonus alone (≤ 0.08 between same-key titles) is just enough.
    let separation = ((s.margin - 0.03) / 0.045).clamp(0.0, 1.0);
    let mut c = 0.25 + 0.75 * quality * separation;
    if matches!(s.tier, Some(MatchTier::Fuzzy)) {
        c *= 0.85;
    }
    if best.extra.is_some() {
        c *= 0.95;
    }
    if s.remake && !s.year {
        c = c.min(0.6);
    }
    c.clamp(0.0, 1.0)
}

/// Score adjustment for kind and year agreement between the name and a record.
fn agreement(best: &ParsedName, numbers: &Numbers, rec: &TitleRecord) -> f32 {
    let mut adj = 0.0;
    let last = numbers.values.iter().copied().max();
    match rec.kind {
        WorkKind::Movie if numbers.episodic => adj -= 0.12,
        WorkKind::Series if !numbers.episodic && best.media == MediaHint::Movie => adj -= 0.06,
        WorkKind::Special => {
            if matches!(
                best.episode_kind,
                Some(EpisodeKind::Ova | EpisodeKind::Ona | EpisodeKind::Special | EpisodeKind::Sp)
            ) {
                adj += 0.03;
            } else if numbers.episodic {
                adj -= 0.03;
            }
        }
        WorkKind::Series
            if matches!(best.episode_kind, Some(EpisodeKind::Ova | EpisodeKind::Ona)) =>
        {
            adj -= 0.03
        }
        _ => {}
    }
    // Episode number past the end of a record that cannot run on.
    if rec.kind != WorkKind::Series
        && let (Some(n), Some(count)) = (last, rec.episodes.filter(|&c| c > 0))
        && n > count
    {
        adj -= 0.15;
    }
    // Dated episodes are western daily shows, never anime.
    if let Some(date) = best.date {
        if rec.anime {
            adj -= 0.08;
        }
        if rec.year.is_some_and(|y| date.year + 1 < y) {
            adj -= 0.10;
        }
    }
    if rec.anime && !best.anime_hint && numbers.season.is_some() && !rec.ids.is_empty() {
        // `Show.S01E02` without fansub markers leans western.
        adj -= 0.01;
    }
    if let (Some(a), Some(b)) = (best.year, rec.year)
        && a.abs_diff(b) > 1
    {
        adj -= 0.10;
    }
    // AniDB is the canonical anime space; anime-offline-database entries
    // without an AniDB id are mostly split duplicates of one that has it.
    if rec.anime && !rec.ids.contains_key("anidb") {
        adj -= 0.03;
    }
    adj
}

/// Entry (position in `chain`) and episode for number `n` counted from the
/// start of entry `from`. An entry with an unknown count takes everything.
fn locate(index: &TitleIndex, chain: &[usize], from: usize, n: u32) -> Option<(usize, u32)> {
    if n == 0 {
        return None;
    }
    let mut e = n;
    let mut i = from;
    loop {
        match index.record(*chain.get(i)?).episodes.filter(|&c| c > 0) {
            Some(count) if e > count => {
                e -= count;
                i += 1;
            }
            _ => return Some((i, e)),
        }
    }
}

/// Regular episodes of the entries before `pos`, if all are known.
fn offset(index: &TitleIndex, chain: &[usize], pos: usize) -> Option<u32> {
    if chain.len() < 2 {
        return None;
    }
    chain[..pos]
        .iter()
        .map(|&r| index.record(r).episodes.filter(|&c| c > 0))
        .sum()
}

/// Alternatives of a file from its ranked readings.
fn alternatives(ranked: &[Hyp]) -> Vec<Alternative> {
    ranked
        .iter()
        .take(ALTERNATIVES)
        .map(|h| Alternative {
            record: h.record,
            score: h.score,
            reading: h.reading,
            episodes: h.episodes.clone(),
        })
        .collect()
}

/// Applies a user rule or a hash hit naming the work `ids`: its readings
/// gain [`pair::DECISIVE`], every other one is [`pair::OVERRULED`]; a work
/// the title did not retrieve joins as a reading of its own.
fn hint(
    index: &TitleIndex,
    ids: &BTreeMap<String, String>,
    numbers: &Numbers,
    hyps: &mut Vec<Hyp>,
) {
    let mut found = false;
    for h in hyps.iter_mut() {
        if pair::ids_agree(&index.record(h.record).ids, ids) {
            h.score += pair::DECISIVE;
            found = true;
        } else {
            h.score += pair::OVERRULED;
        }
    }
    if found {
        return;
    }
    let Some(record) = index
        .records()
        .iter()
        .position(|r| pair::ids_agree(&r.ids, ids))
    else {
        return;
    };
    let rec = index.record(record);
    let episodes = numbers
        .values
        .iter()
        .map(|&e| EpisodeRef {
            season: if rec.anime { None } else { numbers.season },
            episode: e,
            absolute: None,
        })
        .filter(|_| rec.kind == WorkKind::Series)
        .collect();
    hyps.push(Hyp {
        record,
        head: record,
        score: 0.75 + pair::DECISIVE,
        episodes,
        reading: if rec.anime && rec.kind == WorkKind::Series {
            Reading::Direct
        } else {
            Reading::Title
        },
        tier: MatchTier::Exact,
        remake: false,
        pair: pair::Features::default(),
    });
}

/// Score a file of a series group loses when it has no reading through
/// the head the group is assigned to.
const MISSING: f32 = 0.20;

/// A file supports a head when its best reading through it is at most this
/// far below the file's best reading.
const SUPPORT: f32 = 0.15;

/// Penalty per violated group constraint: two files of one release on the
/// same episode, or consecutive numbers mapped to non-consecutive episodes.
const VIOLATION: f32 = 0.08;

/// Written numbers further apart than this are a gap in the user's files,
/// not a constraint on the mapping.
const NEAR: u32 = 3;

/// Joint decision for the series of a group (research 03 §4.2 \[5\]):
/// numbered files of one show folder (season folders merged) are decided
/// together instead of one by one.
///
/// In a folder, the head (franchise head, or the western series itself)
/// that most files support ([`SUPPORT`]) takes those files; the objective is
/// the sum of their readings through the head minus [`VIOLATION`] for every
/// broken constraint (episodes of one release unique, consecutive numbers on
/// consecutive episodes, across entry boundaries too). Inside the head the
/// readings follow one rule for the whole set when that breaks fewer
/// constraints (all absolute, all through the season…). The rest of the
/// folder is decided the same way (a dump folder holds several shows).
///
/// The confidence of a file then uses the set's margin over the best other
/// head on the same files ([`MISSING`] for a file with no reading through
/// it): `Shameless.S09E01` alone fits the US and the UK series equally, but
/// next to `Бесстыжие - 9 сезон 5 серия`, which only the US series is
/// called, both are decided.
fn joint(
    files: &[GroupFile<'_>],
    parsed: &[ParsedName],
    ranked: &[Vec<Hyp>],
    out: &mut [FileMatch],
) {
    let mut folders: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, p) in parsed.iter().enumerate() {
        if p.extra.is_some()
            || ranked[i].is_empty()
            || out[i].signals.hinted
            || p.title.is_none()
            || Numbers::of(p).values.is_empty()
        {
            continue;
        }
        folders
            .entry(series_root(parent(files[i].path)))
            .or_default()
            .push(i);
    }
    for mut remaining in folders.into_values() {
        while remaining.len() >= 2 {
            let mut heads: Vec<usize> = remaining
                .iter()
                .flat_map(|&i| ranked[i].iter().map(|h| h.head))
                .collect();
            heads.sort_unstable();
            heads.dedup();
            // (support size, objective, head, supporters)
            let mut best: Option<(usize, f32, usize, Vec<usize>)> = None;
            for &h in &heads {
                let support: Vec<usize> = remaining
                    .iter()
                    .copied()
                    .filter(|&i| {
                        ranked[i]
                            .iter()
                            .find(|x| x.head == h)
                            .is_some_and(|x| x.score >= ranked[i][0].score - SUPPORT)
                    })
                    .collect();
                if support.len() < 2 {
                    continue;
                }
                let (objective, _) = assign(h, &support, parsed, ranked);
                let better = best.as_ref().is_none_or(|(n, o, bh, _)| {
                    (support.len(), objective) > (*n, *o)
                        || (support.len() == *n && objective == *o && h < *bh)
                });
                if better {
                    best = Some((support.len(), objective, h, support));
                }
            }
            let Some((_, objective, head, support)) = best else {
                break;
            };
            let (_, assignment) = assign(head, &support, parsed, ranked);
            let rival = heads
                .iter()
                .filter(|&&h| h != head)
                .map(|&h| assign(h, &support, parsed, ranked).0)
                .fold(f32::NEG_INFINITY, f32::max);
            let group_margin = if rival.is_finite() {
                (objective - rival) / support.len() as f32
            } else {
                f32::INFINITY
            };
            for (k, &i) in support.iter().enumerate() {
                if let Some(j) = assignment[k] {
                    decide(&mut out[i], &parsed[i], &ranked[i], j, head, group_margin);
                }
            }
            remaining.retain(|i| !support.contains(i));
        }
    }
}

/// Sets a file's decision to its reading `j` through `head`, with the
/// group's margin over other heads.
fn decide(
    m: &mut FileMatch,
    best: &ParsedName,
    ranked: &[Hyp],
    j: usize,
    head: usize,
    group_margin: f32,
) {
    let c = &ranked[j];
    let best_other = |same_head: bool| {
        ranked
            .iter()
            .filter(|h| h.record != c.record && (h.head == head) == same_head)
            .map(|h| (h.score, h.record))
            .max_by(|a, b| a.0.total_cmp(&b.0))
    };
    let (same, cross) = (best_other(true), best_other(false));
    let m_same = same.map_or(f32::INFINITY, |(s, _)| c.score - s);
    let m_cross = cross
        .map_or(f32::INFINITY, |(s, _)| c.score - s)
        .max(group_margin);
    let margin = m_same.min(m_cross);
    m.record = Some(c.record);
    m.episodes = c.episodes.clone();
    m.signals.top = c.score;
    m.signals.margin = if margin.is_finite() { margin } else { c.score };
    m.signals.runner_up = match (same, cross) {
        (Some(a), Some(b)) => Some(if a.0 >= b.0 { a.1 } else { b.1 }),
        (a, b) => a.or(b).map(|x| x.1),
    };
    m.signals.tier = Some(c.tier);
    m.signals.reading = c.reading;
    m.signals.remake = c.remake;
    m.signals.pair = c.pair;
    m.signals.joint = true;
    m.confidence = confidence(&m.signals, best);
}

/// The show folder of a folder: a trailing season folder (`Season 2`,
/// `Сезон 9`, `S02`, `Specials`) is part of its parent.
fn series_root(dir: &str) -> &str {
    let (head, last) = dir.rsplit_once('/').unwrap_or(("", dir));
    let lower = last.trim().to_lowercase();
    let word =
        lower.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ' || c == '.' || c == '_');
    let has_number = word.len() < lower.len();
    let seasonal = matches!(
        word,
        "season" | "сезон" | "series" | "staffel" | "saison" | "temporada" | "s"
    ) && has_number
        || matches!(lower.as_str(), "specials" | "extras" | "ova" | "ovas");
    if seasonal && !last.is_empty() {
        head
    } else {
        dir
    }
}

/// The best assignment of a series group to `head`: per file the index of
/// its reading (`None`: no reading through `head`), and the objective.
fn assign(
    head: usize,
    members: &[usize],
    parsed: &[ParsedName],
    ranked: &[Vec<Hyp>],
) -> (f32, Vec<Option<usize>>) {
    let mut modes: Vec<Option<Reading>> = vec![None];
    for &i in members {
        for h in ranked[i].iter().filter(|h| h.head == head) {
            if !modes.contains(&Some(h.reading)) {
                modes.push(Some(h.reading));
            }
        }
    }
    let mut best: Option<(f32, Vec<Option<usize>>)> = None;
    for mode in modes {
        let assignment: Vec<Option<usize>> = members
            .iter()
            .map(|&i| {
                let through = |want: Option<Reading>| {
                    ranked[i]
                        .iter()
                        .position(|h| h.head == head && want.is_none_or(|r| r == h.reading))
                };
                through(mode).or_else(|| through(None))
            })
            .collect();
        let mut objective: f32 = members
            .iter()
            .zip(&assignment)
            .map(|(&i, a)| match a {
                Some(j) => ranked[i][*j].score,
                None => ranked[i][0].score - MISSING,
            })
            .sum();
        objective -= VIOLATION * violations(members, &assignment, parsed, ranked) as f32;
        if best.as_ref().is_none_or(|(o, _)| objective > *o + 1e-6) {
            best = Some((objective, assignment));
        }
    }
    best.unwrap_or_default()
}

/// Broken constraints of an assignment, per release (release group):
/// duplicate episodes and consecutive written numbers on non-consecutive
/// episodes.
fn violations(
    members: &[usize],
    assignment: &[Option<usize>],
    parsed: &[ParsedName],
    ranked: &[Vec<Hyp>],
) -> usize {
    // (release, written season, written number, record, episode)
    let mut placed: Vec<(Option<&str>, Option<u32>, u32, usize, EpisodeRef)> = Vec::new();
    for (&i, a) in members.iter().zip(assignment) {
        let Some(j) = a else { continue };
        let h = &ranked[i][*j];
        let numbers = Numbers::of(&parsed[i]);
        let (Some(first), Some(&n)) = (h.episodes.first(), numbers.values.first()) else {
            continue;
        };
        placed.push((
            parsed[i].release_group.as_deref(),
            numbers.season,
            n,
            h.record,
            *first,
        ));
    }
    placed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut count = 0;
    for w in placed.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if a.0 != b.0 || a.1 != b.1 {
            continue;
        }
        if a.3 == b.3 && a.4.season == b.4.season && a.4.episode == b.4.episode {
            count += 1;
            continue;
        }
        let d = b.2.saturating_sub(a.2);
        if d == 0 || d > NEAR {
            continue;
        }
        let ok = if a.3 == b.3 && a.4.season == b.4.season {
            b.4.episode == a.4.episode + d
        } else if let (Some(x), Some(y)) = (a.4.absolute, b.4.absolute) {
            y == x + d
        } else {
            // A new entry starts within the gap.
            b.4.episode <= d
        };
        if !ok {
            count += 1;
        }
    }
    count
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// Extras (`Extras/Trailer 2.mkv`, `sample.mkv`) and files without a title
/// take the record of the nearest enclosing folder whose confident main
/// files all agree. A main file never inherits: `The Lion King.mkv` next to
/// `The Lion King (1994).mkv` may well be the remake.
fn inherit_from_folder(files: &[GroupFile<'_>], parsed: &[ParsedName], out: &mut [FileMatch]) {
    const CONFIDENT: f32 = 0.6;
    let own: Vec<Option<(usize, f32)>> = out
        .iter()
        .zip(parsed)
        .map(|(m, p)| {
            m.record
                .filter(|_| p.extra.is_none() && m.confidence >= CONFIDENT)
                .map(|r| (r, m.confidence))
        })
        .collect();
    for (i, file) in files.iter().enumerate() {
        let extra = parsed[i].extra.is_some();
        if !(extra || out[i].record.is_none()) {
            continue;
        }
        let mut dir = parent(file.path);
        loop {
            let prefix = format!("{dir}/");
            let mut found: Option<(usize, f32)> = None;
            let mut conflict = false;
            for (j, other) in files.iter().enumerate() {
                if j == i || !(dir.is_empty() || other.path.starts_with(&prefix)) {
                    continue;
                }
                if let Some((r, c)) = own[j] {
                    match found {
                        Some((fr, fc)) if fr == r => found = Some((r, fc.min(c))),
                        Some(_) => conflict = true,
                        None => found = Some((r, c)),
                    }
                }
            }
            if conflict {
                break;
            }
            if let Some((r, c)) = found {
                let m = &mut out[i];
                m.record = Some(r);
                m.confidence = c * 0.95;
                m.episodes.clear();
                m.signals.reading = Reading::Inherited;
                break;
            }
            if dir.is_empty() {
                break;
            }
            dir = parent(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anime(aid: u32, title: &str, year: u16, episodes: u32, synonyms: &[&str]) -> TitleRecord {
        let mut titles = vec![title.to_owned()];
        titles.extend(synonyms.iter().map(|s| (*s).to_owned()));
        TitleRecord {
            ids: [("anidb".to_owned(), aid.to_string())].into(),
            kind: WorkKind::Series,
            anime: true,
            year: Some(year),
            episodes: Some(episodes),
            titles,
            ..Default::default()
        }
    }

    fn movie(tmdb: u32, title: &str, year: u16) -> TitleRecord {
        TitleRecord {
            ids: [("tmdb".to_owned(), format!("movie:{tmdb}"))].into(),
            kind: WorkKind::Movie,
            year: Some(year),
            titles: vec![title.to_owned()],
            ..Default::default()
        }
    }

    fn index() -> TitleIndex {
        TitleIndex::from_records([
            anime(
                11739,
                "Boku no Hero Academia",
                2016,
                13,
                &["My Hero Academia"],
            ),
            anime(
                12233,
                "Boku no Hero Academia 2nd Season",
                2017,
                25,
                &["Boku no Hero Academia (2017)"],
            ),
            anime(
                13485,
                "Boku no Hero Academia 3rd Season",
                2018,
                25,
                &["Boku no Hero Academia (2018)"],
            ),
            anime(5841, "Clannad: After Story", 2008, 24, &[]),
            anime(5000, "Clannad", 2007, 23, &[]),
            anime(3468, "Gintama", 2006, 201, &["Gintama: Season 1"]),
            anime(
                8126,
                "Gintama'",
                2011,
                51,
                &["Gintama (Episodes 202-252)", "Gintama Season 2"],
            ),
            anime(
                16803,
                "Kimetsu no Yaiba: Mugen Ressha-hen",
                2021,
                7,
                &["Demon Slayer: Kimetsu no Yaiba Season 2"],
            ),
            anime(
                14107,
                "Kimetsu no Yaiba",
                2019,
                26,
                &["Demon Slayer", "Demon Slayer: Kimetsu no Yaiba"],
            ),
            anime(
                16054,
                "Kimetsu no Yaiba: Yuukaku-hen",
                2022,
                11,
                &["Demon Slayer Season 2"],
            ),
            anime(34, "Fruits Basket", 2001, 26, &["Fruits Basket (2001)"]),
            anime(
                14490,
                "Fruits Basket 1st Season",
                2019,
                25,
                &["Fruits Basket (2019)"],
            ),
            movie(8587, "The Lion King", 1994),
            movie(420818, "The Lion King", 2019),
            movie(603, "The Matrix", 1999),
        ])
    }

    fn one(index: &TitleIndex, path: &str) -> FileMatch {
        score_group(
            index,
            &[GroupFile {
                path,
                duration_ms: None,
                ..Default::default()
            }],
        )
        .remove(0)
    }

    fn aid(index: &TitleIndex, m: &FileMatch) -> String {
        index.record(m.record.expect("decided")).ids["anidb"].clone()
    }

    #[test]
    fn absolute_number_runs_into_later_season() {
        let ix = index();
        let m = one(
            &ix,
            "Anime/Boku no Hero Academia/[Commie] Boku no Hero Academia - 35 (1080p).mkv",
        );
        assert_eq!(aid(&ix, &m), "12233");
        assert_eq!(
            m.episodes,
            vec![EpisodeRef {
                season: None,
                episode: 22,
                absolute: Some(35)
            }]
        );
        assert_eq!(m.signals.reading, Reading::Absolute);
        assert!(m.confidence >= 0.9, "{m:?}");
    }

    #[test]
    fn small_number_stays_in_first_season() {
        let ix = index();
        let m = one(
            &ix,
            "Anime/Boku no Hero Academia/[Commie] Boku no Hero Academia - 05 (1080p).mkv",
        );
        assert_eq!(aid(&ix, &m), "11739");
        assert_eq!(m.episodes[0].episode, 5);
    }

    #[test]
    fn explicit_season_picks_entry() {
        let ix = index();
        let m = one(
            &ix,
            "TV/Boku no Hero Academia/Season 03/Boku no Hero Academia S03E04.mkv",
        );
        assert_eq!(aid(&ix, &m), "13485");
        assert_eq!(m.episodes[0].episode, 4);
    }

    #[test]
    fn season_overflows_into_next_entry() {
        let ix = index();
        let m = one(
            &ix,
            "TV/Demon Slayer/Season 02/Demon.Slayer.S02E12.1080p.WEB.mkv",
        );
        assert_eq!(aid(&ix, &m), "16054");
        assert_eq!(m.episodes[0].episode, 5);
    }

    #[test]
    fn written_range_wins() {
        let ix = index();
        let m = one(&ix, "Anime/Gintama/[FFF] Gintama - 210 [720p].mkv");
        assert_eq!(aid(&ix, &m), "8126");
        assert_eq!(m.episodes[0].episode, 9);
    }

    #[test]
    fn literal_title_breaks_folded_tie() {
        let ix = index();
        let m = one(&ix, "Anime/Gintama'/Gintama' - 31 [1080p].mkv");
        assert_eq!(aid(&ix, &m), "8126");
        let m = one(&ix, "Anime/Gintama/Gintama - 31 [1080p].mkv");
        assert_eq!(aid(&ix, &m), "3468");
    }

    #[test]
    fn sequel_title_is_its_own_entry() {
        let ix = index();
        let m = one(
            &ix,
            "Anime/Clannad - After Story/[MTBB] Clannad - After Story - 03 [1080p].mkv",
        );
        assert_eq!(aid(&ix, &m), "5841");
    }

    #[test]
    fn remakes_are_not_auto() {
        let ix = index();
        let m = one(&ix, "Movies/The Lion King/The Lion King.mkv");
        assert!(m.confidence < 0.5, "{m:?}");
        let m = one(&ix, "Movies/The Lion King (1994)/The Lion King (1994).mkv");
        assert_eq!(ix.record(m.record.unwrap()).ids["tmdb"], "movie:8587");
        assert!(m.confidence >= 0.9, "{m:?}");
        let m = one(&ix, "Anime/Fruits Basket/Fruits Basket - 05 [720p].mkv");
        assert!(m.signals.remake && m.confidence < 0.9, "{m:?}");
    }

    #[test]
    fn extra_inherits_folder_decision() {
        let ix = index();
        let files = [
            GroupFile {
                path: "Movies/The Lion King (2019)/The Lion King (2019).mkv",
                duration_ms: None,
                ..Default::default()
            },
            GroupFile {
                path: "Movies/The Lion King (2019)/Extras/Trailer 2.mkv",
                duration_ms: None,
                ..Default::default()
            },
        ];
        let out = score_group(&ix, &files);
        assert_eq!(
            ix.record(out[0].record.unwrap()).ids["tmdb"],
            "movie:420818"
        );
        assert_eq!(out[1].record, out[0].record);
        assert_eq!(out[1].signals.reading, Reading::Inherited);
    }

    #[test]
    fn range_parsing() {
        assert_eq!(
            episode_range("Gintama (Episodes 202-252)"),
            Some((202, 252))
        );
        assert_eq!(
            episode_range("Gintama (Episoden 354-367)"),
            Some((354, 367))
        );
        assert_eq!(episode_range("Gintama (2011)"), None);
        assert!(shares_prefix(
            "kimetsu no yaiba",
            "kimetsu no yaiba yuukaku hen"
        ));
        assert!(!shares_prefix("boku no hero academia", "boku no pico"));
        assert!(shares_prefix("gintama", "gintama porori hen"));
        assert!(!shares_prefix("dragon ball super", "dragon ball"));
    }

    fn with_runtime(mut r: TitleRecord, minutes: u32) -> TitleRecord {
        r.runtime_min = Some(minutes);
        r
    }

    fn series(tmdb: u32, title: &str, year: u16, seasons: &[(u32, u32)]) -> TitleRecord {
        TitleRecord {
            ids: [("tmdb".to_owned(), format!("tv:{tmdb}"))].into(),
            kind: WorkKind::Series,
            year: Some(year),
            seasons: seasons.iter().copied().collect(),
            titles: vec![title.to_owned()],
            ..Default::default()
        }
    }

    fn tmdb(index: &TitleIndex, m: &FileMatch) -> String {
        index.record(m.record.expect("decided")).ids["tmdb"].clone()
    }

    #[test]
    fn duration_tells_remakes_apart() {
        let ix = TitleIndex::from_records([
            with_runtime(movie(8587, "The Lion King", 1994), 88),
            with_runtime(movie(420818, "The Lion King", 2019), 118),
        ]);
        let file = |minutes: u64| GroupFile {
            path: "Movies/The Lion King/The Lion King.mkv",
            duration_ms: Some(minutes * 60_000 + 25_000),
            ..Default::default()
        };
        let m = score_group(&ix, &[file(118)]).remove(0);
        assert_eq!(tmdb(&ix, &m), "movie:420818");
        assert!(m.confidence >= 0.92, "{m:?}");
        assert!(m.signals.pair.duration_error.is_some_and(|e| e < 0.01));
        let m = score_group(&ix, &[file(88)]).remove(0);
        assert_eq!(tmdb(&ix, &m), "movie:8587");
        assert!(m.confidence >= 0.92, "{m:?}");
        // A trailer is not compared with the runtime.
        let trailer = GroupFile {
            path: "Movies/The Lion King/The Lion King.mkv",
            duration_ms: Some(150_000),
            ..Default::default()
        };
        assert!(score_group(&ix, &[trailer])[0].confidence < 0.5);
    }

    #[test]
    fn season_list_and_siblings_decide_a_same_name_series() {
        let ix = TitleIndex::from_records([
            series(1906, "Shameless", 2004, &[(8, 12), (9, 10)]),
            series(34307, "Shameless", 2011, &[(8, 12), (9, 14)]),
        ]);
        let f = |path| GroupFile {
            path,
            ..Default::default()
        };
        // On its own `S09E02` fits both series.
        let alone = score_group(&ix, &[f("TV/Shameless/Season 09/Shameless.S09E02.mkv")]);
        assert!(alone[0].confidence < 0.5, "{:?}", alone[0]);
        // `S09E14` exists only in the 2011 series, and in one show folder
        // the files are one series: both are decided.
        let out = score_group(
            &ix,
            &[
                f("TV/Shameless/Season 09/Shameless.S09E02.mkv"),
                f("TV/Shameless/Season 09/Shameless.S09E14.mkv"),
            ],
        );
        for m in &out {
            assert_eq!(tmdb(&ix, m), "tv:34307");
            assert!(m.confidence >= 0.92, "{m:?}");
        }
        assert!(out[0].signals.joint);
        assert_eq!(out[1].signals.pair.episodes_bad, 0);
    }

    #[test]
    fn user_rules_and_hash_hits_are_decisive() {
        let ix = index();
        let file = GroupFile {
            path: "Movies/The Lion King/The Lion King.mkv",
            ..Default::default()
        };
        let rule = Hints {
            rules: vec![UserRule {
                prefix: "Movies/The Lion King/".into(),
                ids: [("tmdb".to_owned(), "movie:8587".to_owned())].into(),
            }],
            ..Default::default()
        };
        let m = Scorer::new()
            .score_group_with(&ix, &[file], &rule)
            .remove(0);
        assert_eq!(tmdb(&ix, &m), "movie:8587");
        assert!(m.signals.hinted && m.confidence >= 0.92, "{m:?}");
        // A rule for another folder does not apply.
        let other = Hints {
            rules: vec![UserRule {
                prefix: "Movies/The Lion".into(),
                ids: [("tmdb".to_owned(), "movie:8587".to_owned())].into(),
            }],
            ..Default::default()
        };
        let m = Scorer::new()
            .score_group_with(&ix, &[file], &other)
            .remove(0);
        assert!(!m.signals.hinted && m.confidence < 0.5, "{m:?}");
        // A confirmed hash names a work the title did not retrieve.
        let hashes = ["8e245d9679d31e12".to_owned()];
        let hashed = GroupFile {
            path: "Movies/Unknown/video.mkv",
            hashes: &hashes,
            ..Default::default()
        };
        let hit = Hints {
            hashes: [(
                "8e245d9679d31e12".to_owned(),
                [("tmdb".to_owned(), "movie:603".to_owned())].into(),
            )]
            .into(),
            ..Default::default()
        };
        let m = Scorer::new()
            .score_group_with(&ix, &[hashed], &hit)
            .remove(0);
        assert_eq!(tmdb(&ix, &m), "movie:603");
        assert!(m.signals.hinted && m.confidence >= 0.92, "{m:?}");
    }

    #[test]
    fn consecutive_numbers_stay_consecutive() {
        // `- 12`, `- 13`, `- 14` of Boku no Hero Academia: S1 has 13
        // episodes, so 14 is S2 ep. 1 — the group keeps one run without
        // duplicates or gaps.
        let ix = index();
        let names = ["12", "13", "14"].map(|n| {
            format!("Anime/Boku no Hero Academia/[Commie] Boku no Hero Academia - {n} (1080p).mkv")
        });
        let files: Vec<GroupFile<'_>> = names
            .iter()
            .map(|p| GroupFile {
                path: p,
                ..Default::default()
            })
            .collect();
        let out = score_group(&ix, &files);
        let got: Vec<(String, u32)> = out
            .iter()
            .map(|m| (aid(&ix, m), m.episodes[0].episode))
            .collect();
        assert_eq!(
            got,
            [
                ("11739".to_owned(), 12),
                ("11739".to_owned(), 13),
                ("12233".to_owned(), 1)
            ]
        );
        assert!(out.iter().all(|m| m.signals.joint));
    }

    #[test]
    fn season_folders_join_their_show() {
        assert_eq!(series_root("TV/Shameless/Season 09"), "TV/Shameless");
        assert_eq!(
            series_root("Сериалы/Бесстыжие/Сезон 9"),
            "Сериалы/Бесстыжие"
        );
        assert_eq!(series_root("TV/Shameless/S02"), "TV/Shameless");
        assert_eq!(series_root("TV/Shameless"), "TV/Shameless");
        assert_eq!(
            series_root("Anime/Boku no Hero Academia"),
            "Anime/Boku no Hero Academia"
        );
        assert_eq!(series_root("TV/Seasons"), "TV/Seasons");
    }
}
