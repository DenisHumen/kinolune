// SPDX-License-Identifier: MPL-2.0
//! Folder rules and release templates learned from the user's fixes
//! (META-017, research 03 §4.6, ADR-0112).
//!
//! Every fix of a recognition — another result in the review inbox
//! ([`Catalog::review_choose`]), «Верно» ([`Catalog::review_confirm`]),
//! «Исправить распознавание» ([`Catalog::fix_match`]), the episode
//! numbering ([`Catalog::set_numbering`]) or the type
//! ([`Catalog::set_work_kind`]) — teaches the folder of the work:
//!
//! - **the rule** (`folder_rules`): the deepest folder holding the work's
//!   files in a source → the work, its type, the numbering scheme, the
//!   season and the episode offset;
//! - **release templates** (`folder_rules.templates_json`): the fixed file
//!   names with the episode number cut out (`grp sousou no frieren {n}
//!   1080p {crc}`), each with where its number lands (season, offset,
//!   absolute count) as the fixed files show it now;
//! - **the spread**: every file of the folder the rule covers is decided
//!   again by the rule at once (the whole group, not only the work that
//!   was fixed), and every new file of the folder is decided by it in the
//!   scan ([`crate::recognize`]) — locally, without a provider.
//!
//! A rule covers a file under its folder that matches one of its templates
//! or whose parsed title is one of the work's titles (a movie also by its
//! year), so a rule on a shared folder never takes another show's files.
//! Files decided by NFO ids or by the user for another work are never
//! touched. Rule matches are `user_set` with `decided_by = 'rule'`: no
//! scan, parser or scorer decides them again (`numbering::recomputable!`).
//!
//! **Sync** (`folder_rule.set`): a rule is keyed by `<source sync id>:<hash
//! of the folder>` — never a path; another device of the family finds the
//! folder among its own folders of the same source by the hash. The rule
//! names the work by its provider ids (and title, type and year to create
//! it where it is not known yet). Last writer wins by the op stamp
//! (`folder_rule_sync`). A rule whose folder is not scanned there yet waits
//! and is applied by the next scan. Only rules of sources that are synced
//! (SYNC-010) travel.
//!
//! **Removal**: fixing again replaces a rule; [`Catalog::forget_folder_rules`]
//! removes the rules of a work (the decisions already made stay; new files
//! are recognised as before) and tells the family.

use std::collections::{BTreeMap, HashMap};

use pa_library::groups::Group;
use pa_library::rusqlite::{Connection, OptionalExtension, Transaction, params};
use pa_recognition::title::TitleKey;
use pa_sync::{Hlc, Op, OpBody, OpId, Stamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::items::ItemRef;
use crate::recognize::{EpisodeNo, FilePlan, PlanRole, Planner, WorkIndex, WorkPlan};
use crate::{Catalog, Error};

/// Release templates kept per rule (the most frequent ones).
pub const MAX_TEMPLATES: usize = 8;

/// `matches.explanation_json` key of a match a folder rule decided.
const EXPLANATION_KEY: &str = "folder_rule";

/// Where the number of a file lands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Target {
    /// The number is a running count (`absolute_number`), not an episode
    /// of a season.
    #[serde(default)]
    pub absolute: bool,
    /// Season of the episode. `None`: the season written in the name
    /// (`S02E05`), else season 1 (none for a running count).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<i64>,
    /// Added to the number in the name.
    #[serde(default)]
    pub offset: i64,
    /// For a season episode: its running count is the number in the name
    /// plus this (as the fixed files had it), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abs_offset: Option<i64>,
}

impl Target {
    /// `matches.scheme_used` / `folder_rules.scheme` of the target.
    #[must_use]
    pub fn scheme(&self) -> &'static str {
        if self.absolute {
            "tvdb_absolute"
        } else {
            "tmdb"
        }
    }

    /// The episode of number `n` (season `written` in the name).
    fn land(&self, n: i64, written: Option<i64>, kind: &'static str) -> Option<EpisodeNo> {
        let number = n.checked_add(self.offset).filter(|x| *x >= 0)?;
        Some(if self.absolute {
            EpisodeNo {
                season: self.season,
                episode: None,
                absolute: Some(number),
                kind,
                date: None,
            }
        } else {
            EpisodeNo {
                season: self.season.or(written).or(Some(1)),
                episode: Some(number),
                absolute: self
                    .abs_offset
                    .and_then(|a| n.checked_add(a))
                    .filter(|x| *x > 0),
                kind,
                date: None,
            }
        })
    }
}

/// A file name with the episode number cut out, and where that number
/// lands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseTemplate {
    /// Lowercase name tokens: words as written, `{n}` the episode number,
    /// `s{s}e{n}` a season and episode, `{crc}` a CRC32.
    pub pattern: String,
    #[serde(flatten)]
    pub target: Target,
}

/// A rule as the app shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRuleInfo {
    pub id: i64,
    pub source_id: i64,
    /// Folder relative to the source root, without slashes; `""` the whole
    /// source.
    pub folder: String,
    pub work_id: i64,
    /// `movie`, `show` or `anime` if the user set the type.
    pub kind: Option<String>,
    /// `tmdb` (season episodes) or `tvdb_absolute` (running count).
    pub scheme: Option<String>,
    pub season: Option<i64>,
    pub offset: i64,
    pub templates: Vec<ReleaseTemplate>,
}

fn json<T: Serialize + ?Sized>(value: &T) -> Result<String, Error> {
    serde_json::to_string(value).map_err(|e| Error::Invalid(format!("folder rule: {e}")))
}

// ---------------------------------------------------------------------------
// Release templates

const NUMBER: &str = "{n}";
const SEASON_NUMBER: &str = "s{s}e{n}";
const CRC: &str = "{crc}";

fn file_stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// Lowercase alphanumeric runs of a file name.
fn tokens(path: &str) -> Vec<String> {
    file_stem(path)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn is_crc(t: &str) -> bool {
    t.len() == 8
        && t.chars().all(|c| c.is_ascii_hexdigit())
        && t.chars().any(|c| c.is_ascii_digit())
        && t.chars().any(|c| c.is_ascii_alphabetic())
}

/// An episode number token: `13`, `13v2`.
fn number_of(t: &str) -> Option<i64> {
    let digits = t.split_once('v').map_or(t, |(n, v)| {
        if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) {
            n
        } else {
            t
        }
    });
    (!digits.is_empty() && digits.len() <= 4 && digits.chars().all(|c| c.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

/// `s02e05` → `(2, 5)`.
fn season_episode_of(t: &str) -> Option<(i64, i64)> {
    let rest = t.strip_prefix('s')?;
    let (s, e) = rest.split_once('e')?;
    let ok = |x: &str| !x.is_empty() && x.len() <= 4 && x.chars().all(|c| c.is_ascii_digit());
    (ok(s) && ok(e)).then(|| Some((s.parse().ok()?, number_of(e)?)))?
}

/// The template of `path` whose episode number is `n`, and the season
/// written next to it, if any.
fn learn_pattern(path: &str, n: i64) -> Option<(String, Option<i64>)> {
    let mut toks = tokens(path);
    let at = toks.iter().rposition(|t| {
        number_of(t) == Some(n) || season_episode_of(t).is_some_and(|(_, e)| e == n)
    })?;
    let written = season_episode_of(&toks[at]).map(|(s, _)| s);
    toks[at] = if written.is_some() {
        SEASON_NUMBER
    } else {
        NUMBER
    }
    .to_owned();
    for (i, t) in toks.iter_mut().enumerate() {
        if i != at && is_crc(t) {
            *t = CRC.to_owned();
        }
    }
    Some((toks.join(" "), written))
}

/// The number (and written season) of `path` if it is named like
/// `pattern`.
fn match_pattern(pattern: &str, path: &str) -> Option<(i64, Option<i64>)> {
    let toks = tokens(path);
    let pat: Vec<&str> = pattern.split(' ').collect();
    if pat.len() != toks.len() {
        return None;
    }
    let mut found = None;
    for (p, t) in pat.iter().zip(&toks) {
        match *p {
            NUMBER => found = Some((number_of(t)?, None)),
            SEASON_NUMBER => {
                let (s, e) = season_episode_of(t)?;
                found = Some((e, Some(s)));
            }
            CRC => {
                if !is_crc(t) {
                    return None;
                }
            }
            lit => {
                if lit != t {
                    return None;
                }
            }
        }
    }
    found
}

/// The number written in the name of `path`, as the parser reads it.
fn parsed_number(path: &str) -> Option<i64> {
    let parsed = pa_recognition::parse::parse(path);
    let p = &parsed.best().parsed;
    p.episodes
        .iter()
        .chain(&p.absolute)
        .find(|e| e.fraction.is_none())
        .map(|e| i64::from(e.number))
}

// ---------------------------------------------------------------------------
// Rules of a source and their decisions

/// `path` (with or without a leading slash) is under folder `prefix`.
fn under(path: &str, prefix: &str) -> bool {
    let p = path.trim_start_matches('/');
    prefix.is_empty()
        || p == prefix
        || (p.starts_with(prefix) && p[prefix.len()..].starts_with('/'))
}

fn loose(title: &str) -> String {
    let key = TitleKey::new(title);
    if key.loose.is_empty() {
        key.plain
    } else {
        key.loose
    }
}

#[derive(Debug, Clone)]
struct Rule {
    id: i64,
    prefix: String,
    work: i64,
    movie: bool,
    year: Option<i64>,
    target: Option<Target>,
    templates: Vec<ReleaseTemplate>,
    /// Loose keys of the work's titles.
    keys: Vec<String>,
}

/// What a rule decided for one file.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Decision {
    pub rule: i64,
    pub work: i64,
    pub role: PlanRole,
    pub episodes: Vec<EpisodeNo>,
    pub scheme: Option<&'static str>,
}

impl Rule {
    fn covers(&self, plan: &FilePlan) -> bool {
        if plan.role == PlanRole::Extra {
            return false;
        }
        // The title the file is named by; alternative titles may be the
        // folder's name, which any file of the folder shares.
        let titled = self.keys.contains(&loose(&plan.work.title));
        let year_ok = !(self.movie && !plan.work.is_series())
            || match (self.year, plan.work.year) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            };
        titled && year_ok
    }

    fn decide(&self, path: &str, plan: Option<&FilePlan>) -> Option<Decision> {
        if !under(path, &self.prefix) {
            return None;
        }
        let templated = self
            .templates
            .iter()
            .find_map(|t| match_pattern(&t.pattern, path).map(|m| (t, m)));
        if templated.is_none() && !plan.is_some_and(|p| self.covers(p)) {
            return None;
        }
        if plan.is_some_and(|p| p.role == PlanRole::Extra) {
            return None;
        }
        let role = plan.map_or(PlanRole::Main, |p| p.role);
        if self.movie {
            return Some(Decision {
                rule: self.id,
                work: self.work,
                role,
                episodes: Vec::new(),
                scheme: None,
            });
        }
        let kind = plan
            .and_then(|p| p.episodes.first())
            .map_or("regular", |e| e.kind);
        let numbered: Vec<&EpisodeNo> = plan
            .map(|p| {
                p.episodes
                    .iter()
                    .filter(|e| e.episode.or(e.absolute).is_some())
                    .collect()
            })
            .unwrap_or_default();
        let (episodes, scheme) = match templated {
            Some((t, (n, written))) => {
                let eps: Option<Vec<EpisodeNo>> = if numbered.len() > 1 {
                    numbered
                        .iter()
                        .map(|e| t.target.land(e.episode.or(e.absolute)?, written, e.kind))
                        .collect()
                } else {
                    t.target.land(n, written, kind).map(|e| vec![e])
                };
                (eps?, Some(t.target.scheme()))
            }
            None => {
                let plan = plan?;
                match self.target {
                    Some(t) => {
                        let mut out = Vec::with_capacity(plan.episodes.len());
                        for e in &plan.episodes {
                            match e.episode.or(e.absolute) {
                                Some(n) => out.push(t.land(n, e.season, e.kind)?),
                                None => out.push(*e),
                            }
                        }
                        (out, Some(t.scheme()))
                    }
                    None => (plan.episodes.clone(), None),
                }
            }
        };
        let episodes = if episodes.is_empty() {
            // A file the parser read as a movie under a series rule (the
            // user set the type): an unnumbered episode of its own.
            vec![EpisodeNo {
                season: self.target.and_then(|t| t.season),
                episode: None,
                absolute: None,
                kind: "regular",
                date: None,
            }]
        } else {
            episodes
        };
        Some(Decision {
            rule: self.id,
            work: self.work,
            role,
            episodes,
            scheme,
        })
    }
}

/// The rules of one source, the deepest folder first.
#[derive(Debug, Default)]
pub(crate) struct Rules {
    rules: Vec<Rule>,
}

impl Rules {
    pub(crate) fn load(conn: &Connection, source_id: i64) -> Result<Self, Error> {
        let mut stmt = conn.prepare_cached(
            "SELECT fr.id, trim(fr.path_prefix, '/'), fr.work_id, w.kind, w.year, fr.scheme,
                    fr.season_override, fr.episode_offset, fr.templates_json
             FROM folder_rules fr JOIN works w ON w.id = fr.work_id
             WHERE fr.source_id = ?1
             ORDER BY length(trim(fr.path_prefix, '/')) DESC, fr.id",
        )?;
        let mut rules: Vec<Rule> = stmt
            .query_map([source_id], |r| {
                let scheme: Option<String> = r.get(5)?;
                let season: Option<i64> = r.get(6)?;
                let offset: i64 = r.get(7)?;
                Ok(Rule {
                    id: r.get(0)?,
                    prefix: r.get(1)?,
                    work: r.get(2)?,
                    movie: r.get::<_, String>(3)? == "movie",
                    year: r.get(4)?,
                    target: scheme.map(|s| Target {
                        absolute: s == "tvdb_absolute",
                        season,
                        offset,
                        abs_offset: None,
                    }),
                    templates: serde_json::from_str(&r.get::<_, String>(8)?).unwrap_or_default(),
                    keys: Vec::new(),
                })
            })?
            .collect::<Result<_, _>>()?;
        let mut titles = conn.prepare_cached(
            "SELECT text FROM titles WHERE work_id = ?1
             UNION SELECT title FROM works WHERE id = ?1",
        )?;
        for rule in &mut rules {
            let mut keys: Vec<String> = titles
                .query_map([rule.work], |r| r.get::<_, String>(0))?
                .map(|t| t.map(|t| loose(&t)))
                .collect::<Result<_, _>>()?;
            keys.retain(|k| !k.is_empty());
            keys.sort_unstable();
            keys.dedup();
            rule.keys = keys;
        }
        Ok(Self { rules })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The decision of the deepest rule that covers the file at `path`.
    pub(crate) fn decide(&self, path: &str, plan: Option<&FilePlan>) -> Option<Decision> {
        self.rules.iter().find_map(|r| r.decide(path, plan))
    }
}

/// Writes the match of a rule decision for `file_id` (which has no live
/// match). Returns whether a match was written.
pub(crate) fn write_decision(
    tx: &Transaction<'_>,
    file_id: i64,
    d: &Decision,
    model_version: Option<&str>,
    now: i64,
) -> Result<bool, Error> {
    let mut eps = Vec::with_capacity(d.episodes.len());
    for no in &d.episodes {
        eps.push(crate::recognize::ensure_episode(tx, d.work, *no, None)?);
    }
    let written = crate::recognize::insert_match(
        tx,
        file_id,
        d.work,
        &eps,
        1.0,
        "user_set",
        "rule",
        model_version,
        now,
    )?;
    if written {
        tx.execute(
            "UPDATE matches SET scheme_used = ?2, explanation_json = ?3
             WHERE file_id = ?1 AND state <> 'rejected'",
            params![
                file_id,
                d.scheme,
                serde_json::json!({ EXPLANATION_KEY: d.rule }).to_string()
            ],
        )?;
    }
    Ok(written)
}

// ---------------------------------------------------------------------------
// Learning from a fix

/// Folder of every file of `work`, per source: the deepest folder holding
/// all its live files there.
fn work_folders(tx: &Transaction<'_>, work: i64) -> Result<Vec<(i64, String, i64)>, Error> {
    let rows: Vec<(i64, String, i64)> = {
        let mut stmt = tx.prepare(concat!(
            "SELECT f.source_id, f.path, m.id FROM matches m JOIN files f ON f.id = m.file_id
             WHERE m.work_id = ?1 AND m.state <> 'rejected' AND f.missing_since_ms IS NULL
               AND NOT ",
            pa_library::file_personal!("f"),
            " ORDER BY f.source_id, m.id"
        ))?;
        stmt.query_map([work], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?
    };
    let mut out: Vec<(i64, String, i64)> = Vec::new();
    for (source, path, match_id) in rows {
        let dir = path
            .trim_start_matches('/')
            .rsplit_once('/')
            .map_or("", |(d, _)| d)
            .to_owned();
        match out.iter_mut().find(|(s, _, _)| *s == source) {
            Some((_, common, _)) => {
                while !under(&dir, common) {
                    *common = common.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
                }
            }
            None => out.push((source, dir, match_id)),
        }
    }
    Ok(out)
}

/// What the fixed files of `work` under `prefix` of `source` say: one
/// template per naming, with the place its number lands most often.
fn learn_templates(
    tx: &Transaction<'_>,
    source: i64,
    prefix: &str,
    work: i64,
) -> Result<Vec<ReleaseTemplate>, Error> {
    /// A fixed file of one episode: its path and the episode's season,
    /// number and running count.
    type Fixed = (String, [Option<i64>; 3]);
    let rows: Vec<Fixed> = {
        let mut stmt = tx.prepare(
            "SELECT f.path, e.season_number, e.episode_number, e.absolute_number
             FROM matches m JOIN files f ON f.id = m.file_id
             JOIN match_episodes me ON me.match_id = m.id
             JOIN episodes e ON e.id = me.episode_id
             WHERE m.work_id = ?1 AND f.source_id = ?2 AND m.state <> 'rejected'
               AND f.missing_since_ms IS NULL AND e.episode_type = 'regular'
               AND (SELECT count(*) FROM match_episodes x WHERE x.match_id = m.id) = 1
             ORDER BY f.path",
        )?;
        stmt.query_map(params![work, source], |r| {
            Ok((r.get(0)?, [r.get(1)?, r.get(2)?, r.get(3)?]))
        })?
        .collect::<Result<_, _>>()?
    };
    // pattern → (files, first seen, target → count)
    let mut seen: BTreeMap<String, (usize, usize, HashMap<Target, usize>)> = BTreeMap::new();
    for (i, (path, [season, episode, absolute])) in rows.into_iter().enumerate() {
        if !under(&path, prefix) {
            continue;
        }
        let Some(n) = parsed_number(&path) else {
            continue;
        };
        let Some((pattern, written)) = learn_pattern(&path, n) else {
            continue;
        };
        let target = match (episode, absolute) {
            (Some(e), _) => Target {
                absolute: false,
                season: if written.is_some() && written == season {
                    None
                } else {
                    season
                },
                offset: e - n,
                abs_offset: absolute.map(|a| a - n),
            },
            (None, Some(a)) => Target {
                absolute: true,
                season,
                offset: a - n,
                abs_offset: None,
            },
            (None, None) => continue,
        };
        let entry = seen.entry(pattern).or_insert((0, i, HashMap::new()));
        entry.0 += 1;
        *entry.2.entry(target).or_default() += 1;
    }
    let mut out: Vec<(usize, usize, ReleaseTemplate)> = seen
        .into_iter()
        .map(|(pattern, (files, first, targets))| {
            let target = targets
                .into_iter()
                .max_by(|a, b| {
                    a.1.cmp(&b.1).then_with(|| {
                        (b.0.offset.abs(), b.0.season).cmp(&(a.0.offset.abs(), a.0.season))
                    })
                })
                .map(|(t, _)| t)
                .unwrap_or_default();
            (files, first, ReleaseTemplate { pattern, target })
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    out.truncate(MAX_TEMPLATES);
    Ok(out.into_iter().map(|(_, _, t)| t).collect())
}

/// How a fix changes the rules of a work besides what its files say.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Teach {
    /// The numbering the user set: every template and the rule take it.
    pub numbering: Option<Target>,
    /// The user set the type of the work.
    pub kind_set: bool,
}

/// Creates or updates the rules of `work` from its files and decides the
/// files of their folders again. Returns the ids of the rules.
pub(crate) fn teach(
    tx: &Transaction<'_>,
    work: i64,
    how: Teach,
    now: i64,
) -> Result<Vec<i64>, Error> {
    let kind: Option<String> = tx
        .query_row("SELECT kind FROM works WHERE id = ?1", [work], |r| r.get(0))
        .optional()?;
    let Some(kind) = kind else {
        return Ok(Vec::new());
    };
    let mut ids = Vec::new();
    for (source, prefix, match_id) in work_folders(tx, work)? {
        let other: Option<i64> = tx
            .query_row(
                "SELECT work_id FROM folder_rules
                 WHERE source_id = ?1 AND trim(path_prefix, '/') = ?2",
                params![source, prefix],
                |r| r.get(0),
            )
            .optional()?;
        if other.is_some_and(|w| w != work) {
            // The folder already teaches another work (loose files of two
            // fixed shows in one folder): the first rule stays; this
            // work's files keep their decisions and new ones join it by
            // title.
            tracing::info!(
                target: "pa_catalog::folder_rules",
                work, source, "folder already has a rule of another work"
            );
            continue;
        }
        let templates = if kind == "movie" {
            Vec::new()
        } else {
            let mut t = learn_templates(tx, source, &prefix, work)?;
            if let Some(n) = how.numbering {
                for x in &mut t {
                    x.target = n;
                }
            }
            t
        };
        let target = how
            .numbering
            .or_else(|| templates.first().map(|t| t.target))
            .filter(|_| kind != "movie");
        let kind_override = how.kind_set.then_some(kind.as_str());
        tx.execute(
            "INSERT INTO folder_rules (source_id, path_prefix, work_id, scheme, episode_offset,
                                       season_override, kind_override, created_from_match_id,
                                       created_at_ms, templates_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (source_id, path_prefix) DO UPDATE SET
                 work_id = excluded.work_id, scheme = excluded.scheme,
                 episode_offset = excluded.episode_offset,
                 season_override = excluded.season_override,
                 kind_override = ifnull(excluded.kind_override, folder_rules.kind_override),
                 templates_json = excluded.templates_json",
            params![
                source,
                prefix,
                work,
                target.map(|t| t.scheme()),
                target.map_or(0, |t| t.offset),
                target.and_then(|t| t.season).filter(|s| *s >= 0),
                kind_override,
                match_id,
                now,
                json(&templates)?,
            ],
        )?;
        let id: i64 = tx.query_row(
            "SELECT id FROM folder_rules WHERE source_id = ?1 AND path_prefix = ?2",
            params![source, prefix],
            |r| r.get(0),
        )?;
        spread(tx, source, &prefix, now)?;
        ids.push(id);
    }
    Ok(ids)
}

/// Decides every file under `prefix` of `source` again by the rules of the
/// source. Files decided by NFO ids, by the user for another work or by a
/// deeper rule of another folder stay. Returns the number of files whose
/// match changed.
pub(crate) fn spread(
    tx: &Transaction<'_>,
    source: i64,
    prefix: &str,
    now: i64,
) -> Result<usize, Error> {
    let rules = Rules::load(tx, source)?;
    if rules.is_empty() {
        return Ok(0);
    }
    let groups: Vec<Group> = pa_library::groups::source_groups(tx, source)?
        .into_iter()
        .map(|g| g.group)
        .collect();
    let mut planner = Planner::new(&groups);
    struct Current {
        id: i64,
        work: i64,
        state: String,
        decided_by: String,
        episodes: Vec<i64>,
    }
    let files: Vec<(i64, String)> = crate::recognize::video_files(tx, source, false)?;
    let mut changed = 0;
    for (file_id, path) in files {
        if !under(&path, prefix) {
            continue;
        }
        let plan = planner.plan(&path);
        let Some(d) = rules.decide(&path, plan.as_ref()) else {
            continue;
        };
        let current: Option<Current> = tx
            .query_row(
                "SELECT m.id, m.work_id, m.state, m.decided_by,
                        (SELECT group_concat(episode_id, ',') FROM
                            (SELECT episode_id FROM match_episodes WHERE match_id = m.id
                             ORDER BY position))
                 FROM matches m WHERE m.file_id = ?1 AND m.state <> 'rejected'",
                [file_id],
                |r| {
                    Ok(Current {
                        id: r.get(0)?,
                        work: r.get(1)?,
                        state: r.get(2)?,
                        decided_by: r.get(3)?,
                        episodes: r
                            .get::<_, Option<String>>(4)?
                            .unwrap_or_default()
                            .split(',')
                            .filter_map(|s| s.parse().ok())
                            .collect(),
                    })
                },
            )
            .optional()?;
        if let Some(c) = &current {
            let user = c.state.starts_with("user_") && c.decided_by != "rule";
            if c.decided_by == "nfo" || (user && c.work != d.work) {
                continue;
            }
            if c.work == d.work {
                let mut eps = Vec::with_capacity(d.episodes.len());
                for no in &d.episodes {
                    eps.push(crate::recognize::ensure_episode(tx, d.work, *no, None)?);
                }
                if eps == c.episodes {
                    continue;
                }
            }
            tx.execute("DELETE FROM matches WHERE id = ?1", [c.id])?;
        }
        if write_decision(tx, file_id, &d, None, now)? {
            changed += 1;
        }
    }
    Ok(changed)
}

impl Catalog {
    /// After a fix: the rules of `work` are taught, emptied works and
    /// unused episodes go, and the change is recorded for sync.
    pub(crate) fn teach_rules(&self, work: i64, how: Teach) -> Result<Vec<i64>, Error> {
        let ids = {
            let mut conn = self.lock();
            let tx = conn.transaction()?;
            let ids = teach(&tx, work, how, crate::now_ms())?;
            crate::recognize::remove_orphans(&tx)?;
            tx.commit()?;
            pa_library::search::sync(&conn)?;
            ids
        };
        if !ids.is_empty() {
            self.sync_folder_rules()?;
        }
        Ok(ids)
    }

    /// The episode numbering of the files of `item`'s work (an episode:
    /// its show), for the whole folder: the number in each file name plus
    /// `offset` is the episode of `season` (`None`: the season written in
    /// the name), or with `absolute` the running count. Files of the
    /// folder, also new ones, follow it; the next metadata run refreshes
    /// the episode data.
    pub fn set_numbering(
        &self,
        item: ItemRef,
        season: Option<i64>,
        offset: i64,
        absolute: bool,
    ) -> Result<(), Error> {
        if season.is_some_and(|s| !(0..=1000).contains(&s)) || !(-10_000..=10_000).contains(&offset)
        {
            return Err(Error::Invalid(format!(
                "numbering season {season:?} offset {offset}"
            )));
        }
        let work = self.work_of_item(item)?;
        {
            let conn = self.lock();
            let kind: String =
                conn.query_row("SELECT kind FROM works WHERE id = ?1", [work], |r| r.get(0))?;
            if kind == "movie" {
                return Err(Error::Invalid("a movie has no episodes".into()));
            }
        }
        self.decide_by_user(work)?;
        let ids = self.teach_rules(
            work,
            Teach {
                numbering: Some(Target {
                    absolute,
                    season,
                    offset,
                    abs_offset: None,
                }),
                kind_set: false,
            },
        )?;
        if ids.is_empty() {
            return Err(Error::Invalid(format!(
                "work {work} has no folder of its own for a rule"
            )));
        }
        self.refresh_metadata_of(work)
    }

    /// The type of `item`'s work: `movie`, `show` or `anime`, for the whole
    /// folder. Between a movie and a series the provider data goes and the
    /// next metadata run looks the work up anew.
    pub fn set_work_kind(&self, item: ItemRef, kind: &str) -> Result<(), Error> {
        if !matches!(kind, "movie" | "show" | "anime") {
            return Err(Error::Invalid(format!("kind {kind:?}")));
        }
        let work = self.work_of_item(item)?;
        {
            let mut conn = self.lock();
            let tx = conn.transaction()?;
            let old: String =
                tx.query_row("SELECT kind FROM works WHERE id = ?1", [work], |r| r.get(0))?;
            let now = crate::now_ms();
            if (old == "movie") != (kind == "movie") {
                crate::review::clear_provider_data(&tx, work, now)?;
                tx.execute("DELETE FROM work_metadata WHERE work_id = ?1", [work])?;
                // Episodes of a series that is a movie now (or numbers of
                // a movie that is a series) are decided again by the rule.
                tx.execute(
                    "DELETE FROM match_episodes WHERE match_id IN
                         (SELECT id FROM matches WHERE work_id = ?1)",
                    [work],
                )?;
            }
            tx.execute(
                "UPDATE works SET kind = ?2, updated_at_ms = ?3 WHERE id = ?1",
                params![work, kind, now],
            )?;
            tx.commit()?;
        }
        self.decide_by_user(work)?;
        self.teach_rules(
            work,
            Teach {
                numbering: None,
                kind_set: true,
            },
        )?;
        Ok(())
    }

    /// The folder rules of `item`'s work.
    pub fn folder_rules(&self, item: ItemRef) -> Result<Vec<FolderRuleInfo>, Error> {
        let work = self.work_of_item(item)?;
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, source_id, trim(path_prefix, '/'), work_id, kind_override, scheme,
                    season_override, episode_offset, templates_json
             FROM folder_rules WHERE work_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([work], |r| {
            Ok(FolderRuleInfo {
                id: r.get(0)?,
                source_id: r.get(1)?,
                folder: r.get(2)?,
                work_id: r.get(3)?,
                kind: r.get(4)?,
                scheme: r.get(5)?,
                season: r.get(6)?,
                offset: r.get(7)?,
                templates: serde_json::from_str(&r.get::<_, String>(8)?).unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The type of `item`'s work as the catalog has it now: `movie`,
    /// `show` or `anime` (META-035: the type switch of «Исправить
    /// распознавание» starts on it).
    pub fn work_kind(&self, item: ItemRef) -> Result<String, Error> {
        let work = self.work_of_item(item)?;
        let conn = self.lock();
        Ok(conn.query_row("SELECT kind FROM works WHERE id = ?1", [work], |r| r.get(0))?)
    }

    /// Removes the folder rules of `item`'s work here and, through sync,
    /// on the family's devices. Decisions already made stay; new files of
    /// the folder are recognised as before. Returns how many went.
    pub fn forget_folder_rules(&self, item: ItemRef) -> Result<usize, Error> {
        let work = self.work_of_item(item)?;
        let n = {
            let mut conn = self.lock();
            let tx = conn.transaction()?;
            let rules: Vec<(i64, String, Option<String>)> = {
                let mut stmt = tx.prepare(
                    "SELECT fr.id, trim(fr.path_prefix, '/'),
                            (SELECT s.sync_id FROM sources s
                             WHERE s.id = fr.source_id AND s.sync_mode > 0)
                     FROM folder_rules fr WHERE fr.work_id = ?1",
                )?;
                stmt.query_map([work], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<Result<_, _>>()?
            };
            for (id, prefix, sync_id) in &rules {
                if let Some(sync_id) = sync_id {
                    tx.execute(
                        "INSERT INTO folder_rule_sync (rule_key, rule, state) VALUES (?1, NULL, 'deleted')
                         ON CONFLICT (rule_key) DO UPDATE SET rule = NULL, state = 'deleted'",
                        [rule_key(sync_id, prefix)],
                    )?;
                }
                tx.execute("DELETE FROM folder_rules WHERE id = ?1", [id])?;
            }
            tx.commit()?;
            rules.len()
        };
        if n > 0 {
            self.sync_folder_rules()?;
        }
        Ok(n)
    }

    pub(crate) fn work_of_item(&self, item: ItemRef) -> Result<i64, Error> {
        let conn = self.lock();
        match item {
            ItemRef::Movie(w) | ItemRef::Show(w) => conn
                .query_row("SELECT id FROM works WHERE id = ?1", [w], |r| r.get(0))
                .optional()?
                .ok_or_else(|| Error::NotFound(item.to_string())),
            ItemRef::Episode(e) => conn
                .query_row("SELECT work_id FROM episodes WHERE id = ?1", [e], |r| {
                    r.get(0)
                })
                .optional()?
                .ok_or_else(|| Error::NotFound(item.to_string())),
            ItemRef::Track(_) => Err(Error::Invalid(format!("{item} is music"))),
        }
    }

    /// Makes the live matches of `work` user decisions (a fix is final).
    fn decide_by_user(&self, work: i64) -> Result<(), Error> {
        let conn = self.lock();
        conn.execute(
            "UPDATE matches SET state = 'user_set', decided_by = 'user', confidence = 1.0,
                                updated_at_ms = ?2
             WHERE work_id = ?1 AND state <> 'rejected' AND decided_by NOT IN ('rule', 'nfo')",
            params![work, crate::now_ms()],
        )?;
        conn.execute("DELETE FROM work_candidates WHERE work_id = ?1", [work])?;
        Ok(())
    }

    /// The next metadata run reads `work` again (by its ids).
    fn refresh_metadata_of(&self, work: i64) -> Result<(), Error> {
        let conn = self.lock();
        conn.execute(
            "UPDATE work_metadata SET fetched_at_ms = 0 WHERE work_id = ?1",
            [work],
        )?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Sync

/// The sync key of folder `prefix` of the source with `sync_id`: no path,
/// a hash of it salted with the source.
#[must_use]
pub fn rule_key(sync_id: &str, prefix: &str) -> String {
    let digest = Sha256::digest(format!("{sync_id}\n{}", prefix.trim_matches('/')).as_bytes());
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("{sync_id}:{hex}")
}

/// A rule in a `folder_rule.set` op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedRule {
    pub work: SyncedWork,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<i64>,
    #[serde(default)]
    pub offset: i64,
    #[serde(default)]
    pub templates: Vec<ReleaseTemplate>,
}

/// The work of a synced rule: its provider ids, else its title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedWork {
    /// `[provider, ext_kind, ext_id]`.
    #[serde(default)]
    pub ids: Vec<[String; 3]>,
    pub title: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i64>,
}

/// Providers whose ids name a work across devices.
const SYNCED_PROVIDERS: &[&str] = &["tmdb", "anilist", "imdb", "tvdb", "anidb", "mal"];

fn synced_rule(conn: &Connection, rule: i64) -> Result<Option<SyncedRule>, Error> {
    let row = conn
        .query_row(
            "SELECT fr.work_id, w.title, w.kind, w.year, fr.kind_override, fr.scheme,
                    fr.season_override, fr.episode_offset, fr.templates_json
             FROM folder_rules fr JOIN works w ON w.id = fr.work_id WHERE fr.id = ?1",
            [rule],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, String>(8)?,
                ))
            },
        )
        .optional()?;
    let Some((work, title, kind, year, kind_override, scheme, season, offset, templates)) = row
    else {
        return Ok(None);
    };
    let mut ids: Vec<[String; 3]> = {
        let mut stmt = conn.prepare_cached(
            "SELECT provider, ext_kind, ext_id FROM external_ids WHERE work_id = ?1
             ORDER BY provider, ext_kind, ext_id",
        )?;
        stmt.query_map([work], |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?]))?
            .collect::<Result<_, _>>()?
    };
    ids.retain(|[p, _, _]| SYNCED_PROVIDERS.contains(&p.as_str()));
    Ok(Some(SyncedRule {
        work: SyncedWork {
            ids,
            title,
            kind,
            year,
        },
        kind_override,
        scheme,
        season,
        offset,
        templates: serde_json::from_str(&templates).unwrap_or_default(),
    }))
}

#[derive(Serialize, Deserialize)]
struct StampJson {
    hlc: Hlc,
    op_id: OpId,
}

fn stamp_json(stamp: &Stamp) -> String {
    serde_json::to_string(&StampJson {
        hlc: stamp.0.clone(),
        op_id: stamp.1,
    })
    .unwrap_or_default()
}

fn parse_stamp(text: Option<String>) -> Option<Stamp> {
    let s: StampJson = serde_json::from_str(&text?).ok()?;
    Some((s.hlc, s.op_id))
}

/// Synced rules here: `(rule id, key)`.
fn synced_rules(conn: &Connection) -> Result<Vec<(i64, String)>, Error> {
    let rows: Vec<(i64, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT fr.id, s.sync_id, trim(fr.path_prefix, '/') FROM folder_rules fr
             JOIN sources s ON s.id = fr.source_id
             WHERE s.sync_mode > 0 AND s.sync_id IS NOT NULL ORDER BY fr.id",
        )?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?
    };
    Ok(rows
        .into_iter()
        .map(|(id, sync_id, prefix)| (id, rule_key(&sync_id, &prefix)))
        .collect())
}

/// The folder of the source with `sync_id` whose key is `key`, if this
/// device has it: `(source id, folder)`.
fn find_folder(conn: &Connection, key: &str) -> Result<Option<(i64, String)>, Error> {
    let Some((sync_id, _)) = key.split_once(':') else {
        return Ok(None);
    };
    let source: Option<i64> = conn
        .query_row(
            "SELECT id FROM sources WHERE sync_id = ?1",
            [sync_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(source) = source else {
        return Ok(None);
    };
    let mut stmt = conn.prepare_cached(
        "SELECT trim(path_prefix, '/') FROM folder_rules WHERE source_id = ?1
         UNION SELECT path FROM files WHERE source_id = ?1 AND missing_since_ms IS NULL",
    )?;
    let mut seen = std::collections::HashSet::new();
    for path in stmt.query_map([source], |r| r.get::<_, String>(0))? {
        let path = path?;
        let mut dir = path.trim_matches('/').to_owned();
        loop {
            if seen.insert(dir.clone()) && rule_key(sync_id, &dir) == key {
                return Ok(Some((source, dir)));
            }
            if dir.is_empty() {
                break;
            }
            dir = dir.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
        }
    }
    Ok(None)
}

fn static_kind(kind: &str) -> &'static str {
    match kind {
        "movie" => "movie",
        "anime" => "anime",
        _ => "show",
    }
}

/// The work of a synced rule here: by a provider id, else the work of its
/// title (created if needed) with the ids attached.
fn resolve_work(tx: &Transaction<'_>, w: &SyncedWork, now: i64) -> Result<i64, Error> {
    for [provider, kind, id] in &w.ids {
        let found: Option<i64> = tx
            .query_row(
                "SELECT work_id FROM external_ids
                 WHERE provider = ?1 AND ext_kind = ?2 AND ext_id = ?3",
                params![provider, kind, id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(work) = found {
            return Ok(work);
        }
    }
    let kind = static_kind(&w.kind);
    let mut plan = WorkPlan {
        kind,
        title: w.title.clone(),
        year: w.year,
        ids: BTreeMap::new(),
        alt_titles: Vec::new(),
    };
    for [provider, ext_kind, id] in &w.ids {
        match provider.as_str() {
            "tmdb" => {
                plan.ids.insert("tmdb".into(), format!("{ext_kind}:{id}"));
            }
            "imdb" | "tvdb" | "anidb" => {
                plan.ids.insert(provider.clone(), id.clone());
            }
            _ => {}
        }
    }
    let mut index = WorkIndex::load(tx)?;
    let work = index.ensure_plan(tx, &plan, now)?;
    for [provider, ext_kind, id] in &w.ids {
        let taken: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM external_ids WHERE provider = ?1 AND ext_kind = ?2
                            AND (ext_id = ?3 OR work_id = ?4))",
            params![provider, ext_kind, id, work],
            |r| r.get(0),
        )?;
        if !taken {
            tx.execute(
                "INSERT INTO external_ids (work_id, provider, ext_kind, ext_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![work, provider, ext_kind, id],
            )?;
        }
    }
    Ok(work)
}

/// Applies the synced `rule` of `key` if this device has its folder.
/// Returns the local rule id.
fn resolve(
    tx: &Transaction<'_>,
    key: &str,
    rule: &SyncedRule,
    now: i64,
) -> Result<Option<i64>, Error> {
    let Some((source, prefix)) = find_folder(tx, key)? else {
        return Ok(None);
    };
    let work = resolve_work(tx, &rule.work, now)?;
    if let Some(k) = rule
        .kind_override
        .as_deref()
        .filter(|k| matches!(*k, "movie" | "show" | "anime"))
    {
        tx.execute(
            "UPDATE works SET kind = ?2 WHERE id = ?1 AND kind <> ?2",
            params![work, k],
        )?;
    }
    let scheme = rule
        .scheme
        .as_deref()
        .filter(|s| matches!(*s, "tmdb" | "tvdb_absolute"));
    tx.execute(
        "INSERT INTO folder_rules (source_id, path_prefix, work_id, scheme, episode_offset,
                                   season_override, kind_override, created_at_ms, templates_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (source_id, path_prefix) DO UPDATE SET
             work_id = excluded.work_id, scheme = excluded.scheme,
             episode_offset = excluded.episode_offset, season_override = excluded.season_override,
             kind_override = excluded.kind_override, templates_json = excluded.templates_json",
        params![
            source,
            prefix,
            work,
            scheme,
            rule.offset,
            rule.season.filter(|s| *s >= 0),
            rule.kind_override,
            now,
            json(
                &rule
                    .templates
                    .iter()
                    .take(MAX_TEMPLATES)
                    .collect::<Vec<_>>()
            )?,
        ],
    )?;
    let id: i64 = tx.query_row(
        "SELECT id FROM folder_rules WHERE source_id = ?1 AND path_prefix = ?2",
        params![source, prefix],
        |r| r.get(0),
    )?;
    spread(tx, source, &prefix, now)?;
    Ok(Some(id))
}

/// Applies the rules of other devices that wait for a folder of
/// `source_id` (the scan found it now).
pub(crate) fn resolve_received(conn: &mut Connection, source_id: i64) -> Result<usize, Error> {
    let sync_id: Option<String> = conn
        .query_row(
            "SELECT sync_id FROM sources WHERE id = ?1",
            [source_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(sync_id) = sync_id else {
        return Ok(0);
    };
    let waiting: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT rule_key, rule FROM folder_rule_sync
             WHERE state = 'received' AND rule IS NOT NULL AND substr(rule_key, 1, ?2) = ?1",
        )?;
        let prefix = format!("{sync_id}:");
        stmt.query_map(
            params![prefix, i64::try_from(prefix.len()).unwrap_or(0)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .collect::<Result<_, _>>()?
    };
    if waiting.is_empty() {
        return Ok(0);
    }
    let now = crate::now_ms();
    let tx = conn.transaction()?;
    let mut applied = 0;
    for (key, text) in waiting {
        let Ok(rule) = serde_json::from_str::<SyncedRule>(&text) else {
            continue;
        };
        if let Some(id) = resolve(&tx, &key, &rule, now)? {
            let seen = synced_rule(&tx, id)?.map(|r| json(&r)).transpose()?;
            tx.execute(
                "UPDATE folder_rule_sync SET state = 'synced', rule = ?2 WHERE rule_key = ?1",
                params![key, seen],
            )?;
            applied += 1;
        }
    }
    crate::recognize::remove_orphans(&tx)?;
    tx.commit()?;
    Ok(applied)
}

impl Catalog {
    /// Records `folder_rule.set` ops for the rules of synced sources that
    /// changed since they were last sent or applied, and for rules the user
    /// removed. Called after fixes and when sync starts. Returns the number
    /// of ops; 0 while sync is off.
    pub fn sync_folder_rules(&self) -> Result<usize, Error> {
        let mut conn = self.lock();
        let mut sync = self.sync_lock();
        let Some(replica) = sync.as_mut() else {
            return Ok(0);
        };
        let tx = conn.transaction()?;
        let now = crate::now_ms();
        let mut recorded = 0;
        for (id, key) in synced_rules(&tx)? {
            let Some(rule) = synced_rule(&tx, id)? else {
                continue;
            };
            let text = json(&rule)?;
            let known: Option<(Option<String>, String)> = tx
                .query_row(
                    "SELECT rule, state FROM folder_rule_sync WHERE rule_key = ?1",
                    [&key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if known.is_some_and(|(r, s)| s == "synced" && r.as_deref() == Some(text.as_str())) {
                continue;
            }
            let op = replica.record(
                &tx,
                OpBody::FolderRule {
                    rule_key: key.clone(),
                    rule: Some(text.clone()),
                },
                now,
            )?;
            tx.execute(
                "INSERT INTO folder_rule_sync (rule_key, rule, stamp, state)
                 VALUES (?1, ?2, ?3, 'synced')
                 ON CONFLICT (rule_key) DO UPDATE SET rule = excluded.rule,
                     stamp = excluded.stamp, state = 'synced'",
                params![key, text, stamp_json(&(op.hlc, op.op_id))],
            )?;
            recorded += 1;
        }
        let deleted: Vec<String> = {
            let mut stmt =
                tx.prepare("SELECT rule_key FROM folder_rule_sync WHERE state = 'deleted'")?;
            stmt.query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        for key in deleted {
            let op = replica.record(
                &tx,
                OpBody::FolderRule {
                    rule_key: key.clone(),
                    rule: None,
                },
                now,
            )?;
            tx.execute(
                "UPDATE folder_rule_sync SET rule = NULL, stamp = ?2, state = 'synced'
                 WHERE rule_key = ?1",
                params![key, stamp_json(&(op.hlc, op.op_id))],
            )?;
            recorded += 1;
        }
        tx.commit()?;
        Ok(recorded)
    }

    /// Writes a `folder_rule.set` op of another device: last writer wins;
    /// a rule whose folder is not here yet waits for the scan.
    pub(crate) fn apply_remote_folder_rule(&self, op: &Op) -> Result<bool, Error> {
        let OpBody::FolderRule { rule_key, rule } = &op.body else {
            return Ok(false);
        };
        let mut conn = self.lock();
        let mut sync = self.sync_lock();
        let Some(replica) = sync.as_mut() else {
            return Ok(false);
        };
        let now = crate::now_ms();
        replica.receive(op, now)?;
        drop(sync);
        let stamp: Stamp = (op.hlc.clone(), op.op_id);
        let tx = conn.transaction()?;
        let known: Option<Option<String>> = tx
            .query_row(
                "SELECT stamp FROM folder_rule_sync WHERE rule_key = ?1",
                [rule_key],
                |r| r.get(0),
            )
            .optional()?;
        if known.and_then(parse_stamp).is_some_and(|k| k >= stamp) {
            return Ok(false);
        }
        let changed = match rule {
            None => {
                let local = synced_rules(&tx)?
                    .into_iter()
                    .find(|(_, k)| k == rule_key)
                    .map(|(id, _)| id);
                if let Some(id) = local {
                    tx.execute("DELETE FROM folder_rules WHERE id = ?1", [id])?;
                }
                tx.execute(
                    "INSERT INTO folder_rule_sync (rule_key, rule, stamp, state)
                     VALUES (?1, NULL, ?2, 'synced')
                     ON CONFLICT (rule_key) DO UPDATE SET rule = NULL, stamp = excluded.stamp,
                         state = 'synced'",
                    params![rule_key, stamp_json(&stamp)],
                )?;
                local.is_some()
            }
            Some(text) => {
                let Ok(parsed) = serde_json::from_str::<SyncedRule>(text) else {
                    return Ok(false);
                };
                match resolve(&tx, rule_key, &parsed, now)? {
                    Some(id) => {
                        let seen = synced_rule(&tx, id)?.map(|r| json(&r)).transpose()?;
                        tx.execute(
                            "INSERT INTO folder_rule_sync (rule_key, rule, stamp, state)
                             VALUES (?1, ?2, ?3, 'synced')
                             ON CONFLICT (rule_key) DO UPDATE SET rule = excluded.rule,
                                 stamp = excluded.stamp, state = 'synced'",
                            params![rule_key, seen, stamp_json(&stamp)],
                        )?;
                        crate::recognize::remove_orphans(&tx)?;
                        true
                    }
                    None => {
                        tx.execute(
                            "INSERT INTO folder_rule_sync (rule_key, rule, stamp, state)
                             VALUES (?1, ?2, ?3, 'received')
                             ON CONFLICT (rule_key) DO UPDATE SET rule = excluded.rule,
                                 stamp = excluded.stamp, state = 'received'",
                            params![rule_key, text, stamp_json(&stamp)],
                        )?;
                        false
                    }
                }
            }
        };
        tx.commit()?;
        if changed {
            pa_library::search::sync(&conn)?;
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_cut_the_number_and_the_crc() {
        let (p, s) = learn_pattern(
            "/Anime/Frieren/[SubsPlease] Sousou no Frieren - 29 (1080p) [ABCD1234].mkv",
            29,
        )
        .unwrap();
        assert_eq!(p, "subsplease sousou no frieren {n} 1080p {crc}");
        assert_eq!(s, None);
        assert_eq!(
            match_pattern(
                &p,
                "Anime/Frieren/[SubsPlease] Sousou no Frieren - 31 (1080p) [0F0E1D2C].mkv"
            ),
            Some((31, None))
        );
        assert_eq!(
            match_pattern(
                &p,
                "Anime/Frieren/[SubsPlease] Sousou no Frieren - 31v2 (1080p) [0F0E1D2C].mkv"
            ),
            Some((31, None))
        );
        // Another release, another show.
        assert_eq!(
            match_pattern(
                &p,
                "Anime/Frieren/[Erai-raws] Sousou no Frieren - 31 [1080p].mkv"
            ),
            None
        );
        assert_eq!(
            match_pattern(
                &p,
                "Anime/Frieren/[SubsPlease] Dandadan - 31 (1080p) [0F0E1D2C].mkv"
            ),
            None
        );
    }

    #[test]
    fn season_episode_names_keep_their_season() {
        let (p, s) = learn_pattern("Shows/Dark/Season 1/Dark.S01E02.1080p.mkv", 2).unwrap();
        assert_eq!(p, "dark s{s}e{n} 1080p");
        assert_eq!(s, Some(1));
        assert_eq!(
            match_pattern(&p, "Shows/Dark/Season 2/Dark.S02E07.1080p.mkv"),
            Some((7, Some(2)))
        );
    }

    #[test]
    fn targets_land_numbers() {
        let t = Target {
            absolute: false,
            season: Some(2),
            offset: -28,
            abs_offset: Some(0),
        };
        let e = t.land(31, None, "regular").unwrap();
        assert_eq!(
            (e.season, e.episode, e.absolute),
            (Some(2), Some(3), Some(31))
        );
        assert!(t.land(3, None, "regular").is_none());
        let a = Target {
            absolute: true,
            season: None,
            offset: 12,
            abs_offset: None,
        };
        let e = a.land(1, None, "regular").unwrap();
        assert_eq!((e.season, e.episode, e.absolute), (None, None, Some(13)));
    }

    #[test]
    fn keys_hide_the_path() {
        let k = rule_key("abc", "/Anime/Frieren/");
        assert_eq!(k, rule_key("abc", "Anime/Frieren"));
        assert!(k.starts_with("abc:"));
        assert!(!k.to_lowercase().contains("frieren"));
        assert_ne!(k, rule_key("abd", "Anime/Frieren"));
        assert!(under("/Anime/Frieren/x.mkv", "Anime/Frieren"));
        assert!(!under("/Anime/Frieren 2/x.mkv", "Anime/Frieren"));
        assert!(under("/x.mkv", ""));
    }
}
