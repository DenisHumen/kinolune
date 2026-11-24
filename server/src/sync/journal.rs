// SPDX-License-Identifier: MPL-2.0
//! Hub journal in SQLite: the persistent twin of `pa_sync::proto::Journal`.
//!
//! Same semantics as the reference: ops in arrival order, `seq` from 1,
//! deduplicated by `op_id` (a duplicate keeps its first `seq`), payloads
//! stored opaque. The journal lives in its own file (`sync.sqlite3`), not in
//! the library database: it is hub state, not library state, and the library
//! schema and migrations are shared with the apps.
//!
//! SYNC-017 (ADR-0157 §1–2): the same file keeps the node logs
//! (`node_log`): every entry with its origin, gapless `seq` and hash chain.
//! They are loaded into a [`NodeLog`] at start (all of them are held in
//! memory: a household's ops are small). Ops that come without a log place
//! (devices of protocol 1, ops journaled before SYNC-017) go into the
//! hub's own log: the hub vouches for them. Schema 1 journals are migrated
//! on open: their ops, in `seq` order, become the hub's own log, the same
//! way on every start.

use std::path::Path;

use pa_library::rusqlite::{self, Connection, OptionalExtension, params};
use pa_sync::OpId;
use pa_sync::nodelog::{Digest, Entry, Inserted, NodeLog, Range, VersionVector};
use pa_sync::proto::{Cursor, MAX_PULL_LIMIT, OpFrame, Payload, Pull, PullEnd};

/// Schema version in `PRAGMA user_version`.
const SCHEMA_VERSION: i64 = 2;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS hub_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS journal (
    seq         INTEGER PRIMARY KEY,
    op_id       TEXT NOT NULL UNIQUE,
    enc         TEXT NOT NULL,
    body        TEXT NOT NULL,
    received_ms INTEGER NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS node_log (
    origin TEXT NOT NULL,
    seq    INTEGER NOT NULL,
    prev   TEXT NOT NULL,
    op_id  TEXT NOT NULL,
    enc    TEXT NOT NULL,
    body   TEXT NOT NULL,
    PRIMARY KEY (origin, seq)
) STRICT;
";

/// Error of the journal database.
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    /// The file was written by a newer server.
    #[error("sync journal schema {found} is newer than {SCHEMA_VERSION}")]
    TooNew { found: i64 },
    /// A stored id is not valid.
    #[error("corrupt sync journal: {0}")]
    Corrupt(&'static str),
}

/// The journal of this hub and its identity.
#[derive(Debug)]
pub struct SqliteJournal {
    conn: Connection,
    log: String,
    hub_id: String,
    head: u64,
    nodes: NodeLog,
}

impl SqliteJournal {
    /// Opens (creating if needed) the journal file.
    pub fn open(path: &Path) -> Result<Self, JournalError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // A crash may lose the last ops but never corrupts the file; the
        // devices resend everything they did not see acked.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::init(conn)
    }

    /// An in-memory journal (tests).
    pub fn open_in_memory() -> Result<Self, JournalError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, JournalError> {
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(JournalError::TooNew { found: version });
        }
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        let log = meta_or_insert(&conn, "log_id")?;
        let hub_id = meta_or_insert(&conn, "hub_id")?;
        for id in [&log, &hub_id] {
            if !pa_sync::hlc::is_valid_device_id(id) {
                return Err(JournalError::Corrupt("id"));
            }
        }
        let head: i64 = conn.query_row("SELECT coalesce(max(seq), 0) FROM journal", [], |r| {
            r.get(0)
        })?;
        let nodes = load_nodes(&conn, &hub_id)?;
        let mut journal = Self {
            conn,
            log,
            hub_id,
            head: u64::try_from(head).map_err(|_| JournalError::Corrupt("seq"))?,
            nodes,
        };
        if version < 2 && journal.nodes.is_empty() && journal.head > 0 {
            journal.migrate_to_node_log()?;
        }
        Ok(journal)
    }

    /// Schema 1 → 2: the journal's ops, in `seq` order, become the hub's
    /// own log.
    fn migrate_to_node_log(&mut self) -> Result<(), JournalError> {
        let frames = {
            let mut stmt = self
                .conn
                .prepare("SELECT op_id, enc, body FROM journal ORDER BY seq")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            let mut frames = Vec::new();
            for row in rows {
                let (op_id, enc, body) = row?;
                frames.push(OpFrame {
                    op_id: op_id
                        .parse::<OpId>()
                        .map_err(|_| JournalError::Corrupt("op id"))?,
                    seq: None,
                    log: None,
                    payload: Payload { enc, body },
                });
            }
            frames
        };
        let tx = self.conn.unchecked_transaction()?;
        for frame in &frames {
            let entry = self.nodes.append_frame(frame);
            store_entry(&tx, &entry)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The hub's version vector.
    pub fn vector(&self) -> VersionVector {
        self.nodes.vector()
    }

    /// Whether the journal holds `op_id`.
    pub fn contains(&self, op_id: OpId) -> Result<bool, JournalError> {
        Ok(self.seq_of(&op_id.to_string())?.is_some())
    }

    /// Stores the log entry a frame carries; `None` for a frame without
    /// one. Entries that are kept (gapless or waiting) are persisted.
    pub fn insert_entry(&mut self, frame: &OpFrame) -> Result<Option<Inserted>, JournalError> {
        let Some((entry, hash)) = Entry::from_frame(frame) else {
            return Ok(None);
        };
        let inserted = self.nodes.insert(entry.clone(), &hash);
        if matches!(inserted, Inserted::New { .. } | Inserted::Gap { .. }) {
            store_entry(&self.conn, &entry)?;
        }
        Ok(Some(inserted))
    }

    /// Appends an op without a log place to the hub's own log.
    pub fn vouch(&mut self, frame: &OpFrame) -> Result<Entry, JournalError> {
        let entry = self.nodes.append_frame(frame);
        store_entry(&self.conn, &entry)?;
        Ok(entry)
    }

    /// Entries a peer with vector `theirs` lacks.
    pub fn missing_for(&self, theirs: &VersionVector) -> Vec<Entry> {
        self.nodes.missing_for(theirs)
    }

    /// Held entries of a range.
    pub fn range(&self, range: &Range) -> Vec<Entry> {
        self.nodes.range(range)
    }

    /// Holes behind waiting entries.
    pub fn gaps(&self) -> Vec<Range> {
        self.nodes.gaps()
    }

    /// Digest of the state the logs materialise, and the entries of the
    /// partitions where `theirs` differs.
    pub fn digest_diff(&self, theirs: &Digest) -> (Digest, Vec<Entry>) {
        let mine = self.nodes.digest();
        let diff = mine.diff(theirs);
        let entries = self.nodes.entries_in(&diff);
        (mine, entries)
    }

    /// Stable `device_id` of this hub (sent in its `hello`).
    pub fn hub_id(&self) -> &str {
        &self.hub_id
    }

    /// The cursor after the last op.
    pub fn head(&self) -> Cursor {
        Cursor {
            log: self.log.clone(),
            seq: self.head,
        }
    }

    /// Stores an op frame. Returns its `seq` and whether it is new; a
    /// duplicate keeps its first `seq`, so the ack is the same.
    pub fn append(&mut self, frame: &OpFrame, now_ms: i64) -> Result<(u64, bool), JournalError> {
        let op_id = frame.op_id.to_string();
        if let Some(seq) = self.seq_of(&op_id)? {
            return Ok((seq, false));
        }
        let seq = self.head + 1;
        self.conn.execute(
            "INSERT INTO journal (seq, op_id, enc, body, received_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                i64::try_from(seq).map_err(|_| JournalError::Corrupt("seq"))?,
                op_id,
                frame.payload.enc,
                frame.payload.body,
                now_ms
            ],
        )?;
        self.head = seq;
        Ok((seq, true))
    }

    fn seq_of(&self, op_id: &str) -> Result<Option<u64>, JournalError> {
        let seq: Option<i64> = self
            .conn
            .query_row(
                "SELECT seq FROM journal WHERE op_id = ?1",
                params![op_id],
                |r| r.get(0),
            )
            .optional()?;
        seq.map(|s| u64::try_from(s).map_err(|_| JournalError::Corrupt("seq")))
            .transpose()
    }

    /// Answers a [`Pull`] like `pa_sync::proto::Journal::pull`: up to `limit`
    /// frames after the cursor (from the start for another journal) and the
    /// closing [`PullEnd`].
    pub fn pull(&self, pull: &Pull) -> Result<(Vec<OpFrame>, PullEnd), JournalError> {
        let after = match &pull.cursor {
            Some(c) if c.log == self.log => c.seq.min(self.head),
            _ => 0,
        };
        let limit = pull.limit.clamp(1, MAX_PULL_LIMIT);
        let mut stmt = self.conn.prepare_cached(
            "SELECT seq, op_id, enc, body FROM journal WHERE seq > ?1 ORDER BY seq LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![i64::try_from(after).unwrap_or(i64::MAX), limit],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )?;
        let mut frames = Vec::new();
        for row in rows {
            let (seq, op_id, enc, body) = row?;
            frames.push(OpFrame {
                op_id: op_id
                    .parse::<OpId>()
                    .map_err(|_| JournalError::Corrupt("op id"))?,
                seq: Some(u64::try_from(seq).map_err(|_| JournalError::Corrupt("seq"))?),
                log: None,
                payload: Payload { enc, body },
            });
        }
        let seq = frames.last().and_then(|f| f.seq).unwrap_or(after);
        let end = PullEnd {
            cursor: Cursor {
                log: self.log.clone(),
                seq,
            },
            more: seq < self.head,
        };
        Ok((frames, end))
    }

    /// Moves the WAL into the main file (on shutdown).
    pub fn checkpoint(&self) -> Result<(), JournalError> {
        self.conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        Ok(())
    }
}

fn store_entry(conn: &Connection, e: &Entry) -> Result<(), JournalError> {
    conn.execute(
        "INSERT OR IGNORE INTO node_log (origin, seq, prev, op_id, enc, body)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            e.origin,
            i64::try_from(e.seq).map_err(|_| JournalError::Corrupt("seq"))?,
            e.prev,
            e.op_id.to_string(),
            e.payload.enc,
            e.payload.body
        ],
    )?;
    Ok(())
}

/// The node logs of the file. Rows that do not chain any more (changed on
/// disk) are left out and fetched again from peers.
fn load_nodes(conn: &Connection, hub_id: &str) -> Result<NodeLog, JournalError> {
    let mut stmt = conn.prepare("SELECT origin, seq, prev, op_id, enc, body FROM node_log")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    let mut entries = Vec::new();
    for row in rows {
        let (origin, seq, prev, op_id, enc, body) = row?;
        entries.push(Entry {
            origin,
            seq: u64::try_from(seq).map_err(|_| JournalError::Corrupt("seq"))?,
            prev,
            op_id: op_id
                .parse::<OpId>()
                .map_err(|_| JournalError::Corrupt("op id"))?,
            payload: Payload { enc, body },
        });
    }
    let (nodes, refused) =
        NodeLog::restore(hub_id, entries).map_err(|_| JournalError::Corrupt("id"))?;
    if refused > 0 {
        tracing::warn!(
            refused,
            "sync: node log entries do not chain; fetched again"
        );
    }
    Ok(nodes)
}

/// Reads a meta value or stores a fresh UUID under `key`.
fn meta_or_insert(conn: &Connection, key: &str) -> Result<String, JournalError> {
    let fresh = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT OR IGNORE INTO hub_meta (key, value) VALUES (?1, ?2)",
        params![key, fresh],
    )?;
    Ok(conn.query_row(
        "SELECT value FROM hub_meta WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use pa_sync::proto::Journal;

    use super::*;

    fn frame(n: u8) -> OpFrame {
        OpFrame {
            op_id: OpId(uuid::Uuid::from_bytes([n; 16])),
            seq: None,
            log: None,
            payload: Payload {
                enc: "secretbox.v1".to_owned(),
                body: format!("opaque-{n}"),
            },
        }
    }

    /// The SQLite journal answers exactly like the reference journal.
    #[test]
    fn matches_reference_journal() {
        let mut db = SqliteJournal::open_in_memory().unwrap();
        let mut reference = Journal::new(db.head().log).unwrap();
        for n in [1, 2, 3, 2, 4, 1, 5] {
            let f = frame(n);
            assert_eq!(db.append(&f, 0).unwrap(), reference.append(&f));
        }
        assert_eq!(db.head(), reference.head());
        let other = Cursor {
            log: "other-log".to_owned(),
            seq: 2,
        };
        let mine = |seq| Cursor {
            log: db.head().log,
            seq,
        };
        for cursor in [
            None,
            Some(other),
            Some(mine(0)),
            Some(mine(2)),
            Some(mine(5)),
            Some(mine(99)),
        ] {
            for limit in [1, 2, 10] {
                let pull = Pull {
                    cursor: cursor.clone(),
                    limit,
                };
                assert_eq!(db.pull(&pull).unwrap(), reference.pull(&pull), "{pull:?}");
            }
        }
    }

    #[test]
    fn survives_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sync.sqlite3");
        let (head, hub) = {
            let mut db = SqliteJournal::open(&path).unwrap();
            db.append(&frame(1), 0).unwrap();
            db.append(&frame(2), 0).unwrap();
            db.checkpoint().unwrap();
            (db.head(), db.hub_id().to_owned())
        };
        let mut db = SqliteJournal::open(&path).unwrap();
        assert_eq!(db.head(), head);
        assert_eq!(db.hub_id(), hub);
        assert_eq!(db.append(&frame(1), 0).unwrap(), (1, false));
        assert_eq!(db.append(&frame(3), 0).unwrap(), (3, true));
    }

    /// SYNC-017: a schema-1 journal becomes the hub's own log on open, the
    /// same on every open; entries survive a restart; a row changed on
    /// disk is left out.
    #[test]
    fn schema_1_migrates_into_the_hubs_log_and_entries_persist() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sync.sqlite3");
        {
            let mut db = SqliteJournal::open(&path).unwrap();
            for n in 1..=3 {
                db.append(&frame(n), 0).unwrap();
            }
            // Back to what a schema-1 server left.
            db.conn
                .execute_batch("DELETE FROM node_log; PRAGMA user_version = 1;")
                .unwrap();
        }
        let (hub, vector) = {
            let db = SqliteJournal::open(&path).unwrap();
            (db.hub_id().to_owned(), db.vector())
        };
        assert_eq!(vector.get(&hub), 3);
        let first = SqliteJournal::open(&path)
            .unwrap()
            .missing_for(&VersionVector::default());
        assert_eq!(first.len(), 3);
        assert_eq!(first[0].op_id, frame(1).op_id);

        // An entry of another origin persists.
        let mut other = NodeLog::new("phone").unwrap();
        let e = other.append_frame(&frame(9));
        {
            let mut db = SqliteJournal::open(&path).unwrap();
            assert!(matches!(
                db.insert_entry(&e.frame()).unwrap(),
                Some(Inserted::New { .. })
            ));
        }
        let db = SqliteJournal::open(&path).unwrap();
        assert_eq!(db.vector().get("phone"), 1);
        // A damaged row is refused on the next open.
        db.conn
            .execute("UPDATE node_log SET body = 'x' WHERE origin = 'phone'", [])
            .unwrap();
        drop(db);
        let db = SqliteJournal::open(&path).unwrap();
        assert_eq!(db.vector().get("phone"), 0);
        assert_eq!(db.vector().get(&hub), 3);
    }
}
