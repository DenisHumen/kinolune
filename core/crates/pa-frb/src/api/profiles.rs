// SPDX-License-Identifier: MPL-2.0
//! Types of profiles and parental control (LIB-012) for Dart. The calls are
//! methods of [`CoreLibrary`](crate::api::library::CoreLibrary):
//! `profiles`, `active_profile`, `switch_profile`, `create_profile`,
//! `update_profile`, `delete_profile`, `set_profile_pin`,
//! `profile_folders`, `set_profile_folders`, `certifications`,
//! `set_user_certification`.

use pa_catalog as c;

/// A profile (without its PIN).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreProfile {
    pub id: i64,
    /// Stable id shared by the devices of a household (`default` for the
    /// first profile).
    pub sync_id: String,
    /// '' = the default name; the app shows it localized.
    pub name: String,
    /// Avatar reference chosen by the app (`color:<index>`…).
    pub avatar: Option<String>,
    pub is_kid: bool,
    /// Switching to the profile needs its PIN.
    pub has_pin: bool,
    pub position: i64,
    /// Rating limit as a minimum viewer age; `None` = no limit.
    pub max_age: Option<u8>,
    /// ISO 3166-1 alpha-2 country whose rating wins ('' = the strictest).
    pub rating_country: String,
    pub hide_unrated: bool,
    pub hide_artwork: bool,
    /// Only the white-listed folders are visible.
    pub restrict_folders: bool,
    /// Any filter hides something from the profile.
    pub is_restricted: bool,
    pub last_active_at_ms: Option<i64>,
}

impl From<c::Profile> for CoreProfile {
    fn from(p: c::Profile) -> Self {
        Self {
            is_restricted: p.is_restricted(),
            id: p.id,
            sync_id: p.sync_id,
            name: p.name,
            avatar: p.avatar,
            is_kid: p.is_kid,
            has_pin: p.has_pin,
            position: p.position,
            max_age: p.max_age,
            rating_country: p.rating_country,
            hide_unrated: p.hide_unrated,
            hide_artwork: p.hide_artwork,
            restrict_folders: p.restrict_folders,
            last_active_at_ms: p.last_active_at_ms,
        }
    }
}

/// Editable settings of a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreProfileSettings {
    pub name: String,
    pub avatar: Option<String>,
    pub is_kid: bool,
    pub max_age: Option<u8>,
    pub rating_country: String,
    pub hide_unrated: bool,
    pub hide_artwork: bool,
    pub restrict_folders: bool,
}

impl From<CoreProfileSettings> for c::ProfileSettings {
    fn from(s: CoreProfileSettings) -> Self {
        Self {
            name: s.name,
            avatar: s.avatar,
            is_kid: s.is_kid,
            max_age: s.max_age,
            rating_country: s.rating_country,
            hide_unrated: s.hide_unrated,
            hide_artwork: s.hide_artwork,
            restrict_folders: s.restrict_folders,
        }
    }
}

/// One folder of a profile's white list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreFolderAccess {
    pub source_id: i64,
    /// Folder relative to the source root (`Movies/Kids`); '' = the whole
    /// source.
    pub rel_path: String,
}

impl From<c::FolderAccess> for CoreFolderAccess {
    fn from(f: c::FolderAccess) -> Self {
        Self {
            source_id: f.source_id,
            rel_path: f.path.trim_matches('/').to_owned(),
        }
    }
}

impl From<CoreFolderAccess> for c::FolderAccess {
    fn from(f: CoreFolderAccess) -> Self {
        c::FolderAccess::new(f.source_id, &f.rel_path)
    }
}

/// One content rating of a movie or show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreCertification {
    /// `tmdb`, or `user` for the parent's own rating.
    pub provider: String,
    /// ISO 3166-1 alpha-2; '' for the parent's rating.
    pub country: String,
    /// As written by the provider: `PG-13`, `16+`.
    pub certification: String,
    /// Minimum viewer age, `None` if the rating is not understood.
    pub min_age: Option<u8>,
}

impl From<c::Certification> for CoreCertification {
    fn from(x: c::Certification) -> Self {
        Self {
            provider: x.provider,
            country: x.country,
            certification: x.certification,
            min_age: x.min_age,
        }
    }
}
