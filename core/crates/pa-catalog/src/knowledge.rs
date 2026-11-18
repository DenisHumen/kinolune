// SPDX-License-Identifier: MPL-2.0
//! Knowledge out to the files (LIB-014, ADR-0122): on the user's command
//! the library writes what it knows about a work next to its files, so
//! another device, a reinstall or another player (Jellyfin, Kodi, Plex)
//! gets it without guessing again.
//!
//! - **NFO** ([`Catalog::export_nfo`]): Kodi/Jellyfin XML with the title,
//!   original title, year, runtime, plot, genres and the provider ids
//!   (`<uniqueid type="tmdb" default="true">`, plus the old `<tmdbid>` /
//!   `<imdbid>` / `<tvdbid>` tags). Where it goes:
//!   - a movie in its own folder or a disc folder: `movie.nfo` there;
//!   - a movie among other files (and a disc image): `<video name>.nfo`
//!     next to it;
//!   - a show: `tvshow.nfo` in the show's folder (the folder above
//!     `Season 1/`), never in the source root or a dump folder.
//! - **Id in the name** ([`Catalog::rename_with_ids`]): the folder of a
//!   movie or show gets ` [tmdbid-603]` (or `[imdbid-tt…]`, `[tvdbid-…]`)
//!   at the end of its name — the tag Jellyfin, Kodi scrapers and
//!   Kinolune itself read. Only a folder that belongs to the work alone;
//!   loose files keep their names (their subtitles and other sidecars
//!   would lose them).
//!
//! **Safety.** Everything goes through the source-write core of NET-023
//! (`pa_vfs::files`, ADR-0104): read-only sources are left out by the
//! write probe; an existing NFO is never overwritten silently — without
//! `replace` it is skipped, with `replace` (after the app asked) it goes
//! to the source's trash first (`Conflict::Replace`); a rename never
//! replaces anything (`Conflict::Skip`). The library is refreshed only
//! in the touched folders (`refresh_after_write`); a renamed folder keeps
//! its files' matches and progress (LIB-004 follows moved files).
//!
//! **Import** needs nothing here: the scan reads NFO ids (LIB-007) and ids
//! in names (`pa_scanner::structure::ids_in_name`, used by
//! [`crate::recognize`]).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::io::Cursor;
use std::sync::Arc;

use pa_library::groups::{self, DiscUnit, GroupKind, Role};
use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_recognition::title::TitleKey;
use pa_scanner::CancelToken;
use pa_scanner::structure::{ids_in_name, is_season_folder};
use pa_vfs::files::{self, Conflict, Touched};
use pa_vfs::{Vfs, VfsPath};

use crate::{Catalog, Error, ItemRef, sources};

/// One NFO file the export writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NfoTarget {
    pub source_id: i64,
    /// Path in the source (`/Movies/Dune (2021)/movie.nfo`).
    pub path: String,
    /// A file is there already (the app asks before replacing it).
    pub exists: bool,
}

/// One folder that gets the id at the end of its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameTarget {
    pub source_id: i64,
    /// The folder now (`/Movies/Dune (2021)`).
    pub path: String,
    /// Its new name (`Dune (2021) [tmdbid-438631]`).
    pub new_name: String,
}

/// What [`Catalog::knowledge_plan`] found for a work.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnowledgePlan {
    pub work_id: i64,
    /// Provider ids an NFO gets: `(provider, id)`, `tmdb` first.
    pub ids: Vec<(String, String)>,
    /// NFO files to write, on writable sources only.
    pub nfo: Vec<NfoTarget>,
    /// Folders to rename; empty without a TMDB/IMDb/TVDB id or when the
    /// folders carry one already.
    pub rename: Vec<RenameTarget>,
    /// Some files of the work are on a source that cannot be written (or
    /// cannot be reached now) and were left out.
    pub read_only: bool,
}

/// What [`Catalog::export_nfo`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NfoReport {
    /// NFO files written (new or replacing an old one).
    pub written: usize,
    /// Old NFO files moved to the source's trash.
    pub replaced: usize,
    /// Existing NFO files left alone (`replace` was false).
    pub skipped: usize,
}

/// The work as an NFO tells it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NfoWork {
    /// `movie`, `show` or `anime`.
    pub kind: String,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i64>,
    pub runtime_ms: Option<i64>,
    pub overview: Option<String>,
    pub genres: Vec<String>,
    /// `(provider, id)`, the default (first) one first.
    pub ids: Vec<(String, String)>,
}

impl NfoWork {
    fn is_series(&self) -> bool {
        self.kind != "movie"
    }
}

/// The NFO XML of `work` (Kodi/Jellyfin, UTF-8).
#[must_use]
pub fn nfo_xml(work: &NfoWork) -> String {
    let root = if work.is_series() { "tvshow" } else { "movie" };
    let mut x = String::from("<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\n");
    let _ = writeln!(x, "<{root}>");
    let mut tag = |name: &str, value: &str| {
        if !value.trim().is_empty() {
            let _ = writeln!(x, "  <{name}>{}</{name}>", escape(value.trim()));
        }
    };
    tag("title", &work.title);
    if let Some(o) = work.original_title.as_deref().filter(|o| *o != work.title) {
        tag("originaltitle", o);
    }
    if let Some(y) = work.year {
        tag("year", &y.to_string());
    }
    if let Some(ms) = work.runtime_ms.filter(|ms| *ms > 0) {
        tag("runtime", &((ms + 30_000) / 60_000).to_string());
    }
    if let Some(p) = &work.overview {
        tag("plot", p);
    }
    for g in &work.genres {
        tag("genre", g);
    }
    for (provider, id) in &work.ids {
        let legacy = match provider.as_str() {
            "tmdb" => Some("tmdbid"),
            "imdb" => Some("imdbid"),
            "tvdb" => Some("tvdbid"),
            "anidb" => Some("anidbid"),
            _ => None,
        };
        if let Some(t) = legacy {
            tag(t, id);
        }
    }
    for (i, (provider, id)) in work.ids.iter().enumerate() {
        let default = if i == 0 { " default=\"true\"" } else { "" };
        let _ = writeln!(
            x,
            "  <uniqueid type=\"{}\"{default}>{}</uniqueid>",
            escape(provider),
            escape(id)
        );
    }
    let _ = writeln!(x, "</{root}>");
    x
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 forbids most control characters.
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\t' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// The tag a folder gets: `[tmdbid-603]`, `[imdbid-tt0133093]`,
/// `[tvdbid-81189]` (the Jellyfin form; Kinolune reads the Plex
/// `{tmdb-603}` too).
#[must_use]
pub fn id_tag(provider: &str, id: &str) -> String {
    format!("[{provider}id-{id}]")
}

/// Where a work's files are, as the export sees them.
#[derive(Debug, Default)]
struct Places {
    /// `(source, dir, file name)` of NFO files.
    nfo: BTreeSet<(i64, String, String)>,
    /// `(source, folder)` that belong to the work alone.
    folders: BTreeSet<(i64, String)>,
}

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn parent_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/".to_owned(),
        Some((p, _)) => p.to_owned(),
    }
}

fn stem_of(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

fn join(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
}

impl Catalog {
    /// What the export would do for `item`'s work (an episode: its show):
    /// its ids, the NFO files (and which exist already) and the folders
    /// that would get the id in their name. Sources are probed for writes
    /// (NET-023), so this reaches them; a source that cannot be reached
    /// counts as read-only.
    ///
    /// # Errors
    ///
    /// Music or an unknown item; the database.
    pub fn knowledge_plan(&self, item: ItemRef) -> Result<KnowledgePlan, Error> {
        let work_id = self.work_of_item(item)?;
        let (work, places) = {
            let conn = self.lock();
            crate::profiles::require_visible(&conn, item)?;
            let work = nfo_work(&conn, work_id)?;
            let places = places(&conn, work_id, &work)?;
            (work, places)
        };
        let mut plan = KnowledgePlan {
            work_id,
            ids: work.ids.clone(),
            ..KnowledgePlan::default()
        };
        let mut vfs: HashMap<i64, Option<Arc<dyn Vfs>>> = HashMap::new();
        for (source, dir, name) in &places.nfo {
            let Some(v) = self.writable_vfs(*source, &mut vfs) else {
                plan.read_only = true;
                continue;
            };
            let path = join(dir, name);
            let exists = match v.stat(&VfsPath::parse(&path)?) {
                Ok(_) => true,
                Err(e) if e.kind() == pa_vfs::ErrorKind::NotFound => false,
                Err(e) => return Err(e.into()),
            };
            plan.nfo.push(NfoTarget {
                source_id: *source,
                path,
                exists,
            });
        }
        if let Some((provider, id)) = work
            .ids
            .iter()
            .find(|(p, _)| matches!(p.as_str(), "tmdb" | "imdb" | "tvdb"))
        {
            for (source, folder) in &places.folders {
                let name = name_of(folder);
                if !ids_in_name(name).is_empty() {
                    continue;
                }
                if self.writable_vfs(*source, &mut vfs).is_none() {
                    plan.read_only = true;
                    continue;
                }
                plan.rename.push(RenameTarget {
                    source_id: *source,
                    path: folder.clone(),
                    new_name: format!("{} {}", name.trim_end(), id_tag(provider, id)),
                });
            }
        }
        Ok(plan)
    }

    /// Writes the NFO files of `item`'s work ([`knowledge_plan`]). An
    /// existing file is skipped unless `replace`; with `replace` it goes
    /// to the source's trash first. The touched folders are re-read.
    ///
    /// [`knowledge_plan`]: Self::knowledge_plan
    ///
    /// # Errors
    ///
    /// Music or an unknown item, a work without ids, a write that failed.
    pub fn export_nfo(&self, item: ItemRef, replace: bool) -> Result<NfoReport, Error> {
        let plan = self.knowledge_plan(item)?;
        if plan.ids.is_empty() {
            return Err(Error::Invalid(
                "the work has no provider ids to write".to_owned(),
            ));
        }
        let xml = {
            let conn = self.lock();
            nfo_xml(&nfo_work(&conn, plan.work_id)?)
        };
        let mut report = NfoReport::default();
        let mut touched: BTreeMap<i64, Vec<Touched>> = BTreeMap::new();
        for t in &plan.nfo {
            if t.exists && !replace {
                report.skipped += 1;
                continue;
            }
            let vfs = self.connect_source(t.source_id)?;
            let path = VfsPath::parse(&t.path)?;
            let dir = path.parent().unwrap_or_else(VfsPath::root);
            let name = path.file_name().unwrap_or("movie.nfo").to_owned();
            let conflict = if replace {
                Conflict::Replace
            } else {
                Conflict::Skip
            };
            let outcome = files::upload(
                vfs.as_ref(),
                &dir,
                &name,
                &mut Cursor::new(xml.clone().into_bytes()),
                conflict,
                &mut |_: u64, _: u64| true,
            )?;
            if outcome.skipped {
                report.skipped += 1;
                continue;
            }
            report.written += 1;
            if t.exists {
                report.replaced += 1;
            }
            touched
                .entry(t.source_id)
                .or_default()
                .extend(outcome.touched);
        }
        self.refresh_touched(touched);
        Ok(report)
    }

    /// Adds the id to the names of the folders of `item`'s work
    /// ([`knowledge_plan`]); a taken name is skipped. Returns how many
    /// folders were renamed.
    ///
    /// [`knowledge_plan`]: Self::knowledge_plan
    ///
    /// # Errors
    ///
    /// Music or an unknown item, a rename that failed.
    pub fn rename_with_ids(&self, item: ItemRef) -> Result<usize, Error> {
        let plan = self.knowledge_plan(item)?;
        let mut renamed = 0;
        let mut touched: BTreeMap<i64, Vec<Touched>> = BTreeMap::new();
        for t in &plan.rename {
            let vfs = self.connect_source(t.source_id)?;
            let outcome = files::rename(
                vfs.as_ref(),
                &VfsPath::parse(&t.path)?,
                &t.new_name,
                Conflict::Skip,
            )?;
            if outcome.skipped {
                continue;
            }
            renamed += 1;
            touched
                .entry(t.source_id)
                .or_default()
                .extend(outcome.touched);
        }
        self.refresh_touched(touched);
        Ok(renamed)
    }

    fn connect_source(&self, source_id: i64) -> Result<Arc<dyn Vfs>, Error> {
        let source = {
            let conn = self.lock();
            sources::get(&conn, source_id)?
                .ok_or_else(|| Error::NotFound(format!("source {source_id}")))?
        };
        self.connector().connect(&source)
    }

    /// The source, connected, if it can be written; probed once per call.
    fn writable_vfs<'a>(
        &self,
        source_id: i64,
        cache: &'a mut HashMap<i64, Option<Arc<dyn Vfs>>>,
    ) -> Option<&'a dyn Vfs> {
        cache
            .entry(source_id)
            .or_insert_with(|| {
                let vfs = self.connect_source(source_id).ok()?;
                files::writability(vfs.as_ref())
                    .ok()
                    .filter(|w| *w)
                    .map(|_| vfs)
            })
            .as_deref()
    }

    /// The library update after the writes; a failure waits for the next
    /// scan (the files are written).
    fn refresh_touched(&self, touched: BTreeMap<i64, Vec<Touched>>) {
        for (source, t) in touched {
            if let Err(e) = self.refresh_after_write(source, &t, &CancelToken::new()) {
                tracing::warn!(
                    target: "pa_catalog::knowledge",
                    error = %e,
                    "library refresh after an NFO export or rename failed"
                );
            }
        }
    }
}

/// The work as its NFO tells it; ids of the work's type only.
fn nfo_work(conn: &Connection, work_id: i64) -> Result<NfoWork, Error> {
    let mut w = conn
        .query_row(
            "SELECT kind, title, original_title, year, runtime_ms, overview
             FROM works WHERE id = ?1",
            [work_id],
            |r| {
                Ok(NfoWork {
                    kind: r.get(0)?,
                    title: r.get(1)?,
                    original_title: r.get(2)?,
                    year: r.get(3)?,
                    runtime_ms: r.get(4)?,
                    overview: r.get(5)?,
                    genres: Vec::new(),
                    ids: Vec::new(),
                })
            },
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("work {work_id}")))?;
    let mut stmt =
        conn.prepare("SELECT DISTINCT name FROM work_genres WHERE work_id = ?1 ORDER BY name")?;
    w.genres = stmt
        .query_map([work_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let series = w.is_series();
    let mut stmt =
        conn.prepare("SELECT provider, ext_kind, ext_id FROM external_ids WHERE work_id = ?1")?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map([work_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    for provider in ["tmdb", "imdb", "tvdb", "anidb"] {
        let id = rows.iter().find_map(|(p, kind, id)| {
            let fits = match p.as_str() {
                "tmdb" => kind == if series { "tv" } else { "movie" },
                "tvdb" => series == (kind != "movie"),
                _ => true,
            };
            (p == provider && fits).then(|| id.clone())
        });
        if let Some(id) = id {
            w.ids.push((provider.to_owned(), id));
        }
    }
    Ok(w)
}

/// Where the NFO files of `work` go and which folders are its own.
fn places(conn: &Connection, work_id: i64, work: &NfoWork) -> Result<Places, Error> {
    let files: Vec<(i64, i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT f.id, f.source_id, f.path FROM matches m
             JOIN files f ON f.id = m.file_id
             WHERE m.work_id = ?1 AND m.state <> 'rejected' AND f.missing_since_ms IS NULL
             ORDER BY f.source_id, f.path",
        )?;
        stmt.query_map([work_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?
    };
    let keys = title_keys(conn, work_id)?;
    let mut out = Places::default();
    for (file_id, source, path) in files {
        let stored = groups::group_of_file(conn, file_id)?;
        let group = stored.as_ref().map(|s| &s.group);
        let role = group.and_then(|g| g.entry(&path)).map(|e| e.role);
        // Extras, samples, leftovers: the work's NFO is not about them.
        if group.is_some() && !matches!(role, Some(Role::Main | Role::Part)) {
            continue;
        }
        let kind = group.map(|g| g.kind);
        let dir = parent_of(&path);
        if work.is_series() {
            if matches!(kind, None | Some(GroupKind::Loose | GroupKind::Disc)) {
                continue;
            }
            let (folder, from_season) = if is_season_folder(name_of(&dir)) {
                (parent_of(&dir), true)
            } else {
                (dir, false)
            };
            if folder == "/" {
                continue;
            }
            // Above a season folder may be a folder of many shows: only a
            // folder named after the work (or tagged with an id) is its own.
            if from_season && !names_work(name_of(&folder), &keys) {
                continue;
            }
            out.nfo
                .insert((source, folder.clone(), "tvshow.nfo".to_owned()));
            out.folders.insert((source, folder));
            continue;
        }
        let g = group.filter(|_| matches!(kind, Some(GroupKind::MovieFolder | GroupKind::Disc)));
        match g {
            Some(g) if g.disc == Some(DiscUnit::Iso) => {
                let image = g.root.clone();
                let name = format!("{}.nfo", stem_of(name_of(&image)));
                out.nfo.insert((source, parent_of(&image), name));
            }
            Some(g) if g.root != "/" && !g.root.is_empty() => {
                out.nfo
                    .insert((source, g.root.clone(), "movie.nfo".to_owned()));
                out.folders.insert((source, g.root.clone()));
            }
            _ => {
                let name = format!("{}.nfo", stem_of(name_of(&path)));
                out.nfo.insert((source, dir, name));
            }
        }
    }
    Ok(out)
}

/// Loose title keys of every title of the work.
fn title_keys(conn: &Connection, work_id: i64) -> Result<Vec<String>, Error> {
    let mut stmt = conn.prepare(
        "SELECT title FROM works WHERE id = ?1
         UNION SELECT original_title FROM works WHERE id = ?1 AND original_title IS NOT NULL
         UNION SELECT text FROM titles WHERE work_id = ?1",
    )?;
    let titles: Vec<String> = stmt
        .query_map(params![work_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(titles
        .iter()
        .map(|t| TitleKey::new(t).loose)
        .filter(|k| !k.is_empty())
        .collect())
}

/// The folder `name` is named after the work (one of its title `keys`)
/// or carries an id.
fn names_work(name: &str, keys: &[String]) -> bool {
    if !ids_in_name(name).is_empty() {
        return true;
    }
    let Some(title) = pa_recognition::parse::folder_work(name).title else {
        return false;
    };
    let key = TitleKey::new(&title).loose;
    !key.is_empty() && keys.contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movie_nfo_reads_back_as_its_ids() {
        let w = NfoWork {
            kind: "movie".into(),
            title: "Дюна & <друзья>".into(),
            original_title: Some("Dune".into()),
            year: Some(2021),
            runtime_ms: Some(155 * 60_000),
            overview: Some("Пол Атрейдес…".into()),
            genres: vec!["фантастика".into()],
            ids: vec![
                ("tmdb".into(), "438631".into()),
                ("imdb".into(), "tt1160419".into()),
            ],
        };
        let xml = nfo_xml(&w);
        assert!(
            xml.contains("<title>Дюна &amp; &lt;друзья&gt;</title>"),
            "{xml}"
        );
        assert!(xml.contains("<runtime>155</runtime>"));
        assert!(xml.contains("<uniqueid type=\"tmdb\" default=\"true\">438631</uniqueid>"));
        let ids = pa_scanner::structure::parse_nfo_ids(&xml);
        assert_eq!(ids["tmdb"], "movie:438631");
        assert_eq!(ids["imdb"], "tt1160419");
    }

    #[test]
    fn show_nfo_and_tags() {
        let w = NfoWork {
            kind: "show".into(),
            title: "Dark".into(),
            ids: vec![
                ("tmdb".into(), "70523".into()),
                ("tvdb".into(), "334824".into()),
            ],
            ..NfoWork::default()
        };
        let xml = nfo_xml(&w);
        assert!(xml.contains("<tvshow>") && xml.contains("</tvshow>"));
        let ids = pa_scanner::structure::parse_nfo_ids(&xml);
        assert_eq!(
            (ids["tmdb"].as_str(), ids["tvdb"].as_str()),
            ("tv:70523", "334824")
        );
        assert_eq!(id_tag("tmdb", "70523"), "[tmdbid-70523]");
        assert_eq!(
            ids_in_name(&format!("Dark {}", id_tag("tmdb", "70523")))["tmdb"],
            "70523"
        );
    }

    #[test]
    fn paths() {
        assert_eq!(parent_of("/a.mkv"), "/");
        assert_eq!(parent_of("/M/a.mkv"), "/M");
        assert_eq!(join("/", "x.nfo"), "/x.nfo");
        assert_eq!(join("/M", "x.nfo"), "/M/x.nfo");
        assert_eq!(stem_of("Heat.1995.mkv"), "Heat.1995");
    }
}
