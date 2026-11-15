// SPDX-License-Identifier: MPL-2.0
//! Types of the user's own categories, tags and star ratings (LIB-022) for
//! Dart. The calls are methods of
//! [`CoreLibrary`](crate::api::library::CoreLibrary).

use pa_catalog as c;

use crate::api::library::CoreItem;

/// A category of the active profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreCategory {
    pub id: i64,
    pub name: String,
    /// Items in it the active profile may see.
    pub item_count: u32,
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl From<c::CategoryInfo> for CoreCategory {
    fn from(i: c::CategoryInfo) -> Self {
        Self {
            id: i.id,
            name: i.name,
            item_count: count(i.item_count),
        }
    }
}

/// A tag with the number of visible items that have it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTag {
    pub name: String,
    pub item_count: u32,
}

impl From<c::TagInfo> for CoreTag {
    fn from(t: c::TagInfo) -> Self {
        Self {
            name: t.name,
            item_count: count(t.item_count),
        }
    }
}

/// An item with the stars (1–5) the active profile gave it.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreRatedItem {
    pub item: CoreItem,
    pub stars: u8,
}

impl From<c::RatedItem> for CoreRatedItem {
    fn from(r: c::RatedItem) -> Self {
        Self {
            item: r.item.into(),
            stars: r.stars,
        }
    }
}
