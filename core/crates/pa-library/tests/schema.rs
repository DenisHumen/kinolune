// SPDX-License-Identifier: MPL-2.0
//! Schema v1: migration from scratch, foreign-key integrity, CHECKs that
//! mirror pa-model, history checks and docs coverage (LIB-001).

use std::collections::BTreeSet;
use std::path::PathBuf;

use pa_library::rusqlite::{Connection, params};
use pa_library::{Error, HISTORY_TABLE, MIGRATIONS, latest_version, migrate, open, open_in_memory};
use pa_model::{
    AnimeType, BroadcasterType, EpisodeType, MediaKey, MediaKind, NumberingScheme,
    VideoAvailability, VideoType, WorkKind,
};

const TABLES: &[&str] = &[
    "anime_entries",
    "artwork",
    "audiobook_metadata",
    "collection_items",
    "collections",
    "display_preferences",
    "episode_mappings",
    "episodes",
    "external_ids",
    "favorites",
    "file_content_kinds",
    "file_ed2k_progress",
    "file_group_members",
    "file_groups",
    "files",
    "folder_content_overrides",
    "folder_rule_sync",
    "folder_rules",
    "frame_searches",
    "group_assets",
    "import_v0_progress",
    "jobs",
    "label_sync",
    "live_uploads",
    "match_candidates",
    "match_episodes",
    "matches",
    "music_album_artists",
    "music_album_metadata",
    "music_albums",
    "music_artist_metadata",
    "music_artists",
    "music_genres",
    "music_plays",
    "music_track_artists",
    "music_track_genres",
    "music_tracks",
    "offline_copies",
    "offline_files",
    "parse_results",
    "personal_libraries",
    "personal_library_files",
    "personal_library_folders",
    "personal_progress",
    "pin_attempts",
    "playlist_items",
    "playlists",
    "profile_folders",
    "profile_sync_tombstones",
    "profiles",
    "progress",
    "provider_cache",
    "search_docs",
    "search_fts",
    "search_queue",
    "seasons",
    "server_playlists",
    "skip_analysis",
    "skip_segments",
    "smart_shelves",
    "source_sync_tombstones",
    "sources",
    "storage_items",
    "storage_retention",
    "storage_targets",
    "sync_folder",
    "sync_folder_choice",
    "sync_folder_outbox",
    "sync_key_links",
    "sync_outbox",
    "titles",
    "twitch_assets",
    "twitch_categories",
    "twitch_category_segments",
    "twitch_chat_logs",
    "twitch_clips",
    "twitch_favorites",
    "twitch_live_sessions",
    "twitch_record_rules",
    "twitch_search",
    "twitch_streamers",
    "twitch_videos",
    "user_categories",
    "user_category_items",
    "user_ratings",
    "user_tag_items",
    "user_tags",
    "version_prefs",
    "view_prefs",
    "work_airing",
    "work_candidates",
    "work_certifications",
    "work_credits",
    "work_genres",
    "work_metadata",
    "work_ratings",
    "work_reviews",
    "works",
];

/// Shadow tables SQLite keeps for an FTS5 table; they belong to it.
const FTS5_SHADOWS: [&str; 5] = ["config", "content", "data", "docsize", "idx"];

fn tables(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            // Virtual tables count, their shadow tables (search_fts_data…) do not.
            "SELECT name FROM pragma_table_list WHERE schema = 'main' \
             AND type IN ('table', 'virtual') \
             AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' AND name <> ?1 ORDER BY name",
        )
        .unwrap();
    let all: Vec<String> = stmt
        .query_map([HISTORY_TABLE], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let virtual_tables: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND sql LIKE 'CREATE VIRTUAL%'",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    all.into_iter()
        .filter(|t| {
            !virtual_tables.iter().any(|v| {
                FTS5_SHADOWS
                    .iter()
                    .any(|suffix| *t == format!("{v}_{suffix}"))
            })
        })
        .collect()
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn temp_db(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "pa-library-{name}-{}-{nanos}.db",
        std::process::id()
    ))
}

fn remove_db(path: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        let _ = std::fs::remove_file(p);
    }
}

fn serde_name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

// --- Migration from scratch --------------------------------------------------

#[test]
fn migration_from_scratch_creates_every_table() {
    let mut conn = Connection::open_in_memory().unwrap();
    pa_library::configure(&conn).unwrap();
    let report = migrate(&mut conn).unwrap();
    assert_eq!(
        report.applied,
        MIGRATIONS.iter().map(|m| m.version).collect::<Vec<_>>()
    );
    assert_eq!(report.db_version, latest_version());
    assert!(!report.db_is_newer);
    assert_eq!(tables(&conn), TABLES);

    // Second run is a no-op.
    let again = migrate(&mut conn).unwrap();
    assert!(again.applied.is_empty());
    assert_eq!(again.db_version, latest_version());
}

#[test]
fn file_database_uses_wal_and_foreign_keys_and_reopens() {
    let path = temp_db("wal");
    {
        let conn = open(&path).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }
    let conn = open(&path).unwrap();
    assert_eq!(tables(&conn), TABLES);
    drop(conn);
    remove_db(&path);
}

#[test]
fn default_profile_exists() {
    let conn = open_in_memory().unwrap();
    let (id, sync_id, name): (i64, String, String) = conn
        .query_row("SELECT id, sync_id, name FROM profiles", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!((id, sync_id.as_str(), name.as_str()), (1, "default", ""));
}

#[test]
fn every_migration_file_is_embedded() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let on_disk: BTreeSet<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    let embedded: BTreeSet<String> = MIGRATIONS
        .iter()
        .map(|m| format!("{}.sql", m.name))
        .collect();
    assert_eq!(on_disk, embedded);
    for name in &on_disk {
        let (stamp, rest) = name.split_once('_').unwrap();
        assert!(stamp.len() == 12 && stamp.bytes().all(|b| b.is_ascii_digit()));
        let (id, slug) = rest.split_once('_').unwrap();
        assert!(
            id.contains('-'),
            "{name}: task ID expected after the timestamp"
        );
        assert!(slug.ends_with(".sql"));
    }
}

// --- Foreign-key integrity ----------------------------------------------------

struct ForeignKey {
    table: String,
    parent: String,
    from: Vec<String>,
    to: Vec<String>,
}

fn foreign_keys(conn: &Connection) -> Vec<ForeignKey> {
    let mut out = Vec::new();
    for table in tables(conn) {
        let mut stmt = conn
            .prepare("SELECT id, seq, \"table\", \"from\", \"to\" FROM pragma_foreign_key_list(?1) ORDER BY id, seq")
            .unwrap();
        let rows: Vec<(i64, String, String, Option<String>)> = stmt
            .query_map([&table], |r| {
                Ok((r.get(0)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut current: Option<(i64, ForeignKey)> = None;
        for (id, parent, from, to) in rows {
            if current.as_ref().is_none_or(|(cid, _)| *cid != id) {
                if let Some((_, fk)) = current.take() {
                    out.push(fk);
                }
                current = Some((
                    id,
                    ForeignKey {
                        table: table.clone(),
                        parent,
                        from: Vec::new(),
                        to: Vec::new(),
                    },
                ));
            }
            let fk = &mut current.as_mut().unwrap().1;
            fk.from.push(from);
            fk.to
                .push(to.expect("FKs name their parent columns explicitly"));
        }
        if let Some((_, fk)) = current {
            out.push(fk);
        }
    }
    out
}

/// (columns, unique, usable for `col = ?` lookups) of every index of a table,
/// including the implicit rowid primary key.
fn indexes(conn: &Connection, table: &str) -> Vec<(Vec<String>, bool, bool)> {
    let mut out = Vec::new();
    let pk: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info(?1) WHERE pk > 0 ORDER BY pk")
        .unwrap()
        .query_map([table], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    if !pk.is_empty() {
        out.push((pk, true, true));
    }
    let list: Vec<(String, bool, bool)> = conn
        .prepare("SELECT name, \"unique\", partial FROM pragma_index_list(?1)")
        .unwrap()
        .query_map([table], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (name, unique, partial) in list {
        let cols: Vec<String> = conn
            .prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")
            .unwrap()
            .query_map([&name], |r| {
                Ok(r.get::<_, Option<String>>(0)?.unwrap_or_default())
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        // A partial index `WHERE <first column> IS NOT NULL` still serves
        // `first = ?` lookups; other partial indexes do not.
        let lookup = !partial || {
            let sql: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?1",
                    [&name],
                    |r| r.get(0),
                )
                .unwrap();
            sql.trim_end()
                .ends_with(&format!("WHERE {} IS NOT NULL", cols[0]))
        };
        out.push((cols, unique && !partial, lookup));
    }
    out
}

#[test]
fn foreign_keys_reference_unique_parent_keys() {
    let conn = open_in_memory().unwrap();
    let fks = foreign_keys(&conn);
    assert!(
        fks.len() >= 30,
        "expected the v1 graph, got {} FKs",
        fks.len()
    );
    let all = tables(&conn);
    for fk in &fks {
        assert!(
            all.contains(&fk.parent),
            "{} -> missing {}",
            fk.table,
            fk.parent
        );
        let parent_keys = indexes(&conn, &fk.parent);
        let mut want = fk.to.clone();
        want.sort();
        assert!(
            parent_keys.iter().any(|(cols, unique, _)| {
                let mut c = cols.clone();
                c.sort();
                *unique && c == want
            }),
            "{}({:?}) -> {}({:?}) is not a unique key",
            fk.table,
            fk.from,
            fk.parent,
            fk.to
        );
    }
    // Also catches "foreign key mismatch" at the SQLite level.
    let violations: i64 = conn
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
}

#[test]
fn foreign_key_columns_are_indexed() {
    // Cascades and SET NULL look children up by the FK columns; without an
    // index every parent delete scans the child table.
    let conn = open_in_memory().unwrap();
    for fk in foreign_keys(&conn) {
        let covered = indexes(&conn, &fk.table)
            .iter()
            .any(|(cols, _, lookup)| *lookup && cols.starts_with(&fk.from));
        assert!(covered, "{}({:?}) has no index", fk.table, fk.from);
    }
}

/// Inserts one row into every table; returns the ids that tests delete.
struct Graph {
    source: i64,
    work: i64,
    other_work: i64,
    profile: i64,
}

fn build_graph(conn: &Connection) -> Graph {
    let now = 1_763_000_000_000_i64;
    conn.execute_batch(&format!(
        "
        INSERT INTO sources (id, kind, name, uri, created_at_ms)
            VALUES (10, 'smb', 'NAS', 'smb://nas/media', {now});
        INSERT INTO files (id, source_id, path, size_bytes, fingerprint, oshash, added_at_ms)
            VALUES (100, 10, 'Shows/Show/S01E01.mkv', 1000, 'ab12', '0123456789abcdef', {now}),
                   (101, 10, 'Shows/Show/S01E02.mkv', 1000, NULL, NULL, {now});
        INSERT INTO file_groups (id, source_id, root_path, kind) VALUES (20, 10, 'Shows/Show', 'series_folder');
        INSERT INTO file_group_members (group_id, file_id, role) VALUES (20, 100, 'main'), (20, 101, 'main');
        INSERT INTO group_assets (group_id, path, file_id, role, target_path, lang, forced)
            VALUES (20, 'Shows/Show/S01E01.ru.forced.srt', NULL, 'subtitle', 'Shows/Show/S01E01.mkv', 'rus', 1),
                   (20, 'Shows/Show/S01E02.mkv', 101, 'ignore', NULL, NULL, 0);
        INSERT INTO parse_results (file_id, parser_version, hypotheses_json, parsed_at_ms)
            VALUES (100, '1', '[{{\"title\":\"Show\"}}]', {now});

        INSERT INTO works (id, kind, title, created_at_ms, updated_at_ms)
            VALUES (30, 'show', 'Show', {now}, {now}),
                   (31, 'anime', 'Anime S2', {now}, {now});
        INSERT INTO external_ids (work_id, provider, ext_kind, ext_id) VALUES (30, 'tmdb', 'tv', '1399');
        INSERT INTO titles (work_id, lang, text, kind) VALUES (30, 'en', 'Show', 'main');
        INSERT INTO seasons (id, work_id, scheme, number) VALUES (40, 30, 'tmdb', 1);
        INSERT INTO episodes (id, work_id, season_id, season_number, episode_number)
            VALUES (50, 30, 40, 1, 1), (51, 30, 40, 1, 2);
        INSERT INTO episode_mappings (episode_id, scheme, episode_number, source)
            VALUES (50, 'tvdb_absolute', 1, 'tvdb');
        INSERT INTO anime_entries (work_id, anidb_aid, relation_prev) VALUES (31, 42, 30);
        INSERT INTO artwork (work_id, kind, provider, url, lang)
            VALUES (30, 'poster', 'tmdb', 'https://image.tmdb.org/t/p/w500/p.jpg', 'en');
        INSERT INTO artwork (episode_id, kind, provider, url)
            VALUES (50, 'still', 'tmdb', 'https://image.tmdb.org/t/p/w300/s.jpg');
        INSERT INTO work_metadata (work_id, outcome, language, fetched_at_ms)
            VALUES (30, 'matched', 'ru-RU', {now});

        INSERT INTO matches (id, file_id, work_id, confidence, state, decided_by, created_at_ms, updated_at_ms)
            VALUES (60, 100, 30, 0.9, 'auto', 'scorer', {now}, {now});
        INSERT INTO match_episodes (match_id, position, episode_id) VALUES (60, 0, 50), (60, 1, 51);
        INSERT INTO match_candidates (match_id, rank, work_id, score) VALUES (60, 1, 31, 0.2);
        INSERT INTO folder_rules (source_id, path_prefix, work_id, created_from_match_id, created_at_ms)
            VALUES (10, 'Shows/Show', 30, 60, {now});
        INSERT INTO folder_rule_sync (rule_key, rule, stamp, state)
            VALUES ('src-1:00112233445566778899aabbccddeeff', '{{}}', NULL, 'received');

        INSERT INTO provider_cache (provider, endpoint, key, payload, fetched_at_ms, ttl_ms)
            VALUES ('tmdb', 'tv', '1399', x'7b7d', {now}, 86400000);

        INSERT INTO profiles (id, sync_id, name, created_at_ms, updated_at_ms)
            VALUES (2, 'kid', 'Kid', {now}, {now});
        INSERT INTO display_preferences (profile_id, work_id, scheme) VALUES (2, 30, 'tvdb_aired');
        INSERT INTO progress (profile_id, media_key, source_type, position_ms, duration_ms, updated_at_ms, device_id)
            VALUES (2, 'tmdb:episode:1399:1:1', 'episode', 60000, 3600000, {now}, 'dev-a');
        INSERT INTO favorites (profile_id, work_id, created_at_ms) VALUES (2, 30, {now});
        INSERT INTO favorites (profile_id, media_key, created_at_ms) VALUES (2, 'file:ab12', {now});
        INSERT INTO collections (id, profile_id, name, kind, created_at_ms, updated_at_ms)
            VALUES (70, 2, 'Mine', 'manual', {now}, {now});
        INSERT INTO collection_items (collection_id, position, work_id, added_at_ms) VALUES (70, 0, 30, {now});
        INSERT INTO playlists (id, profile_id, name, kind, created_at_ms, updated_at_ms)
            VALUES (80, 2, 'Mix', 'audio', {now}, {now});
        INSERT INTO playlist_items (playlist_id, position, media_key, added_at_ms)
            VALUES (80, 0, 'mb:recording:b1a9c0e9-d987-4042-ae91-78d6a3267d69', {now});
        -- MUS-012: a playlist of the music server behind source 10.
        INSERT INTO playlists (id, profile_id, name, kind, created_at_ms, updated_at_ms)
            VALUES (81, 2, 'Road', 'audio', {now}, {now});
        INSERT INTO server_playlists (playlist_id, source_id, remote_id, synced_at_ms)
            VALUES (81, 10, 'pl-1', {now});
        INSERT INTO jobs (id, type, owner_profile_id, created_at_ms, updated_at_ms)
            VALUES (90, 'library_scan', 2, {now}, {now});
        -- SYNC-007: with a folder picked, a trigger copies ops to sync_folder_outbox.
        INSERT INTO sync_folder (id, source_id, enabled_at_ms) VALUES (1, 10, {now});
        -- SYNC-015: the user's choice of the folder (automatic).
        INSERT INTO sync_folder_choice (id, mode, evaluated_ms, sources) VALUES (1, 'auto', {now}, '10');
        INSERT INTO sync_outbox (op_id, op, payload, hlc, next_attempt_at_ms, created_at_ms)
            VALUES ('0190f5b8-7d2c-7cc0-8a4e-2b7d6f1e9a10', 'watched', '{{}}', '1.0@dev-a', {now}, {now});

        INSERT INTO files (id, source_id, path, size_bytes, added_at_ms)
            VALUES (102, 10, 'Music/Artist/Album/01.flac', 1000, {now});
        INSERT INTO music_artists (id, name, name_key, created_at_ms) VALUES (110, 'Artist', 'artist', {now});
        INSERT INTO music_albums (id, album_key, title, art_kind, art_source_id, art_path, created_at_ms, updated_at_ms)
            VALUES (111, 'aa:artist|album', 'Album', 'folder', 10, 'Music/Artist/Album/cover.jpg', {now}, {now});
        INSERT INTO music_album_artists (album_id, artist_id, position) VALUES (111, 110, 0);
        INSERT INTO music_tracks (id, file_id, album_id, title, tagged_size_bytes, tagged_at_ms)
            VALUES (112, 102, 111, 'Song', 1000, {now});
        INSERT INTO music_track_artists (track_id, artist_id, role, position) VALUES (112, 110, 'artist', 0);
        INSERT INTO music_genres (id, name, name_key) VALUES (113, 'Rock', 'rock');
        INSERT INTO music_track_genres (track_id, genre_id, position) VALUES (112, 113, 0);
        INSERT INTO music_plays (profile_id, media_key, play_count, last_played_at_ms) VALUES (2, 'file:ab12', 3, {now});
        INSERT INTO music_album_metadata (album_id, outcome, decided_by, confidence, release_group_mbid, genres,
                                          cover_url, cover_checked, fetched_at_ms)
            VALUES (111, 'matched', 'search', 0.9, '11111111-2222-4333-8444-555555555555', '[\"Rock\"]',
                    'https://coverartarchive.org/release-group/11111111-2222-4333-8444-555555555555/front-500', 1, {now});
        INSERT INTO music_artist_metadata (artist_id, outcome, decided_by, mbid, genres, fetched_at_ms)
            VALUES (110, 'matched', 'album', '11111111-2222-4333-8444-666666666666', '[]', {now});
        INSERT INTO audiobook_metadata (query, outcome, confidence, work_id, title, authors, cover_url,
                                        description, fetched_at_ms)
            VALUES ('author\x1ftitle', 'matched', 0.95, 'OL1W', 'Title', '[\"Author\"]',
                    'https://covers.openlibrary.org/b/id/1-L.jpg', 'About.', {now});

        INSERT INTO twitch_streamers (id, user_id, login, display_name, fetched_at_ms, updated_at_ms)
            VALUES (120, '141981764', 'twitchdev', 'TwitchDev', {now}, {now});
        INSERT INTO twitch_categories (id, game_id, name, fetched_at_ms) VALUES (121, '509658', 'Just Chatting', {now});
        INSERT INTO twitch_favorites (profile_id, user_id, added_at_ms) VALUES (2, '141981764', {now});
        INSERT INTO twitch_live_sessions (id, stream_id, user_id, started_at_ms, title_last, fetched_at_ms)
            VALUES (122, '40000000001', '141981764', {now}, 'Live', {now});
        INSERT INTO twitch_videos (id, video_id, user_id, stream_id, video_type, title, published_at_ms,
                                   duration_ms, fetched_at_ms)
            VALUES (123, '2000000001', '141981764', '40000000001', 'archive', 'VOD', {now}, 3600000, {now});
        INSERT INTO twitch_category_segments (video_id, game_id, start_ms, end_ms, source)
            VALUES ('2000000001', '509658', 0, 600000, 'eventsub');
        INSERT INTO twitch_category_segments (stream_id, game_id, start_ms, source)
            VALUES ('40000000001', '509658', 0, 'eventsub');
        INSERT INTO twitch_clips (id, slug, broadcaster_id, video_id, game_id, title, created_at_ms,
                                  duration_ms, fetched_at_ms)
            VALUES (124, 'Funny-Clip_1', '141981764', '2000000001', '509658', 'Clip', {now}, 30000, {now});
        INSERT INTO files (id, source_id, path, size_bytes, added_at_ms)
            VALUES (103, 10, 'Twitch/twitchdev/2000000001.mkv', 1000, {now});
        INSERT INTO twitch_assets (id, file_id, kind, user_id, video_id, title, origin, created_at_ms)
            VALUES (125, 103, 'vod', '141981764', '2000000001', 'VOD', 'server', {now});
        INSERT INTO twitch_chat_logs (asset_id, path, created_at_ms)
            VALUES (125, 'Twitch/twitchdev/2000000001.chat.json.gz', {now});
        INSERT INTO twitch_record_rules (profile_id, user_id, categories_filter_json, created_at_ms, updated_at_ms)
            VALUES (2, '141981764', '[\"509658\"]', {now}, {now});

        INSERT INTO work_genres (work_id, provider, name) VALUES (30, 'tmdb', 'Drama');
        INSERT INTO work_credits (work_id, provider, role, position, person_ref, name, character)
            VALUES (30, 'tmdb', 'cast', 0, '17419', 'Actor', 'Hero');
        INSERT INTO work_ratings (work_id, provider, value, votes) VALUES (30, 'tmdb', 8.4, 1200);
        INSERT INTO work_airing (work_id, status, next_season, next_episode, next_air_date, source, checked_at_ms)
            VALUES (30, 'airing', 1, 4, '2026-12-01', 'tmdb', {now});
        INSERT INTO skip_segments (file_id, kind, origin, start_ms, end_ms, confidence, updated_at_ms)
            VALUES (100, 'intro', 'audio', 60000, 90000, 0.9, {now});
        INSERT INTO skip_analysis (file_id, version, size_bytes, duration_ms, intro_fp,
                                   intro_fp_start_ms, analyzed_at_ms)
            VALUES (100, 1, 1000, 1400000, x'01000000', 0, {now});
        INSERT INTO view_prefs (profile_id, view, sort, descending, folders_first, filter_json,
                                updated_at_ms)
            VALUES (2, 'movies', 'added', 1, 0, '{{\"genres\":[\"Drama\"]}}', {now});
        INSERT INTO smart_shelves (profile_id, name, query_json, pinned_home, position,
                                   created_at_ms, updated_at_ms)
            VALUES (2, '4K', '{{\"kind\":\"movie\"}}', 1, 0, {now}, {now});
        INSERT INTO version_prefs (profile_id, work_id, file_id, updated_at_ms)
            VALUES (2, 30, 100, {now});
        INSERT INTO profile_folders (profile_id, source_id, path) VALUES (2, 10, '/Shows');
        INSERT INTO pin_attempts (one, failures, locked_until_ms) VALUES (1, 5, {now});
        INSERT INTO work_certifications (work_id, provider, country, certification, min_age)
            VALUES (30, 'tmdb', 'US', 'TV-14', 14), (30, 'user', '', '12+', 12);
        INSERT INTO storage_targets (id, kind, path, quota_bytes, reserve_percent, created_at_ms, updated_at_ms)
            VALUES ('downloads', 'downloads', '/srv/downloads', 1000000, 5, {now}, {now});
        INSERT INTO storage_items (target_id, rel_path, group_key, size_bytes, profile_id, created_at_ms)
            VALUES ('downloads', 'vod/1.mp4', 'twitch:channel:1', 1000, 2, {now});
        INSERT INTO storage_retention (target_id, group_key, keep_last, updated_at_ms)
            VALUES ('downloads', NULL, 5, {now});
        INSERT INTO offline_copies (id, storage_item_id, item_key, source_id, file_id, title,
                                    profile_id, created_at_ms)
            VALUES (1, (SELECT id FROM storage_items WHERE rel_path = 'vod/1.mp4'), 'm30', 10, 100,
                    'Movie', 2, {now});
        INSERT INTO offline_files (copy_id, source_id, path, role, size_bytes)
            VALUES (1, 10, '/Movies/a.mkv', 'video', 1000);
        INSERT INTO import_v0_progress (file_id, source_type, position_ms, duration_ms, watched,
                                        updated_at_ms)
            VALUES (100, 'movie', 1000, 5000, 0, {now});
        INSERT INTO file_ed2k_progress (file_id, size_bytes, mtime_ms, chunk_md4, updated_at_ms)
            VALUES (100, 20000000, 0, zeroblob(16), {now});
        INSERT INTO user_categories (id, profile_id, space, sync_id, name, name_key, position,
                                     created_at_ms, updated_at_ms)
            VALUES (1, 2, '', '0123456789abcdef0123456789abcdef', 'Weekend', 'weekend', 0,
                    {now}, {now});
        INSERT INTO user_category_items (category_id, work_id, media_key, added_at_ms)
            VALUES (1, 30, NULL, {now}), (1, NULL, 'file:8e245d9679d31e12', {now});
        INSERT INTO user_tags (id, profile_id, space, name, name_key, created_at_ms)
            VALUES (1, 2, '', 'Epic', 'epic', {now});
        INSERT INTO user_tag_items (tag_id, work_id, media_key, added_at_ms)
            VALUES (1, 30, NULL, {now});
        INSERT INTO user_ratings (profile_id, work_id, media_key, stars, updated_at_ms)
            VALUES (2, 30, NULL, 5, {now});
        INSERT INTO personal_libraries (id, profile_id, sync_id, name, lock_kind, lock_hash,
                                        created_at_ms, updated_at_ms)
            VALUES (1, 2, 'fedcba9876543210fedcba9876543210', 'Home', 'pin', 'argon2id$x',
                    {now}, {now});
        INSERT INTO personal_library_folders (source_id, path, library_id, added_at_ms)
            VALUES (10, '/Home videos', 1, {now});
        INSERT INTO personal_library_files (file_id, library_id, moved_at_ms)
            VALUES (102, 1, {now});
        INSERT INTO personal_progress (library_id, media_key, position_ms, duration_ms, watched,
                                       updated_at_ms)
            VALUES (1, 'file:0123456789abcdef0123456789abcdef-150000', 60000, 120000, 0, {now});
        INSERT INTO work_candidates (work_id, rank, provider, ext_kind, ext_id, title, year,
                                     score, explanation_json)
            VALUES (31, 1, 'tmdb', 'movie', '27205', 'Inception', 2010, 0.74,
                    '[]');
        INSERT INTO work_reviews (work_id, decision, decided_at_ms) VALUES (31, 'not_media', {now});
        "
    ))
    .unwrap();
    // Index the graph, then leave one entity queued so that every search
    // table has a row.
    // A synced source deleted here, its delete not sent yet (SYNC-010).
    conn.execute(
        "INSERT INTO source_sync_tombstones (sync_id, stamp, pending) VALUES ('gone', NULL, 1)",
        [],
    )
    .unwrap();
    // A profile deleted here, its delete not sent yet (SYNC-009).
    conn.execute(
        "INSERT INTO profile_sync_tombstones (sync_id, stamp, pending) VALUES ('gone', NULL, 1)",
        [],
    )
    .unwrap();
    // Labels of another device (SYNC-013): one applied, one waiting for an
    // item this device does not have, one category merged into another.
    conn.execute_batch(
        r#"INSERT INTO label_sync (register, profile, op, state, merged_into) VALUES
             ('rating'||char(31)||'default'||char(31)||'tmdb:movie:603', 'default',
              '{"schema_version":1,"data":{}}', 'applied', NULL),
             ('tag'||char(31)||'default'||char(31)||'epic'||char(31)||'tmdb:tv:1399', 'default',
              '{"schema_version":1,"data":{}}', 'parked', NULL),
             ('category'||char(31)||'default'||char(31)||'ffff', 'default', NULL, 'merged',
              '0123456789abcdef0123456789abcdef');"#,
    )
    .unwrap();
    // A file another device is copying into the library (LIB-028).
    conn.execute_batch(
        r#"INSERT INTO live_uploads (upload_id, phase, info, device_id, stamp, updated_ms)
           VALUES ('up-m3k9x2-0-5d21aa07', 'started', '{"v":1,"name":"Dune.mkv"}',
                   'kpc', NULL, 1763000000000);"#,
    )
    .unwrap();
    // Another device said its TMDB key and this `file:` key are one film
    // (SYNC-004).
    conn.execute(
        "INSERT INTO sync_key_links (media_key, alias, learned_at_ms)
         VALUES ('tmdb:movie:603', 'file:8e245d9679d31e12', 1)",
        [],
    )
    .unwrap();
    // LIB-025: what a file holds and a folder's «Это не то».
    conn.execute(
        "INSERT INTO file_content_kinds
             (file_id, kind, detected, version, score, reasons_json, classified_at_ms)
         VALUES (100, 'video', 'video', 1, -4.0, '[\"episode\"]', 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO folder_content_overrides (source_id, path, kind, set_at_ms)
         VALUES (10, '/Shows', 'video', 0)",
        [],
    )
    .unwrap();
    // META-031: the frame search of a file, by its fingerprint.
    conn.execute(
        "INSERT INTO frame_searches (fingerprint, outcome, anilist_id, episode, agreement,
                                     frames, candidates_json, searched_at_ms)
         VALUES ('8e245d9679d31e12', 'matched', 100002, 8, 1.0, 5, '[]', 0)",
        [],
    )
    .unwrap();
    pa_library::search::sync(conn).unwrap();
    conn.execute("INSERT INTO search_queue VALUES ('work', 31)", [])
        .unwrap();
    Graph {
        source: 10,
        work: 30,
        other_work: 31,
        profile: 2,
    }
}

#[test]
fn full_graph_has_no_violations() {
    let conn = open_in_memory().unwrap();
    build_graph(&conn);
    for table in TABLES {
        assert!(
            count(&conn, table) > 0,
            "{table} is empty in the test graph"
        );
    }
    let violations: i64 = conn
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
}

#[test]
fn orphans_are_rejected() {
    let conn = open_in_memory().unwrap();
    let orphans = [
        "INSERT INTO files (source_id, path, size_bytes, added_at_ms) VALUES (999, 'a', 1, 0)",
        "INSERT INTO episodes (work_id) VALUES (999)",
        "INSERT INTO progress (profile_id, media_key, source_type, position_ms, updated_at_ms, device_id) \
         VALUES (999, 'file:ab', 'video', 0, 0, 'd')",
        "INSERT INTO jobs (type, owner_profile_id, created_at_ms, updated_at_ms) VALUES ('cleanup', 999, 0, 0)",
    ];
    for sql in orphans {
        let err = conn.execute(sql, []).unwrap_err();
        assert!(err.to_string().contains("FOREIGN KEY"), "{sql}: {err}");
    }
}

#[test]
fn deleting_a_source_removes_its_files_but_keeps_the_catalog() {
    let conn = open_in_memory().unwrap();
    let g = build_graph(&conn);
    conn.execute("DELETE FROM sources WHERE id = ?1", [g.source])
        .unwrap();
    for table in [
        "files",
        "file_groups",
        "file_group_members",
        "group_assets",
        "parse_results",
        "matches",
        "match_episodes",
        "match_candidates",
        "folder_rules",
        "music_tracks",
        "music_track_artists",
        "music_track_genres",
        "profile_folders",
        "import_v0_progress",
        "file_ed2k_progress",
        "server_playlists",
    ] {
        assert_eq!(count(&conn, table), 0, "{table}");
    }
    // MUS-012: the server's playlist goes with it, the user's stays.
    assert_eq!(count(&conn, "playlists"), 1);
    // Albums and artists stay until `music::remove_orphans`; the cover
    // reference loses its source.
    assert_eq!(count(&conn, "music_albums"), 1);
    let art_source: Option<i64> = conn
        .query_row("SELECT art_source_id FROM music_albums", [], |r| r.get(0))
        .unwrap();
    assert_eq!(art_source, None);
    assert_eq!(count(&conn, "works"), 2);
    assert_eq!(count(&conn, "episodes"), 2);
    // User state is keyed by media key and survives the source.
    assert_eq!(count(&conn, "progress"), 1);
}

#[test]
fn deleting_a_work_cascades_through_the_catalog() {
    let conn = open_in_memory().unwrap();
    let g = build_graph(&conn);
    conn.execute("DELETE FROM works WHERE id = ?1", [g.work])
        .unwrap();
    for table in [
        "external_ids",
        "titles",
        "seasons",
        "episodes",
        "episode_mappings",
        "matches",
        "match_episodes",
        "folder_rules",
        "display_preferences",
        "collection_items",
        "artwork",
        "version_prefs",
        "work_metadata",
        "work_airing",
        "work_credits",
        "work_ratings",
        "work_certifications",
        "user_tag_items",
        "user_ratings",
    ] {
        assert_eq!(count(&conn, table), 0, "{table}");
    }
    let favorites_of_work: i64 = conn
        .query_row(
            "SELECT count(*) FROM favorites WHERE work_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(favorites_of_work, 0);
    // Files stay, now unmatched; the anime entry loses its relation.
    assert_eq!(count(&conn, "files"), 4);
    let prev: Option<i64> = conn
        .query_row(
            "SELECT relation_prev FROM anime_entries WHERE work_id = ?1",
            [g.other_work],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(prev, None);
}

#[test]
fn deleting_a_match_keeps_the_folder_rule() {
    let conn = open_in_memory().unwrap();
    build_graph(&conn);
    conn.execute("DELETE FROM matches", []).unwrap();
    let from: Option<i64> = conn
        .query_row("SELECT created_from_match_id FROM folder_rules", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(from, None);
    assert_eq!(count(&conn, "match_candidates"), 0);
}

#[test]
fn deleting_a_profile_removes_its_state_and_keeps_jobs() {
    let conn = open_in_memory().unwrap();
    let g = build_graph(&conn);
    conn.execute("DELETE FROM profiles WHERE id = ?1", [g.profile])
        .unwrap();
    for table in [
        "display_preferences",
        "progress",
        "favorites",
        "collections",
        "collection_items",
        "playlists",
        "playlist_items",
        "server_playlists",
        "version_prefs",
        "profile_folders",
        "user_categories",
        "user_category_items",
        "user_tags",
        "user_tag_items",
        "user_ratings",
        "personal_libraries",
        "personal_library_folders",
        "personal_library_files",
        "personal_progress",
    ] {
        assert_eq!(count(&conn, table), 0, "{table}");
    }
    let owner: Option<i64> = conn
        .query_row("SELECT owner_profile_id FROM jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(owner, None);
}

#[test]
fn one_live_match_per_file() {
    let conn = open_in_memory().unwrap();
    build_graph(&conn);
    let insert = |state: &str| {
        conn.execute(
            "INSERT INTO matches (file_id, work_id, confidence, state, decided_by, created_at_ms, updated_at_ms) \
             VALUES (100, 31, 0.5, ?1, 'user', 0, 0)",
            [state],
        )
    };
    assert!(insert("auto").is_err());
    insert("rejected").unwrap();
    insert("rejected").unwrap();
}

// --- CHECKs mirror pa-model ----------------------------------------------------

#[test]
fn work_and_episode_enums_match_pa_model() {
    let conn = open_in_memory().unwrap();
    for kind in [WorkKind::Movie, WorkKind::Show, WorkKind::Anime] {
        conn.execute(
            "INSERT INTO works (kind, title, created_at_ms, updated_at_ms) VALUES (?1, 't', 0, 0)",
            [serde_name(&kind)],
        )
        .unwrap();
    }
    for t in [
        AnimeType::Tv,
        AnimeType::Ova,
        AnimeType::Ona,
        AnimeType::Movie,
        AnimeType::Special,
        AnimeType::MusicVideo,
    ] {
        conn.execute(
            "INSERT INTO works (kind, anime_type, title, created_at_ms, updated_at_ms) \
             VALUES ('anime', ?1, 't', 0, 0)",
            [serde_name(&t)],
        )
        .unwrap();
    }
    let work = conn.last_insert_rowid();
    for (i, scheme) in [
        NumberingScheme::Tmdb,
        NumberingScheme::TvdbAired,
        NumberingScheme::TvdbDvd,
        NumberingScheme::TvdbAbsolute,
        NumberingScheme::Anidb,
        NumberingScheme::TmdbGroup,
    ]
    .iter()
    .enumerate()
    {
        let name = serde_name(scheme);
        let scheme_ref = if name == "tmdb_group" { "5eb7" } else { "" };
        conn.execute(
            "INSERT INTO seasons (work_id, scheme, scheme_ref, number) VALUES (?1, ?2, ?3, ?4)",
            params![work, name, scheme_ref, i as i64],
        )
        .unwrap();
    }
    for t in [
        EpisodeType::Regular,
        EpisodeType::Special,
        EpisodeType::Credit,
        EpisodeType::Trailer,
        EpisodeType::Parody,
        EpisodeType::Recap,
        EpisodeType::Other,
    ] {
        conn.execute(
            "INSERT INTO episodes (work_id, episode_type) VALUES (?1, ?2)",
            params![work, serde_name(&t)],
        )
        .unwrap();
    }
}

#[test]
fn twitch_enums_match_pa_model() {
    let conn = open_in_memory().unwrap();
    for (i, t) in [
        BroadcasterType::Partner,
        BroadcasterType::Affiliate,
        BroadcasterType::Normal,
    ]
    .iter()
    .enumerate()
    {
        conn.execute(
            "INSERT INTO twitch_streamers (user_id, login, display_name, broadcaster_type, \
             fetched_at_ms, updated_at_ms) VALUES (?1, 'l', 'D', ?2, 0, 0)",
            params![(i + 1).to_string(), serde_name(t)],
        )
        .unwrap();
    }
    let mut n = 0;
    for t in [VideoType::Archive, VideoType::Highlight, VideoType::Upload] {
        for a in [
            VideoAvailability::Online,
            VideoAvailability::Deleted,
            VideoAvailability::Unknown,
        ] {
            n += 1;
            conn.execute(
                "INSERT INTO twitch_videos (video_id, user_id, video_type, title, published_at_ms, \
                 duration_ms, availability, fetched_at_ms) VALUES (?1, '1', ?2, 't', 0, 0, ?3, 0)",
                params![n.to_string(), serde_name(&t), serde_name(&a)],
            )
            .unwrap();
        }
    }
}

#[test]
fn progress_accepts_every_media_kind_and_key() {
    let conn = open_in_memory().unwrap();
    let keys = [
        (
            MediaKey::File {
                fingerprint: "ab12".into(),
            },
            MediaKind::Video,
        ),
        (MediaKey::TmdbMovie { id: 603 }, MediaKind::Movie),
        (
            MediaKey::TmdbEpisode {
                show_id: 1399,
                season: 0,
                episode: 1,
            },
            MediaKind::Episode,
        ),
        (
            MediaKey::MbRecording {
                mbid: "b1a9c0e9-d987-4042-ae91-78d6a3267d69".into(),
            },
            MediaKind::Track,
        ),
        (
            MediaKey::TwitchVod {
                video_id: "123".into(),
            },
            MediaKind::TwitchVod,
        ),
        (
            MediaKey::TwitchRecording {
                stream_id: "456".into(),
            },
            MediaKind::TwitchRecording,
        ),
        (
            MediaKey::TwitchClip {
                slug: "Funny-Clip_1".into(),
            },
            MediaKind::TwitchClip,
        ),
    ];
    for (key, kind) in &keys {
        assert!(key.is_valid());
        conn.execute(
            "INSERT INTO progress (profile_id, media_key, source_type, position_ms, updated_at_ms, device_id) \
             VALUES (1, ?1, ?2, 0, 0, 'dev')",
            params![key.to_string(), serde_name(kind)],
        )
        .unwrap();
    }
}

#[test]
fn checks_reject_bad_values() {
    let conn = open_in_memory().unwrap();
    let bad = [
        "INSERT INTO works (kind, title, created_at_ms, updated_at_ms) VALUES ('series', 't', 0, 0)",
        "INSERT INTO works (kind, anime_type, title, created_at_ms, updated_at_ms) VALUES ('show', 'tv', 't', 0, 0)",
        "INSERT INTO works (kind, title, created_at_ms, updated_at_ms) VALUES ('movie', '', 0, 0)",
        // STRICT: a string is not an integer.
        "INSERT INTO works (kind, title, year, created_at_ms, updated_at_ms) VALUES ('movie', 't', 'abc', 0, 0)",
        "INSERT INTO sources (kind, name, uri, created_at_ms) VALUES ('SMB', 'n', 'smb://x', 0)",
        "INSERT INTO progress (profile_id, media_key, source_type, position_ms, updated_at_ms, device_id) \
         VALUES (1, 'nokey', 'movie', 0, 0, 'd')",
        "INSERT INTO progress (profile_id, media_key, source_type, position_ms, updated_at_ms, device_id) \
         VALUES (1, 'tmdb:movie:1', 'film', 0, 0, 'd')",
        "INSERT INTO progress (profile_id, media_key, source_type, position_ms, updated_at_ms, device_id) \
         VALUES (1, 'tmdb:movie:1', 'movie', -1, 0, 'd')",
        "INSERT INTO favorites (profile_id, created_at_ms) VALUES (1, 0)",
        "INSERT INTO jobs (type, state, created_at_ms, updated_at_ms) VALUES ('cleanup', 'stuck', 0, 0)",
        "INSERT INTO jobs (type, params_json, created_at_ms, updated_at_ms) VALUES ('cleanup', '{', 0, 0)",
        "INSERT INTO collections (name, kind, created_at_ms, updated_at_ms) VALUES ('s', 'smart', 0, 0)",
        "INSERT INTO file_groups (source_id, root_path, kind, disc_unit) VALUES (1, 'x', 'disc', 'hddvd')",
        "INSERT INTO group_assets (group_id, path, role) VALUES (1, 'x.srt', 'sidecar')",
        "INSERT INTO group_assets (group_id, path, role, art_kind) VALUES (1, 'x.jpg', 'art', 'wallpaper')",
        "INSERT INTO music_artists (name, name_key, mbid, created_at_ms) VALUES ('a', 'a', 'B1A9C0E9-D987-4042-AE91-78D6A3267D69', 0)",
        "INSERT INTO music_albums (album_key, title, art_kind, created_at_ms, updated_at_ms) VALUES ('k', 't', 'folder', 0, 0)",
        "INSERT INTO music_albums (album_key, title, art_kind, art_path, created_at_ms, updated_at_ms) VALUES ('k', 't', 'web', 'x', 0, 0)",
        "INSERT INTO music_genres (name, name_key) VALUES ('', '')",
        "INSERT INTO twitch_streamers (user_id, login, display_name, fetched_at_ms, updated_at_ms) \
         VALUES ('0123', 'a', 'A', 0, 0)",
        "INSERT INTO twitch_streamers (user_id, login, display_name, broadcaster_type, fetched_at_ms, updated_at_ms) \
         VALUES ('1', 'a', 'A', 'staff', 0, 0)",
        "INSERT INTO twitch_videos (video_id, user_id, video_type, title, published_at_ms, duration_ms, fetched_at_ms) \
         VALUES ('1', '1', 'premiere', 't', 0, 0, 0)",
        "INSERT INTO twitch_videos (video_id, user_id, video_type, title, published_at_ms, duration_ms, \
         availability, fetched_at_ms) VALUES ('1', '1', 'archive', 't', 0, 0, 'gone', 0)",
        "INSERT INTO twitch_favorites (profile_id, user_id, source, added_at_ms) VALUES (1, '1', 'twitch', 0)",
        "INSERT INTO twitch_category_segments (game_id, start_ms, source) VALUES ('1', 0, 'eventsub')",
        "INSERT INTO twitch_clips (slug, broadcaster_id, title, created_at_ms, duration_ms, fetched_at_ms) \
         VALUES ('bad slug', '1', 't', 0, 0, 0)",
        "INSERT INTO twitch_assets (file_id, kind, user_id, title, origin, created_at_ms) \
         VALUES (1, 'vod', '1', 't', 'server', 0)",
        "INSERT INTO twitch_record_rules (profile_id, user_id, categories_filter_json, created_at_ms, updated_at_ms) \
         VALUES (1, '1', '{}', 0, 0)",
    ];
    for sql in bad {
        let err = conn.execute(sql, []).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("CHECK") || msg.contains("cannot store") || msg.contains("NOT NULL"),
            "{sql}: {msg}"
        );
    }
}

// --- History and foreign databases ---------------------------------------------

/// The table set of the pa_library v0 (Dart, release 0.1) database.
const LEGACY_V0_SQL: &str = "
CREATE TABLE sources (id TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, path TEXT NOT NULL,
  username TEXT, position INTEGER NOT NULL DEFAULT 0);
CREATE TABLE items (id TEXT PRIMARY KEY, kind TEXT NOT NULL, source_id TEXT, title TEXT NOT NULL,
  file_path TEXT NOT NULL, added_at INTEGER NOT NULL);
CREATE TABLE progress (item_id TEXT PRIMARY KEY, position_ms INTEGER NOT NULL, duration_ms INTEGER NOT NULL,
  updated_at INTEGER NOT NULL, watched INTEGER NOT NULL DEFAULT 0);
CREATE TABLE tmdb_cache (key TEXT PRIMARY KEY, body TEXT NOT NULL, fetched_at INTEGER NOT NULL);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
PRAGMA user_version = 1;
";

#[test]
fn legacy_v0_database_is_detected_and_left_untouched() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(LEGACY_V0_SQL).unwrap();
    assert!(pa_library::is_legacy_v0(&conn).unwrap());
    let before = tables(&conn);
    match migrate(&mut conn) {
        Err(Error::ForeignDatabase { legacy_v0: true }) => {}
        other => panic!("expected ForeignDatabase, got {other:?}"),
    }
    assert_eq!(tables(&conn), before);

    let fresh = open_in_memory().unwrap();
    assert!(!pa_library::is_legacy_v0(&fresh).unwrap());
}

#[test]
fn unknown_tables_are_refused() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE notes (x)").unwrap();
    assert!(matches!(
        migrate(&mut conn),
        Err(Error::ForeignDatabase { legacy_v0: false })
    ));
}

fn add_history_row(conn: &Connection, version: i64) {
    conn.execute(
        &format!(
            "INSERT INTO {HISTORY_TABLE} (version, name, applied_on, checksum) \
             VALUES (?1, 'future', '2030-01-01T00:00:00Z', '1')"
        ),
        [version],
    )
    .unwrap();
}

#[test]
fn database_from_a_newer_build_is_accepted() {
    let mut conn = open_in_memory().unwrap();
    add_history_row(&conn, 209_912_312_359);
    let report = migrate(&mut conn).unwrap();
    assert!(report.applied.is_empty());
    assert!(report.db_is_newer);
    assert_eq!(report.db_version, 209_912_312_359);
}

#[test]
fn unknown_older_migration_is_refused() {
    let mut conn = open_in_memory().unwrap();
    add_history_row(&conn, 202_601_010_000);
    assert!(matches!(
        migrate(&mut conn),
        Err(Error::UnknownMigration {
            version: 202_601_010_000
        })
    ));
}

#[test]
fn out_of_order_migration_is_refused() {
    let mut conn = open_in_memory().unwrap();
    conn.execute(&format!("DELETE FROM {HISTORY_TABLE}"), [])
        .unwrap();
    add_history_row(&conn, 209_912_312_359);
    assert!(matches!(
        migrate(&mut conn),
        Err(Error::OutOfOrder { version }) if version == MIGRATIONS[0].version
    ));
}

#[test]
fn edited_migration_is_refused() {
    let mut conn = open_in_memory().unwrap();
    conn.execute(&format!("UPDATE {HISTORY_TABLE} SET checksum = '1'"), [])
        .unwrap();
    assert!(matches!(
        migrate(&mut conn),
        Err(Error::ChecksumMismatch { version }) if version == MIGRATIONS[0].version
    ));
}

// --- Documentation ----------------------------------------------------------------

#[test]
fn every_table_is_documented() {
    let doc = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../docs/dev/db.md");
    let text = std::fs::read_to_string(&doc).expect("docs/dev/db.md exists");
    for table in TABLES {
        assert!(
            text.contains(&format!("### `{table}`")),
            "docs/dev/db.md has no section for `{table}`"
        );
    }
}
