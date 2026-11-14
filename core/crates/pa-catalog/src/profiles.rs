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
//! # The parent PIN (ADR-0055 amendment)
//!
//! The PIN of a profile that is not supervised ([`Profile::is_parent`]) is a
//! parent PIN. While a supervised profile (a kid profile or one with any
//! filter) is active and a parent PIN exists, the management calls —
//! creating, changing, reordering and deleting profiles, setting or
//! removing PINs, the folder white list, the parent's own ratings — and
//! leaving the profile ([`Catalog::switch_profile`]) answer
//! [`Error::Denied`] unless the caller passes a parent token:
//! [`Catalog::verify_parent_pin`] (or [`Catalog::verify_profile_pin`] with a
//! parent PIN) hands one out. The token lives in memory only, expires after
//! [`PARENT_TOKEN_TTL`] without use (each accepted call renews it) and is
//! dropped when the active profile changes ([`Catalog::end_parent_session`]
//! drops it at once). Unsupervised profiles need no token, nor does anyone
//! while no parent PIN exists.
//!
//! Every PIN check of the catalog shares one wrong-PIN backoff, kept in the
//! database (`pa_library::profiles::pin_attempts`): 5 free tries, then 30 s
//! doubling per wrong PIN up to 1 h; while it runs no PIN is checked.

use std::time::{Duration, Instant};

use pa_library::profiles::{self as lib, FolderAccess, PinAttempts, Profile, ProfileSettings};
use pa_library::rusqlite::{Connection, OptionalExtension, params};

use crate::items::{self, ItemRef};
use crate::{Catalog, Error, now_ms};

/// How long a parent token stays valid without use.
pub const PARENT_TOKEN_TTL: Duration = Duration::from_secs(5 * 60);

/// The parent elevation of a [`Catalog`]: the current token and when it
/// expires.
pub(crate) struct ParentSession {
    token: Option<(String, Instant)>,
    ttl: Duration,
}

impl Default for ParentSession {
    fn default() -> Self {
        Self {
            token: None,
            ttl: PARENT_TOKEN_TTL,
        }
    }
}

impl std::fmt::Debug for ParentSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the token itself.
        f.debug_struct("ParentSession")
            .field("active", &self.token.is_some())
            .field("ttl", &self.ttl)
            .finish()
    }
}

impl ParentSession {
    /// A new token, replacing the previous one.
    fn issue(&mut self) -> Result<String, Error> {
        let token = lib::random_token()?;
        self.token = Some((token.clone(), Instant::now() + self.ttl));
        Ok(token)
    }

    /// `true` if `token` is the current, unexpired token; its time starts
    /// over.
    fn accept(&mut self, token: Option<&str>) -> bool {
        let now = Instant::now();
        let Some((current, expires)) = self.token.as_mut() else {
            return false;
        };
        if now >= *expires {
            self.token = None;
            return false;
        }
        let Some(token) = token else {
            return false;
        };
        if !same(current.as_bytes(), token.as_bytes()) {
            return false;
        }
        *expires = now + self.ttl;
        true
    }

    fn end(&mut self) {
        self.token = None;
    }
}

/// Byte equality without an early exit.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The answer to a PIN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinVerdict {
    /// The PIN is right.
    pub ok: bool,
    /// A parent token when the PIN is a parent PIN (pass it to the
    /// management calls).
    pub token: Option<String>,
    /// The PIN was not checked: too many wrong PINs, wait
    /// ([`PinAttempts::locked_until_ms`]).
    pub locked: bool,
    /// The backoff after this check.
    pub attempts: PinAttempts,
}

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

fn active(conn: &Connection) -> Result<Profile, Error> {
    let id = crate::progress::profile(conn)?;
    lib::get(conn, id)?.ok_or_else(|| Error::NotFound(format!("profile {id}")))
}

fn denied_parent() -> Error {
    Error::Denied("the parent PIN is needed".into())
}

fn denied_locked(a: PinAttempts) -> Error {
    Error::Denied(format!(
        "too many wrong PINs, wait until {}",
        a.locked_until_ms.unwrap_or_default()
    ))
}

impl Catalog {
    fn parent(&self) -> std::sync::MutexGuard<'_, ParentSession> {
        self.parent.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `true` if the active profile needs the parent token for management:
    /// it is supervised and a parent PIN exists.
    fn guarded(conn: &Connection) -> Result<bool, Error> {
        Ok(active(conn)?.is_supervised() && lib::has_parent_pin(conn)?)
    }

    /// `Ok` if the active profile may manage profiles: it is not guarded,
    /// or `token` is the current parent token.
    fn require_parent(&self, conn: &Connection, token: Option<&str>) -> Result<(), Error> {
        if !Self::guarded(conn)? || self.parent().accept(token) {
            Ok(())
        } else {
            Err(denied_parent())
        }
    }

    /// Runs one rate-limited PIN check: `matches` says whether the PIN is
    /// right and whether it is a parent PIN.
    fn check_pin(
        &self,
        conn: &Connection,
        matches: impl FnOnce(&Connection) -> Result<(bool, bool), Error>,
    ) -> Result<PinVerdict, Error> {
        let now = now_ms();
        let attempts = lib::pin_attempts(conn, now)?;
        if attempts.locked() {
            return Ok(PinVerdict {
                ok: false,
                token: None,
                locked: true,
                attempts,
            });
        }
        let (ok, parent) = matches(conn)?;
        if !ok {
            return Ok(PinVerdict {
                ok: false,
                token: None,
                locked: false,
                attempts: lib::record_pin_failure(conn, now)?,
            });
        }
        lib::record_pin_success(conn)?;
        let token = if parent {
            Some(self.parent().issue()?)
        } else {
            None
        };
        Ok(PinVerdict {
            ok: true,
            token,
            locked: false,
            attempts: PinAttempts::default(),
        })
    }

    /// Every profile by position.
    pub fn profiles(&self) -> Result<Vec<Profile>, Error> {
        Ok(lib::list(&self.lock())?)
    }

    /// The active profile.
    pub fn active_profile(&self) -> Result<Profile, Error> {
        active(&self.lock())
    }

    /// Makes `id` the active profile. A profile with a PIN needs it
    /// ([`Error::Denied`] for a wrong or missing one, or while the wrong-PIN
    /// backoff runs). Leaving a supervised profile needs the parent token
    /// unless `id` is a parent profile (its PIN is a parent PIN). The
    /// choice is remembered for the next [`Catalog::open`]; the parent token
    /// ends.
    pub fn switch_profile(
        &self,
        id: i64,
        pin: Option<&str>,
        parent_token: Option<&str>,
    ) -> Result<Profile, Error> {
        let conn = self.lock();
        let target =
            lib::get(&conn, id)?.ok_or_else(|| Error::NotFound(format!("profile {id}")))?;
        let current = active(&conn)?;
        if target.id != current.id
            && !target.is_parent()
            && Self::guarded(&conn)?
            && !self.parent().accept(parent_token)
        {
            return Err(denied_parent());
        }
        if target.has_pin {
            let v = self.check_pin(&conn, |c| Ok((lib::verify_pin(c, id, pin)?, false)))?;
            if v.locked {
                return Err(denied_locked(v.attempts));
            }
            if !v.ok {
                return Err(Error::Denied(format!("wrong PIN for profile {id}")));
            }
        }
        lib::touch(&conn, id, now_ms())?;
        let profile = lib::activate(&conn, id)?;
        self.parent().end();
        Ok(profile)
    }

    /// Checks a parent PIN (the PIN of any profile that is not supervised).
    /// A right one returns a parent token. Counted by the wrong-PIN
    /// backoff.
    pub fn verify_parent_pin(&self, pin: &str) -> Result<PinVerdict, Error> {
        let conn = self.lock();
        self.check_pin(&conn, |c| Ok((lib::parent_pin_matches(c, pin)?, true)))
    }

    /// Checks the PIN of profile `id`; with `or_parent`, a parent PIN is
    /// right too (App Lock). A profile without a PIN takes any PIN without
    /// a check. A parent PIN returns a parent token. Counted by the
    /// wrong-PIN backoff.
    pub fn verify_profile_pin(
        &self,
        id: i64,
        pin: &str,
        or_parent: bool,
    ) -> Result<PinVerdict, Error> {
        let conn = self.lock();
        let profile =
            lib::get(&conn, id)?.ok_or_else(|| Error::NotFound(format!("profile {id}")))?;
        if !profile.has_pin && !or_parent {
            return Ok(PinVerdict {
                ok: true,
                token: None,
                locked: false,
                attempts: lib::pin_attempts(&conn, now_ms())?,
            });
        }
        self.check_pin(&conn, |c| {
            if profile.has_pin && lib::verify_pin(c, id, Some(pin))? {
                return Ok((true, profile.is_parent()));
            }
            if or_parent && lib::parent_pin_matches(c, pin)? {
                return Ok((true, true));
            }
            Ok((false, false))
        })
    }

    /// The wrong-PIN backoff now.
    pub fn pin_attempts(&self) -> Result<PinAttempts, Error> {
        Ok(lib::pin_attempts(&self.lock(), now_ms())?)
    }

    /// Ends the parent elevation (the app locked its settings again).
    pub fn end_parent_session(&self) {
        self.parent().end();
    }

    /// How long a parent token lives without use ([`PARENT_TOKEN_TTL`] by
    /// default); for tests and embedders with their own policy.
    pub fn set_parent_token_ttl(&self, ttl: Duration) {
        self.parent().ttl = ttl;
    }

    /// Creates a profile. Needs the parent token while a supervised profile
    /// is active.
    pub fn create_profile(
        &self,
        settings: &ProfileSettings,
        parent_token: Option<&str>,
    ) -> Result<Profile, Error> {
        let conn = self.lock();
        self.require_parent(&conn, parent_token)?;
        Ok(lib::create(&conn, settings, now_ms())?)
    }

    /// Replaces the settings of a profile; the active one applies them at
    /// once. Needs the parent token while a supervised profile is active.
    pub fn update_profile(
        &self,
        id: i64,
        settings: &ProfileSettings,
        parent_token: Option<&str>,
    ) -> Result<Profile, Error> {
        let conn = self.lock();
        self.require_parent(&conn, parent_token)?;
        Ok(lib::update(&conn, id, settings, now_ms())?)
    }

    /// Deletes a profile with its progress, favorites and lists. The last
    /// profile stays; deleting the active one activates the default (and
    /// ends the parent token). Needs the parent token while a supervised
    /// profile is active.
    pub fn delete_profile(&self, id: i64, parent_token: Option<&str>) -> Result<(), Error> {
        let conn = self.lock();
        self.require_parent(&conn, parent_token)?;
        let before = crate::progress::profile(&conn)?;
        lib::delete(&conn, id)?;
        if crate::progress::profile(&conn)? != before {
            self.parent().end();
        }
        Ok(())
    }

    /// Orders the profiles as `ids`. Needs the parent token while a
    /// supervised profile is active.
    pub fn reorder_profiles(&self, ids: &[i64], parent_token: Option<&str>) -> Result<(), Error> {
        let mut conn = self.lock();
        self.require_parent(&conn, parent_token)?;
        Ok(lib::reorder(&mut conn, ids, now_ms())?)
    }

    /// Sets (`Some`, 4–8 digits) or removes the PIN of a profile. Needs the
    /// parent token while a supervised profile is active.
    pub fn set_profile_pin(
        &self,
        id: i64,
        pin: Option<&str>,
        parent_token: Option<&str>,
    ) -> Result<(), Error> {
        let conn = self.lock();
        self.require_parent(&conn, parent_token)?;
        Ok(lib::set_pin(&conn, id, pin, now_ms())?)
    }

    /// The folder white list of a profile.
    pub fn profile_folders(&self, id: i64) -> Result<Vec<FolderAccess>, Error> {
        Ok(lib::folders(&self.lock(), id)?)
    }

    /// Replaces the folder white list of a profile (`path` relative to the
    /// source root, `""` for the whole source). It applies while
    /// [`ProfileSettings::restrict_folders`] is on. Needs the parent token
    /// while a supervised profile is active.
    pub fn set_profile_folders(
        &self,
        id: i64,
        folders: &[FolderAccess],
        parent_token: Option<&str>,
    ) -> Result<(), Error> {
        let mut conn = self.lock();
        self.require_parent(&conn, parent_token)?;
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
    /// removes it (`None`); it wins over the providers' ratings. Needs the
    /// parent token while a supervised profile is active.
    pub fn set_user_certification(
        &self,
        item: ItemRef,
        min_age: Option<u8>,
        parent_token: Option<&str>,
    ) -> Result<(), Error> {
        let (ItemRef::Movie(w) | ItemRef::Show(w)) = item else {
            return Err(Error::Invalid(format!(
                "ratings are per movie or show, not {item}"
            )));
        };
        let conn = self.lock();
        self.require_parent(&conn, parent_token)?;
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
