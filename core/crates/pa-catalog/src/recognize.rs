// SPDX-License-Identifier: MPL-2.0
//! Local recognition: video files → works, episodes and matches, from the
//! `pa-recognition` parser and the folder groups of LIB-007 (no providers).
//!
//! [`plan_files`] decides, without IO, what every file is:
//!
//! - files outside any group: the relative path is parsed (folders give
//!   context) and the best hypothesis decides — an episode (season /
//!   episode / absolute markers) joins the show work with the same title
//!   key (`anime` if the name looks like an anime release), a movie joins
//!   the movie work with the same title key and year; extras are skipped;
//! - **discs** (`BDMV/`, `VIDEO_TS/`, `.iso`): the main title of the disc
//!   (its VOBs as parts on a DVD) is one item named by the disc folder or
//!   image, not by `00001.m2ts`; the other streams of the disc (menus,
//!   bonus clips) are ignored leftovers and get no match (LIB-017);
//! - **split files** (`cd1`/`cd2`, `part1`…) of a movie folder join the work
//!   of the folder, one item;
//! - **extras and samples** belong to the work of their group (the work an
//!   extra is named after, or the one most videos of the group are); they
//!   never become works of their own. In a loose dump folder an extra that
//!   names no work of the folder is skipped;
//! - **NFO ids** (`tmdb`, `imdb`, `tvdb`, `anidb`; LIB-007 reads them) win
//!   over the title: `movie.nfo` / `<video>.nfo` for movies, `tvshow.nfo` of
//!   the series folder or a parent folder for episodes. A file whose NFO
//!   names an id joins the work that already carries it, and its match is
//!   `decided_by = nfo` with confidence 1.
//!
//! [`run`] writes the plans: the N-best parse goes to `parse_results`, the
//! work is found or created ([`WorkIndex`]), the file gets a live match
//! (`state = auto`, `decided_by = rule` or `nfo`, confidence = hypothesis
//! weight) and, for episodes, `match_episodes`. Extras get a match without
//! episodes; item lists leave them out.
//!
//! Title keys are the loose keys of `pa_recognition::title::TitleKey`, so
//! `Ведьмак`, `Vedmak`, `The Witcher` spelled differently in two folders
//! stay apart but case, punctuation and transliteration do not split a
//! show. All titles of a work (`titles`) are keys, so a renamed work keeps
//! catching files named the old way.

use std::collections::{BTreeMap, HashMap};

use pa_library::groups::{Group, GroupEntry, GroupKind, Role};
use pa_library::rusqlite::{Connection, OptionalExtension, Transaction, params};
use pa_recognition::parse::{self, EpisodeKind, MediaHint, ParsedName};
use pa_recognition::title::TitleKey;
use pa_scanner::MediaKind;

use crate::Error;

/// Files recognized per transaction.
const BATCH: usize = 200;

/// `external_ids` providers an NFO file may decide.
pub const NFO_PROVIDERS: &[&str] = &["tmdb", "imdb", "tvdb", "anidb"];

/// Title key of a work: kind group, loose title and (movies) year.
fn work_key(series: bool, title: &str, year: Option<i64>) -> String {
    let key = TitleKey::new(title);
    let loose = if key.loose.is_empty() {
        key.plain
    } else {
        key.loose
    };
    if series {
        format!("series|{loose}")
    } else {
        format!("movie|{loose}|{}", year.unwrap_or(0))
    }
}

/// The work a file belongs to, as the parser and the folder see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkPlan {
    /// `movie`, `show` or `anime`.
    pub kind: &'static str,
    pub title: String,
    pub year: Option<i64>,
    /// Ids from NFO files: provider ([`NFO_PROVIDERS`]) → id. TMDB ids carry
    /// their kind (`movie:603`, `tv:1399`).
    pub ids: BTreeMap<String, String>,
    /// Other titles the files are named by (the other half of a double
    /// title, a folder title in another language, another spelling of the
    /// same key): stored as local synonyms, so the metadata run can search
    /// by them too.
    pub alt_titles: Vec<String>,
}

impl WorkPlan {
    /// Adds `title` to [`alt_titles`](Self::alt_titles) unless the work
    /// already has it.
    fn add_title(&mut self, title: &str) {
        let t = title.trim();
        if !t.is_empty() && t != self.title && !self.alt_titles.iter().any(|a| a == t) {
            self.alt_titles.push(t.to_owned());
        }
    }

    /// A show or anime.
    #[must_use]
    pub fn is_series(&self) -> bool {
        self.kind != "movie"
    }

    /// The title key works are looked up by.
    #[must_use]
    pub fn key(&self) -> String {
        work_key(self.is_series(), &self.title, self.year)
    }

    /// `(provider, ext_kind, ext_id)` rows of `external_ids` for [`ids`](Self::ids).
    #[must_use]
    pub fn external_ids(&self) -> Vec<(String, String, String)> {
        let tmdb_kind = if self.is_series() { "tv" } else { "movie" };
        self.ids
            .iter()
            .filter_map(|(provider, v)| {
                let (kind, id) = match provider.as_str() {
                    "tmdb" => match v.split_once(':') {
                        Some((k, id)) => (k.to_owned(), id.to_owned()),
                        None => (tmdb_kind.to_owned(), v.clone()),
                    },
                    "tvdb" if self.is_series() => ("series".to_owned(), v.clone()),
                    "tvdb" => ("movie".to_owned(), v.clone()),
                    "imdb" | "anidb" => (String::new(), v.clone()),
                    _ => return None,
                };
                (!id.is_empty()).then(|| (provider.clone(), kind, id))
            })
            .collect()
    }
}

/// What a file is to its work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanRole {
    /// The video of an item.
    Main,
    /// One piece of an item split into files (or a disc).
    Part,
    /// Bonus material or a sample of the work.
    Extra,
}

/// The decision for one video file ([`plan_files`]).
#[derive(Debug, Clone, PartialEq)]
pub struct FilePlan {
    pub work: WorkPlan,
    pub role: PlanRole,
    /// Episodes in file order; empty for movies and extras.
    pub episodes: Vec<EpisodeNo>,
    /// Title of the episode, for a file of one episode.
    pub episode_title: Option<String>,
    /// Weight of the parser hypothesis; 1 when NFO ids decided the work.
    pub confidence: f64,
}

impl FilePlan {
    /// `matches.decided_by` of the plan.
    #[must_use]
    pub fn decided_by(&self) -> &'static str {
        if self.work.ids.is_empty() {
            "rule"
        } else {
            "nfo"
        }
    }

    fn as_extra(&self) -> Self {
        Self {
            work: self.work.clone(),
            role: PlanRole::Extra,
            episodes: Vec::new(),
            episode_title: None,
            confidence: self.confidence,
        }
    }
}

/// Lookup of existing works by NFO id and title key.
pub(crate) struct WorkIndex {
    keys: HashMap<String, i64>,
}

impl WorkIndex {
    pub(crate) fn load(conn: &Connection) -> Result<Self, Error> {
        let mut keys = HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT w.id, w.kind, w.year, w.title FROM works w
             UNION ALL
             SELECT w.id, w.kind, w.year, t.text FROM titles t JOIN works w ON w.id = t.work_id
             ORDER BY 1",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (id, kind, year, title) = row?;
            let series = kind != "movie";
            keys.entry(work_key(series, &title, year)).or_insert(id);
            if !series {
                // A file without a year joins a movie with one.
                keys.entry(work_key(false, &title, None)).or_insert(id);
            }
        }
        Ok(Self { keys })
    }

    /// The work for `title` (and `year` for movies), created if needed.
    pub(crate) fn ensure(
        &mut self,
        conn: &Connection,
        kind: &str,
        title: &str,
        year: Option<i64>,
        now: i64,
    ) -> Result<i64, Error> {
        let series = kind != "movie";
        let key = work_key(series, title, year);
        if let Some(id) = self.keys.get(&key) {
            return Ok(*id);
        }
        let id = create_work(conn, kind, title, year, now)?;
        self.keys.insert(key, id);
        if !series {
            self.keys.entry(work_key(false, title, None)).or_insert(id);
        }
        Ok(id)
    }

    /// The work of `plan`: the one carrying one of its NFO ids, else the one
    /// with its title key (unless that work has another id of the same
    /// provider), else a new one. The NFO ids are attached.
    pub(crate) fn ensure_plan(
        &mut self,
        conn: &Connection,
        plan: &WorkPlan,
        now: i64,
    ) -> Result<i64, Error> {
        let ext = plan.external_ids();
        for (provider, kind, id) in &ext {
            let owner: Option<i64> = conn
                .query_row(
                    "SELECT work_id FROM external_ids
                     WHERE provider = ?1 AND ext_kind = ?2 AND ext_id = ?3",
                    params![provider, kind, id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(w) = owner {
                return Ok(w);
            }
        }
        let mut work = self.ensure(conn, plan.kind, &plan.title, plan.year, now)?;
        add_local_titles(conn, work, plan)?;
        if !ext.is_empty() {
            let mut conflict = false;
            for (provider, kind, id) in &ext {
                let other: Option<String> = conn
                    .query_row(
                        "SELECT ext_id FROM external_ids
                         WHERE work_id = ?1 AND provider = ?2 AND ext_kind = ?3",
                        params![work, provider, kind],
                        |r| r.get(0),
                    )
                    .optional()?;
                conflict |= other.is_some_and(|o| &o != id);
            }
            if conflict {
                // Same title, another work (a remake the NFO tells apart).
                work = create_work(conn, plan.kind, &plan.title, plan.year, now)?;
                add_local_titles(conn, work, plan)?;
            }
            for (provider, kind, id) in &ext {
                conn.execute(
                    "INSERT OR IGNORE INTO external_ids (work_id, provider, ext_kind, ext_id)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![work, provider, kind, id],
                )?;
            }
        }
        Ok(work)
    }
}

/// The plan's title and alternative titles as local synonyms of `work`
/// (titles the work already has are skipped).
fn add_local_titles(conn: &Connection, work: i64, plan: &WorkPlan) -> Result<(), Error> {
    for t in std::iter::once(&plan.title).chain(&plan.alt_titles) {
        conn.execute(
            "INSERT INTO titles (work_id, text, kind, source)
             SELECT ?1, ?2, 'synonym', 'local'
             WHERE NOT EXISTS (SELECT 1 FROM titles WHERE work_id = ?1 AND text = ?2)",
            params![work, t],
        )?;
    }
    Ok(())
}

fn create_work(
    conn: &Connection,
    kind: &str,
    title: &str,
    year: Option<i64>,
    now: i64,
) -> Result<i64, Error> {
    let year = year.filter(|y| (1800..=3000).contains(y));
    conn.execute(
        "INSERT INTO works (kind, title, year, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![kind, title, year, now],
    )?;
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT OR IGNORE INTO titles (work_id, text, kind, source) VALUES (?1, ?2, 'main', 'local')",
        params![id, title],
    )?;
    Ok(id)
}

/// Works of a set of plans, the way [`run`] joins files into works: by NFO
/// id first, then by title key (a movie without a year joins one with a
/// year of the same title; a series takes the first year it sees). Returns
/// the works and, per plan, the index of its work.
///
/// This is the in-memory twin of the database path, for tools that run the
/// recognition without a library (`pa-eval`).
#[must_use]
pub fn group_works(plans: &[Option<FilePlan>]) -> (Vec<WorkPlan>, Vec<Option<usize>>) {
    let mut works: Vec<WorkPlan> = Vec::new();
    let mut keys: HashMap<String, usize> = HashMap::new();
    let mut ids: HashMap<(String, String, String), usize> = HashMap::new();
    let mut of = Vec::with_capacity(plans.len());
    for plan in plans {
        let Some(plan) = plan else {
            of.push(None);
            continue;
        };
        let ext = plan.work.external_ids();
        let by_id = ext.iter().find_map(|e| ids.get(e).copied());
        let idx = match by_id {
            Some(i) => i,
            None => {
                let key = plan.work.key();
                let conflict = |i: usize| {
                    let theirs = works[i].external_ids();
                    ext.iter().any(|(p, k, v)| {
                        theirs
                            .iter()
                            .any(|(p2, k2, v2)| p == p2 && k == k2 && v != v2)
                    })
                };
                match keys.get(&key).copied().filter(|&i| !conflict(i)) {
                    Some(i) => i,
                    None => {
                        let i = works.len();
                        works.push(WorkPlan {
                            year: plan.work.year.filter(|y| (1800..=3000).contains(y)),
                            ..plan.work.clone()
                        });
                        keys.entry(key).or_insert(i);
                        if !plan.work.is_series() {
                            keys.entry(work_key(false, &plan.work.title, None))
                                .or_insert(i);
                        }
                        i
                    }
                }
            }
        };
        let work = &mut works[idx];
        if work.is_series() && work.year.is_none() {
            work.year = plan.work.year.filter(|y| (1800..=3000).contains(y));
        }
        for (p, v) in &plan.work.ids {
            work.ids.entry(p.clone()).or_insert_with(|| v.clone());
        }
        work.add_title(&plan.work.title);
        for a in &plan.work.alt_titles {
            work.add_title(a);
        }
        for e in work.external_ids() {
            ids.entry(e).or_insert(idx);
        }
        of.push(Some(idx));
    }
    (works, of)
}

/// Numbers of one catalog episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpisodeNo {
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub absolute: Option<i64>,
    /// `episodes.episode_type`.
    pub kind: &'static str,
}

/// The episode of `work_id` with these numbers, created if needed.
pub(crate) fn ensure_episode(
    conn: &Connection,
    work_id: i64,
    no: EpisodeNo,
    title: Option<&str>,
) -> Result<i64, Error> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT id FROM episodes
             WHERE work_id = ?1 AND season_number IS ?2 AND episode_number IS ?3
               AND absolute_number IS ?4 AND episode_type = ?5
             ORDER BY id LIMIT 1",
            params![work_id, no.season, no.episode, no.absolute, no.kind],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = found {
        if let Some(t) = title {
            conn.execute(
                "UPDATE episodes SET title = ?2 WHERE id = ?1 AND title IS NULL",
                params![id, t],
            )?;
        }
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO episodes (work_id, episode_type, season_number, episode_number,
                               absolute_number, title)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![work_id, no.kind, no.season, no.episode, no.absolute, title],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Live match of `file_id` → `work_id` with its episodes in file order.
/// Does nothing if the file already has a live match.
#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_match(
    conn: &Connection,
    file_id: i64,
    work_id: i64,
    episodes: &[i64],
    confidence: f64,
    state: &str,
    decided_by: &str,
    model_version: Option<&str>,
    now: i64,
) -> Result<bool, Error> {
    let inserted = conn.execute(
        "INSERT INTO matches (file_id, work_id, scheme_used, confidence, state, decided_by,
                              model_version, created_at_ms, updated_at_ms)
         SELECT ?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?7
         WHERE NOT EXISTS (SELECT 1 FROM matches WHERE file_id = ?1 AND state <> 'rejected')",
        params![
            file_id,
            work_id,
            confidence.clamp(0.0, 1.0),
            state,
            decided_by,
            model_version,
            now
        ],
    )?;
    if inserted == 0 {
        return Ok(false);
    }
    let match_id = conn.last_insert_rowid();
    for (pos, ep) in episodes.iter().enumerate() {
        conn.execute(
            "INSERT OR IGNORE INTO match_episodes (match_id, position, episode_id) VALUES (?1, ?2, ?3)",
            params![match_id, i64::try_from(pos).unwrap_or(i64::MAX), ep],
        )?;
    }
    Ok(true)
}

fn episode_type(kind: Option<EpisodeKind>) -> &'static str {
    match kind {
        None | Some(EpisodeKind::Regular) => "regular",
        Some(EpisodeKind::Special | EpisodeKind::Ova | EpisodeKind::Ona | EpisodeKind::Sp) => {
            "special"
        }
        Some(EpisodeKind::Recap) => "recap",
        Some(EpisodeKind::Ncop | EpisodeKind::Nced) => "credit",
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn file_stem(path: &str) -> &str {
    let name = file_name(path);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// A lowercase file-name title (`inception.mkv`) takes the casing of the
/// same title written in an alternative title or in its folder
/// (`Inception (2010)/`).
fn better_cased(title: String, alts: &[String], path: &str) -> String {
    if title.chars().any(char::is_uppercase) {
        return title;
    }
    let key = TitleKey::new(&title).loose;
    let mut folders = path.rsplit('/').skip(1).take(2).map(|f| {
        // `Inception (2010) [1080p]` → `Inception`.
        f.split(['(', '[']).next().unwrap_or(f).trim().to_owned()
    });
    alts.iter()
        .cloned()
        .chain(&mut folders)
        .find(|a| a.chars().any(char::is_uppercase) && TitleKey::new(a).loose == key)
        .unwrap_or(title)
}

/// The plan of one parsed name. Extras give `None` unless `as_main`, which
/// reads an extra's name for the work it names.
fn plan(path: &str, p: &ParsedName, weight: f64, as_main: bool) -> Option<FilePlan> {
    if !as_main && (p.media == MediaHint::Extra || p.extra.is_some()) {
        return None;
    }
    let title = p
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| file_stem(path).to_owned());
    let title = better_cased(title, &p.alt_titles, path);
    let alts = |work: &mut WorkPlan| {
        for a in &p.alt_titles {
            work.add_title(a);
        }
    };
    let year = p.year.map(i64::from);
    let is_episode = p.media == MediaHint::Episode
        || !p.episodes.is_empty()
        || !p.absolute.is_empty()
        || p.date.is_some();
    let movie = |title| FilePlan {
        work: WorkPlan {
            kind: "movie",
            title,
            year,
            ids: BTreeMap::new(),
            alt_titles: Vec::new(),
        },
        role: PlanRole::Main,
        episodes: Vec::new(),
        episode_title: None,
        confidence: weight,
    };
    if !is_episode {
        let mut m = movie(title);
        alts(&mut m.work);
        return Some(m);
    }
    let kind = episode_type(p.episode_kind);
    // `Title 2 s1e05`, `Title Часть 2 s1e05`: the sequel number is the
    // season (META-032); the full title stays a synonym for the search.
    let (title, sequel_of) = match sequel_season(&title, p) {
        Some((base, n)) => {
            let full = title.clone();
            (base, Some((full, n)))
        }
        None => (title, None),
    };
    let season = sequel_of
        .as_ref()
        .map(|(_, n)| i64::from(*n))
        .or_else(|| p.season.map(i64::from))
        .or_else(|| {
            // A season qualifier in the show title (`Mob Psycho 100 III`).
            TitleKey::new(&title).season.map(i64::from)
        });
    let mut numbers: Vec<EpisodeNo> = p
        .episodes
        .iter()
        .filter(|e| e.fraction.is_none())
        .map(|e| EpisodeNo {
            season: season.or(Some(1)),
            episode: Some(i64::from(e.number)),
            absolute: None,
            kind,
        })
        .collect();
    if numbers.is_empty() {
        numbers = p
            .absolute
            .iter()
            .filter(|e| e.fraction.is_none())
            .map(|e| EpisodeNo {
                season,
                episode: None,
                absolute: Some(i64::from(e.number)),
                kind,
            })
            .collect();
    }
    if numbers.is_empty() {
        // A dated or unnumbered episode: its own catalog row.
        numbers.push(EpisodeNo {
            season,
            episode: None,
            absolute: None,
            kind,
        });
    }
    let mut work = WorkPlan {
        kind: if p.anime_hint { "anime" } else { "show" },
        title,
        year,
        ids: BTreeMap::new(),
        alt_titles: Vec::new(),
    };
    if let Some((full, _)) = &sequel_of {
        work.add_title(full);
    }
    alts(&mut work);
    Some(FilePlan {
        work,
        role: PlanRole::Main,
        episodes: numbers,
        episode_title: p.episode_title.clone(),
        confidence: weight,
    })
}

/// Sequel numbers that are seasons (META-032), for a file that names its
/// season (`s1e05`, `S02E01`): a trailing `2`…`4` of the title when the
/// written season is 1 or the same number (`Title 2 s1e05`, `Title 2 -
/// S02E01`), or a part (`Часть 2`, `Part 2`) with season 1. Returns the
/// title without the number and the season. Bare `Title 2 - 05` keeps its
/// title: anime entries are named that way (AniDB) and the numbering
/// (META-015) maps them.
fn sequel_season(title: &str, p: &ParsedName) -> Option<(String, u32)> {
    let written = p.season?;
    if p.episodes.is_empty() {
        return None;
    }
    let trimmed = title.trim();
    if let Some((head, last)) = trimmed.rsplit_once(' ')
        && !last.starts_with('0')
        && last.chars().all(|c| c.is_ascii_digit())
        && let Ok(n) = last.parse::<u32>()
        && (2..=4).contains(&n)
        && (written == 1 || written == n)
        && head
            .rsplit(' ')
            .next()
            .is_some_and(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 2)
    {
        return Some((head.trim_end().to_owned(), n));
    }
    match p.part {
        Some(n) if n >= 2 && written == 1 => Some((trimmed.to_owned(), n)),
        _ => None,
    }
}

/// The plan of a path from its own name.
fn own_plan(path: &str, as_main: bool) -> Option<FilePlan> {
    let result = parse::parse(path);
    let best = result.best();
    plan(path, &best.parsed, f64::from(best.weight), as_main)
}

/// `01 The Legend Begins` → `(1, "The Legend Begins")`: one to three
/// digits at the start, then the end or a separator.
fn leading_number(name: &str) -> Option<(u32, &str)> {
    let name = name.trim_start();
    let digits = name.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    let (num, rest) = name.split_at(digits);
    if !(rest.is_empty() || rest.starts_with([' ', '.', '_', '-', ')'])) {
        return None;
    }
    let rest = rest.trim_start_matches([' ', '.', '_', '-', ')']).trim();
    Some((num.parse().ok()?, rest))
}

/// Lower-case letters and digits only, for title comparison.
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Disc folders are named like files: `Movies/Inception (2010)` is parsed
/// as `Movies/Inception (2010).mkv`; an image is parsed as it is.
fn disc_name(group: &Group) -> String {
    let lower = group.root.to_ascii_lowercase();
    if lower.ends_with(".iso") {
        group.root.clone()
    } else {
        format!("{}.mkv", group.root.trim_end_matches('/'))
    }
}

/// Parent folders of a path, nearest first (`/a/b/c` → `/a/b`, `/a`, `` / `/`).
fn parents(path: &str) -> impl Iterator<Item = &str> {
    let mut rest = path;
    std::iter::from_fn(move || {
        let (parent, _) = rest.rsplit_once('/')?;
        rest = parent;
        Some(if parent.is_empty() && path.starts_with('/') {
            "/"
        } else {
            parent
        })
    })
}

/// Plans of a set of files with the groups of their source; see the
/// [module docs](self). `None` — the file is not recognized (an ignored
/// leftover, an extra that names no work).
pub struct Planner<'g> {
    groups: &'g [Group],
    entry: HashMap<&'g str, (usize, &'g GroupEntry)>,
    root: HashMap<&'g str, usize>,
    /// Plans of the playable entries of a group, by own name.
    playable: HashMap<usize, Vec<Option<FilePlan>>>,
    anchor: HashMap<usize, Option<FilePlan>>,
    /// Episode plans of a folder of numbered files, by path (META-032).
    numbered: HashMap<usize, HashMap<String, FilePlan>>,
}

impl<'g> Planner<'g> {
    /// A planner over the groups of one source.
    #[must_use]
    pub fn new(groups: &'g [Group]) -> Self {
        let mut entry = HashMap::new();
        let mut root = HashMap::new();
        for (gi, g) in groups.iter().enumerate() {
            root.insert(g.root.as_str(), gi);
            for e in &g.entries {
                entry.insert(e.path.as_str(), (gi, e));
            }
        }
        Self {
            groups,
            entry,
            root,
            playable: HashMap::new(),
            anchor: HashMap::new(),
            numbered: HashMap::new(),
        }
    }

    /// The plan of the video at `path`.
    pub fn plan(&mut self, path: &str) -> Option<FilePlan> {
        let Some(&(gi, entry)) = self.entry.get(path) else {
            return own_plan(path, false);
        };
        let groups = self.groups;
        let group = &groups[gi];
        match entry.role {
            Role::Main | Role::Part => {
                let role = if entry.role == Role::Part {
                    PlanRole::Part
                } else {
                    PlanRole::Main
                };
                let joined = group.kind == GroupKind::Disc
                    || (entry.role == Role::Part && group.kind == GroupKind::MovieFolder);
                if joined {
                    return self.anchor(gi).map(|mut a| {
                        a.role = role;
                        a
                    });
                }
                if let Some(p) = self.numbered(gi).get(path) {
                    return Some(p.clone());
                }
                match own_plan(path, false) {
                    Some(mut p) => {
                        p.role = role;
                        self.add_nfo_ids(gi, Some(path), &mut p);
                        Some(p)
                    }
                    // The parser says extra: the group's work.
                    None => self.extra(gi, path),
                }
            }
            Role::Extra | Role::Sample => self.extra(gi, path),
            _ => None,
        }
    }

    /// The plan of an extra of group `gi`.
    fn extra(&mut self, gi: usize, path: &str) -> Option<FilePlan> {
        if let Some(named) = own_plan(path, true) {
            let key = named.work.key();
            let same = self
                .playable(gi)
                .iter()
                .flatten()
                .find(|p| p.work.key() == key)
                .cloned();
            if let Some(mut p) = same {
                self.add_nfo_ids(gi, None, &mut p);
                return Some(p.as_extra());
            }
        }
        if self.groups[gi].kind == GroupKind::Loose {
            return None;
        }
        self.anchor(gi).map(|a| a.as_extra())
    }

    /// A folder whose videos all start with a running number and say nothing
    /// else (`01 The Legend Begins.m4v` … `43 Final Battle.m4v`) is one
    /// series named by the folder: each file is the episode of its number,
    /// the rest of the name its title (META-032). A year or the folder's
    /// title in the file names (`1. Saga Part One (2001).mkv`) means a
    /// collection of films, and the folder is left alone.
    fn numbered(&mut self, gi: usize) -> &HashMap<String, FilePlan> {
        if !self.numbered.contains_key(&gi) {
            let found = self.numbered_plans(gi).unwrap_or_default();
            self.numbered.insert(gi, found);
        }
        &self.numbered[&gi]
    }

    fn numbered_plans(&mut self, gi: usize) -> Option<HashMap<String, FilePlan>> {
        let groups = self.groups;
        let group = &groups[gi];
        if group.kind == GroupKind::Disc {
            return None;
        }
        let entries: Vec<&GroupEntry> = group
            .entries
            .iter()
            .filter(|e| e.role == Role::Main)
            .collect();
        if entries.len() < 2 {
            return None;
        }
        let folder = parse::folder_work(&group.root);
        let show = folder.title.clone().filter(|t| !t.trim().is_empty())?;
        let show_norm = norm(&show);
        let mut seen = std::collections::HashSet::new();
        let mut numbered = Vec::with_capacity(entries.len());
        let mut named_after_folder = 0;
        for e in &entries {
            let (n, written) = leading_number(file_stem(&e.path))?;
            if !show_norm.is_empty() && norm(written).contains(&show_norm) {
                named_after_folder += 1;
            }
            if !seen.insert(n) {
                return None;
            }
            let own = own_plan(&e.path, false)?;
            if own.work.year.is_some() {
                return None;
            }
            let rest = if own.work.is_series() {
                // `01. Title` read as episode 1 already: same number only.
                if own.episodes.first().and_then(|x| x.episode) != Some(i64::from(n)) {
                    return None;
                }
                own.episode_title.clone()
            } else {
                leading_number(&own.work.title).map(|(_, rest)| rest.to_owned())
            }
            .filter(|r| !r.is_empty());
            numbered.push((e.path.as_str(), n, rest, own.confidence));
        }
        if named_after_folder * 2 >= numbered.len() {
            return None;
        }
        let season = folder.season.map(i64::from).or(Some(1));
        let mut out = HashMap::with_capacity(numbered.len());
        for (path, n, rest, confidence) in numbered {
            let mut plan = FilePlan {
                work: WorkPlan {
                    kind: if folder.anime { "anime" } else { "show" },
                    title: show.clone(),
                    year: folder.year.map(i64::from),
                    ids: BTreeMap::new(),
                    alt_titles: Vec::new(),
                },
                role: PlanRole::Main,
                episodes: vec![EpisodeNo {
                    season,
                    episode: Some(i64::from(n)),
                    absolute: None,
                    kind: "regular",
                }],
                episode_title: rest,
                confidence,
            };
            self.add_nfo_ids(gi, Some(path), &mut plan);
            out.insert(path.to_owned(), plan);
        }
        Some(out)
    }

    /// Own-name plans of the playable entries of group `gi`, in path order.
    fn playable(&mut self, gi: usize) -> &[Option<FilePlan>] {
        let groups = self.groups;
        self.playable.entry(gi).or_insert_with(|| {
            let mut entries: Vec<&GroupEntry> = groups[gi].playable().collect();
            entries.sort_by(|a, b| a.path.cmp(&b.path));
            entries.iter().map(|e| own_plan(&e.path, false)).collect()
        })
    }

    /// The work of group `gi`: the disc's name, or the work most of its
    /// videos are (the first in path order on a tie), with NFO ids.
    fn anchor(&mut self, gi: usize) -> Option<FilePlan> {
        if let Some(a) = self.anchor.get(&gi) {
            return a.clone();
        }
        let groups = self.groups;
        let group = &groups[gi];
        let found = if group.kind == GroupKind::Disc {
            own_plan(&disc_name(group), true)
        } else {
            let plans = self.playable(gi);
            let mut count: HashMap<String, (usize, usize)> = HashMap::new();
            for (i, p) in plans.iter().enumerate() {
                if let Some(p) = p {
                    count.entry(p.work.key()).or_insert((0, i)).0 += 1;
                }
            }
            count
                .values()
                .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)))
                .and_then(|&(_, i)| plans[i].clone())
        };
        let found = found.map(|mut p| {
            p.role = PlanRole::Main;
            self.add_nfo_ids(gi, None, &mut p);
            p
        });
        self.anchor.insert(gi, found.clone());
        found
    }

    /// Adds the NFO ids that apply to `plan` of a video in group `gi`
    /// (`target`: its path, for `<video>.nfo`).
    fn add_nfo_ids(&self, gi: usize, target: Option<&str>, plan: &mut FilePlan) {
        let series = plan.work.is_series();
        let group = &self.groups[gi];
        let mut ids = BTreeMap::new();
        for e in group.entries.iter().filter(|e| e.role == Role::Nfo) {
            let show_nfo = file_name(&e.path).eq_ignore_ascii_case("tvshow.nfo");
            let applies = if show_nfo {
                series
            } else if let Some(t) = &e.target {
                !series && target == Some(t.as_str())
            } else {
                !series && matches!(group.kind, GroupKind::MovieFolder | GroupKind::Disc)
            };
            if applies {
                merge_ids(&mut ids, &e.ids, series);
            }
        }
        if series {
            for dir in parents(&group.root) {
                let Some(&ai) = self.root.get(dir) else {
                    continue;
                };
                for e in self.groups[ai].entries.iter().filter(|e| {
                    e.role == Role::Nfo && file_name(&e.path).eq_ignore_ascii_case("tvshow.nfo")
                }) {
                    merge_ids(&mut ids, &e.ids, true);
                }
            }
        }
        if !ids.is_empty() {
            plan.work.ids = ids;
            plan.confidence = 1.0;
        }
    }
}

/// Adds NFO `from` ids to `ids` (first one wins); a TMDB id of the other
/// kind (`tv:` for a movie) is dropped.
fn merge_ids(ids: &mut BTreeMap<String, String>, from: &BTreeMap<String, String>, series: bool) {
    let kind = if series { "tv" } else { "movie" };
    for (provider, v) in from {
        if !NFO_PROVIDERS.contains(&provider.as_str()) || v.trim().is_empty() {
            continue;
        }
        let v = v.trim();
        let v = if provider == "tmdb" {
            match v.split_once(':') {
                Some((k, _)) if k != kind => continue,
                Some(_) => v.to_owned(),
                None => format!("{kind}:{v}"),
            }
        } else {
            v.to_owned()
        };
        ids.entry(provider.clone()).or_insert(v);
    }
}

/// Plans of `paths` (videos of one source) with the source's `groups`.
#[must_use]
pub fn plan_files(paths: &[&str], groups: &[Group]) -> Vec<Option<FilePlan>> {
    let mut planner = Planner::new(groups);
    paths.iter().map(|p| planner.plan(p)).collect()
}

/// Recognizes the video files of `source_id` without a live match. Returns
/// how many got one.
pub(crate) fn run(
    conn: &mut Connection,
    source_id: i64,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<usize, Error> {
    let groups: Vec<Group> = pa_library::groups::source_groups(conn, source_id)?
        .into_iter()
        .map(|g| g.group)
        .collect();
    let mut planner = Planner::new(&groups);
    replan_outdated(conn, source_id, &mut planner)?;
    let files: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(concat!(
            "SELECT f.id, f.path FROM files f
             WHERE f.source_id = ?1 AND f.missing_since_ms IS NULL
               AND NOT EXISTS (SELECT 1 FROM matches m WHERE m.file_id = f.id AND m.state <> 'rejected')
               -- Leftovers of a group (LIB-007): the other streams of a
               -- disc (LIB-017), a one-minute part 3. Never items.
               AND NOT EXISTS (SELECT 1 FROM group_assets ga
                               WHERE ga.file_id = f.id AND ga.role = 'ignore')
               -- LIB-025: only titled videos; own footage, decided by
               -- `content_kind`, is never recognized. Personal media
               -- (UI-025) never is either.
               AND NOT EXISTS (SELECT 1 FROM file_content_kinds k
                               WHERE k.file_id = f.id AND k.kind <> 'video')
               AND NOT ",
            pa_library::file_personal!("f"),
            " ORDER BY f.path"
        ))?;
        stmt.query_map([source_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .filter(|r| {
                r.as_ref().map_or(true, |(_, p): &(i64, String)| {
                    pa_scanner::media_kind(p) == Some(MediaKind::Video)
                })
            })
            .collect::<Result<_, _>>()?
    };
    let total = files.len();
    if total == 0 {
        return Ok(0);
    }
    let mut index = WorkIndex::load(conn)?;
    let mut matched = 0;
    for (n, batch) in files.chunks(BATCH).enumerate() {
        let tx = conn.transaction()?;
        for (file_id, path) in batch {
            let plan = planner.plan(path);
            if recognize_file(&tx, &mut index, *file_id, path, plan)? {
                matched += 1;
            }
        }
        tx.commit()?;
        on_progress(((n + 1) * BATCH).min(total), total);
    }
    Ok(matched)
}

/// Automatic decisions made by an older parser (META-032): every file of
/// `source_id` whose live match is automatic (`rule` or `scorer`, no
/// folder rule) and whose parse is older than [`parse::PARSER_VERSION`] is
/// planned again. The match is dropped, so [`run`] recognises the file
/// anew, when the plan now says something else: a movie that is an episode
/// (or the other way round), episodes of another season (shows; anime
/// episodes are renumbered by the metadata run), or another title for a
/// work no provider matched (site junk in the old title). Otherwise only
/// the parse result is brought up to date. User and NFO decisions are never
/// touched. Works left without files go with `remove_orphans`.
fn replan_outdated(
    conn: &mut Connection,
    source_id: i64,
    planner: &mut Planner<'_>,
) -> Result<(), Error> {
    struct Old {
        file_id: i64,
        path: String,
        match_id: i64,
        kind: String,
        title: String,
        provider_matched: bool,
        seasons: Vec<i64>,
    }
    let old: Vec<Old> = {
        let mut stmt = conn.prepare(concat!(
            "SELECT f.id, f.path, m.id, w.kind, w.title,
                    EXISTS (SELECT 1 FROM work_metadata wm
                            WHERE wm.work_id = w.id AND wm.outcome = 'matched'),
                    (SELECT group_concat(s, ',') FROM (
                        SELECT DISTINCT ifnull(e.season_number, -1) AS s
                        FROM match_episodes me JOIN episodes e ON e.id = me.episode_id
                        WHERE me.match_id = m.id ORDER BY 1))
             FROM files f
             JOIN parse_results pr ON pr.file_id = f.id
             JOIN matches m ON m.file_id = f.id
             JOIN works w ON w.id = m.work_id
             WHERE f.source_id = ?1 AND f.missing_since_ms IS NULL
               AND pr.parser_version <> ?2
               AND m.decided_by IN ('rule', 'scorer')
               AND NOT ",
            pa_library::file_personal!("f"),
            " AND ",
            crate::numbering::recomputable!(),
            " ORDER BY f.path"
        ))?;
        stmt.query_map(params![source_id, parse::PARSER_VERSION], |r| {
            let seasons: Option<String> = r.get(6)?;
            Ok(Old {
                file_id: r.get(0)?,
                path: r.get(1)?,
                match_id: r.get(2)?,
                kind: r.get(3)?,
                title: r.get(4)?,
                provider_matched: r.get(5)?,
                seasons: seasons
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|s| s.parse().ok())
                    .collect(),
            })
        })?
        .collect::<Result<_, _>>()?
    };
    if old.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    for o in old {
        let changed = match planner.plan(&o.path) {
            None => false,
            Some(plan) => {
                let was_series = o.kind != "movie";
                let mut seasons: Vec<i64> = plan
                    .episodes
                    .iter()
                    .map(|e| e.season.unwrap_or(-1))
                    .collect();
                seasons.sort_unstable();
                seasons.dedup();
                was_series != plan.work.is_series()
                    || (o.kind == "show" && plan.work.is_series() && seasons != o.seasons)
                    || (!o.provider_matched
                        && TitleKey::new(&o.title).loose != TitleKey::new(&plan.work.title).loose)
            }
        };
        if changed {
            tx.execute("DELETE FROM matches WHERE id = ?1", [o.match_id])?;
        } else {
            tx.execute(
                "UPDATE parse_results SET parser_version = ?2 WHERE file_id = ?1",
                params![o.file_id, parse::PARSER_VERSION],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn recognize_file(
    tx: &Transaction<'_>,
    index: &mut WorkIndex,
    file_id: i64,
    path: &str,
    plan: Option<FilePlan>,
) -> Result<bool, Error> {
    let now = crate::now_ms();
    let result = parse::parse(path);
    let hypotheses = serde_json::to_string(&result.hypotheses)
        .map_err(|e| Error::Invalid(format!("parse result: {e}")))?;
    tx.execute(
        "INSERT INTO parse_results (file_id, parser_version, hypotheses_json, parsed_at_ms)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (file_id) DO UPDATE SET parser_version = excluded.parser_version,
             hypotheses_json = excluded.hypotheses_json, parsed_at_ms = excluded.parsed_at_ms",
        params![file_id, result.parser_version, hypotheses, now],
    )?;
    let Some(plan) = plan else {
        return Ok(false);
    };
    let version = Some(result.parser_version.as_str());
    let work = index.ensure_plan(tx, &plan.work, now)?;
    if plan.work.is_series()
        && let Some(y) = plan.work.year.filter(|y| (1800..=3000).contains(y))
    {
        tx.execute(
            "UPDATE works SET year = ?2 WHERE id = ?1 AND year IS NULL",
            params![work, y],
        )?;
    }
    let single = plan.episodes.len() == 1;
    let mut eps = Vec::with_capacity(plan.episodes.len());
    for no in &plan.episodes {
        let t = if single {
            plan.episode_title.as_deref()
        } else {
            None
        };
        eps.push(ensure_episode(tx, work, *no, t)?);
    }
    insert_match(
        tx,
        file_id,
        work,
        &eps,
        plan.confidence,
        "auto",
        plan.decided_by(),
        version,
        now,
    )
}

/// Deletes works without any match and episodes without any file, after
/// files were removed. Works the user touched (a `user_*` match, which
/// cascades away with its file) go too: their files are gone.
pub(crate) fn remove_orphans(conn: &Connection) -> Result<(), Error> {
    conn.execute(
        "DELETE FROM works WHERE NOT EXISTS (SELECT 1 FROM matches m WHERE m.work_id = works.id)",
        [],
    )?;
    conn.execute(
        "DELETE FROM episodes WHERE NOT EXISTS
             (SELECT 1 FROM match_episodes me WHERE me.episode_id = episodes.id)",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use pa_scanner::structure::{ListedFile, StructureOptions, analyze};

    use super::*;

    fn plan_of(path: &str) -> FilePlan {
        own_plan(path, false).expect("a plan")
    }

    fn groups(paths: &[&str]) -> Vec<Group> {
        let files: Vec<ListedFile> = paths
            .iter()
            .map(|p| ListedFile::new(*p, 1_000_000))
            .collect();
        analyze(&files, &StructureOptions::default())
    }

    #[test]
    fn movie_and_episode_plans() {
        let p = plan_of("Movies/Dune (2021)/Dune.2021.2160p.mkv");
        assert_eq!(
            (p.work.kind, p.work.title.as_str(), p.work.year),
            ("movie", "Dune", Some(2021))
        );
        assert!(p.episodes.is_empty());
        let p = plan_of("Shows/Dark/Season 1/Dark.S01E02.Lies.1080p.mkv");
        assert_eq!((p.work.kind, p.work.title.as_str()), ("show", "Dark"));
        assert_eq!(p.episodes[0].season, Some(1));
        assert_eq!(p.episodes[0].episode, Some(2));
        assert_eq!(p.episodes[0].kind, "regular");
        assert_eq!(p.decided_by(), "rule");
    }

    #[test]
    fn folder_casing_wins_over_a_lowercase_file_name() {
        let p = plan_of("Movies/Inception (2010)/inception.mkv");
        assert_eq!(
            (p.work.title.as_str(), p.work.year),
            ("Inception", Some(2010))
        );
    }

    #[test]
    fn work_keys_fold_case_and_punctuation() {
        assert_eq!(
            work_key(true, "The Office", None),
            work_key(true, "the office!", None)
        );
        assert_ne!(
            work_key(false, "Dune", Some(1984)),
            work_key(false, "Dune", Some(2021))
        );
    }

    #[test]
    fn a_disc_is_one_item_named_by_its_folder() {
        let paths = [
            "/Movies/Inception (2010)/BDMV/STREAM/00000.m2ts",
            "/Movies/Inception (2010)/BDMV/STREAM/00001.m2ts",
            "/Movies/Inception (2010)/BDMV/index.bdmv",
        ];
        // The feature is the largest stream; the menu clip is a leftover
        // (LIB-017).
        let files: Vec<ListedFile> = paths
            .iter()
            .zip([4_500_000, 1_332_000_000, 300])
            .map(|(p, size)| ListedFile::new(*p, size))
            .collect();
        let g = analyze(&files, &StructureOptions::default());
        let plans = plan_files(&paths[..2], &g);
        assert_eq!(plans[0], None, "the menu clip is no item");
        let p = plans[1].as_ref().expect("main title planned");
        assert_eq!(
            (p.work.kind, p.work.title.as_str(), p.work.year, p.role),
            ("movie", "Inception", Some(2010), PlanRole::Main)
        );
        let (works, of) = group_works(&plans);
        assert_eq!(works.len(), 1);
        assert_eq!(of, [None, Some(0)]);
    }

    #[test]
    fn a_dvd_title_set_and_an_image_are_one_item_each() {
        let paths = [
            "/Movies/Alien (1979)/VIDEO_TS/VIDEO_TS.VOB",
            "/Movies/Alien (1979)/VIDEO_TS/VTS_01_1.VOB",
            "/Movies/Alien (1979)/VIDEO_TS/VTS_01_2.VOB",
            "/Images/Spirited Away (2001).iso",
        ];
        let plans = plan_files(&paths, &groups(&paths));
        assert_eq!(plans[0], None, "the DVD menu is no item");
        let got: Vec<_> = plans[1..]
            .iter()
            .map(|p| {
                let p = p.as_ref().unwrap();
                (p.work.title.as_str(), p.work.year, p.role)
            })
            .collect();
        assert_eq!(
            got,
            [
                ("Alien", Some(1979), PlanRole::Part),
                ("Alien", Some(1979), PlanRole::Part),
                ("Spirited Away", Some(2001), PlanRole::Main),
            ]
        );
        let (works, _) = group_works(&plans);
        assert_eq!(works.len(), 2);
    }

    #[test]
    fn split_files_join_the_work_of_their_folder() {
        let paths = [
            "/Movies/12 Angry Men (1957)/12.Angry.Men.1957.DVDRip.CD1.avi",
            "/Movies/12 Angry Men (1957)/12.Angry.Men.1957.DVDRip.CD2.avi",
        ];
        let plans = plan_files(&paths, &groups(&paths));
        let roles: Vec<_> = plans.iter().map(|p| p.as_ref().unwrap().role).collect();
        assert_eq!(roles, [PlanRole::Part, PlanRole::Part]);
        let (works, _) = group_works(&plans);
        assert_eq!(works.len(), 1);
        assert_eq!(works[0].title, "12 Angry Men");
    }

    #[test]
    fn extras_and_samples_attach_to_the_work_of_their_group() {
        let paths = [
            "/Movies/Dune (2021)/Dune (2021).mkv",
            "/Movies/Dune (2021)/Dune (2021)-trailer.mkv",
            "/Movies/Dune (2021)/Featurettes/Making of.mkv",
            "/Movies/Dune (2021)/sample.mkv",
        ];
        let plans = plan_files(&paths, &groups(&paths));
        let (works, of) = group_works(&plans);
        assert_eq!(works.len(), 1, "{works:?}");
        assert_eq!(of, [Some(0); 4]);
        assert_eq!(plans[0].as_ref().unwrap().role, PlanRole::Main);
        for p in &plans[1..] {
            let p = p.as_ref().unwrap();
            assert_eq!(p.role, PlanRole::Extra);
            assert!(p.episodes.is_empty());
        }
    }

    #[test]
    fn extras_of_a_show_join_the_show_without_episodes() {
        let paths = [
            "/Anime/Frieren/[Grp] Sousou no Frieren - 01 [1080p].mkv",
            "/Anime/Frieren/[Grp] Sousou no Frieren - 02 [1080p].mkv",
            "/Anime/Frieren/[Grp] Sousou no Frieren - NCOP [1080p].mkv",
        ];
        let plans = plan_files(&paths, &groups(&paths));
        let (works, of) = group_works(&plans);
        assert_eq!(works.len(), 1, "{works:?}");
        assert_eq!(of, [Some(0); 3]);
        let ncop = plans[2].as_ref().unwrap();
        assert_eq!(ncop.role, PlanRole::Extra);
        assert!(ncop.episodes.is_empty());
    }

    fn with_nfo(mut g: Vec<Group>, nfo: &str, ids: &[(&str, &str)]) -> Vec<Group> {
        let e = g
            .iter_mut()
            .flat_map(|g| g.entries.iter_mut())
            .find(|e| e.path == nfo)
            .expect("nfo entry");
        e.ids = ids
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        g
    }

    #[test]
    fn nfo_ids_decide_movies() {
        let paths = [
            "/Movies/Dune/Dune.mkv",
            "/Movies/Dune/movie.nfo",
            "/Movies/Dune/Dune-trailer.mkv",
        ];
        let g = with_nfo(
            groups(&paths),
            "/Movies/Dune/movie.nfo",
            &[
                ("tmdb", "movie:438631"),
                ("imdb", "tt1160419"),
                ("tvdb", ""),
            ],
        );
        let plans = plan_files(&[paths[0], paths[2]], &g);
        let main = plans[0].as_ref().unwrap();
        assert_eq!(main.decided_by(), "nfo");
        assert_eq!(main.confidence, 1.0);
        assert_eq!(
            main.work.external_ids(),
            [
                ("imdb".into(), String::new(), "tt1160419".into()),
                ("tmdb".into(), "movie".into(), "438631".into()),
            ]
        );
        let extra = plans[1].as_ref().unwrap();
        assert_eq!((extra.role, extra.decided_by()), (PlanRole::Extra, "nfo"));
    }

    #[test]
    fn tvshow_nfo_of_a_parent_folder_decides_episodes() {
        let paths = [
            "/TV/Dark/tvshow.nfo",
            "/TV/Dark/fanart.jpg",
            "/TV/Dark/Season 1/Dark.S01E01.mkv",
            "/TV/Dark/Season 1/Dark.S01E02.mkv",
        ];
        let g = with_nfo(groups(&paths), "/TV/Dark/tvshow.nfo", &[("tmdb", "70523")]);
        let plans = plan_files(&paths[2..], &g);
        for p in &plans {
            let p = p.as_ref().unwrap();
            assert_eq!(p.work.ids.get("tmdb").map(String::as_str), Some("tv:70523"));
        }
        // A TV id never decides a movie.
        let movie = ["/M/Heat/Heat.mkv", "/M/Heat/movie.nfo"];
        let g = with_nfo(groups(&movie), "/M/Heat/movie.nfo", &[("tmdb", "tv:1")]);
        let p = plan_files(&movie[..1], &g)[0].clone().unwrap();
        assert!(p.work.ids.is_empty());
    }

    #[test]
    fn nfo_ids_join_and_split_works() {
        let plan = |title: &str, tmdb: Option<&str>| {
            let mut p = plan_of(&format!("/M/{title}.mkv"));
            if let Some(id) = tmdb {
                p.work.ids.insert("tmdb".into(), id.into());
            }
            Some(p)
        };
        // Two names, one id: one work.
        let (works, of) = group_works(&[
            plan("Dune (2021)", Some("movie:438631")),
            plan("Dune Part One (2021)", Some("movie:438631")),
        ]);
        assert_eq!((works.len(), of), (1, vec![Some(0), Some(0)]));
        // One name, two ids: two works.
        let (works, _) = group_works(&[
            plan("Dune", Some("movie:841")),
            plan("Dune", Some("movie:438631")),
        ]);
        assert_eq!(works.len(), 2);
    }

    #[test]
    fn loose_extras_that_name_no_work_are_skipped() {
        let paths = [
            "/Dump/Heat.1995.mkv",
            "/Dump/Alien.1979.mkv",
            "/Dump/Trailer-trailer.mkv",
        ];
        let g = groups(&paths);
        let plans = plan_files(&paths, &g);
        if g.iter()
            .any(|g| g.kind == GroupKind::Loose && g.entry(paths[2]).is_some())
        {
            assert!(plans[2].is_none(), "{:?}", plans[2]);
        }
        assert!(plans[0].is_some() && plans[1].is_some());
    }

    #[test]
    fn parents_walk_up_to_the_root() {
        assert_eq!(
            parents("/TV/Dark/Season 1").collect::<Vec<_>>(),
            ["/TV/Dark", "/TV", "/"]
        );
        assert_eq!(parents("TV/Dark").collect::<Vec<_>>(), ["TV"]);
    }

    type Numbers = Vec<(Option<i64>, Option<i64>)>;

    fn summary(p: &FilePlan) -> (&'static str, &str, Numbers) {
        (
            p.work.kind,
            p.work.title.as_str(),
            p.episodes.iter().map(|e| (e.season, e.episode)).collect(),
        )
    }

    #[test]
    fn a_folder_of_numbered_files_is_one_series() {
        let paths = [
            "/Anime/Some Teacher/Season 1/01 The Lesson Begins.m4v",
            "/Anime/Some Teacher/Season 1/02 Enter the Rival.m4v",
            "/Anime/Some Teacher/Season 1/10 Destination_ Hokkaido.m4v",
        ];
        let plans = plan_files(&paths, &groups(&paths));
        let p: Vec<FilePlan> = plans.into_iter().map(Option::unwrap).collect();
        assert_eq!(
            summary(&p[0]),
            ("anime", "Some Teacher", vec![(Some(1), Some(1))])
        );
        assert_eq!(p[0].episode_title.as_deref(), Some("The Lesson Begins"));
        assert_eq!(summary(&p[2]).2, [(Some(1), Some(10))]);
        assert_eq!(p[2].episode_title.as_deref(), Some("Destination Hokkaido"));
        let (works, _) = group_works(&p.into_iter().map(Some).collect::<Vec<_>>());
        assert_eq!(works.len(), 1);

        // Without a season folder: season 1, kind from the folder.
        let paths = [
            "/TV/Night Club/1. Opening.mkv",
            "/TV/Night Club/2. Closing.mkv",
        ];
        let plans = plan_files(&paths, &groups(&paths));
        assert_eq!(
            summary(plans[1].as_ref().unwrap()),
            ("show", "Night Club", vec![(Some(1), Some(2))])
        );
    }

    #[test]
    fn numbered_films_stay_films() {
        // A year: a collection.
        let paths = [
            "/Saga/1 Saga Begins (2001).mkv",
            "/Saga/2 Saga Returns (2003).mkv",
        ];
        let plans = plan_files(&paths, &groups(&paths));
        assert!(plans.iter().flatten().all(|p| p.work.kind == "movie"));
        // The folder's title in the names: a collection too.
        let paths = [
            "/Star Quest/1. Star Quest The Beginning.mkv",
            "/Star Quest/2. Star Quest Into the Void.mkv",
        ];
        let plans = plan_files(&paths, &groups(&paths));
        assert!(
            plans.iter().flatten().all(|p| p.work.kind == "movie"),
            "{plans:?}"
        );
        // One numbered file is not a series.
        let paths = ["/Films/12 Lanterns.mkv"];
        let plans = plan_files(&paths, &groups(&paths));
        assert_eq!(plans[0].as_ref().unwrap().work.kind, "movie");
    }

    #[test]
    fn sequel_numbers_and_parts_are_seasons() {
        let p = plan_of("/Видео/Хроника волшебницы 2/Хроника волшебницы 2 s1e10.mp4");
        assert_eq!(
            summary(&p),
            ("show", "Хроника волшебницы", vec![(Some(2), Some(10))])
        );
        assert_eq!(p.work.alt_titles, ["Хроника волшебницы 2"]);
        let p = plan_of("/Стражи/Стражи 2/Стражи 2 - S02E01.mp4");
        assert_eq!(summary(&p), ("show", "Стражи", vec![(Some(2), Some(1))]));
        let p = plan_of("/Ворота/Ворота Там стоят стражи Часть 2 s1e3.mp4");
        assert_eq!(
            summary(&p),
            ("show", "Ворота Там стоят стражи", vec![(Some(2), Some(3))])
        );
        // A folder season wins over the `s1` sites write for every season.
        let p = plan_of("/Show season 2/Show s1e4.mp4");
        assert_eq!(summary(&p).2, [(Some(2), Some(4))]);
        // A number that is part of the title stays: out of range, other
        // season, or no written season (anime entry names, META-015).
        assert_eq!(plan_of("/Babylon 5 S01E01.mkv").work.title, "Babylon 5");
        assert_eq!(plan_of("/Show 2 S03E01.mkv").work.title, "Show 2");
        assert_eq!(plan_of("/Some Anime 2 - 05.mkv").work.title, "Some Anime 2");
    }
}
