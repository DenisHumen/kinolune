// SPDX-License-Identifier: MPL-2.0
//! PLAY-017: a Blu-ray folder, a Blu-ray image and a DVD image in the
//! library play their main title with every stream, chapter and angle; the
//! files inside an image are served by the opener without unpacking; a
//! protected image is reported. Generated navigation files and a few bytes
//! per clip, no real disc data.

use std::path::Path;
use std::sync::Arc;

use pa_catalog::disc::DiscPlayback;
use pa_catalog::{Catalog, SourceOpener};
use pa_scanner::CancelToken;
use pa_scanner::disc::fixture::ifo_bytes;
use pa_scanner::disc::{DiscKind, Protection, clpi_bytes};
use pa_scanner::mpls::{Mark, PlayItem, Playlist, Stream, StreamKind};
use pa_vfs::VfsPath;
use pa_vfs::iso::build::{ImageBuilder, Layout};

const T0: u32 = 27_000_000;

fn item(clip: &str, secs: u32) -> PlayItem {
    PlayItem {
        clip_id: clip.into(),
        in_time: T0,
        out_time: T0 + secs * 45_000,
        angles: Vec::new(),
    }
}

fn clip_bytes(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i % 241).unwrap().wrapping_add(seed))
        .collect()
}

/// A Blu-ray: the film (two items, the second with two angles), 3 audio
/// streams, 2 PGS, 4 chapters; a 5-minute extra and a menu loop.
fn blu_ray(extra_files: &[(&str, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
    let mut angled = item("00011", 1800);
    angled.angles = vec!["00012".into()];
    let stream = |kind, pid, coding, lang: &str| Stream {
        kind,
        pid,
        coding,
        lang: lang.into(),
    };
    let film = Playlist {
        items: vec![item("00010", 3600), angled],
        streams: vec![
            stream(StreamKind::Video, 0x1011, 0x1B, ""),
            stream(StreamKind::Audio, 0x1100, 0x81, "eng"),
            stream(StreamKind::Audio, 0x1101, 0x81, "rus"),
            stream(StreamKind::Audio, 0x1102, 0x81, "ukr"),
            stream(StreamKind::Subtitle, 0x1200, 0x90, "eng"),
            stream(StreamKind::Subtitle, 0x1201, 0x90, "rus"),
        ],
        marks: vec![
            Mark { item: 0, time: T0 },
            Mark {
                item: 0,
                time: T0 + 1800 * 45_000,
            },
            Mark { item: 1, time: T0 },
            Mark {
                item: 1,
                time: T0 + 900 * 45_000,
            },
        ],
    };
    let simple = |items| {
        Playlist {
            items,
            ..Playlist::default()
        }
        .to_bytes()
    };
    let mut files = vec![
        ("BDMV/index.bdmv".to_owned(), b"INDX0200".to_vec()),
        (
            "BDMV/PLAYLIST/00000.mpls".into(),
            simple(vec![item("00099", 20)]),
        ),
        ("BDMV/PLAYLIST/00001.mpls".into(), film.to_bytes()),
        (
            "BDMV/PLAYLIST/00002.mpls".into(),
            simple(vec![item("00020", 300)]),
        ),
        (
            "BDMV/CLIPINF/00010.clpi".into(),
            clpi_bytes(T0, T0 + 3600 * 45_000),
        ),
    ];
    // The film's clips are the biggest files.
    for (clip, seed, len) in [
        ("00010", 1, 30_000),
        ("00011", 2, 20_000),
        ("00012", 3, 20_000),
        ("00020", 4, 5_000),
        ("00099", 5, 1_000),
    ] {
        files.push((format!("BDMV/STREAM/{clip}.m2ts"), clip_bytes(seed, len)));
    }
    for (p, d) in extra_files {
        files.push(((*p).to_owned(), d.clone()));
    }
    files
}

fn dvd() -> Vec<(String, Vec<u8>)> {
    vec![
        ("VIDEO_TS/VIDEO_TS.IFO".into(), b"DVDVIDEO-VMG".to_vec()),
        (
            "VIDEO_TS/VTS_01_0.IFO".into(),
            ifo_bytes(&["en", "ru"], &["en", "ru"], &[1_200_000, 1_800_000]),
        ),
        ("VIDEO_TS/VTS_01_1.VOB".into(), clip_bytes(7, 40_960)),
        ("VIDEO_TS/VTS_01_2.VOB".into(), clip_bytes(8, 20_480)),
    ]
}

fn image(layout: Layout, volume: &str, files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut b = ImageBuilder::new(layout, volume);
    for (p, d) in files {
        b = b.file(p, d.clone());
    }
    b.build()
}

fn put(root: &Path, rel: &str, data: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

fn library(root: &Path) {
    for (p, d) in blu_ray(&[]) {
        put(root, &format!("Movies/Arrival (2016)/{p}"), &d);
    }
    put(
        root,
        "Movies/Dune (2021).iso",
        &image(Layout::BluRay, "DUNE", &blu_ray(&[])),
    );
    put(
        root,
        "Movies/Alien (1979).iso",
        &image(Layout::Dvd, "ALIEN", &dvd()),
    );
    let aacs = blu_ray(&[("AACS/Unit_Key_RO.inf", vec![1; 16])]);
    put(
        root,
        "Movies/Heat (1995).iso",
        &image(Layout::BluRay, "HEAT", &aacs),
    );
}

fn check_blu_ray(p: &DiscPlayback) {
    assert_eq!(p.kind, DiscKind::BluRay);
    assert_eq!(p.protection, None);
    let main = p.main.as_ref().unwrap();
    assert_eq!(main.id, "00001");
    assert_eq!(main.duration_ms, 5_400_000);
    assert_eq!(main.chapters_ms, [0, 1_800_000, 3_600_000, 4_500_000]);
    assert_eq!(main.angles.len(), 2);
    let audio: Vec<&str> = main
        .streams
        .iter()
        .filter(|s| s.kind == StreamKind::Audio)
        .map(|s| s.lang.as_str())
        .collect();
    assert_eq!(audio, ["eng", "rus", "ukr"]);
    let subs = main
        .streams
        .iter()
        .filter(|s| s.kind == StreamKind::Subtitle && s.codec == "PGS")
        .count();
    assert_eq!(subs, 2);
    let extras: Vec<&str> = p.extras.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(extras, ["00002"]);
}

/// Reads a file the way the stream proxy does and compares it.
fn served(opener: &SourceOpener, id: i64, rel: &str, expected: &[u8]) {
    let file = pa_proxy::Opener::open(opener, &format!("{id}/{rel}")).unwrap();
    assert_eq!(file.size(), expected.len() as u64, "{rel}");
    let mut buf = vec![0u8; expected.len()];
    file.read_exact_at(0, &mut buf).unwrap();
    assert_eq!(buf, expected, "{rel}");
}

#[test]
fn discs_play_from_folders_and_images() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    library(&root);
    let cat = Arc::new(Catalog::open(dir.path().join("library.db")).unwrap());
    let id = cat
        .add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap()
        .id;
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let movies = cat.movies().unwrap();
    let movie = |title: &str| {
        movies
            .iter()
            .find(|m| m.title == title)
            .unwrap_or_else(|| panic!("{title} in {movies:#?}"))
            .clone()
    };
    let opener = SourceOpener::new(Arc::clone(&cat));
    let clips = blu_ray(&[]);
    let clip = |name: &str| &clips.iter().find(|(p, _)| p.ends_with(name)).unwrap().1;

    // Blu-ray folder: the files play directly from the local folder.
    let arrival = movie("Arrival");
    assert_eq!(arrival.unplayable, None);
    let p = opener
        .disc_plan(arrival.source_id, &arrival.rel_path)
        .unwrap()
        .unwrap();
    check_blu_ray(&p);
    assert_eq!(p.volume_id, "Arrival (2016)");
    let angle2 = &p.main.as_ref().unwrap().angles[1];
    assert_eq!(
        angle2[1].rel_path,
        "Movies/Arrival (2016)/BDMV/STREAM/00012.m2ts"
    );
    assert!(Path::new(angle2[1].local_path.as_deref().unwrap()).is_file());

    // Blu-ray image: the same, every file inside the image, served by the
    // opener without unpacking.
    let dune = movie("Dune");
    assert_eq!(dune.rel_path, "Movies/Dune (2021).iso");
    assert_eq!(dune.unplayable, None);
    let p = opener.disc_plan(id, &dune.rel_path).unwrap().unwrap();
    check_blu_ray(&p);
    assert_eq!(p.volume_id, "DUNE");
    let main = p.main.as_ref().unwrap();
    let files: Vec<&str> = main.angles[0].iter().map(|f| f.rel_path.as_str()).collect();
    assert_eq!(
        files,
        [
            "Movies/Dune (2021).iso/BDMV/STREAM/00010.m2ts",
            "Movies/Dune (2021).iso/BDMV/STREAM/00011.m2ts",
        ]
    );
    assert!(main.angles[0].iter().all(|f| f.local_path.is_none()));
    served(&opener, id, files[0], clip("00010.m2ts"));
    served(&opener, id, &main.angles[1][1].rel_path, clip("00012.m2ts"));
    served(
        &opener,
        id,
        &p.extras[0].angles[0][0].rel_path,
        clip("00020.m2ts"),
    );

    // DVD image: the title set's VOBs, languages and chapters.
    let alien = movie("Alien");
    let p = opener.disc_plan(id, &alien.rel_path).unwrap().unwrap();
    assert_eq!(p.kind, DiscKind::Dvd);
    let main = p.main.unwrap();
    assert_eq!(main.chapters_ms, [0, 1_200_000]);
    assert_eq!(main.angles[0].len(), 2);
    let subs: Vec<(u16, &str, &str)> = main
        .streams
        .iter()
        .filter(|s| s.kind == StreamKind::Subtitle)
        .map(|s| (s.id, s.codec.as_str(), s.lang.as_str()))
        .collect();
    assert_eq!(subs, [(0x20, "VobSub", "en"), (0x21, "VobSub", "ru")]);
    served(&opener, id, &main.angles[0][1].rel_path, &dvd()[3].1);

    // Protected image: reported, never decrypted.
    let heat = movie("Heat");
    let p = opener.disc_plan(id, &heat.rel_path).unwrap().unwrap();
    assert_eq!(p.protection, Some(Protection::Aacs));

    // Not a disc; a path into an image that does not exist.
    assert!(opener.disc_plan(id, "Movies/none.mkv").unwrap().is_none());
    let err = opener
        .open_file(
            id,
            &VfsPath::parse("Movies/Dune (2021).iso/BDMV/nope").unwrap(),
        )
        .unwrap_err();
    assert_eq!(err.kind(), pa_vfs::ErrorKind::NotFound);
}
