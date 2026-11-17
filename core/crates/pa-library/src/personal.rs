// SPDX-License-Identifier: MPL-2.0
//! Personal media (UI-025, "Личные медиа"): libraries of the user's own
//! videos that are never recognized and never part of the main library.
//!
//! A profile has any number of personal libraries ([`create`]); each has a
//! lock ([`Lock`]): none, a PIN (4–8 digits) or a password, stored as an
//! Argon2id hash like profile PINs (ADR-0055). Files get into a library in
//! two ways:
//!
//! - a folder ([`add_folder`]): every file under it, `/` for a whole
//!   source. A source whose [`ContentType`] is `personal` is never
//!   recognized; its files are personal even before a folder ties them to a
//!   library;
//! - a file moved from the main library ([`move_files`]): it keeps its
//!   match, so [`unmove_files`] restores the item as it was.
//!
//! # Scope
//!
//! The SQL fragment [`file_in_scope!`](crate::file_in_scope), part of
//! [`file_visible!`](crate::file_visible), decides which library a
//! connection reads. Without a row in `temp.personal_scope` it is the main
//! library, where no personal file exists; with one ([`set_scope`]) it is
//! that personal library and nothing else. The catalog sets the scope only
//! for the duration of one call on an unlocked library, so every list,
//! search and lookup of the main library leaves personal files out with
//! the same predicate as the parental filters.

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::Error;
use crate::profiles::{PIN_DIGITS, hash_pin, hash_secret, pin_matches, random_hex};

/// Shortest and longest password of a personal library, in characters.
pub const PASSWORD_CHARS: std::ops::RangeInclusive<usize> = 6..=128;

/// What a source holds (`sources.content_type`), chosen when it is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ContentType {
    /// Movies and shows: recognized and matched with providers.
    #[default]
    Video,
    /// Music: tags are read, video files are not recognized.
    Music,
    /// The user's own videos: never recognized, never sent to a provider,
    /// only in personal libraries.
    Personal,
}

impl ContentType {
    /// Value of `sources.content_type`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Music => "music",
            Self::Personal => "personal",
        }
    }

    /// From `sources.content_type`.
    pub fn parse(s: &str) -> Result<Self, Error> {
        match s {
            "video" => Ok(Self::Video),
            "music" => Ok(Self::Music),
            "personal" => Ok(Self::Personal),
            _ => Err(Error::Invalid("content type")),
        }
    }
}

/// How a personal library is locked, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LockKind {
    #[default]
    None,
    Pin,
    Password,
}

impl LockKind {
    /// Value of `personal_libraries.lock_kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pin => "pin",
            Self::Password => "password",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "pin" => Self::Pin,
            "password" => Self::Password,
            _ => Self::None,
        }
    }
}

/// A new lock with its secret.
#[derive(Clone, PartialEq, Eq)]
pub enum Lock<'a> {
    None,
    /// 4–8 digits.
    Pin(&'a str),
    /// 6–128 characters.
    Password(&'a str),
}

impl std::fmt::Debug for Lock<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the secret.
        f.write_str(self.kind().as_str())
    }
}

impl Lock<'_> {
    /// The kind of this lock.
    pub fn kind(&self) -> LockKind {
        match self {
            Self::None => LockKind::None,
            Self::Pin(_) => LockKind::Pin,
            Self::Password(_) => LockKind::Password,
        }
    }

    /// The Argon2id hash to store, `None` without a lock.
    fn hash(&self) -> Result<Option<String>, Error> {
        match self {
            Self::None => Ok(None),
            Self::Pin(pin) => hash_pin(pin).map(Some),
            Self::Password(p) => {
                let n = p.chars().count();
                if !PASSWORD_CHARS.contains(&n) || p.trim().is_empty() {
                    return Err(Error::Invalid("password (6-128 characters)"));
                }
                hash_secret(p).map(Some)
            }
        }
    }
}

/// A personal library, without its lock hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalLibrary {
    pub id: i64,
    pub profile_id: i64,
    pub sync_id: String,
    pub name: String,
    pub lock: LockKind,
    pub position: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// A folder of a personal library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalFolder {
    pub source_id: i64,
    /// VfsPath of the folder, `/` for the whole source.
    pub path: String,
}

const COLUMNS: &str =
    "id, profile_id, sync_id, name, lock_kind, position, created_at_ms, updated_at_ms";

fn from_row(r: &Row<'_>) -> rusqlite::Result<PersonalLibrary> {
    Ok(PersonalLibrary {
        id: r.get(0)?,
        profile_id: r.get(1)?,
        sync_id: r.get(2)?,
        name: r.get(3)?,
        lock: LockKind::parse(&r.get::<_, String>(4)?),
        position: r.get(5)?,
        created_at_ms: r.get(6)?,
        updated_at_ms: r.get(7)?,
    })
}

fn clean_name(name: &str) -> Result<String, Error> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.chars().count() > 100 {
        return Err(Error::Invalid("library name (1-100 characters)"));
    }
    Ok(name)
}

/// A folder path as stored: `/`-rooted, no trailing slash, `/` for the
/// root.
pub fn folder_path(path: &str) -> String {
    let trimmed = path.trim().trim_matches('/');
    if trimmed.is_empty() {
        "/".to_owned()
    } else {
        format!("/{trimmed}")
    }
}

/// Creates a library of `profile_id` at the end of its list.
pub fn create(
    conn: &Connection,
    profile_id: i64,
    name: &str,
    lock: &Lock<'_>,
    now_ms: i64,
) -> Result<PersonalLibrary, Error> {
    let name = clean_name(name)?;
    let hash = lock.hash()?;
    conn.execute(
        "INSERT INTO personal_libraries
             (profile_id, sync_id, name, lock_kind, lock_hash, position, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5,
                 (SELECT ifnull(max(position), 0) + 1 FROM personal_libraries WHERE profile_id = ?1),
                 ?6, ?6)",
        params![
            profile_id,
            random_hex(16)?,
            name,
            lock.kind().as_str(),
            hash,
            now_ms
        ],
    )?;
    get(conn, conn.last_insert_rowid())?.ok_or(Error::Invalid("unknown personal library"))
}

/// One library.
pub fn get(conn: &Connection, id: i64) -> Result<Option<PersonalLibrary>, Error> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM personal_libraries WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?)
}

/// The libraries of `profile_id` in the user's order.
pub fn list(conn: &Connection, profile_id: i64) -> Result<Vec<PersonalLibrary>, Error> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM personal_libraries WHERE profile_id = ?1 ORDER BY position, id"
    ))?;
    let rows = stmt.query_map([profile_id], from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Renames a library.
pub fn rename(conn: &Connection, id: i64, name: &str, now_ms: i64) -> Result<(), Error> {
    let name = clean_name(name)?;
    let n = conn.execute(
        "UPDATE personal_libraries SET name = ?2, updated_at_ms = ?3 WHERE id = ?1",
        params![id, name, now_ms],
    )?;
    if n == 0 {
        return Err(Error::Invalid("unknown personal library"));
    }
    Ok(())
}

/// Replaces the lock of a library.
pub fn set_lock(conn: &Connection, id: i64, lock: &Lock<'_>, now_ms: i64) -> Result<(), Error> {
    let hash = lock.hash()?;
    let n = conn.execute(
        "UPDATE personal_libraries SET lock_kind = ?2, lock_hash = ?3, updated_at_ms = ?4
         WHERE id = ?1",
        params![id, lock.kind().as_str(), hash, now_ms],
    )?;
    if n == 0 {
        return Err(Error::Invalid("unknown personal library"));
    }
    Ok(())
}

/// `true` if the library has no lock or `secret` opens it. A PIN lock
/// takes only digits of a PIN's length; anything else is wrong without a
/// hash computation.
pub fn verify_secret(conn: &Connection, id: i64, secret: &str) -> Result<bool, Error> {
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT lock_kind, lock_hash FROM personal_libraries WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((kind, hash)) = row else {
        return Ok(false);
    };
    match (LockKind::parse(&kind), hash) {
        (LockKind::None, _) => Ok(true),
        (LockKind::Pin, Some(h)) => Ok(PIN_DIGITS.contains(&secret.len())
            && secret.bytes().all(|b| b.is_ascii_digit())
            && pin_matches(&h, secret)),
        (LockKind::Password, Some(h)) => Ok(pin_matches(&h, secret)),
        _ => Ok(false),
    }
}

/// Deletes a library with its folders and moved-file marks: moved files go
/// back to the main library. Files of `personal` sources stay personal.
pub fn delete(conn: &Connection, id: i64) -> Result<(), Error> {
    conn.execute("DELETE FROM personal_libraries WHERE id = ?1", [id])?;
    Ok(())
}

/// Folders of a library.
pub fn folders(conn: &Connection, id: i64) -> Result<Vec<PersonalFolder>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT source_id, path FROM personal_library_folders WHERE library_id = ?1
         ORDER BY source_id, path",
    )?;
    let rows = stmt.query_map([id], |r| {
        Ok(PersonalFolder {
            source_id: r.get(0)?,
            path: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Adds a folder (`path` relative to the source root, `""` or `/` for the
/// whole source) to a library. A folder already in another library is
/// `Invalid`; in this one, nothing changes.
pub fn add_folder(
    conn: &Connection,
    id: i64,
    source_id: i64,
    path: &str,
    now_ms: i64,
) -> Result<(), Error> {
    let path = folder_path(path);
    let other: Option<i64> = conn
        .query_row(
            "SELECT library_id FROM personal_library_folders WHERE source_id = ?1 AND path = ?2",
            params![source_id, path],
            |r| r.get(0),
        )
        .optional()?;
    match other {
        Some(l) if l == id => Ok(()),
        Some(_) => Err(Error::Invalid("folder is in another personal library")),
        None => {
            conn.execute(
                "INSERT INTO personal_library_folders (source_id, path, library_id, added_at_ms)
                 VALUES (?1, ?2, ?3, ?4)",
                params![source_id, path, id, now_ms],
            )?;
            Ok(())
        }
    }
}

/// Removes a folder from a library; `false` if it was not there.
pub fn remove_folder(
    conn: &Connection,
    id: i64,
    source_id: i64,
    path: &str,
) -> Result<bool, Error> {
    Ok(conn.execute(
        "DELETE FROM personal_library_folders
         WHERE library_id = ?1 AND source_id = ?2 AND path = ?3",
        params![id, source_id, folder_path(path)],
    )? > 0)
}

/// Moves files from the main library into library `id`.
pub fn move_files(conn: &Connection, id: i64, file_ids: &[i64], now_ms: i64) -> Result<(), Error> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO personal_library_files (file_id, library_id, moved_at_ms) VALUES (?1, ?2, ?3)
         ON CONFLICT (file_id) DO UPDATE SET library_id = excluded.library_id,
                                             moved_at_ms = excluded.moved_at_ms",
    )?;
    for f in file_ids {
        stmt.execute(params![f, id, now_ms])?;
    }
    Ok(())
}

/// Moves files back to the main library; how many were moved.
pub fn unmove_files(conn: &Connection, file_ids: &[i64]) -> Result<usize, Error> {
    let mut stmt = conn.prepare_cached("DELETE FROM personal_library_files WHERE file_id = ?1")?;
    let mut n = 0;
    for f in file_ids {
        n += stmt.execute([f])?;
    }
    Ok(n)
}

/// `true` if the file was moved from the main library (not a file of a
/// personal folder).
pub fn is_moved(conn: &Connection, file_id: i64) -> Result<bool, Error> {
    Ok(conn
        .prepare_cached("SELECT 1 FROM personal_library_files WHERE file_id = ?1")?
        .exists([file_id])?)
}

/// The personal library of a file: `None` for a main-library file,
/// `Some(None)` for a file of a `personal` source that no library holds.
pub fn library_of_file(conn: &Connection, file_id: i64) -> Result<Option<Option<i64>>, Error> {
    Ok(conn
        .prepare_cached(concat!(
            "SELECT ",
            crate::file_personal_library!("f"),
            ", ",
            crate::file_personal!("f"),
            " FROM files f WHERE f.id = ?1"
        ))?
        .query_row([file_id], |r| {
            Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, bool>(1)?))
        })
        .optional()?
        .and_then(|(lib, personal)| personal.then_some(lib)))
}

/// The personal library of a path (VfsPath) of a source that need not be a
/// library file (a sidecar subtitle, a folder): as [`library_of_file`].
pub fn library_of_path(
    conn: &Connection,
    source_id: i64,
    path: &str,
) -> Result<Option<Option<i64>>, Error> {
    let path = folder_path(path);
    let file: Option<i64> = conn
        .query_row(
            "SELECT id FROM files WHERE source_id = ?1 AND path = ?2",
            params![source_id, path],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(f) = file {
        return library_of_file(conn, f);
    }
    let folder: Option<i64> = conn
        .query_row(
            "SELECT library_id FROM personal_library_folders
             WHERE source_id = ?1
               AND (path = '/' OR ?2 = path OR substr(?2, 1, length(path) + 1) = path || '/')
             ORDER BY length(path) DESC LIMIT 1",
            params![source_id, path],
            |r| r.get(0),
        )
        .optional()?;
    if folder.is_some() {
        return Ok(Some(folder));
    }
    Ok((content_type(conn, source_id)? == Some(ContentType::Personal)).then_some(None))
}

/// The content type of a source, `None` for an unknown source.
pub fn content_type(conn: &Connection, source_id: i64) -> Result<Option<ContentType>, Error> {
    let s: Option<String> = conn
        .query_row(
            "SELECT content_type FROM sources WHERE id = ?1",
            [source_id],
            |r| r.get(0),
        )
        .optional()?;
    s.map(|s| ContentType::parse(&s)).transpose()
}

/// Sets the content type of a source.
pub fn set_content_type(
    conn: &Connection,
    source_id: i64,
    content: ContentType,
) -> Result<(), Error> {
    let n = conn.execute(
        "UPDATE sources SET content_type = ?2 WHERE id = ?1",
        params![source_id, content.as_str()],
    )?;
    if n == 0 {
        return Err(Error::Invalid("unknown source"));
    }
    Ok(())
}

/// Makes the connection read personal library `id` (`Some`) or the main
/// library (`None`). The caller checks the lock first.
pub fn set_scope(conn: &Connection, id: Option<i64>) -> Result<(), Error> {
    conn.execute("DELETE FROM temp.personal_scope", [])?;
    if let Some(id) = id {
        conn.execute(
            "INSERT INTO temp.personal_scope (one, library_id) VALUES (1, ?1)",
            [id],
        )?;
    }
    Ok(())
}

/// The personal library the connection reads, `None` for the main one.
pub fn scope(conn: &Connection) -> Result<Option<i64>, Error> {
    let exists: bool = conn.query_row(
        "SELECT count(*) FROM sqlite_temp_schema WHERE name = 'personal_scope'",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(None);
    }
    Ok(conn
        .query_row("SELECT library_id FROM temp.personal_scope", [], |r| {
            r.get(0)
        })
        .optional()?)
}

/// SQL: the personal library of the file row `$f` (`files` alias): the
/// library it was moved into, else the library of its longest personal
/// folder; NULL for none.
#[macro_export]
macro_rules! file_personal_library {
    ($f:literal) => {
        concat!(
            "coalesce((SELECT plm.library_id FROM personal_library_files plm WHERE plm.file_id = ",
            $f,
            ".id), (SELECT plr.library_id FROM personal_library_folders plr WHERE plr.source_id = ",
            $f,
            ".source_id AND (plr.path = '/' OR ",
            $f,
            ".path = plr.path OR substr(",
            $f,
            ".path, 1, length(plr.path) + 1) = plr.path || '/') ORDER BY length(plr.path) DESC LIMIT 1))"
        )
    };
}

/// SQL: `1` if the file row `$f` (`files` alias) is personal media: in a
/// personal library or on a `personal` source.
#[macro_export]
macro_rules! file_personal {
    ($f:literal) => {
        concat!(
            "(",
            $crate::file_personal_library!($f),
            " IS NOT NULL OR ifnull((SELECT pcs.content_type = 'personal' FROM sources pcs WHERE pcs.id = ",
            $f,
            ".source_id), 0))"
        )
    };
}

/// SQL: `1` if the file row `$f` (`files` alias) belongs to the library the
/// connection reads: without a `temp.personal_scope` row, a file that is
/// not personal media; with one, a file of that personal library.
#[macro_export]
macro_rules! file_in_scope {
    ($f:literal) => {
        concat!(
            "(CASE WHEN NOT EXISTS (SELECT 1 FROM temp.personal_scope) THEN NOT ",
            $crate::file_personal!($f),
            " ELSE ifnull(",
            $crate::file_personal_library!($f),
            " = (SELECT psc.library_id FROM temp.personal_scope psc), 0) END)"
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        crate::open_in_memory().unwrap()
    }

    #[test]
    fn locks_hash_and_verify() {
        let c = conn();
        let none = create(&c, 1, "Дом", &Lock::None, 1).unwrap();
        assert!(verify_secret(&c, none.id, "").unwrap());
        let pin = create(&c, 1, " Семья  ", &Lock::Pin("1234"), 1).unwrap();
        assert_eq!(pin.name, "Семья");
        assert_eq!(pin.lock, LockKind::Pin);
        assert!(verify_secret(&c, pin.id, "1234").unwrap());
        assert!(!verify_secret(&c, pin.id, "4321").unwrap());
        assert!(!verify_secret(&c, pin.id, "x").unwrap());
        let pw = create(&c, 1, "Архив", &Lock::Password("correct horse"), 1).unwrap();
        assert!(verify_secret(&c, pw.id, "correct horse").unwrap());
        assert!(!verify_secret(&c, pw.id, "Correct horse").unwrap());
        let stored: String = c
            .query_row(
                "SELECT lock_hash FROM personal_libraries WHERE id = ?1",
                [pw.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(stored.starts_with("argon2id$"), "{stored}");
        assert!(!stored.contains("horse"));
        assert!(create(&c, 1, "x", &Lock::Password("short"), 1).is_err());
        assert!(create(&c, 1, "x", &Lock::Pin("12a4"), 1).is_err());
        assert!(create(&c, 1, "   ", &Lock::None, 1).is_err());
        set_lock(&c, pw.id, &Lock::None, 2).unwrap();
        assert!(verify_secret(&c, pw.id, "").unwrap());
        assert_eq!(
            list(&c, 1)
                .unwrap()
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>(),
            ["Дом", "Семья", "Архив"]
        );
        assert!(list(&c, 2).unwrap().is_empty());
        assert_eq!(format!("{:?}", Lock::Password("secret!")), "password");
    }

    #[test]
    fn scope_decides_which_files_are_visible() {
        let c = conn();
        c.execute_batch(
            "INSERT INTO sources (id, kind, name, uri, created_at_ms) VALUES
                 (1, 'local', 'Фильмы', 'file:///m', 0),
                 (2, 'local', 'Своё', 'file:///p', 0);
             INSERT INTO files (id, source_id, path, size_bytes, added_at_ms) VALUES
                 (1, 1, '/Movies/A.mkv', 1, 0),
                 (2, 1, '/Home/B.mkv', 1, 0),
                 (3, 2, '/C.mkv', 1, 0);",
        )
        .unwrap();
        set_content_type(&c, 2, ContentType::Personal).unwrap();
        let lib = create(&c, 1, "Дом", &Lock::None, 1).unwrap();
        add_folder(&c, lib.id, 1, "Home/", 1).unwrap();
        let visible = |c: &Connection| -> Vec<i64> {
            let mut s = c
                .prepare(concat!(
                    "SELECT f.id FROM files f WHERE ",
                    crate::file_visible!("f"),
                    " ORDER BY f.id"
                ))
                .unwrap();
            s.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(visible(&c), [1]);
        assert_eq!(library_of_file(&c, 2).unwrap(), Some(Some(lib.id)));
        assert_eq!(library_of_file(&c, 3).unwrap(), Some(None));
        assert_eq!(library_of_file(&c, 1).unwrap(), None);
        assert_eq!(
            library_of_path(&c, 1, "Home/B.en.srt").unwrap(),
            Some(Some(lib.id))
        );
        set_scope(&c, Some(lib.id)).unwrap();
        assert_eq!(scope(&c).unwrap(), Some(lib.id));
        assert_eq!(visible(&c), [2]);
        move_files(&c, lib.id, &[1], 2).unwrap();
        assert_eq!(visible(&c), [1, 2]);
        set_scope(&c, None).unwrap();
        assert_eq!(visible(&c), Vec::<i64>::new());
        assert_eq!(unmove_files(&c, &[1]).unwrap(), 1);
        assert_eq!(visible(&c), [1]);
        // The source itself joins the library: file 3 too.
        add_folder(&c, lib.id, 2, "", 3).unwrap();
        set_scope(&c, Some(lib.id)).unwrap();
        assert_eq!(visible(&c), [2, 3]);
        set_scope(&c, None).unwrap();
        let other = create(&c, 1, "Другая", &Lock::None, 1).unwrap();
        assert!(add_folder(&c, other.id, 1, "/Home", 4).is_err());
        delete(&c, lib.id).unwrap();
        // Folders went with it; the personal source stays out of the main
        // library.
        assert_eq!(visible(&c), [1, 2]);
    }
}
