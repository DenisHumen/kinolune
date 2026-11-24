// SPDX-License-Identifier: MPL-2.0
//! Accounts and sessions on disk: `auth.sqlite3` in the data directory.
//!
//! - `users`: local accounts (no cloud): name, role, Argon2id hashes of the
//!   password and of the PIN (`pa_library::profiles` format).
//! - `sessions`: one login of a user or a device; refresh tokens rotate
//!   within it. Revoking a session kills its access tokens at once (they are
//!   checked against this table) and its refresh tokens.
//! - `refresh_tokens`: SHA-256 of each refresh token ever issued, with the
//!   time it was used. Presenting a used one again is reuse: the whole
//!   session is revoked (RFC 9700 §4.14.2).

use std::path::Path;

use pa_library::rusqlite::{self, Connection, OptionalExtension, params};

use super::{Method, Role, Scope};

/// Database file inside the data directory.
pub const FILE: &str = "auth.sqlite3";

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS users (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE COLLATE NOCASE,
    role          TEXT NOT NULL CHECK (role IN ('admin', 'member')),
    password_hash TEXT,
    pin_hash      TEXT,
    created_ms    INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
    id            TEXT PRIMARY KEY,
    user_id       TEXT,
    device_id     TEXT,
    scope         TEXT NOT NULL,
    method        TEXT NOT NULL,
    created_ms    INTEGER NOT NULL,
    refreshed_ms  INTEGER NOT NULL,
    revoked_ms    INTEGER,
    revoke_reason TEXT
);
CREATE INDEX IF NOT EXISTS sessions_user ON sessions (user_id);
CREATE INDEX IF NOT EXISTS sessions_device ON sessions (device_id);
CREATE TABLE IF NOT EXISTS refresh_tokens (
    hash       TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    expires_ms INTEGER NOT NULL,
    used_ms    INTEGER
);
CREATE INDEX IF NOT EXISTS refresh_tokens_session ON refresh_tokens (session_id);
";

/// A local account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// Stable id (UUID).
    pub id: String,
    /// Login and display name, unique without regard to case.
    pub name: String,
    /// Administrator or household member.
    pub role: Role,
    /// Argon2id hash of the password.
    pub password_hash: Option<String>,
    /// Argon2id hash of the PIN.
    pub pin_hash: Option<String>,
    /// Created, Unix ms.
    pub created_ms: i64,
}

/// One login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Session id (`sid` of its access tokens).
    pub id: String,
    /// The user, absent for a device session.
    pub user_id: Option<String>,
    /// The paired device the session is bound to.
    pub device_id: Option<String>,
    /// What the session may do.
    pub scope: Scope,
    /// How it was opened.
    pub method: Method,
    /// Opened, Unix ms.
    pub created_ms: i64,
    /// Revoked, Unix ms.
    pub revoked_ms: Option<i64>,
}

/// What a presented refresh token is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refresh {
    /// Never issued here.
    Unknown,
    /// Issued and not used yet.
    Fresh {
        /// Its session.
        session_id: String,
        /// Expiry, Unix ms.
        expires_ms: i64,
    },
    /// Already exchanged once: somebody replays it.
    Used {
        /// Its session.
        session_id: String,
    },
}

/// The open database.
pub struct Store {
    conn: Connection,
}

fn role_str(role: Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::Member => "member",
    }
}

fn parse_role(s: &str) -> Role {
    if s == "admin" {
        Role::Admin
    } else {
        Role::Member
    }
}

fn scope_str(scope: Scope) -> &'static str {
    match scope {
        Scope::Admin => "admin",
        Scope::Member => "member",
        Scope::Device => "device",
    }
}

fn parse_scope(s: &str) -> Scope {
    match s {
        "admin" => Scope::Admin,
        "device" => Scope::Device,
        _ => Scope::Member,
    }
}

fn method_str(method: Method) -> &'static str {
    match method {
        Method::Password => "password",
        Method::Pin => "pin",
        Method::Device => "device",
    }
}

fn parse_method(s: &str) -> Method {
    match s {
        "password" => Method::Password,
        "pin" => Method::Pin,
        _ => Method::Device,
    }
}

fn user_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: r.get(0)?,
        name: r.get(1)?,
        role: parse_role(&r.get::<_, String>(2)?),
        password_hash: r.get(3)?,
        pin_hash: r.get(4)?,
        created_ms: r.get(5)?,
    })
}

const USER_COLUMNS: &str = "id, name, role, password_hash, pin_hash, created_ms";

impl Store {
    /// Opens (creating if needed) the database at `path`.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// An in-memory database (tests).
    pub fn memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(Self { conn })
    }

    /// Whether an administrator account exists.
    pub fn has_admin(&self) -> rusqlite::Result<bool> {
        self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM users WHERE role = 'admin')",
            [],
            |r| r.get(0),
        )
    }

    /// Number of administrators.
    pub fn admin_count(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT count(*) FROM users WHERE role = 'admin'",
            [],
            |r| r.get(0),
        )
    }

    /// Adds `user`; `Ok(false)` when the name is taken.
    pub fn insert_user(&self, user: &User) -> rusqlite::Result<bool> {
        let inserted = self.conn.execute(
            "INSERT INTO users (id, name, role, password_hash, pin_hash, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT DO NOTHING",
            params![
                user.id,
                user.name,
                role_str(user.role),
                user.password_hash,
                user.pin_hash,
                user.created_ms
            ],
        )?;
        Ok(inserted == 1)
    }

    /// The account named `name` (any case).
    pub fn user_by_name(&self, name: &str) -> rusqlite::Result<Option<User>> {
        self.conn
            .query_row(
                &format!("SELECT {USER_COLUMNS} FROM users WHERE name = ?1"),
                [name],
                user_row,
            )
            .optional()
    }

    /// The account with `id`.
    pub fn user(&self, id: &str) -> rusqlite::Result<Option<User>> {
        self.conn
            .query_row(
                &format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?1"),
                [id],
                user_row,
            )
            .optional()
    }

    /// All accounts, by name.
    pub fn users(&self) -> rusqlite::Result<Vec<User>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {USER_COLUMNS} FROM users ORDER BY name COLLATE NOCASE"
        ))?;
        stmt.query_map([], user_row)?.collect()
    }

    /// Replaces the password hash (`Some(None)` removes it) and/or the PIN
    /// hash of `id`.
    pub fn set_credentials(
        &self,
        id: &str,
        password_hash: Option<Option<&str>>,
        pin_hash: Option<Option<&str>>,
    ) -> rusqlite::Result<()> {
        if let Some(hash) = password_hash {
            self.conn.execute(
                "UPDATE users SET password_hash = ?2 WHERE id = ?1",
                params![id, hash],
            )?;
        }
        if let Some(hash) = pin_hash {
            self.conn.execute(
                "UPDATE users SET pin_hash = ?2 WHERE id = ?1",
                params![id, hash],
            )?;
        }
        Ok(())
    }

    /// Deletes the account `id`; `false` if there was none.
    pub fn delete_user(&self, id: &str) -> rusqlite::Result<bool> {
        Ok(self.conn.execute("DELETE FROM users WHERE id = ?1", [id])? == 1)
    }

    /// Records a new session.
    pub fn insert_session(&self, s: &Session) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, user_id, device_id, scope, method, created_ms, refreshed_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![
                s.id,
                s.user_id,
                s.device_id,
                scope_str(s.scope),
                method_str(s.method),
                s.created_ms
            ],
        )?;
        Ok(())
    }

    /// The session `id`.
    pub fn session(&self, id: &str) -> rusqlite::Result<Option<Session>> {
        self.conn
            .query_row(
                "SELECT id, user_id, device_id, scope, method, created_ms, revoked_ms
                 FROM sessions WHERE id = ?1",
                [id],
                |r| {
                    Ok(Session {
                        id: r.get(0)?,
                        user_id: r.get(1)?,
                        device_id: r.get(2)?,
                        scope: parse_scope(&r.get::<_, String>(3)?),
                        method: parse_method(&r.get::<_, String>(4)?),
                        created_ms: r.get(5)?,
                        revoked_ms: r.get(6)?,
                    })
                },
            )
            .optional()
    }

    /// Records an issued refresh token (its hash).
    pub fn insert_refresh(&self, hash: &str, session_id: &str, expires_ms: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO refresh_tokens (hash, session_id, expires_ms) VALUES (?1, ?2, ?3)",
            params![hash, session_id, expires_ms],
        )?;
        Ok(())
    }

    /// Looks a refresh token (its hash) up.
    pub fn refresh(&self, hash: &str) -> rusqlite::Result<Refresh> {
        let row: Option<(String, i64, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT session_id, expires_ms, used_ms FROM refresh_tokens WHERE hash = ?1",
                [hash],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(match row {
            None => Refresh::Unknown,
            Some((session_id, _, Some(_))) => Refresh::Used { session_id },
            Some((session_id, expires_ms, None)) => Refresh::Fresh {
                session_id,
                expires_ms,
            },
        })
    }

    /// Marks a refresh token used; `false` if it already was (a concurrent
    /// exchange won).
    pub fn use_refresh(&self, hash: &str, now_ms: i64) -> rusqlite::Result<bool> {
        let changed = self.conn.execute(
            "UPDATE refresh_tokens SET used_ms = ?2 WHERE hash = ?1 AND used_ms IS NULL",
            params![hash, now_ms],
        )?;
        self.conn.execute(
            "UPDATE sessions SET refreshed_ms = ?2
             WHERE id = (SELECT session_id FROM refresh_tokens WHERE hash = ?1)",
            params![hash, now_ms],
        )?;
        Ok(changed == 1)
    }

    /// Revokes the session `id` (no-op when already revoked).
    pub fn revoke_session(&self, id: &str, reason: &str, now_ms: i64) -> rusqlite::Result<bool> {
        Ok(self.conn.execute(
            "UPDATE sessions SET revoked_ms = ?2, revoke_reason = ?3
             WHERE id = ?1 AND revoked_ms IS NULL",
            params![id, now_ms, reason],
        )? == 1)
    }

    /// Revokes the sessions of user `user_id` except `keep`; returns how many.
    pub fn revoke_user_sessions(
        &self,
        user_id: &str,
        keep: Option<&str>,
        reason: &str,
        now_ms: i64,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE sessions SET revoked_ms = ?3, revoke_reason = ?4
             WHERE user_id = ?1 AND revoked_ms IS NULL AND id IS NOT ?2",
            params![user_id, keep, now_ms, reason],
        )
    }

    /// Revokes every session bound to `device_id`; returns how many.
    pub fn revoke_device_sessions(
        &self,
        device_id: &str,
        reason: &str,
        now_ms: i64,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE sessions SET revoked_ms = ?2, revoke_reason = ?3
             WHERE device_id = ?1 AND revoked_ms IS NULL",
            params![device_id, now_ms, reason],
        )
    }

    /// Drops refresh tokens expired before `before_ms` and sessions revoked
    /// before it. Their tokens could not be used anyway.
    pub fn prune(&self, before_ms: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM refresh_tokens WHERE expires_ms < ?1
                OR session_id IN (SELECT id FROM sessions WHERE revoked_ms < ?1)",
            [before_ms],
        )?;
        self.conn
            .execute("DELETE FROM sessions WHERE revoked_ms < ?1", [before_ms])?;
        Ok(())
    }

    /// Moves the WAL into the main file (shutdown).
    pub fn checkpoint(&self) -> rusqlite::Result<()> {
        self.conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: &str, name: &str, role: Role) -> User {
        User {
            id: id.into(),
            name: name.into(),
            role,
            password_hash: Some("h".into()),
            pin_hash: None,
            created_ms: 1,
        }
    }

    fn session(id: &str, user: Option<&str>, device: Option<&str>) -> Session {
        Session {
            id: id.into(),
            user_id: user.map(Into::into),
            device_id: device.map(Into::into),
            scope: Scope::Member,
            method: Method::Password,
            created_ms: 1,
            revoked_ms: None,
        }
    }

    #[test]
    fn users_are_unique_by_name_in_any_case() {
        let s = Store::memory().unwrap();
        assert!(!s.has_admin().unwrap());
        assert!(s.insert_user(&user("1", "Mom", Role::Admin)).unwrap());
        assert!(!s.insert_user(&user("2", "mom", Role::Member)).unwrap());
        assert!(s.has_admin().unwrap());
        assert_eq!(s.user_by_name("MOM").unwrap().unwrap().id, "1");
        s.set_credentials("1", None, Some(Some("p"))).unwrap();
        assert_eq!(s.user("1").unwrap().unwrap().pin_hash.as_deref(), Some("p"));
        assert!(s.delete_user("1").unwrap());
        assert!(!s.delete_user("1").unwrap());
    }

    #[test]
    fn refresh_tokens_are_single_use() {
        let s = Store::memory().unwrap();
        s.insert_session(&session("s", Some("u"), None)).unwrap();
        s.insert_refresh("h1", "s", 100).unwrap();
        assert_eq!(
            s.refresh("h1").unwrap(),
            Refresh::Fresh {
                session_id: "s".into(),
                expires_ms: 100
            }
        );
        assert!(s.use_refresh("h1", 5).unwrap());
        assert!(!s.use_refresh("h1", 6).unwrap());
        assert_eq!(
            s.refresh("h1").unwrap(),
            Refresh::Used {
                session_id: "s".into()
            }
        );
        assert_eq!(s.refresh("nope").unwrap(), Refresh::Unknown);
    }

    #[test]
    fn revocation_by_session_user_and_device() {
        let s = Store::memory().unwrap();
        s.insert_session(&session("a", Some("u"), Some("tv"))).unwrap();
        s.insert_session(&session("b", Some("u"), None)).unwrap();
        s.insert_session(&session("c", None, Some("tv"))).unwrap();
        assert_eq!(s.revoke_device_sessions("tv", "device revoked", 2).unwrap(), 2);
        assert!(s.session("a").unwrap().unwrap().revoked_ms.is_some());
        assert!(s.session("b").unwrap().unwrap().revoked_ms.is_none());
        assert_eq!(s.revoke_user_sessions("u", Some("b"), "x", 3).unwrap(), 0);
        assert_eq!(s.revoke_user_sessions("u", None, "x", 3).unwrap(), 1);
        assert!(!s.revoke_session("b", "x", 4).unwrap());
        s.prune(10).unwrap();
        assert!(s.session("a").unwrap().is_none());
    }
}
