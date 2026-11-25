// SPDX-License-Identifier: MPL-2.0
//! BUG-021: a movie split over several files (`part1`/`part2`, `cd1`/`cd2`,
//! the VOBs of a DVD title set) plays all its parts as one timeline
//! (LIB-020), so the card, the versions and the technical info show the
//! sum of the parts. A single file, alternative versions (LIB-010) and
//! extras keep their own duration. Synthetic files; the durations are what
//! the probe would write.

use std::path::Path;

use pa_catalog::{Catalog, Item};
use pa_library::rusqlite::{Connection, params};
use pa_scanner::CancelToken;

fn write(root: &Path, rel: &str, size: usize, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..size)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

// The files of the live QA-017 run: two parts of 2 minutes each.
const PART1: &str = "Movies/Charge (2022)/Charge (2022) - part1.mkv";
const PART2: &str = "Movies/Charge (2022)/Charge (2022) - part2.mkv";
const CD1: &str = "Movies/Heat (1995)/Heat.1995.DVDRip.cd1.avi";
const CD2: &str = "Movies/Heat (1995)/Heat.1995.DVDRip.cd2.avi";
const CD3: &str = "Movies/Heat (1995)/Heat.1995.DVDRip.cd3.avi";
const DVD: &str = "Movies/Alien (1979)/VIDEO_TS";
const ONE: &str = "Movies/Arrival (2016)/Arrival.2016.1080p.mkv";
const TRAILER: &str = "Movies/Arrival (2016)/Trailers/Arrival Trailer.mkv";
const HD: &str = "Movies/Inception (2010)/Inception.2010.1080p.BluRay.mkv";
const UHD: &str = "Movies/Inception (2010)/Inception.2010.2160p.HDR10.WEB-DL.mkv";

const MIN: i64 = 60_000;

fn setup() -> (tempfile::TempDir, Catalog) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    write(&root, PART1, 120_000, 1);
    write(&root, PART2, 110_000, 2);
    write(&root, CD1, 130_000, 3);
    write(&root, CD2, 125_000, 4);
    write(&root, CD3, 115_000, 5);
    write(&root, &format!("{DVD}/VIDEO_TS.IFO"), 200, 6);
    write(&root, &format!("{DVD}/VIDEO_TS.VOB"), 9_000, 7);
    write(&root, &format!("{DVD}/VTS_01_0.VOB"), 5_000, 8);
    write(&root, &format!("{DVD}/VTS_01_1.VOB"), 160_000, 9);
    write(&root, &format!("{DVD}/VTS_01_2.VOB"), 90_000, 10);
    write(&root, ONE, 150_000, 11);
    write(&root, TRAILER, 20_000, 12);
    write(&root, HD, 140_000, 13);
    write(&root, UHD, 170_000, 14);
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();

    // What the probe would write (no FFmpeg in tests).
    let conn = Connection::open(cat.path()).unwrap();
    let probe = |rel: &str, ms: i64| {
        let n = conn
            .execute(
                "UPDATE files SET duration_ms = ?2 WHERE path = ?1",
                params![format!("/{rel}"), ms],
            )
            .unwrap();
        assert_eq!(n, 1, "{rel}");
    };
    probe(PART1, 2 * MIN);
    probe(PART2, 2 * MIN);
    probe(CD1, 50 * MIN);
    probe(CD2, 45 * MIN);
    probe(CD3, 75 * MIN);
    probe(&format!("{DVD}/VTS_01_1.VOB"), 60 * MIN);
    probe(&format!("{DVD}/VTS_01_2.VOB"), 57 * MIN);
    probe(ONE, 116 * MIN);
    probe(TRAILER, 2 * MIN);
    probe(HD, 148 * MIN);
    probe(UHD, 148 * MIN + 1_000);
    drop(conn);
    (dir, cat)
}

fn movie(cat: &Catalog, title: &str) -> Item {
    let movies = cat.movies().unwrap();
    movies
        .iter()
        .find(|m| m.title == title)
        .unwrap_or_else(|| panic!("no {title} in {movies:#?}"))
        .clone()
}

/// Duration of the card, the item read by id (detail page), the default
/// version and the technical info.
fn durations(cat: &Catalog, title: &str) -> [Option<i64>; 4] {
    let card = movie(cat, title);
    let item = cat.item(card.id).unwrap().expect("item by id");
    let versions = cat.versions(card.id).unwrap();
    let default = versions
        .iter()
        .find(|v| v.is_default)
        .expect("a default version");
    let tech = cat
        .details(card.id)
        .unwrap()
        .tech
        .expect("tech info of the default version");
    [
        card.duration_ms,
        item.duration_ms,
        default.duration_ms,
        tech.duration_ms,
    ]
}

#[test]
fn a_split_movie_runs_for_all_its_parts() {
    let (_dir, cat) = setup();

    // part1 + part2 of 2 minutes each: 4 minutes everywhere, one version.
    let charge = movie(&cat, "Charge");
    assert_eq!(charge.rel_path, PART1);
    assert_eq!(cat.versions(charge.id).unwrap().len(), 1);
    assert_eq!(durations(&cat, "Charge"), [Some(4 * MIN); 4]);
    // The sum is that of what plays (LIB-020).
    let played: i64 = cat
        .play_parts(charge.source_id, &charge.rel_path)
        .unwrap()
        .iter()
        .filter_map(|p| p.duration_ms)
        .sum();
    assert_eq!(played, 4 * MIN);

    // Three CDs, and the main title set of a DVD.
    assert_eq!(durations(&cat, "Heat"), [Some(170 * MIN); 4]);
    assert_eq!(durations(&cat, "Alien"), [Some(117 * MIN); 4]);
}

#[test]
fn single_files_versions_and_extras_keep_their_own_duration() {
    let (_dir, cat) = setup();

    // A single file with a trailer next to it: the trailer is not a part.
    assert_eq!(durations(&cat, "Arrival"), [Some(116 * MIN); 4]);
    let extras = cat.details(movie(&cat, "Arrival").id).unwrap().extras;
    assert_eq!(extras.len(), 1, "{extras:?}");
    assert_eq!(extras[0].duration_ms, Some(2 * MIN));

    // Two versions of one film are alternatives, never added up.
    let inception = movie(&cat, "Inception");
    assert_eq!(inception.version_count, 2);
    let mut versions: Vec<_> = cat
        .versions(inception.id)
        .unwrap()
        .into_iter()
        .map(|v| (v.rel_path, v.duration_ms))
        .collect();
    versions.sort();
    assert_eq!(
        versions,
        [
            (HD.to_owned(), Some(148 * MIN)),
            (UHD.to_owned(), Some(148 * MIN + 1_000)),
        ]
    );
    let [card, item, default, tech] = durations(&cat, "Inception");
    assert!(
        card == Some(148 * MIN) || card == Some(148 * MIN + 1_000),
        "{card:?}"
    );
    assert_eq!([item, default, tech], [card; 3]);
}

#[test]
fn a_missing_or_unprobed_part_is_left_out_of_the_sum() {
    let (_dir, cat) = setup();
    let conn = Connection::open(cat.path()).unwrap();
    // cd3 is gone from the share, cd2 is not probed yet.
    conn.execute(
        "UPDATE files SET missing_since_ms = 1 WHERE path = ?1",
        [format!("/{CD3}")],
    )
    .unwrap();
    conn.execute(
        "UPDATE files SET duration_ms = NULL WHERE path = ?1",
        [format!("/{CD2}")],
    )
    .unwrap();
    drop(conn);
    assert_eq!(durations(&cat, "Heat"), [Some(50 * MIN); 4]);

    // No part probed at all: no duration rather than a wrong one.
    let conn = Connection::open(cat.path()).unwrap();
    conn.execute(
        "UPDATE files SET duration_ms = NULL WHERE path IN (?1, ?2)",
        [format!("/{PART1}"), format!("/{PART2}")],
    )
    .unwrap();
    drop(conn);
    assert_eq!(durations(&cat, "Charge"), [None; 4]);
}
