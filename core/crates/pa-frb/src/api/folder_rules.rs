// SPDX-License-Identifier: MPL-2.0
//! Folder rules (META-017, META-035) for Dart: the numbering and the type
//! of a work for its whole folder, the rules a fix taught and «Забыть
//! правило папки». The calls are methods of [`CoreLibrary`]; see
//! `pa_catalog::folder_rules` and ADR-0112.

use pa_catalog as c;

use crate::api::library::{CoreLibrary, LibraryError, item_ref};

type Result<T> = std::result::Result<T, LibraryError>;

/// A folder rule as «Исправить распознавание» shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreFolderRule {
    pub id: i64,
    pub source_id: i64,
    /// Folder relative to the source root, without slashes; `""` the whole
    /// source.
    pub folder: String,
    /// `movie`, `show` or `anime` if the user set the type.
    pub kind: Option<String>,
    /// The number in a file name is a running count (absolute), not an
    /// episode of a season.
    pub absolute: bool,
    /// Season the episodes land in; `None`: the season written in the name.
    pub season: Option<i32>,
    /// Added to the number in the name.
    pub offset: i32,
}

/// The type of a work and the folder rules it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreFolderRules {
    /// `movie`, `show` or `anime`.
    pub kind: String,
    pub rules: Vec<CoreFolderRule>,
}

fn narrow(n: i64) -> i32 {
    i32::try_from(n).unwrap_or(if n < 0 { i32::MIN } else { i32::MAX })
}

impl From<c::folder_rules::FolderRuleInfo> for CoreFolderRule {
    fn from(r: c::folder_rules::FolderRuleInfo) -> Self {
        Self {
            id: r.id,
            source_id: r.source_id,
            folder: r.folder,
            kind: r.kind,
            absolute: r.scheme.as_deref() == Some("tvdb_absolute"),
            season: r.season.map(narrow),
            offset: narrow(r.offset),
        }
    }
}

impl CoreLibrary {
    /// The type of an item's work (an episode: its show) and its folder
    /// rules.
    pub fn folder_rules(&self, item_id: String) -> Result<CoreFolderRules> {
        let item = item_ref(&item_id)?;
        Ok(CoreFolderRules {
            kind: self.catalog().work_kind(item)?,
            rules: self
                .catalog()
                .folder_rules(item)?
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }

    /// The numbering of an item's work for its whole folder: the number in
    /// each file name plus `offset` is the episode of `season` (`None`: the
    /// season written in the name), or with `absolute` the running count.
    /// Files of the folder, also new ones, follow it; final. Run the
    /// metadata fetch afterwards for the episode data.
    pub fn set_numbering(
        &self,
        item_id: String,
        season: Option<i32>,
        offset: i32,
        absolute: bool,
    ) -> Result<()> {
        Ok(self.catalog().set_numbering(
            item_ref(&item_id)?,
            season.map(i64::from),
            i64::from(offset),
            absolute,
        )?)
    }

    /// The type of an item's work for its whole folder: `movie`, `show` or
    /// `anime`; final. Between a movie and a series the provider data goes
    /// and the next metadata fetch looks the work up anew.
    pub fn set_work_kind(&self, item_id: String, kind: String) -> Result<()> {
        Ok(self.catalog().set_work_kind(item_ref(&item_id)?, &kind)?)
    }

    /// «Забыть правило папки»: the folder rules of an item's work go here
    /// and on the family's devices; decisions already made stay. Returns
    /// how many went.
    pub fn forget_folder_rules(&self, item_id: String) -> Result<u32> {
        let n = self.catalog().forget_folder_rules(item_ref(&item_id)?)?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }
}
