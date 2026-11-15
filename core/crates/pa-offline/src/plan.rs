// SPDX-License-Identifier: MPL-2.0
//! What an offline copy of an item holds: the job parameters.

use std::collections::HashSet;

use pa_catalog::{Catalog, Item, ItemKind, ItemRef};
use serde::{Deserialize, Serialize};

use crate::Error;

/// What a file of a copy is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileRole {
    /// The file the item plays.
    Video,
    /// A later part of a split movie or a DVD title set (LIB-020).
    Part,
    /// A sidecar subtitle (PLAY-007).
    Subtitle,
    /// The local poster or cover next to the files (LIB-007).
    Art,
}

impl FileRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            FileRole::Video => "video",
            FileRole::Part => "part",
            FileRole::Subtitle => "subtitle",
            FileRole::Art => "art",
        }
    }

    /// The copy is useless without this file.
    pub const fn required(self) -> bool {
        matches!(self, FileRole::Video | FileRole::Part)
    }
}

/// One source file to copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyFile {
    /// Path in the source, `/`-rooted as `files.path`.
    pub path: String,
    pub role: FileRole,
    /// Size the library knows, if any (the copy stats the file anyway).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// Parameters of an `offline_copy` job (`jobs.params_json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyPlan {
    /// Item id (`m12`, `e40`, `t7`).
    pub item: String,
    pub title: String,
    pub source_id: i64,
    /// `files.id` of the version copied, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Retention group of the copy.
    pub group: String,
    pub files: Vec<CopyFile>,
}

impl CopyPlan {
    /// Bytes known before the copy starts.
    pub fn known_bytes(&self) -> u64 {
        self.files.iter().filter_map(|f| f.size).sum()
    }

    /// Folder of the copy in the target: the title and the item, unique
    /// per job.
    pub fn folder(&self, job: i64) -> String {
        format!("{} [{}-{job}]", safe_name(&self.title), self.item)
    }
}

/// A name that every file system takes: no separators, reserved or control
/// characters, no leading dots, at most 80 characters.
pub(crate) fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                ' '
            } else {
                c
            }
        })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let joined = words.join(" ");
    let trimmed = joined.trim_start_matches('.').trim_end_matches(['.', ' ']);
    let short: String = trimmed.chars().take(80).collect();
    let short = short.trim_end_matches(['.', ' ']).to_owned();
    if short.is_empty() {
        "item".to_owned()
    } else {
        short
    }
}

fn abs(rel: &str) -> String {
    format!("/{}", rel.trim_start_matches('/'))
}

/// Retention group of an item.
fn group(catalog: &Catalog, item: &Item) -> Result<String, Error> {
    Ok(match item.kind {
        ItemKind::Movie => "movies".to_owned(),
        ItemKind::Episode => match item.show_id {
            Some(show) => format!("show:{show}"),
            None => "shows".to_owned(),
        },
        ItemKind::Track => {
            let album = catalog
                .music_tracks()?
                .into_iter()
                .find(|t| t.item.id == item.id)
                .and_then(|t| t.album_id);
            album.map_or_else(|| "music".to_owned(), |a| format!("album:{a}"))
        }
        ItemKind::Show => return Err(Error::Invalid("a show is not one item".into())),
    })
}

/// The copy plan of a movie, an episode or a track: the version
/// `version_file_id` (else the default one) with all its parts, their
/// sidecar subtitles and the item's local poster.
pub fn plan(
    catalog: &Catalog,
    item: ItemRef,
    version_file_id: Option<i64>,
) -> Result<CopyPlan, Error> {
    plan_in_group(catalog, item, version_file_id, None)
}

/// [`plan`] with the retention group already known (an album download
/// looks the album up once for all its tracks).
pub(crate) fn plan_in_group(
    catalog: &Catalog,
    item: ItemRef,
    version_file_id: Option<i64>,
    group_key: Option<String>,
) -> Result<CopyPlan, Error> {
    let found = catalog
        .item(item)?
        .ok_or_else(|| Error::NotFound(item.to_string()))?;
    if found.kind == ItemKind::Show {
        return Err(Error::Invalid("a show is not one item".into()));
    }
    if found.local_path.is_some() {
        return Err(Error::Local(item.to_string()));
    }
    if found.unplayable.is_some() {
        return Err(Error::Unplayable(item.to_string()));
    }
    let (rel_path, file_id, version) = match found.kind {
        ItemKind::Movie | ItemKind::Episode => {
            let versions = catalog.versions(item)?;
            let chosen = match version_file_id {
                Some(id) => Some(
                    versions
                        .iter()
                        .find(|v| v.file_id == id)
                        .ok_or_else(|| Error::NotFound(format!("version {id} of {item}")))?,
                ),
                None => versions
                    .iter()
                    .find(|v| v.is_default)
                    .or_else(|| versions.first()),
            };
            match chosen {
                Some(v) if v.unplayable().is_some() => {
                    return Err(Error::Unplayable(item.to_string()));
                }
                Some(v) => (
                    v.rel_path.clone(),
                    Some(v.file_id),
                    (versions.len() > 1).then(|| v.label()),
                ),
                None => (found.rel_path.clone(), None, None),
            }
        }
        _ => (found.rel_path.clone(), None, None),
    };
    let source_id = found.source_id;
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |files: &mut Vec<CopyFile>, path: String, role, size: Option<i64>| {
        if seen.insert(path.clone()) {
            files.push(CopyFile {
                path,
                role,
                size: size.and_then(|s| u64::try_from(s).ok()),
            });
        }
    };
    let parts = catalog.play_parts(source_id, &rel_path)?;
    let part_paths: Vec<String> = if parts.is_empty() {
        push(&mut files, abs(&rel_path), FileRole::Video, None);
        vec![rel_path.clone()]
    } else {
        for (i, p) in parts.iter().enumerate() {
            let role = if i == 0 {
                FileRole::Video
            } else {
                FileRole::Part
            };
            push(&mut files, abs(&p.rel_path), role, Some(p.size_bytes));
        }
        parts.iter().map(|p| p.rel_path.clone()).collect()
    };
    if matches!(found.kind, ItemKind::Movie | ItemKind::Episode) {
        for part in &part_paths {
            for s in catalog.sidecar_subtitles(source_id, part)? {
                push(&mut files, abs(&s.rel_path), FileRole::Subtitle, None);
            }
        }
    }
    if let Some(poster) = found.local_poster.as_deref().filter(|p| !p.is_empty()) {
        push(&mut files, abs(poster), FileRole::Art, None);
    }
    Ok(CopyPlan {
        item: item.to_string(),
        title: found.title.clone(),
        source_id,
        file_id,
        version,
        group: match group_key {
            Some(g) => g,
            None => group(catalog, &found)?,
        },
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_names() {
        assert_eq!(safe_name("Dune: Part Two"), "Dune Part Two");
        assert_eq!(safe_name("a/b\\c"), "a b c");
        assert_eq!(safe_name("..hidden."), "hidden");
        assert_eq!(safe_name("   "), "item");
        assert_eq!(safe_name(&"я".repeat(200)).chars().count(), 80);
    }

    #[test]
    fn folder_is_unique_per_job() {
        let plan = CopyPlan {
            item: "m1".into(),
            title: "Брат".into(),
            source_id: 1,
            file_id: None,
            version: None,
            group: "movies".into(),
            files: vec![],
        };
        assert_eq!(plan.folder(7), "Брат [m1-7]");
        assert_ne!(plan.folder(7), plan.folder(8));
    }
}
