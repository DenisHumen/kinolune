// SPDX-License-Identifier: MPL-2.0
//! Imported markers (PLAY-015) end to end: hand-made Plex, Jellyfin and
//! EDL sidecars next to a season of (empty) videos are read through the
//! `Opener`, stored in the library and come back from
//! `effective_segments` — what `CoreSkip.segments` hands to
//! `PlayRequest.skipSegments` — within ±0.5 s, over detected segments.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pa_library::rusqlite::{Connection, params};
use pa_library::skip::{self, SkipKind, SkipOrigin, SkipProvider, SkipSegment};
use pa_skip::{Opener, import_markers, import_season};
use pa_vfs::{ByteSource, LocalFs, Vfs, VfsPath};

/// Done when of PLAY-015.
const TOLERANCE_MS: u64 = 500;

const DURATION_MS: i64 = 1_440_000;

struct Dir(PathBuf);

impl Opener for Dir {
    fn open(&self, _source_id: i64, path: &str) -> Result<Arc<dyn ByteSource>, String> {
        let vfs = LocalFs::new(&self.0).map_err(|e| e.to_string())?;
        let path = VfsPath::parse(path).map_err(|e| e.to_string())?;
        vfs.open(&path).map(Arc::from).map_err(|e| e.to_string())
    }

    fn list(&self, _source_id: i64, dir: &str) -> Result<Vec<String>, String> {
        let vfs = LocalFs::new(&self.0).map_err(|e| e.to_string())?;
        let dir = VfsPath::parse(if dir.is_empty() { "/" } else { dir }).map_err(|e| e.to_string())?;
        Ok(vfs
            .list(&dir)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|e| e.name)
            .collect())
    }
}

/// A source that is away: nothing can be listed or opened.
struct Away;

impl Opener for Away {
    fn open(&self, _source_id: i64, _path: &str) -> Result<Arc<dyn ByteSource>, String> {
        Err("host unreachable".into())
    }

    fn list(&self, _source_id: i64, _dir: &str) -> Result<Vec<String>, String> {
        Err("host unreachable".into())
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/import")
}

const SEASON: &str = "Show/Season 01";

/// A season of 7 empty videos with the fixtures next to them, and a
/// library that knows the videos (ids 1–7).
fn season() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let season = dir.path().join(SEASON);
    std::fs::create_dir_all(&season).unwrap();
    for entry in std::fs::read_dir(fixtures()).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), season.join(entry.file_name())).unwrap();
    }
    std::fs::write(season.join("notes.txt"), "not a sidecar").unwrap();
    let conn = pa_library::open_in_memory().unwrap();
    conn.execute(
        "INSERT INTO sources (id, kind, name, uri, created_at_ms) VALUES (1, 'local', 'L', 'file:///m', 0)",
        [],
    )
    .unwrap();
    for n in 1..=7_i64 {
        let name = format!("Show S01E0{n}.mkv");
        std::fs::write(season.join(&name), b"").unwrap();
        conn.execute(
            "INSERT INTO files (id, source_id, path, size_bytes, duration_ms, added_at_ms)
             VALUES (?1, 1, ?2, 0, ?3, 0)",
            params![n, format!("/{SEASON}/{name}"), DURATION_MS],
        )
        .unwrap();
    }
    (dir, conn)
}

type Want = (SkipKind, SkipProvider, &'static str, u64, u64);

fn assert_imported(conn: &Connection, file_id: i64, want: &[Want]) {
    let got: Vec<SkipSegment> = skip::effective_segments(conn, file_id)
        .unwrap()
        .into_iter()
        .filter(|s| s.origin == SkipOrigin::Import)
        .collect();
    assert_eq!(got.len(), want.len(), "file {file_id}: {got:?}");
    for (kind, provider, sidecar, start, end) in want {
        let s = got
            .iter()
            .find(|s| s.kind == *kind)
            .unwrap_or_else(|| panic!("file {file_id}: no {kind:?} in {got:?}"));
        assert_eq!(s.provider, Some(*provider), "file {file_id} {kind:?}");
        assert_eq!(s.provider_ref.as_deref(), Some(*sidecar), "file {file_id}");
        assert!(
            s.start_ms.abs_diff(*start) <= TOLERANCE_MS && s.end_ms.abs_diff(*end) <= TOLERANCE_MS,
            "file {file_id} {kind:?}: {}–{} vs {start}–{end}",
            s.start_ms,
            s.end_ms
        );
    }
}

#[test]
fn sidecars_of_a_season() {
    let (dir, conn) = season();
    // Detection and a chapter found something else for episode 1.
    skip::replace_segments(
        &conn,
        1,
        &[SkipOrigin::Audio, SkipOrigin::Chapter],
        &[
            SkipSegment::new(SkipKind::Intro, SkipOrigin::Audio, 59_000, 99_000, 0.9),
            SkipSegment::new(SkipKind::Intro, SkipOrigin::Chapter, 60_000, 100_000, 1.0),
        ],
        0,
    )
    .unwrap();

    let opener = Dir(dir.path().to_path_buf());
    let report = import_season(&conn, &opener, 3).unwrap();
    assert_eq!(report.files, 7);
    assert_eq!(report.imported, 6);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.invalid, 0);

    use SkipKind::{Credits, Intro, Recap};
    use SkipProvider::{Edl, Jellyfin, Plex};
    assert_imported(
        &conn,
        1,
        &[
            (Intro, Plex, "Show S01E01.plex.json", 61_250, 101_250),
            (Credits, Plex, "Show S01E01.plex.json", 1_350_400, 1_440_000),
        ],
    );
    // The import wins over the chapter and the fingerprint.
    let eff = skip::effective_segments(&conn, 1).unwrap();
    assert_eq!(eff[0].origin, SkipOrigin::Import);
    assert_eq!(skip::segments(&conn, 1).unwrap().len(), 4);
    assert_imported(
        &conn,
        2,
        &[
            (Intro, Plex, "Show S01E02.plex.xml", 5_000, 45_000),
            (Credits, Plex, "Show S01E02.plex.xml", 1_351_750, 1_440_000),
        ],
    );
    assert_imported(
        &conn,
        3,
        &[
            (Recap, Jellyfin, "Show S01E03.jellyfin.json", 0, 25_000),
            (Intro, Jellyfin, "Show S01E03.jellyfin.json", 30_050, 70_050),
            (Credits, Jellyfin, "Show S01E03.jellyfin.json", 1_360_000, 1_440_000),
        ],
    );
    assert_imported(
        &conn,
        4,
        &[
            (Intro, Jellyfin, "Show S01E04.jellyfin.json", 90_125, 130_125),
            (Credits, Jellyfin, "Show S01E04.jellyfin.json", 1_355_500, 1_440_000),
        ],
    );
    assert_imported(
        &conn,
        5,
        &[
            (Intro, Edl, "Show S01E05.edl", 15_375, 55_375),
            (Credits, Edl, "Show S01E05.edl", 1_349_875, 1_440_000),
        ],
    );
    // Plex's intro wins over the EDL's guess; the EDL gives the credits.
    assert_imported(
        &conn,
        6,
        &[
            (Intro, Plex, "Show S01E06.plex.json", 21_000, 61_000),
            (Credits, Edl, "Show S01E06.edl", 1_350_000, 1_440_000),
        ],
    );
    assert_imported(&conn, 7, &[]);

    // A source that is away changes nothing.
    let away = import_season(&conn, &Away, 1).unwrap();
    assert_eq!(away.skipped, 7);
    assert_eq!(skip::effective_segments(&conn, 5).unwrap().len(), 2);

    // A sidecar that is gone takes its markers with it.
    std::fs::remove_file(dir.path().join(SEASON).join("Show S01E05.edl")).unwrap();
    import_season(&conn, &opener, 5).unwrap();
    assert_imported(&conn, 5, &[]);
}

#[test]
fn handed_over_markers_and_sidecars() {
    let (dir, conn) = season();
    // A media server's answer for episode 7 (no sidecars) and episode 1.
    let jellyfin = std::fs::read_to_string(fixtures().join("Show S01E03.jellyfin.json")).unwrap();
    assert_eq!(import_markers(&conn, 7, SkipProvider::Jellyfin, &jellyfin).unwrap(), 3);
    let plex = r#"{"MediaContainer":{"Metadata":[{"Marker":[
        {"type":"intro","startTimeOffset":1000,"endTimeOffset":31000},
        {"type":"credits","startTimeOffset":1300000,"endTimeOffset":1440000}]}]}}"#;
    assert_eq!(import_markers(&conn, 1, SkipProvider::Plex, plex).unwrap(), 2);
    assert!(import_markers(&conn, 1, SkipProvider::Plex, "{oops").is_err());

    import_season(&conn, &Dir(dir.path().to_path_buf()), 1).unwrap();
    // Episode 7 keeps the handed-over markers.
    let e7 = skip::effective_segments(&conn, 7).unwrap();
    assert_eq!(e7.len(), 3);
    assert!(e7.iter().all(|s| s.provider == Some(SkipProvider::Jellyfin)
        && s.provider_ref.is_none()));
    // Episode 1's sidecar wins over the handed-over markers.
    let e1 = skip::effective_segments(&conn, 1).unwrap();
    assert_eq!(e1[0].start_ms, 61_250);
    assert_eq!(e1[0].provider_ref.as_deref(), Some("Show S01E01.plex.json"));
}
