// SPDX-License-Identifier: MPL-2.0
//! Profiles and parental control in the catalog (LIB-012).
//!
//! The catalog has one active profile at a time ([`Catalog::switch_profile`];
//! on open, the most recently used one). Progress, favorites, lists, view
//! preferences and smart shelves are the active profile's, and every query
//! applies its filters (`pa_library::profiles`): the folder white list, the
//! rating limit, "no rating = hidden" and hidden artwork. An item the
//! profile may not see does not exist for it: lists, search, "Continue
//! watching", the file browser, details, versions and playback all leave it
//! out or answer [`Error::NotFound`].
//!
//! Switching to a profile with a PIN needs the PIN ([`Error::Denied`]
//! otherwise). Managing profiles is not restricted here: the app asks for
//! the PIN of a parent profile before it shows the profile settings to a
//! restricted one.

use pa_library::profiles::{self as lib, FolderAccess, Profile, ProfileSettings};
use pa_library::rusqlite::{Connection, OptionalExtension, params};

use crate::items::{self, ItemRef};
use crate::{Catalog, Error, now_ms};

/// `Ok` if the active profile may see `item`, else [`Error::NotFound`]
/// (the same answer as for an item that does not exist).
pub(crate) fn require_visible(conn: &Connection, item: ItemRef) -> Result<(), Error> {
    if items::item(conn, item)?.is_some() {
        Ok(())
    } else {
        Err(Error::NotFound(format!("item {item}")))
    }
}

/// Folder white-list check of a path that need not be a library file
/// (sidecar subtitles, artwork, trickplay next to a video).
const PATH_VISIBLE_SQL: &str = "
SELECT ifnull((SELECT ap.restrict_folders = 0 FROM temp.active_profile ap), 0)
    OR EXISTS (SELECT 1 FROM temp.active_profile ap
                 JOIN profile_folders pf ON pf.profile_id = ap.profile_id
                WHERE pf.source_id = ?1
                  AND (pf.path = '/' OR ?2 = pf.path
                       OR substr(?2, 1, length(pf.path) + 1) = pf.path || '/'))";

/// A library file the active profile may not open (rating filters).
const FILE_DENIED_SQL: &str = concat!(
    "SELECT NOT ",
    pa_library::file_allowed!("f"),
    " FROM files f WHERE f.source_id = ?1 AND f.path = ?2"
);

/// `true` if the active profile may open `path` (a VfsPath, `/a/b.mkv`) of
/// source `source_id`: it lies in the white list, and if it is a library
/// file, the rating filters allow it.
pub(crate) fn may_open(conn: &Connection, source_id: i64, path: &str) -> Result<bool, Error> {
    let path = format!("/{}", path.trim_start_matches('/'));
    let visible: bool = conn.query_row(PATH_VISIBLE_SQL, params![source_id, path], |r| r.get(0))?;
    if !visible {
        return Ok(false);
    }
    let denied: Option<bool> = conn
        .query_row(FILE_DENIED_SQL, params![source_id, path], |r| r.get(0))
        .optional()?;
    Ok(!denied.unwrap_or(false))
}

impl Catalog {
    /// Every profile by position.
    pub fn profiles(&self) -> Result<Vec<Profile>, Error> {
        Ok(lib::list(&self.lock())?)
    }

    /// The active profile.
    pub fn active_profile(&self) -> Result<Profile, Error> {
        let conn = self.lock();
        let id = crate::progress::profile(&conn)?;
        lib::get(&conn, id)?.ok_or_else(|| Error::NotFound(format!("profile {id}")))
    }

    /// Makes `id` the active profile. A profile with a PIN needs it
    /// ([`Error::Denied`] for a wrong or missing one). The choice is
    /// remembered for the next [`Catalog::open`].
    pub fn switch_profile(&self, id: i64, pin: Option<&str>) -> Result<Profile, Error> {
        let conn = self.lock();
        lib::get(&conn, id)?.ok_or_else(|| Error::NotFound(format!("profile {id}")))?;
        if !lib::verify_pin(&conn, id, pin)? {
            return Err(Error::Denied(format!("wrong PIN for profile {id}")));
        }
        lib::touch(&conn, id, now_ms())?;
        Ok(lib::activate(&conn, id)?)
    }

    /// `true` if `pin` is the PIN of profile `id` (or it has none): the
    /// check before a parent's settings are shown.
    pub fn verify_profile_pin(&self, id: i64, pin: Option<&str>) -> Result<bool, Error> {
        Ok(lib::verify_pin(&self.lock(), id, pin)?)
    }

    /// Creates a profile.
    pub fn create_profile(&self, settings: &ProfileSettings) -> Result<Profile, Error> {
        Ok(lib::create(&self.lock(), settings, now_ms())?)
    }

    /// Replaces the settings of a profile; the active one applies them at
    /// once.
    pub fn update_profile(&self, id: i64, settings: &ProfileSettings) -> Result<Profile, Error> {
        Ok(lib::update(&self.lock(), id, settings, now_ms())?)
    }

    /// Deletes a profile with its progress, favorites and lists. The last
    /// profile stays; deleting the active one activates the default.
    pub fn delete_profile(&self, id: i64) -> Result<(), Error> {
        Ok(lib::delete(&self.lock(), id)?)
    }

    /// Orders the profiles as `ids`.
    pub fn reorder_profiles(&self, ids: &[i64]) -> Result<(), Error> {
        Ok(lib::reorder(&mut self.lock(), ids, now_ms())?)
    }

    /// Sets (`Some`, 4–8 digits) or removes the PIN of a profile.
    pub fn set_profile_pin(&self, id: i64, pin: Option<&str>) -> Result<(), Error> {
        Ok(lib::set_pin(&self.lock(), id, pin, now_ms())?)
    }

    /// The folder white list of a profile.
    pub fn profile_folders(&self, id: i64) -> Result<Vec<FolderAccess>, Error> {
        Ok(lib::folders(&self.lock(), id)?)
    }

    /// Replaces the folder white list of a profile (`path` relative to the
    /// source root, `""` for the whole source). It applies while
    /// [`ProfileSettings::restrict_folders`] is on.
    pub fn set_profile_folders(&self, id: i64, folders: &[FolderAccess]) -> Result<(), Error> {
        let mut conn = self.lock();
        lib::set_folders(&mut conn, id, folders, now_ms())?;
        lib::refresh(&conn)?;
        Ok(())
    }

    /// Content ratings of a movie or show, the parent's own first.
    pub fn certifications(&self, item: ItemRef) -> Result<Vec<lib::Certification>, Error> {
        let conn = self.lock();
        require_visible(&conn, item)?;
        match item {
            ItemRef::Movie(w) | ItemRef::Show(w) => Ok(lib::certifications(&conn, w)?),
            _ => Ok(Vec::new()),
        }
    }

    /// Sets the parent's own rating of a movie or show as a minimum age, or
    /// removes it (`None`); it wins over the providers' ratings.
    pub fn set_user_certification(&self, item: ItemRef, min_age: Option<u8>) -> Result<(), Error> {
        let (ItemRef::Movie(w) | ItemRef::Show(w)) = item else {
            return Err(Error::Invalid(format!(
                "ratings are per movie or show, not {item}"
            )));
        };
        let conn = self.lock();
        require_visible(&conn, item)?;
        lib::set_user_certification(&conn, w, min_age)?;
        Ok(())
    }

    /// `true` if the active profile may open `path` of source `source_id`
    /// (the playback proxy asks before it serves a file).
    pub fn may_open(&self, source_id: i64, path: &str) -> Result<bool, Error> {
        may_open(&self.lock(), source_id, path)
    }
}
