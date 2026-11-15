// SPDX-License-Identifier: MPL-2.0
//! LIB-004 storage: ed2k bookkeeping and moved files.

use pa_library::fingerprints::{
    ED2K_RETRY_MS, Ed2kPolicy, MOVE_WINDOW_MS, ed2k_candidates, ed2k_policy, follow_moves,
    mark_ed2k_failed, set_ed2k, set_ed2k_policy,
};
use pa_library::rusqlite::{Connection, params};

const HOUR: i64 = 60 * 60 * 1000;

fn db() -> (Connection, i64) {
    let conn = pa_library::open_in_memory().unwrap();
    conn.execute(
        "INSERT INTO sources (kind, name, uri, created_at_ms) VALUES ('local', 'M', 'file:///m', 0)",
        [],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    (conn, id)
}

fn file(conn: &Connection, source: i64, path: &str, hash: &str, added: i64, missing: Option<i64>) -> i64 {
    conn.execute(
        "INSERT INTO files (source_id, path, size_bytes, mtime_ms, added_at_ms, fingerprint, oshash,
                            missing_since_ms)
         VALUES (?1, ?2, 1000, 5, ?3, ?4, ?4, ?5)",
        params![source, path, added, hash, missing],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn path_of(conn: &Connection, id: i64) -> Option<String> {
    conn.query_row("SELECT path FROM files WHERE id = ?1", [id], |r| r.get(0))
        .ok()
}

#[test]
fn policy_defaults_to_auto_and_can_be_set() {
    let (conn, src) = db();
    assert_eq!(ed2k_policy(&conn, src).unwrap(), Some(Ed2kPolicy::Auto));
    assert!(set_ed2k_policy(&conn, src, Ed2kPolicy::Never).unwrap());
    assert_eq!(ed2k_policy(&conn, src).unwrap(), Some(Ed2kPolicy::Never));
    assert!(!set_ed2k_policy(&conn, src + 1, Ed2kPolicy::Always).unwrap());
    assert_eq!(ed2k_policy(&conn, src + 1).unwrap(), None);
    assert!(
        conn.execute("UPDATE sources SET ed2k_policy = 'sometimes'", [])
            .is_err()
    );
}

#[test]
fn candidates_skip_hashed_missing_and_recent_failures() {
    let (conn, src) = db();
    let a = file(&conn, src, "/a.mkv", "00000000000000aa", 0, None);
    let b = file(&conn, src, "/b.mkv", "00000000000000bb", 0, None);
    file(&conn, src, "/gone.mkv", "00000000000000cc", 0, Some(1));
    let now = 10 * ED2K_RETRY_MS;
    let all = ed2k_candidates(&conn, src, now, 10).unwrap();
    assert_eq!(all.iter().map(|c| c.file_id).collect::<Vec<_>>(), [a, b]);

    assert!(set_ed2k(&conn, &all[0], "0123456789abcdef0123456789abcdef").unwrap());
    // Changed since it was listed: not stored.
    let mut stale = all[1].clone();
    stale.mtime_ms = Some(6);
    assert!(!set_ed2k(&conn, &stale, "0123456789abcdef0123456789abcdef").unwrap());

    mark_ed2k_failed(&conn, b, now - 1).unwrap();
    assert!(ed2k_candidates(&conn, src, now, 10).unwrap().is_empty());
    // Retried a day later.
    let later = ed2k_candidates(&conn, src, now + ED2K_RETRY_MS, 10).unwrap();
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].file_id, b);
}

#[test]
fn a_moved_file_keeps_its_row_and_takes_the_new_structure() {
    let (mut conn, src) = db();
    let now = 100 * HOUR;
    let old = file(&conn, src, "/Old/movie.mkv", "1111111111111111", 0, Some(now));
    let new = file(&conn, src, "/New/movie (2020).mkv", "1111111111111111", now, None);
    conn.execute(
        "UPDATE files SET container = 'matroska', duration_ms = 5000 WHERE id = ?1",
        [old],
    )
    .unwrap();
    conn.execute("UPDATE files SET inode = 77, mtime_ms = 9 WHERE id = ?1", [new])
        .unwrap();
    for (root, f) in [("/Old", old), ("/New", new)] {
        conn.execute(
            "INSERT INTO file_groups (source_id, root_path, kind) VALUES (?1, ?2, 'movie_folder')",
            params![src, root],
        )
        .unwrap();
        let g = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO file_group_members (group_id, file_id, role) VALUES (?1, ?2, 'main')",
            params![g, f],
        )
        .unwrap();
    }

    let moved = follow_moves(&mut conn, src).unwrap();
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].file_id, old);
    assert_eq!(moved[0].from_path, "/Old/movie.mkv");
    assert_eq!(moved[0].to_path, "/New/movie (2020).mkv");

    assert_eq!(path_of(&conn, new), None);
    assert_eq!(path_of(&conn, old).as_deref(), Some("/New/movie (2020).mkv"));
    let (missing, inode, mtime, container): (Option<i64>, Option<i64>, Option<i64>, Option<String>) =
        conn.query_row(
            "SELECT missing_since_ms, inode, mtime_ms, container FROM files WHERE id = ?1",
            [old],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!((missing, inode, mtime), (None, Some(77), Some(9)));
    assert_eq!(container.as_deref(), Some("matroska"));
    let roots: Vec<String> = conn
        .prepare(
            "SELECT g.root_path FROM file_group_members m JOIN file_groups g ON g.id = m.group_id
             WHERE m.file_id = ?1",
        )
        .unwrap()
        .query_map([old], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(roots, ["/New"]);
    // Done: nothing more to pair.
    assert!(follow_moves(&mut conn, src).unwrap().is_empty());
}

#[test]
fn pairs_need_the_same_source_size_window_and_no_user_decision() {
    let (mut conn, src) = db();
    conn.execute(
        "INSERT INTO sources (kind, name, uri, created_at_ms) VALUES ('local', 'N', 'file:///n', 0)",
        [],
    )
    .unwrap();
    let other = conn.last_insert_rowid();
    let t = 1000 * HOUR;
    // Other source.
    file(&conn, other, "/x.mkv", "2222222222222222", 0, Some(t));
    file(&conn, src, "/x2.mkv", "2222222222222222", t, None);
    // Different size.
    let o = file(&conn, src, "/y.mkv", "3333333333333333", 0, Some(t));
    conn.execute("UPDATE files SET size_bytes = 999 WHERE id = ?1", [o]).unwrap();
    file(&conn, src, "/y2.mkv", "3333333333333333", t, None);
    // The new row is older than the window before the loss: a copy that
    // lived on while the original was deleted.
    file(&conn, src, "/z.mkv", "4444444444444444", 0, Some(t));
    file(&conn, src, "/z2.mkv", "4444444444444444", t - MOVE_WINDOW_MS - 1, None);
    // A user decision on the new row.
    file(&conn, src, "/w.mkv", "5555555555555555", 0, Some(t));
    let w2 = file(&conn, src, "/w2.mkv", "5555555555555555", t, None);
    conn.execute(
        "INSERT INTO works (kind, title, created_at_ms, updated_at_ms) VALUES ('movie', 'W', 0, 0)",
        [],
    )
    .unwrap();
    let work = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO matches (file_id, work_id, confidence, state, decided_by, created_at_ms,
                              updated_at_ms)
         VALUES (?1, ?2, 1.0, 'user_set', 'user', 0, 0)",
        params![w2, work],
    )
    .unwrap();
    assert!(follow_moves(&mut conn, src).unwrap().is_empty());

    // Two missing candidates: the one lost last wins, once.
    let first = file(&conn, src, "/v1.mkv", "6666666666666666", 0, Some(t - HOUR));
    let last = file(&conn, src, "/v2.mkv", "6666666666666666", 0, Some(t));
    file(&conn, src, "/v3.mkv", "6666666666666666", t, None);
    let moved = follow_moves(&mut conn, src).unwrap();
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].file_id, last);
    assert_eq!(path_of(&conn, first).as_deref(), Some("/v1.mkv"));
}
