// SPDX-License-Identifier: MPL-2.0
//! Profiles, PINs, the active profile and the parental-control predicates
//! (LIB-012).

use pa_library::profiles::{
    self, DEFAULT_PROFILE, FolderAccess, ProfileSettings, certifications, set_certifications,
    set_user_certification,
};
use pa_library::rusqlite::Connection;
use pa_library::{file_visible, open_in_memory, work_allowed};

const NOW: i64 = 1_763_000_000_000;

fn kid() -> ProfileSettings {
    ProfileSettings {
        name: "Kid".into(),
        avatar: Some("color:3".into()),
        is_kid: true,
        max_age: Some(12),
        rating_country: "US".into(),
        hide_unrated: true,
        hide_artwork: true,
        restrict_folders: true,
    }
}

/// Two sources; works 1 (PG, kids folder), 2 (R, kids folder), 3 (unrated,
/// kids folder), 4 (G, other folder), 5 (G, other source).
fn library(conn: &Connection) {
    conn.execute_batch(&format!(
        "
        INSERT INTO sources (id, kind, name, uri, created_at_ms) VALUES
            (1, 'local', 'Media', 'file:///media', {NOW}),
            (2, 'local', 'Other', 'file:///other', {NOW});
        INSERT INTO files (id, source_id, path, size_bytes, added_at_ms) VALUES
            (11, 1, '/Kids/Cartoon.mkv', 1, {NOW}),
            (12, 1, '/Kids/Thriller.mkv', 1, {NOW}),
            (13, 1, '/Kids/Home video.mkv', 1, {NOW}),
            (14, 1, '/Movies/Family.mkv', 1, {NOW}),
            (15, 2, '/Family 2.mkv', 1, {NOW}),
            (16, 1, '/KidsExtra/Sneaky.mkv', 1, {NOW});
        INSERT INTO works (id, kind, title, created_at_ms, updated_at_ms) VALUES
            (1, 'movie', 'Cartoon', {NOW}, {NOW}),
            (2, 'movie', 'Thriller', {NOW}, {NOW}),
            (3, 'movie', 'Home video', {NOW}, {NOW}),
            (4, 'movie', 'Family', {NOW}, {NOW}),
            (5, 'movie', 'Family 2', {NOW}, {NOW}),
            (6, 'movie', 'Sneaky', {NOW}, {NOW});
        "
    ))
    .unwrap();
    for (work, cert) in [(1, "PG"), (2, "R"), (4, "G"), (5, "G"), (6, "G")] {
        set_certifications(conn, work, "tmdb", &[("US".into(), cert.into())]).unwrap();
    }
}

fn visible_files(conn: &Connection) -> Vec<i64> {
    let sql = concat!(
        "SELECT f.id FROM files f WHERE ",
        file_visible!("f"),
        " ORDER BY f.id"
    );
    let mut stmt = conn.prepare(sql).unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn allowed_works(conn: &Connection) -> Vec<i64> {
    let sql = concat!(
        "SELECT w.id FROM works w WHERE ",
        work_allowed!("w.id"),
        " ORDER BY w.id"
    );
    let mut stmt = conn.prepare(sql).unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn default_profile_is_active_and_unrestricted() {
    let conn = open_in_memory().unwrap();
    library(&conn);
    assert_eq!(profiles::active(&conn).unwrap(), Some(DEFAULT_PROFILE));
    let p = profiles::get(&conn, DEFAULT_PROFILE).unwrap().unwrap();
    assert_eq!(p.sync_id, "default");
    assert!(!p.is_restricted() && !p.has_pin);
    assert_eq!(visible_files(&conn), [11, 12, 13, 14, 15, 16]);
    assert_eq!(allowed_works(&conn), [1, 2, 3, 4, 5, 6]);
}

#[test]
fn crud_and_order() {
    let mut conn = open_in_memory().unwrap();
    let k = profiles::create(&conn, &kid(), NOW).unwrap();
    assert_ne!(k.sync_id, "default");
    assert_eq!(k.sync_id.len(), 32);
    assert_eq!(k.position, 1);
    let mut s = kid();
    s.name = "Little one".into();
    s.max_age = Some(7);
    let k = profiles::update(&conn, k.id, &s, NOW + 1).unwrap();
    assert_eq!((k.name.as_str(), k.max_age), ("Little one", Some(7)));
    profiles::reorder(&mut conn, &[k.id], NOW).unwrap();
    let ids: Vec<i64> = profiles::list(&conn)
        .unwrap()
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, [k.id, DEFAULT_PROFILE]);

    let bad = ProfileSettings {
        rating_country: "usa".into(),
        ..kid()
    };
    assert!(profiles::create(&conn, &bad, NOW).is_err());
    let bad = ProfileSettings {
        max_age: Some(30),
        ..kid()
    };
    assert!(profiles::create(&conn, &bad, NOW).is_err());

    profiles::delete(&conn, k.id).unwrap();
    assert!(
        profiles::delete(&conn, DEFAULT_PROFILE).is_err(),
        "last profile"
    );
}

#[test]
fn pin_is_hashed_and_checked() {
    let conn = open_in_memory().unwrap();
    let k = profiles::create(&conn, &kid(), NOW).unwrap();
    assert!(profiles::verify_pin(&conn, k.id, None).unwrap());
    profiles::set_pin(&conn, k.id, Some("1234"), NOW).unwrap();
    let stored: String = conn
        .query_row("SELECT pin_hash FROM profiles WHERE id = ?1", [k.id], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(stored.starts_with("argon2id$") && !stored.contains("1234"));
    assert!(profiles::get(&conn, k.id).unwrap().unwrap().has_pin);
    assert!(profiles::verify_pin(&conn, k.id, Some("1234")).unwrap());
    assert!(!profiles::verify_pin(&conn, k.id, Some("4321")).unwrap());
    assert!(!profiles::verify_pin(&conn, k.id, None).unwrap());
    assert!(profiles::set_pin(&conn, k.id, Some("12"), NOW).is_err());
    profiles::set_pin(&conn, k.id, None, NOW).unwrap();
    assert!(profiles::verify_pin(&conn, k.id, None).unwrap());
}

#[test]
fn kid_filters_hide_folders_ratings_and_unrated() {
    let mut conn = open_in_memory().unwrap();
    library(&conn);
    let k = profiles::create(&conn, &kid(), NOW).unwrap();
    profiles::set_folders(&mut conn, k.id, &[FolderAccess::new(1, "Kids/")], NOW).unwrap();
    profiles::activate(&conn, k.id).unwrap();

    // `/KidsExtra` shares the prefix but is another folder.
    assert_eq!(visible_files(&conn), [11, 12, 13]);
    // R (17) is over 12, work 3 has no rating.
    assert_eq!(allowed_works(&conn), [1, 4, 5, 6]);
    assert!(profiles::artwork_hidden(&conn).unwrap());

    // Unrated shown when the policy allows it.
    let mut s = kid();
    s.hide_unrated = false;
    profiles::update(&conn, k.id, &s, NOW).unwrap();
    assert_eq!(allowed_works(&conn), [1, 3, 4, 5, 6]);

    // The parent's rating wins over the provider's.
    set_user_certification(&conn, 2, Some(10)).unwrap();
    assert_eq!(allowed_works(&conn), [1, 2, 3, 4, 5, 6]);
    assert_eq!(certifications(&conn, 2).unwrap()[0].provider, "user");
    set_user_certification(&conn, 2, None).unwrap();

    // A whole source.
    profiles::set_folders(&mut conn, k.id, &[FolderAccess::new(2, "")], NOW).unwrap();
    profiles::refresh(&conn).unwrap();
    assert_eq!(visible_files(&conn), [15]);

    // Back to the parent: everything.
    profiles::activate(&conn, DEFAULT_PROFILE).unwrap();
    assert_eq!(visible_files(&conn), [11, 12, 13, 14, 15, 16]);
    assert_eq!(allowed_works(&conn), [1, 2, 3, 4, 5, 6]);
}

#[test]
fn rating_country_and_strictest_fallback() {
    let conn = open_in_memory().unwrap();
    library(&conn);
    set_certifications(
        &conn,
        1,
        "tmdb",
        &[
            ("US".into(), "PG".into()),
            ("DE".into(), "16".into()),
            ("xx".into(), "18".into()),
        ],
    )
    .unwrap();
    assert_eq!(
        certifications(&conn, 1).unwrap().len(),
        2,
        "bad country skipped"
    );
    assert_eq!(profiles::work_age(&conn, 1, "US").unwrap(), Some(10));
    assert_eq!(profiles::work_age(&conn, 1, "DE").unwrap(), Some(16));
    // No rating for the country: the strictest one.
    assert_eq!(profiles::work_age(&conn, 1, "FR").unwrap(), Some(16));
    assert_eq!(profiles::work_age(&conn, 1, "").unwrap(), Some(16));
    assert_eq!(profiles::work_age(&conn, 3, "US").unwrap(), None);
}

#[test]
fn deleting_the_active_profile_falls_back_to_the_default() {
    let conn = open_in_memory().unwrap();
    let k = profiles::create(&conn, &kid(), NOW).unwrap();
    profiles::activate(&conn, k.id).unwrap();
    profiles::delete(&conn, k.id).unwrap();
    assert_eq!(profiles::active(&conn).unwrap(), Some(DEFAULT_PROFILE));
}

#[test]
fn most_recent_profile_is_activated_on_open() {
    let dir = tempdir();
    let path = dir.join("lib.db");
    let id = {
        let conn = pa_library::open(&path).unwrap();
        let k = profiles::create(&conn, &kid(), NOW).unwrap();
        profiles::touch(&conn, k.id, NOW).unwrap();
        k.id
    };
    let conn = pa_library::open(&path).unwrap();
    assert_eq!(profiles::active(&conn).unwrap(), Some(id));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn predicates_fail_closed_without_an_active_profile() {
    let conn = open_in_memory().unwrap();
    library(&conn);
    conn.execute("DELETE FROM temp.active_profile", []).unwrap();
    assert!(visible_files(&conn).is_empty());
    assert!(allowed_works(&conn).is_empty());
}

fn tempdir() -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "pa-library-profiles-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
