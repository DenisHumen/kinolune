// SPDX-License-Identifier: MPL-2.0
//! Profiles and parental-control filters (LIB-012).
//!
//! A profile has a name, an avatar and an optional PIN (stored as an
//! Argon2id hash, never in clear). Every piece of user state (progress,
//! favorites, lists, preferences) is keyed by `profile_id`; data recorded
//! before profiles existed belongs to the default profile (id 1).
//!
//! Parental control is a set of filters on the profile:
//!
//! - a white list of sources and folders ([`set_folders`],
//!   [`Profile::restrict_folders`]): nothing outside it is visible;
//! - a content-rating limit ([`Profile::max_age`]): works rated for an
//!   older audience are hidden. Ratings come from the metadata run
//!   ([`set_certifications`], TMDB certifications by country) or from the
//!   parent ([`set_user_certification`]) and are mapped to a minimum age
//!   ([`certification_age`]);
//! - "no rating = hidden" ([`Profile::hide_unrated`]);
//! - hidden artwork ([`Profile::hide_artwork`]).
//!
//! # The active profile of a connection
//!
//! [`activate`] copies the filters of a profile into the connection's temp
//! table `temp.active_profile`; [`open`](crate::open) activates the most
//! recently used profile. Library queries (pa-catalog) filter rows with the
//! SQL fragments [`file_visible!`](crate::file_visible) and
//! [`work_allowed!`](crate::work_allowed), which read that table, so one
//! predicate covers every list, search, "Continue watching" and the file
//! browser. Without an active row the predicates hide everything (fail
//! closed).

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::Error;

/// Id of the default profile, created by schema v1.
pub const DEFAULT_PROFILE: i64 = 1;

/// Highest age a rating limit can be.
pub const MAX_AGE: u8 = 21;

/// Shortest and longest PIN, in digits.
pub const PIN_DIGITS: std::ops::RangeInclusive<usize> = 4..=8;

/// One profile, without its PIN hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: i64,
    /// Stable id shared by the devices of a household (sync); `default` for
    /// the default profile.
    pub sync_id: String,
    /// '' = the default name, localized by the UI.
    pub name: String,
    /// Avatar reference chosen by the app (a palette index, an asset name).
    pub avatar: Option<String>,
    pub is_kid: bool,
    /// A PIN is required to switch to this profile.
    pub has_pin: bool,
    pub position: i64,
    /// Rating limit as a minimum viewer age; `None` = no limit.
    pub max_age: Option<u8>,
    /// ISO 3166-1 alpha-2 country whose rating wins ('' = the strictest).
    pub rating_country: String,
    /// Video without a known rating is hidden.
    pub hide_unrated: bool,
    /// Posters, backdrops and stills are not shown.
    pub hide_artwork: bool,
    /// Only the white-listed folders ([`folders`]) are visible.
    pub restrict_folders: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_active_at_ms: Option<i64>,
}

impl Profile {
    /// `true` if any filter hides something from this profile.
    pub fn is_restricted(&self) -> bool {
        self.max_age.is_some() || self.hide_unrated || self.restrict_folders
    }
}

/// Editable settings of a profile ([`create`], [`update`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProfileSettings {
    pub name: String,
    pub avatar: Option<String>,
    pub is_kid: bool,
    pub max_age: Option<u8>,
    pub rating_country: String,
    pub hide_unrated: bool,
    pub hide_artwork: bool,
    pub restrict_folders: bool,
}

impl ProfileSettings {
    /// Settings of `profile` as stored.
    pub fn of(profile: &Profile) -> Self {
        Self {
            name: profile.name.clone(),
            avatar: profile.avatar.clone(),
            is_kid: profile.is_kid,
            max_age: profile.max_age,
            rating_country: profile.rating_country.clone(),
            hide_unrated: profile.hide_unrated,
            hide_artwork: profile.hide_artwork,
            restrict_folders: profile.restrict_folders,
        }
    }

    fn check(&self) -> Result<(), Error> {
        if self.name.chars().count() > 64 {
            return Err(Error::Invalid("profile name"));
        }
        if self
            .avatar
            .as_ref()
            .is_some_and(|a| a.chars().count() > 256)
        {
            return Err(Error::Invalid("profile avatar"));
        }
        if self.max_age.is_some_and(|a| a > MAX_AGE) {
            return Err(Error::Invalid("rating limit"));
        }
        if !valid_country(&self.rating_country) {
            return Err(Error::Invalid("rating country"));
        }
        Ok(())
    }
}

/// One folder of a profile's white list.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FolderAccess {
    pub source_id: i64,
    /// VfsPath of the folder: `/` for the whole source, else `/a/b`.
    pub path: String,
}

impl FolderAccess {
    /// `path` normalized: one leading `/`, no trailing one; `''` and `/`
    /// mean the whole source.
    pub fn new(source_id: i64, path: &str) -> Self {
        let trimmed = path.trim_matches('/');
        Self {
            source_id,
            path: if trimmed.is_empty() {
                "/".to_owned()
            } else {
                format!("/{trimmed}")
            },
        }
    }
}

fn valid_country(c: &str) -> bool {
    c.is_empty() || (c.len() == 2 && c.bytes().all(|b| b.is_ascii_uppercase()))
}

const PROFILE_COLS: &str = "id, sync_id, name, avatar, is_kid, pin_hash IS NOT NULL, position, \
     max_age, rating_country, hide_unrated, hide_artwork, restrict_folders, created_at_ms, \
     updated_at_ms, last_active_at_ms";

fn profile_row(r: &Row<'_>) -> rusqlite::Result<Profile> {
    Ok(Profile {
        id: r.get(0)?,
        sync_id: r.get(1)?,
        name: r.get(2)?,
        avatar: r.get(3)?,
        is_kid: r.get(4)?,
        has_pin: r.get(5)?,
        position: r.get(6)?,
        max_age: r.get(7)?,
        rating_country: r.get(8)?,
        hide_unrated: r.get(9)?,
        hide_artwork: r.get(10)?,
        restrict_folders: r.get(11)?,
        created_at_ms: r.get(12)?,
        updated_at_ms: r.get(13)?,
        last_active_at_ms: r.get(14)?,
    })
}

/// Every profile by position, then id.
pub fn list(conn: &Connection) -> Result<Vec<Profile>, Error> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {PROFILE_COLS} FROM profiles ORDER BY position, id"
    ))?;
    let rows = stmt.query_map([], profile_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// One profile, `None` if there is none with this id.
pub fn get(conn: &Connection, id: i64) -> Result<Option<Profile>, Error> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {PROFILE_COLS} FROM profiles WHERE id = ?1"
        ))?
        .query_row([id], profile_row)
        .optional()?)
}

fn require(conn: &Connection, id: i64) -> Result<Profile, Error> {
    get(conn, id)?.ok_or(Error::Invalid("unknown profile"))
}

/// Random lowercase hex of `n` bytes.
fn random_hex(n: usize) -> Result<String, Error> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).map_err(|_| Error::Invalid("system random source"))?;
    Ok(hex(&buf))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Creates a profile at the end of the list.
pub fn create(
    conn: &Connection,
    settings: &ProfileSettings,
    now_ms: i64,
) -> Result<Profile, Error> {
    settings.check()?;
    let sync_id = random_hex(16)?;
    conn.prepare_cached(
        "INSERT INTO profiles (sync_id, name, avatar, is_kid, position, max_age, rating_country,
                               hide_unrated, hide_artwork, restrict_folders, created_at_ms,
                               updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, (SELECT ifnull(max(position), -1) + 1 FROM profiles), ?5, ?6,
                 ?7, ?8, ?9, ?10, ?10)",
    )?
    .execute(params![
        sync_id,
        settings.name,
        settings.avatar,
        settings.is_kid,
        settings.max_age,
        settings.rating_country,
        settings.hide_unrated,
        settings.hide_artwork,
        settings.restrict_folders,
        now_ms,
    ])?;
    require(conn, conn.last_insert_rowid())
}

/// Replaces the settings of a profile. The active profile of the
/// connection picks the new filters up at once.
pub fn update(
    conn: &Connection,
    id: i64,
    settings: &ProfileSettings,
    now_ms: i64,
) -> Result<Profile, Error> {
    settings.check()?;
    let n = conn
        .prepare_cached(
            "UPDATE profiles SET name = ?2, avatar = ?3, is_kid = ?4, max_age = ?5,
                    rating_country = ?6, hide_unrated = ?7, hide_artwork = ?8,
                    restrict_folders = ?9, updated_at_ms = ?10
             WHERE id = ?1",
        )?
        .execute(params![
            id,
            settings.name,
            settings.avatar,
            settings.is_kid,
            settings.max_age,
            settings.rating_country,
            settings.hide_unrated,
            settings.hide_artwork,
            settings.restrict_folders,
            now_ms,
        ])?;
    if n == 0 {
        return Err(Error::Invalid("unknown profile"));
    }
    refresh(conn)?;
    require(conn, id)
}

/// Deletes a profile and all its state. The last profile cannot be
/// deleted; when the active one goes, the default profile (or the first
/// one left) becomes active.
pub fn delete(conn: &Connection, id: i64) -> Result<(), Error> {
    require(conn, id)?;
    let count: i64 = conn.query_row("SELECT count(*) FROM profiles", [], |r| r.get(0))?;
    if count <= 1 {
        return Err(Error::Invalid("last profile"));
    }
    conn.execute("DELETE FROM profiles WHERE id = ?1", [id])?;
    if active(conn)? == Some(id) {
        let next: i64 = conn.query_row(
            "SELECT id FROM profiles ORDER BY id <> ?1, position, id LIMIT 1",
            [DEFAULT_PROFILE],
            |r| r.get(0),
        )?;
        activate(conn, next)?;
    }
    Ok(())
}

/// Orders the profiles as `ids`; unlisted ones follow in their order.
pub fn reorder(conn: &mut Connection, ids: &[i64], now_ms: i64) -> Result<(), Error> {
    let tx = conn.transaction()?;
    let rest: Vec<i64> = list(&tx)?
        .into_iter()
        .map(|p| p.id)
        .filter(|id| !ids.contains(id))
        .collect();
    for (pos, id) in ids.iter().chain(rest.iter()).enumerate() {
        tx.execute(
            "UPDATE profiles SET position = ?2, updated_at_ms = ?3 WHERE id = ?1",
            params![id, i64::try_from(pos).unwrap_or(i64::MAX), now_ms],
        )?;
    }
    tx.commit()?;
    Ok(())
}

// --- PIN ---------------------------------------------------------------------

/// Argon2id parameters of new PIN hashes (OWASP minimum: 19 MiB, 2 passes).
/// Stored with each hash, so they can grow without breaking old ones.
const ARGON2_M_KIB: u32 = 19 * 1024;
const ARGON2_T: u32 = 2;
const ARGON2_P: u32 = 1;
const HASH_LEN: usize = 32;
const SALT_LEN: usize = 16;

fn check_pin(pin: &str) -> Result<(), Error> {
    if !PIN_DIGITS.contains(&pin.len()) || !pin.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Invalid("PIN (4-8 digits)"));
    }
    Ok(())
}

fn argon2_raw(pin: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Result<Vec<u8>, Error> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params =
        Params::new(m, t, p, Some(HASH_LEN)).map_err(|_| Error::Invalid("Argon2 parameters"))?;
    let mut out = vec![0u8; HASH_LEN];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(pin.as_bytes(), salt, &mut out)
        .map_err(|_| Error::Invalid("Argon2 input"))?;
    Ok(out)
}

/// Hashes a PIN: `argon2id$v=19$m=<KiB>,t=<passes>,p=<lanes>$<salt>$<hash>`
/// with a random salt, hex-encoded.
pub fn hash_pin(pin: &str) -> Result<String, Error> {
    check_pin(pin)?;
    let mut salt = [0u8; SALT_LEN];
    getrandom::fill(&mut salt).map_err(|_| Error::Invalid("system random source"))?;
    let hash = argon2_raw(pin, &salt, ARGON2_M_KIB, ARGON2_T, ARGON2_P)?;
    Ok(format!(
        "argon2id$v=19$m={ARGON2_M_KIB},t={ARGON2_T},p={ARGON2_P}${}${}",
        hex(&salt),
        hex(&hash)
    ))
}

/// `true` if `pin` matches a hash made by [`hash_pin`]. A malformed hash
/// matches nothing.
pub fn pin_matches(stored: &str, pin: &str) -> bool {
    let mut parts = stored.split('$');
    let (Some("argon2id"), Some("v=19"), Some(params), Some(salt), Some(hash), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return false;
    };
    let mut m = None;
    let mut t = None;
    let mut p = None;
    for kv in params.split(',') {
        match kv.split_once('=') {
            Some(("m", v)) => m = v.parse::<u32>().ok(),
            Some(("t", v)) => t = v.parse::<u32>().ok(),
            Some(("p", v)) => p = v.parse::<u32>().ok(),
            _ => return false,
        }
    }
    let (Some(m), Some(t), Some(p), Some(salt), Some(expected)) =
        (m, t, p, unhex(salt), unhex(hash))
    else {
        return false;
    };
    // Bound the work a stored value can ask for (a tampered row must not
    // make the check take minutes).
    if m > 256 * 1024 || t > 16 || p > 8 || expected.len() != HASH_LEN {
        return false;
    }
    match argon2_raw(pin, &salt, m, t, p) {
        // Constant time over the hash bytes.
        Ok(got) => {
            got.iter()
                .zip(&expected)
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
        }
        Err(_) => false,
    }
}

/// Sets (`Some`) or removes (`None`) the PIN of a profile.
pub fn set_pin(conn: &Connection, id: i64, pin: Option<&str>, now_ms: i64) -> Result<(), Error> {
    let hash = pin.map(hash_pin).transpose()?;
    let n = conn.execute(
        "UPDATE profiles SET pin_hash = ?2, updated_at_ms = ?3 WHERE id = ?1",
        params![id, hash, now_ms],
    )?;
    if n == 0 {
        return Err(Error::Invalid("unknown profile"));
    }
    Ok(())
}

/// `true` if the profile has no PIN or `pin` is its PIN.
pub fn verify_pin(conn: &Connection, id: i64, pin: Option<&str>) -> Result<bool, Error> {
    let stored: Option<String> = conn
        .query_row("SELECT pin_hash FROM profiles WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?
        .ok_or(Error::Invalid("unknown profile"))?;
    Ok(match (stored, pin) {
        (None, _) => true,
        (Some(hash), Some(pin)) => pin_matches(&hash, pin),
        (Some(_), None) => false,
    })
}

// --- Folder white list --------------------------------------------------------

/// White list of a profile, by source and path.
pub fn folders(conn: &Connection, id: i64) -> Result<Vec<FolderAccess>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT source_id, path FROM profile_folders WHERE profile_id = ?1 ORDER BY source_id, path",
    )?;
    let rows = stmt.query_map([id], |r| {
        Ok(FolderAccess {
            source_id: r.get(0)?,
            path: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Replaces the white list of a profile. It only applies while
/// [`Profile::restrict_folders`] is on.
pub fn set_folders(
    conn: &mut Connection,
    id: i64,
    folders: &[FolderAccess],
    now_ms: i64,
) -> Result<(), Error> {
    let tx = conn.transaction()?;
    require(&tx, id)?;
    tx.execute("DELETE FROM profile_folders WHERE profile_id = ?1", [id])?;
    for f in folders {
        let f = FolderAccess::new(f.source_id, &f.path);
        tx.execute(
            "INSERT OR IGNORE INTO profile_folders (profile_id, source_id, path) VALUES (?1, ?2, ?3)",
            params![id, f.source_id, f.path],
        )?;
    }
    tx.execute(
        "UPDATE profiles SET updated_at_ms = ?2 WHERE id = ?1",
        params![id, now_ms],
    )?;
    tx.commit()?;
    Ok(())
}

// --- Active profile -------------------------------------------------------------

const TEMP_TABLE: &str = "
CREATE TEMP TABLE IF NOT EXISTS active_profile (
    one              INTEGER PRIMARY KEY CHECK (one = 1),
    profile_id       INTEGER NOT NULL,
    max_age          INTEGER,
    rating_country   TEXT    NOT NULL,
    hide_unrated     INTEGER NOT NULL,
    hide_artwork     INTEGER NOT NULL,
    restrict_folders INTEGER NOT NULL
)";

/// Makes `id` the active profile of this connection: its filters apply to
/// the queries that use [`file_visible!`](crate::file_visible) and
/// [`work_allowed!`](crate::work_allowed). Does not check the PIN and does
/// not record the switch ([`touch`]).
pub fn activate(conn: &Connection, id: i64) -> Result<Profile, Error> {
    let profile = require(conn, id)?;
    conn.execute_batch(TEMP_TABLE)?;
    conn.execute(
        "INSERT OR REPLACE INTO temp.active_profile
             (one, profile_id, max_age, rating_country, hide_unrated, hide_artwork, restrict_folders)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            profile.id,
            profile.max_age,
            profile.rating_country,
            profile.hide_unrated,
            profile.hide_artwork,
            profile.restrict_folders,
        ],
    )?;
    Ok(profile)
}

/// Activates the most recently used profile (the default one if none was
/// ever switched to). [`open`](crate::open) calls it.
pub fn activate_recent(conn: &Connection) -> Result<Profile, Error> {
    let id: i64 = conn.query_row(
        "SELECT id FROM profiles
         ORDER BY last_active_at_ms IS NULL, last_active_at_ms DESC, id <> ?1, id LIMIT 1",
        [DEFAULT_PROFILE],
        |r| r.get(0),
    )?;
    activate(conn, id)
}

/// Records that the profile was switched to now, so the next
/// [`activate_recent`] picks it.
pub fn touch(conn: &Connection, id: i64, now_ms: i64) -> Result<(), Error> {
    conn.execute(
        "UPDATE profiles SET last_active_at_ms = ?2 WHERE id = ?1",
        params![id, now_ms],
    )?;
    Ok(())
}

/// The active profile of this connection, `None` before [`activate`].
pub fn active(conn: &Connection) -> Result<Option<i64>, Error> {
    let exists: bool = conn.query_row(
        "SELECT count(*) FROM sqlite_temp_schema WHERE name = 'active_profile'",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(None);
    }
    Ok(conn
        .query_row("SELECT profile_id FROM temp.active_profile", [], |r| {
            r.get(0)
        })
        .optional()?)
}

/// Re-reads the filters of the active profile (after its settings or the
/// white list changed).
pub fn refresh(conn: &Connection) -> Result<(), Error> {
    if let Some(id) = active(conn)? {
        activate(conn, id)?;
    }
    Ok(())
}

/// `true` if the active profile hides artwork.
pub fn artwork_hidden(conn: &Connection) -> Result<bool, Error> {
    if active(conn)?.is_none() {
        return Ok(false);
    }
    Ok(conn
        .query_row("SELECT hide_artwork FROM temp.active_profile", [], |r| {
            r.get(0)
        })
        .optional()?
        .unwrap_or(false))
}

/// SQL: `1` if the file row `$f` (`files` alias) lies in the active
/// profile's white list, or the profile has none; `0` otherwise and when
/// no profile is active.
#[macro_export]
macro_rules! file_visible {
    ($f:literal) => {
        concat!(
            "(ifnull((SELECT ap.restrict_folders = 0 FROM temp.active_profile ap), 0)
  OR EXISTS (SELECT 1 FROM temp.active_profile ap
               JOIN profile_folders pf ON pf.profile_id = ap.profile_id
              WHERE pf.source_id = ",
            $f,
            ".source_id
                AND (pf.path = '/' OR ",
            $f,
            ".path = pf.path
                     OR substr(",
            $f,
            ".path, 1, length(pf.path) + 1) = pf.path || '/')))"
        )
    };
}

/// SQL: the minimum age of the work `$w` (an expression, e.g. `w.id`) for
/// the rating country `$country` (an expression): the parent's rating,
/// else the country's, else the strictest known one; NULL when unrated.
#[macro_export]
macro_rules! work_age {
    ($w:literal, $country:literal) => {
        concat!(
            "coalesce(
  (SELECT c.min_age FROM work_certifications c
    WHERE c.work_id = ",
            $w,
            " AND c.provider = 'user'),
  (SELECT max(c.min_age) FROM work_certifications c
    WHERE c.work_id = ",
            $w,
            " AND c.country = ",
            $country,
            " AND c.country <> ''),
  (SELECT max(c.min_age) FROM work_certifications c WHERE c.work_id = ",
            $w,
            "))"
        )
    };
}

/// SQL: `1` if the active profile's rating filters allow the work `$w` (an
/// expression, e.g. `w.id`); `0` otherwise and when no profile is active.
#[macro_export]
macro_rules! work_allowed {
    ($w:literal) => {
        concat!(
            "ifnull((SELECT CASE WHEN ap.max_age IS NULL AND ap.hide_unrated = 0 THEN 1
              ELSE ifnull(",
            $crate::work_age!($w, "ap.rating_country"),
            " <= ifnull(ap.max_age, 255), ap.hide_unrated = 0) END
  FROM temp.active_profile ap), 0)"
        )
    };
}

/// SQL: `1` if the active profile hides video without a rating.
#[macro_export]
macro_rules! unrated_hidden {
    () => {
        "ifnull((SELECT ap.hide_unrated FROM temp.active_profile ap), 1)"
    };
}

/// SQL: `1` if the active profile may see the file row `$f` (`files`
/// alias) itself, as the file browser and the player reach it: the file is
/// in the white list ([`file_visible!`](crate::file_visible)), no live match
/// ties it to a work the rating filters hide, and an unrecognized file
/// that is not a music track counts as unrated.
#[macro_export]
macro_rules! file_allowed {
    ($f:literal) => {
        concat!(
            "(",
            $crate::file_visible!($f),
            "
  AND NOT EXISTS (SELECT 1 FROM matches fm WHERE fm.file_id = ",
            $f,
            ".id AND fm.state <> 'rejected'
                     AND NOT ",
            $crate::work_allowed!("fm.work_id"),
            ")
  AND (NOT ",
            $crate::unrated_hidden!(),
            "
       OR EXISTS (SELECT 1 FROM matches fm WHERE fm.file_id = ",
            $f,
            ".id AND fm.state <> 'rejected')
       OR EXISTS (SELECT 1 FROM music_tracks ft WHERE ft.file_id = ",
            $f,
            ".id)))"
        )
    };
}

// --- Certifications ------------------------------------------------------------

/// One content rating of a work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Certification {
    /// `tmdb`, `user`…
    pub provider: String,
    /// ISO 3166-1 alpha-2; '' for the user's rating.
    pub country: String,
    /// As written by the provider: `PG-13`, `TV-MA`, `16`, `18+`.
    pub certification: String,
    /// Minimum viewer age, `None` if the rating is not understood.
    pub min_age: Option<u8>,
}

/// Minimum viewer age a certification stands for, `None` if unknown (`NR`,
/// `Unrated`, an unknown scheme).
///
/// Known schemes: US (MPA and TV Parental Guidelines), GB (BBFC), AU, CA,
/// JP, KR, BR, NL and plain ages (`16`, `16+`, `FSK 16`, `-16`, `R18+`)
/// used by RU, DE, FR, ES, IT and most others.
pub fn certification_age(country: &str, certification: &str) -> Option<u8> {
    let c = certification.trim().to_ascii_uppercase();
    if c.is_empty() {
        return None;
    }
    let by_table = match (country, c.as_str()) {
        // MPA, TV Parental Guidelines (also used by many other countries).
        (_, "G" | "TV-Y" | "TV-G") => Some(0),
        (_, "TV-Y7" | "TV-Y7-FV") => Some(7),
        (_, "PG" | "TV-PG") => Some(match country {
            "GB" | "IE" | "AU" | "NZ" => 8,
            _ => 10,
        }),
        (_, "PG-13") => Some(13),
        (_, "TV-14") => Some(14),
        (_, "R" | "TV-MA") => Some(17),
        (_, "NC-17" | "X" | "XXX" | "R18" | "X18+" | "R18+" | "R-18" | "18A") => Some(18),
        // BBFC.
        (_, "U" | "UC") => Some(0),
        ("GB" | "IE", "12A") => Some(12),
        // Australia.
        ("AU" | "NZ", "M") => Some(15),
        ("AU", "MA15+") => Some(15),
        (_, "E") => Some(0),
        // Canada (also 14A, 18A above).
        ("CA", "14A") => Some(14),
        // Japan (Eirin).
        ("JP", "PG12") => Some(12),
        ("JP", "R15+") => Some(15),
        // Brazil, Netherlands, Korea.
        ("BR", "L") | ("NL", "AL") | ("KR", "ALL") => Some(0),
        _ => None,
    };
    if by_table.is_some() {
        return by_table;
    }
    // Plain ages: `16`, `16+`, `FSK 16`, `-16`, `R15+`, `12A`, `KN`…
    let digits: String = c
        .trim_start_matches("FSK")
        .trim_start_matches(|ch: char| !ch.is_ascii_digit())
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let age: u8 = digits.parse().ok()?;
    // Korea rates adult content 19.
    (age <= 21).then(|| age.min(18))
}

/// Ratings of a work, the user's first.
pub fn certifications(conn: &Connection, work_id: i64) -> Result<Vec<Certification>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT provider, country, certification, min_age FROM work_certifications
         WHERE work_id = ?1 ORDER BY provider <> 'user', provider, country",
    )?;
    let rows = stmt.query_map([work_id], |r| {
        Ok(Certification {
            provider: r.get(0)?,
            country: r.get(1)?,
            certification: r.get(2)?,
            min_age: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Replaces the ratings of `provider` for a work: `(country, certification)`
/// pairs; the minimum age is derived with [`certification_age`]. Invalid
/// countries and empty certifications are skipped.
pub fn set_certifications(
    conn: &Connection,
    work_id: i64,
    provider: &str,
    ratings: &[(String, String)],
) -> Result<(), Error> {
    if provider == "user" {
        return Err(Error::Invalid("provider (use set_user_certification)"));
    }
    conn.execute(
        "DELETE FROM work_certifications WHERE work_id = ?1 AND provider = ?2",
        params![work_id, provider],
    )?;
    let mut stmt = conn.prepare_cached(
        "INSERT OR REPLACE INTO work_certifications
             (work_id, provider, country, certification, min_age)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for (country, cert) in ratings {
        let cert = cert.trim();
        if cert.is_empty() || country.is_empty() || !valid_country(country) {
            continue;
        }
        stmt.execute(params![
            work_id,
            provider,
            country,
            cert,
            certification_age(country, cert)
        ])?;
    }
    Ok(())
}

/// Sets the parent's own rating of a work as a minimum age (it wins over
/// every provider's), or removes it.
pub fn set_user_certification(
    conn: &Connection,
    work_id: i64,
    min_age: Option<u8>,
) -> Result<(), Error> {
    conn.execute(
        "DELETE FROM work_certifications WHERE work_id = ?1 AND provider = 'user'",
        [work_id],
    )?;
    if let Some(age) = min_age {
        if age > MAX_AGE {
            return Err(Error::Invalid("rating age"));
        }
        conn.execute(
            "INSERT INTO work_certifications (work_id, provider, country, certification, min_age)
             VALUES (?1, 'user', '', ?2, ?3)",
            params![work_id, format!("{age}+"), age],
        )?;
    }
    Ok(())
}

/// Minimum age of a work for the rating country `country` ('' = the
/// strictest known), `None` if unrated.
pub fn work_age(conn: &Connection, work_id: i64, country: &str) -> Result<Option<u8>, Error> {
    let sql = concat!("SELECT ", crate::work_age!("?1", "?2"));
    Ok(conn.query_row(sql, params![work_id, country], |r| r.get(0))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certification_ages() {
        let cases = [
            ("US", "G", Some(0)),
            ("US", "PG", Some(10)),
            ("US", "PG-13", Some(13)),
            ("US", "R", Some(17)),
            ("US", "NC-17", Some(18)),
            ("US", "TV-Y7", Some(7)),
            ("US", "TV-MA", Some(17)),
            ("US", "NR", None),
            ("GB", "PG", Some(8)),
            ("GB", "12A", Some(12)),
            ("GB", "15", Some(15)),
            ("GB", "18", Some(18)),
            ("RU", "0+", Some(0)),
            ("RU", "16+", Some(16)),
            ("DE", "FSK 12", Some(12)),
            ("DE", "6", Some(6)),
            ("FR", "-16", Some(16)),
            ("KR", "19", Some(18)),
            ("JP", "R15+", Some(15)),
            ("AU", "MA15+", Some(15)),
            ("AU", "R18+", Some(18)),
            ("BR", "L", Some(0)),
            ("US", "Unrated", None),
            ("US", "", None),
        ];
        for (country, cert, age) in cases {
            assert_eq!(certification_age(country, cert), age, "{country} {cert}");
        }
    }

    #[test]
    fn pin_hash_round_trip() {
        let hash = hash_pin("2468").unwrap();
        assert!(hash.starts_with("argon2id$v=19$m=19456,t=2,p=1$"), "{hash}");
        assert!(!hash.contains("2468"));
        assert!(pin_matches(&hash, "2468"));
        assert!(!pin_matches(&hash, "2469"));
        // Salted: the same PIN hashes differently.
        assert_ne!(hash, hash_pin("2468").unwrap());
        assert!(!pin_matches("argon2id$v=19$m=1,t=1,p=1$zz$zz", "2468"));
        assert!(!pin_matches("plain", "2468"));
    }

    #[test]
    fn pin_format_is_checked() {
        for bad in ["", "123", "123456789", "12a4", "١٢٣٤"] {
            assert!(hash_pin(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn folder_paths_are_normalized() {
        assert_eq!(FolderAccess::new(1, "").path, "/");
        assert_eq!(FolderAccess::new(1, "/").path, "/");
        assert_eq!(FolderAccess::new(1, "Kids/").path, "/Kids");
        assert_eq!(FolderAccess::new(1, "/a/b/").path, "/a/b");
    }
}
