// SPDX-License-Identifier: MPL-2.0
//! LIB-002: expand→contract. Every schema a released build could have left
//! behind (every prefix of `MIGRATIONS`, a superset of the released
//! versions) upgrades to the latest one, the result is the schema of a fresh
//! database, and nothing an older build reads disappears: tables and their
//! columns only grow. Removing or renaming happens only in a migration whose
//! name ends with `_contract`, one release after the code stopped using it.

use std::collections::{BTreeMap, BTreeSet};

use pa_library::rusqlite::{Connection, params};
use pa_library::{HISTORY_TABLE, MIGRATIONS, configure, migrate};

/// Tables (without the history and SQLite internals) and their columns.
fn columns(conn: &Connection) -> BTreeMap<String, BTreeSet<String>> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_schema WHERE type = 'table' \
             AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'",
        )
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables
        .into_iter()
        .filter(|t| t != HISTORY_TABLE)
        .map(|t| {
            let mut s = conn
                .prepare(&format!("SELECT name FROM pragma_table_xinfo('{t}')"))
                .unwrap();
            let cols = s
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<BTreeSet<String>, _>>()
                .unwrap();
            (t, cols)
        })
        .collect()
}

/// Every object's SQL, for comparing an upgraded schema with a fresh one.
fn schema(conn: &Connection) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT type || ' ' || name || ' ' || coalesce(sql, '') FROM sqlite_schema \
             WHERE name NOT LIKE 'sqlite\\_%' ESCAPE '\\' AND name != '{HISTORY_TABLE}'"
        ))
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// A database as a build that knew only the first `n` migrations left it.
fn old_database(n: usize) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    configure(&conn).unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE {HISTORY_TABLE} (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_on TEXT NOT NULL,
            checksum TEXT NOT NULL
        ) STRICT"
    ))
    .unwrap();
    for m in &MIGRATIONS[..n] {
        conn.execute_batch(m.sql)
            .unwrap_or_else(|e| panic!("{}: {e}", m.name));
        conn.execute(
            &format!(
                "INSERT INTO {HISTORY_TABLE} (version, name, applied_on, checksum) \
                 VALUES (?1, ?2, '2026-01-01T00:00:00Z', ?3)"
            ),
            params![m.version, m.name, m.checksum()],
        )
        .unwrap();
    }
    conn
}

#[test]
fn every_older_schema_upgrades_to_the_fresh_one() {
    let mut fresh = Connection::open_in_memory().unwrap();
    configure(&fresh).unwrap();
    migrate(&mut fresh).unwrap();
    let want = schema(&fresh);
    for n in 1..MIGRATIONS.len() {
        let mut conn = old_database(n);
        let report = migrate(&mut conn)
            .unwrap_or_else(|e| panic!("upgrade from {}: {e}", MIGRATIONS[n - 1].name));
        assert_eq!(report.applied.len(), MIGRATIONS.len() - n);
        assert!(!report.db_is_newer);
        assert_eq!(
            schema(&conn),
            want,
            "upgrade from {} differs from a fresh database",
            MIGRATIONS[n - 1].name
        );
    }
}

#[test]
fn older_builds_still_find_their_tables_and_columns() {
    let latest = columns(&old_database(MIGRATIONS.len()));
    for n in 1..MIGRATIONS.len() {
        let name = MIGRATIONS[n - 1].name;
        // A contract migration may remove what builds before it used.
        if MIGRATIONS[n..]
            .iter()
            .any(|m| m.name.ends_with("_contract"))
        {
            continue;
        }
        for (table, cols) in columns(&old_database(n)) {
            let now = latest
                .get(&table)
                .unwrap_or_else(|| panic!("table {table} (known since {name}) was removed"));
            let gone: Vec<_> = cols.difference(now).collect();
            assert!(
                gone.is_empty(),
                "columns {gone:?} of {table} (known at {name}) were removed"
            );
        }
    }
}

#[test]
fn only_contract_migrations_drop_or_rename() {
    for m in MIGRATIONS {
        if m.name.ends_with("_contract") {
            continue;
        }
        let sql: String = m
            .sql
            .lines()
            .map(|l| l.split("--").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n")
            .to_uppercase();
        let words: Vec<&str> = sql.split_whitespace().collect();
        for w in words.windows(2) {
            let bad = (w[0] == "DROP" && (w[1] == "TABLE" || w[1] == "COLUMN"))
                || (w[0] == "RENAME" && (w[1] == "TO" || w[1] == "COLUMN"));
            assert!(
                !bad,
                "{}: `{} {}` belongs in a `_contract` migration (LIB-002)",
                m.name, w[0], w[1]
            );
        }
    }
}
