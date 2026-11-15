// SPDX-License-Identifier: MPL-2.0
//! File name templates of downloads and recordings (TWI-019, research 06
//! §4.3).
//!
//! A saved VOD or recording is one folder (ADR-0064): `<folder>/<base>/`
//! with `<base>.mkv`, `<base>.info.json`, `<base>-thumb.jpg` and
//! `<base>.chat.json.gz`. Templates set `<folder>` (may contain `/`) and the
//! head of `<base>`; the id suffix (` [v<video_id>]`, ` [s<stream_id>]`) is
//! always appended, so a file moved by hand is still found by its id
//! (import, deduplication).
//!
//! # Syntax
//!
//! - `{name}` — a variable: `date` (`YYYY-MM-DD`), `year`, `month`, `time`
//!   (`HH-MM`, UTC), `login`, `display_name`, `user_id`, `title`,
//!   `categories` (`, `-separated), `part` (chapter of a partial download),
//!   `id` (video or stream id), `type` (`vod`, `live`).
//! - `<…>` — an optional group: dropped with its text when a variable in it
//!   is empty. `<` and `>` cannot appear in a Windows file name, so they
//!   are free as syntax.
//! - Every path component is sanitized for Windows, SMB, macOS and Linux
//!   ([`sanitize`]) after rendering; the base name with its suffix is at
//!   most [`MAX_NAME`] bytes (the title is cut, never the id).
//!
//! Defaults follow 06 §4.3: `{login} [{user_id}]/{year}` and
//! `{date} <{categories} — >{title}` (`2026-11-12 Elden Ring, Hades II —
//! Title [v2290012345]`), recordings `{date} LIVE <{categories} — >{title}`.

use serde::{Deserialize, Serialize};

use super::meta::{MAX_NAME, sanitize};

/// Default folder of a channel's files.
pub const DEFAULT_FOLDER: &str = "{login} [{user_id}]/{year}";
/// Default base name of a downloaded VOD.
pub const DEFAULT_VOD: &str = "{date} <{categories} — >{title}";
/// Default base name of a recording.
pub const DEFAULT_LIVE: &str = "{date} LIVE <{categories} — >{title}";

/// Variables a template may use.
pub const VARIABLES: &[&str] = &[
    "date",
    "year",
    "month",
    "time",
    "login",
    "display_name",
    "user_id",
    "title",
    "categories",
    "part",
    "id",
    "type",
];

/// Longest template we accept, bytes.
const MAX_TEMPLATE: usize = 400;

/// Name templates of a storage target's files (job parameter `naming`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Naming {
    /// Folder of the item folders, `/`-separated.
    #[serde(default = "default_folder")]
    pub folder: String,
    /// Base name of a downloaded VOD (before ` [v<id>]`).
    #[serde(default = "default_vod")]
    pub vod: String,
    /// Base name of a recording (before ` [s<id>]`).
    #[serde(default = "default_live")]
    pub live: String,
}

fn default_folder() -> String {
    DEFAULT_FOLDER.to_owned()
}
fn default_vod() -> String {
    DEFAULT_VOD.to_owned()
}
fn default_live() -> String {
    DEFAULT_LIVE.to_owned()
}

impl Default for Naming {
    fn default() -> Self {
        Self {
            folder: default_folder(),
            vod: default_vod(),
            live: default_live(),
        }
    }
}

/// A template error, for the settings screen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct TemplateError {
    /// `empty`, `too_long`, `unknown_variable`, `unbalanced`,
    /// `nested_group`, `slash_in_name`, `bad_folder`.
    pub code: &'static str,
    pub message: String,
}

fn terr(code: &'static str, message: impl Into<String>) -> TemplateError {
    TemplateError {
        code,
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    Var(String),
    Group(Vec<Piece>),
}

fn parse(template: &str) -> Result<Vec<Piece>, TemplateError> {
    if template.trim().is_empty() {
        return Err(terr("empty", "the template is empty"));
    }
    if template.len() > MAX_TEMPLATE {
        return Err(terr(
            "too_long",
            format!("at most {MAX_TEMPLATE} bytes"),
        ));
    }
    let mut top: Vec<Piece> = Vec::new();
    let mut group: Option<Vec<Piece>> = None;
    let mut text = String::new();
    let mut chars = template.chars();
    let flush = |text: &mut String, out: &mut Vec<Piece>| {
        if !text.is_empty() {
            out.push(Piece::Text(std::mem::take(text)));
        }
    };
    while let Some(c) = chars.next() {
        let out = group.as_mut().unwrap_or(&mut top);
        match c {
            '{' => {
                flush(&mut text, out);
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) if c.is_ascii_alphanumeric() || c == '_' => name.push(c),
                        _ => return Err(terr("unbalanced", "`{` without a matching `}`")),
                    }
                }
                if !VARIABLES.contains(&name.as_str()) {
                    return Err(terr(
                        "unknown_variable",
                        format!("unknown variable {{{name}}}"),
                    ));
                }
                out.push(Piece::Var(name));
            }
            '}' => return Err(terr("unbalanced", "`}` without a matching `{`")),
            '<' => {
                if group.is_some() {
                    return Err(terr("nested_group", "`<…>` groups cannot nest"));
                }
                flush(&mut text, out);
                group = Some(Vec::new());
            }
            '>' => {
                let Some(mut g) = group.take() else {
                    return Err(terr("unbalanced", "`>` without a matching `<`"));
                };
                flush(&mut text, &mut g);
                top.push(Piece::Group(g));
            }
            c => text.push(c),
        }
    }
    if group.is_some() {
        return Err(terr("unbalanced", "`<` without a matching `>`"));
    }
    flush(&mut text, &mut top);
    Ok(top)
}

/// Values of the variables for one item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameVars {
    /// RFC 3339 time of the item (VOD created, broadcast started).
    pub at: String,
    pub login: String,
    pub display_name: String,
    pub user_id: String,
    pub title: String,
    pub categories: Vec<String>,
    pub part: Option<String>,
    pub id: String,
    /// `vod` or `live`.
    pub kind: &'static str,
}

impl NameVars {
    fn get(&self, name: &str) -> String {
        let at = self.at.as_str();
        let slice = |a: usize, b: usize| at.get(a..b).unwrap_or("").to_owned();
        let date = super::meta::date_of(at).unwrap_or("0000-00-00");
        match name {
            "date" => date.to_owned(),
            "year" => date[..4].to_owned(),
            "month" => date[5..7].to_owned(),
            "time" => {
                let (h, m) = (slice(11, 13), slice(14, 16));
                if h.len() == 2 && m.len() == 2 {
                    format!("{h}-{m}")
                } else {
                    String::new()
                }
            }
            "login" => self.login.clone(),
            "display_name" => self.display_name.clone(),
            "user_id" => self.user_id.clone(),
            "title" => self.title.trim().to_owned(),
            "categories" => self.categories.join(", "),
            "part" => self.part.clone().unwrap_or_default().trim().to_owned(),
            "id" => self.id.clone(),
            "type" => self.kind.to_owned(),
            _ => String::new(),
        }
    }
}

fn render_pieces(pieces: &[Piece], vars: &NameVars) -> String {
    let mut out = String::new();
    for p in pieces {
        match p {
            Piece::Text(t) => out.push_str(t),
            Piece::Var(v) => out.push_str(&vars.get(v)),
            Piece::Group(g) => {
                let empty = g.iter().any(|p| matches!(p, Piece::Var(v) if vars.get(v).is_empty()));
                if !empty {
                    out.push_str(&render_pieces(g, vars));
                }
            }
        }
    }
    out
}

/// Checks a base-name template (no `/`).
pub fn check_name(template: &str) -> Result<(), TemplateError> {
    parse(template)?;
    if template.contains(['/', '\\']) {
        return Err(terr(
            "slash_in_name",
            "the file name cannot contain `/`; use the folder template",
        ));
    }
    Ok(())
}

/// Checks a folder template (`/`-separated, relative, no `..`).
pub fn check_folder(template: &str) -> Result<(), TemplateError> {
    parse(template)?;
    let t = template.replace('\\', "/");
    if t.starts_with('/') || t.split('/').any(|c| c.trim() == "..") {
        return Err(terr(
            "bad_folder",
            "the folder must be relative and cannot contain `..`",
        ));
    }
    Ok(())
}

impl Naming {
    /// Checks all three templates.
    pub fn check(&self) -> Result<(), TemplateError> {
        check_folder(&self.folder)?;
        check_name(&self.vod)?;
        check_name(&self.live)
    }

    /// Where an item goes: the folder (relative to the target,
    /// `/`-separated, ending with the item folder) and the base name of
    /// its files. `suffix` is the id suffix (` [v123]`). A broken template
    /// falls back to the default one.
    pub fn layout(&self, vars: &NameVars, suffix: &str) -> super::meta::Layout {
        let name_tpl = if vars.kind == "live" {
            &self.live
        } else {
            &self.vod
        };
        let name = check_name(name_tpl)
            .and_then(|()| parse(name_tpl))
            .or_else(|_| parse(if vars.kind == "live" { DEFAULT_LIVE } else { DEFAULT_VOD }))
            .unwrap_or_default();
        let folder = check_folder(&self.folder)
            .and_then(|()| parse(&self.folder))
            .or_else(|_| parse(DEFAULT_FOLDER))
            .unwrap_or_default();

        let mut head = sanitize(&render_pieces(&name, vars));
        let room = MAX_NAME.saturating_sub(suffix.len());
        if head.len() > room {
            let mut cut = room;
            while !head.is_char_boundary(cut) {
                cut -= 1;
            }
            head.truncate(cut);
            head = head.trim_end_matches(['.', ' ']).to_owned();
            if head.is_empty() {
                head.push('_');
            }
        }
        let base = format!("{head}{suffix}");
        let rendered = render_pieces(&folder, vars).replace('\\', "/");
        let mut parts: Vec<String> = rendered
            .split('/')
            .filter(|c| !c.trim().is_empty())
            .map(sanitize)
            .collect();
        parts.push(base.clone());
        super::meta::Layout {
            folder: parts.join("/"),
            base,
        }
    }

    /// A preview of the templates with made-up values, for the settings
    /// screen: `(vod, live)` relative paths of the video files.
    pub fn preview(&self) -> (String, String) {
        let mut vars = NameVars {
            at: "2026-11-12T18:00:00Z".to_owned(),
            login: "streamer".to_owned(),
            display_name: "Streamer".to_owned(),
            user_id: "12345".to_owned(),
            title: "Title here".to_owned(),
            categories: vec!["Elden Ring".to_owned(), "Hades II".to_owned()],
            part: None,
            id: "2290012345".to_owned(),
            kind: "vod",
        };
        let vod = self.layout(&vars, " [v2290012345]");
        vars.kind = "live";
        vars.id = "51234567890".to_owned();
        vars.categories.truncate(1);
        let live = self.layout(&vars, " [s51234567890]");
        (
            format!("{}/{}.mkv", vod.folder, vod.base),
            format!("{}/{}.mkv", live.folder, live.base),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> NameVars {
        NameVars {
            at: "2026-11-12T18:05:00Z".to_owned(),
            login: "somestreamer".to_owned(),
            display_name: "SomeStreamer".to_owned(),
            user_id: "1001".to_owned(),
            title: "Boss rush: part 2/3".to_owned(),
            categories: vec!["Elden Ring".to_owned(), "Hades II".to_owned()],
            part: None,
            id: "2290012345".to_owned(),
            kind: "vod",
        }
    }

    #[test]
    fn defaults_follow_research() {
        let l = Naming::default().layout(&vars(), " [v2290012345]");
        assert_eq!(
            l.base,
            "2026-11-12 Elden Ring, Hades II — Boss rush_ part 2_3 [v2290012345]"
        );
        assert_eq!(l.folder, format!("somestreamer [1001]/2026/{}", l.base));
        let mut v = vars();
        v.categories.clear();
        let l = Naming::default().layout(&v, " [v2290012345]");
        assert_eq!(l.base, "2026-11-12 Boss rush_ part 2_3 [v2290012345]");
        v.kind = "live";
        v.categories = vec!["Just Chatting".to_owned()];
        let l = Naming::default().layout(&v, " [s51234567890]");
        assert_eq!(
            l.base,
            "2026-11-12 LIVE Just Chatting — Boss rush_ part 2_3 [s51234567890]"
        );
    }

    #[test]
    fn custom_templates_and_sanitizing() {
        let n = Naming {
            folder: "Twitch/{display_name}/{year}-{month}".to_owned(),
            vod: "{time} {title}< ({part})>".to_owned(),
            live: DEFAULT_LIVE.to_owned(),
        };
        n.check().unwrap();
        let mut v = vars();
        v.part = Some("Hades II".to_owned());
        let l = n.layout(&v, " [v1]");
        assert_eq!(l.base, "18-05 Boss rush_ part 2_3 (Hades II) [v1]");
        assert_eq!(l.folder, format!("Twitch/SomeStreamer/2026-11/{}", l.base));
        // A login with reserved characters and trailing dots stays safe.
        v.login = "con".to_owned();
        let l = Naming {
            folder: "{login}".to_owned(),
            ..Naming::default()
        }
        .layout(&v, " [v1]");
        assert!(l.folder.starts_with("_con/"), "{}", l.folder);
        v.title = "ж".repeat(300);
        let l = Naming::default().layout(&v, " [v2290012345]");
        assert!(l.base.len() <= MAX_NAME);
        assert!(l.base.ends_with(" [v2290012345]"));
    }

    #[test]
    fn bad_templates_are_refused_and_fall_back() {
        assert_eq!(check_name("").unwrap_err().code, "empty");
        assert_eq!(check_name("{nope}").unwrap_err().code, "unknown_variable");
        assert_eq!(check_name("{title").unwrap_err().code, "unbalanced");
        assert_eq!(check_name("<a<b>>").unwrap_err().code, "nested_group");
        assert_eq!(check_name("{login}/{title}").unwrap_err().code, "slash_in_name");
        assert_eq!(check_folder("../x").unwrap_err().code, "bad_folder");
        assert_eq!(check_folder("/abs").unwrap_err().code, "bad_folder");
        let broken = Naming {
            folder: "{oops".to_owned(),
            vod: "{nope}".to_owned(),
            live: String::new(),
        };
        assert_eq!(
            broken.layout(&vars(), " [v1]"),
            Naming::default().layout(&vars(), " [v1]")
        );
    }

    #[test]
    fn preview_shows_both() {
        let (vod, live) = Naming::default().preview();
        assert_eq!(
            vod,
            "streamer [12345]/2026/2026-11-12 Elden Ring, Hades II — Title here [v2290012345]/2026-11-12 Elden Ring, Hades II — Title here [v2290012345].mkv"
        );
        assert!(live.contains("/2026-11-12 LIVE Elden Ring — Title here [s51234567890].mkv"));
    }
}
