// SPDX-License-Identifier: MPL-2.0

use std::sync::Arc;

use pa_vfs::iso::IsoFs;
use pa_vfs::iso::build::{ImageBuilder, Layout};
use pa_vfs::{ByteSource, LocalFs};

use super::fixture::ifo_bytes;
use super::*;
use crate::mpls::{Mark, PlayItem, Stream};

#[derive(Debug)]
struct Mem(Vec<u8>);

impl ByteSource for Mem {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> pa_vfs::Result<usize> {
        let start = usize::try_from(offset).unwrap().min(self.0.len());
        let n = buf.len().min(self.0.len() - start);
        buf[..n].copy_from_slice(&self.0[start..start + n]);
        Ok(n)
    }
}

const T0: u32 = 27_000_000;

fn item(clip: &str, secs: u32) -> PlayItem {
    PlayItem {
        clip_id: clip.into(),
        in_time: T0,
        out_time: T0 + secs * 45_000,
        angles: Vec::new(),
    }
}

fn pl(items: Vec<PlayItem>) -> Vec<u8> {
    Playlist {
        items,
        ..Playlist::default()
    }
    .to_bytes()
}

fn stream(kind: StreamKind, pid: u16, coding: u8, lang: &str) -> Stream {
    Stream {
        kind,
        pid,
        coding,
        lang: lang.into(),
    }
}

/// A Blu-ray with a menu loop, a film of three items (the second with two
/// angles, the third using part of its clip), an extra, a copy of the
/// extra, a part of the film and a short trailer.
fn blu_ray_files() -> Vec<(String, Vec<u8>)> {
    let mut angled = item("00011", 600);
    angled.angles = vec!["00012".into()];
    let mut partial = item("00013", 300);
    partial.in_time = T0 + 100 * 45_000;
    partial.out_time = T0 + 400 * 45_000;
    let film = Playlist {
        items: vec![item("00010", 1200), angled, partial],
        streams: vec![
            stream(StreamKind::Video, 0x1011, 0x24, ""),
            stream(StreamKind::Audio, 0x1100, 0x83, "eng"),
            stream(StreamKind::Audio, 0x1101, 0x81, "rus"),
            stream(StreamKind::Audio, 0x1102, 0x81, "ukr"),
            stream(StreamKind::Subtitle, 0x1200, 0x90, "eng"),
            stream(StreamKind::Subtitle, 0x1201, 0x90, "rus"),
        ],
        marks: vec![
            Mark { item: 0, time: T0 },
            Mark {
                item: 0,
                time: T0 + 600 * 45_000,
            },
            Mark { item: 1, time: T0 },
            Mark {
                item: 2,
                time: T0 + 100 * 45_000,
            },
        ],
    };
    let mut files = vec![
        ("BDMV/index.bdmv".to_owned(), b"INDX0200".to_vec()),
        (
            "BDMV/PLAYLIST/00000.mpls".into(),
            pl(vec![item("00099", 30)]),
        ),
        ("BDMV/PLAYLIST/00001.mpls".into(), film.to_bytes()),
        (
            "BDMV/PLAYLIST/00002.mpls".into(),
            pl(vec![item("00020", 300)]),
        ),
        (
            "BDMV/PLAYLIST/00003.mpls".into(),
            pl(vec![item("00020", 300)]),
        ),
        (
            "BDMV/PLAYLIST/00004.mpls".into(),
            pl(vec![item("00010", 1200)]),
        ),
        (
            "BDMV/PLAYLIST/00005.mpls".into(),
            pl(vec![item("00021", 60)]),
        ),
        // A playlist with a missing clip is not usable.
        (
            "BDMV/PLAYLIST/00006.mpls".into(),
            pl(vec![item("00404", 9000)]),
        ),
        (
            "BDMV/CLIPINF/00013.clpi".into(),
            clpi_bytes(T0, T0 + 900 * 45_000),
        ),
        (
            "BDMV/CLIPINF/00010.clpi".into(),
            clpi_bytes(T0, T0 + 1200 * 45_000),
        ),
    ];
    for clip in [
        "00010", "00011", "00012", "00013", "00020", "00021", "00099",
    ] {
        files.push((format!("BDMV/STREAM/{clip}.m2ts"), vec![0x47; 192 * 4]));
    }
    files
}

fn image(layout: Layout, files: &[(String, Vec<u8>)]) -> IsoFs {
    let mut b = ImageBuilder::new(layout, "TEST_DISC");
    for (p, d) in files {
        b = b.file(p, d.clone());
    }
    IsoFs::open(Arc::new(Mem(b.build()))).unwrap()
}

fn check_blu_ray(plan: &DiscPlan) {
    assert_eq!(plan.kind, DiscKind::BluRay);
    assert_eq!(plan.protection, None);
    let main = plan.main.as_ref().unwrap();
    assert_eq!(main.id, "00001");
    assert_eq!(main.duration_ms, 2_100_000);
    assert!(main.uhd);
    assert!(!main.is_3d);
    assert_eq!(main.chapters_ms, [0, 600_000, 1_200_000, 1_800_000]);
    let names = |angle: usize| -> Vec<&str> {
        main.angles[angle]
            .iter()
            .map(|s| s.path.file_name().unwrap())
            .collect()
    };
    assert_eq!(main.angles.len(), 2);
    assert_eq!(names(0), ["00010.m2ts", "00011.m2ts", "00013.m2ts"]);
    assert_eq!(names(1), ["00010.m2ts", "00012.m2ts", "00013.m2ts"]);
    assert_eq!(main.angles[0][0].start_ms, None);
    // The third item plays 300 s from 100 s into its 900 s clip.
    assert_eq!(main.angles[0][2].start_ms, Some(100_000));
    assert_eq!(main.angles[0][2].length_ms, Some(300_000));
    assert_eq!(main.angles[0][0].size, 192 * 4);
    let langs: Vec<(StreamKind, &str, &str)> = main
        .streams
        .iter()
        .map(|s| (s.kind, s.lang.as_str(), s.codec.as_str()))
        .collect();
    assert_eq!(
        langs,
        [
            (StreamKind::Video, "", "HEVC"),
            (StreamKind::Audio, "eng", "TrueHD"),
            (StreamKind::Audio, "rus", "AC-3"),
            (StreamKind::Audio, "ukr", "AC-3"),
            (StreamKind::Subtitle, "eng", "PGS"),
            (StreamKind::Subtitle, "rus", "PGS"),
        ]
    );
    assert_eq!(main.streams[1].id, 0x1100);
    let extras: Vec<&str> = plan.extras.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(extras, ["00002"]);
    assert_eq!(plan.extras[0].duration_ms, 300_000);
}

#[test]
fn blu_ray_image() {
    let fs = image(Layout::BluRay, &blu_ray_files());
    let plan = plan(&fs, &VfsPath::root()).unwrap().unwrap();
    check_blu_ray(&plan);
    assert_eq!(
        plan.main.unwrap().angles[0][1].path.as_str(),
        "/BDMV/STREAM/00011.m2ts"
    );
}

#[test]
fn blu_ray_folder() {
    let dir = tempfile::tempdir().unwrap();
    for (p, d) in blu_ray_files() {
        let path = dir.path().join("Film (2020)").join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, d).unwrap();
    }
    let fs = LocalFs::new(dir.path()).unwrap();
    let root = VfsPath::parse("/Film (2020)").unwrap();
    let plan = plan(&fs, &root).unwrap().unwrap();
    check_blu_ray(&plan);
    assert_eq!(
        plan.main.unwrap().angles[1][1].path.as_str(),
        "/Film (2020)/BDMV/STREAM/00012.m2ts"
    );
    assert!(super::plan(&fs, &VfsPath::root()).unwrap().is_none());
}

#[test]
fn protected_blu_ray() {
    let mut files = blu_ray_files();
    files.push(("AACS/Unit_Key_RO.inf".into(), vec![1; 16]));
    let fs = image(Layout::BluRay, &files);
    let plan = plan(&fs, &VfsPath::root()).unwrap().unwrap();
    assert_eq!(plan.protection, Some(Protection::Aacs));
    assert_eq!(plan.protection.unwrap().code(), "aacs");
}

/// MPEG-2 pack + PES packet with the scrambling bits `scrambling`.
fn vob(packs: usize, scrambling: u8) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..packs {
        let mut p = vec![0u8; 2048];
        p[..4].copy_from_slice(&[0, 0, 1, 0xBA]);
        p[4] = 0x44;
        p[13] = 0xF8;
        p[14..18].copy_from_slice(&[0, 0, 1, 0xE0]);
        p[18..20].copy_from_slice(&2000u16.to_be_bytes());
        p[20] = 0x80 | (scrambling << 4);
        out.extend_from_slice(&p);
    }
    out
}

fn dvd_files(scrambling: u8) -> Vec<(String, Vec<u8>)> {
    vec![
        ("VIDEO_TS/VIDEO_TS.IFO".into(), b"DVDVIDEO-VMG".to_vec()),
        ("VIDEO_TS/VIDEO_TS.VOB".into(), vob(8, 0)),
        (
            "VIDEO_TS/VTS_01_0.IFO".into(),
            ifo_bytes(
                &["en", "ru"],
                &["en", "ru", "uk"],
                &[600_000, 900_000, 300_000],
            ),
        ),
        ("VIDEO_TS/VTS_01_0.VOB".into(), vob(4, 0)),
        ("VIDEO_TS/VTS_01_1.VOB".into(), vob(20, scrambling)),
        ("VIDEO_TS/VTS_01_2.VOB".into(), vob(10, 0)),
        ("VIDEO_TS/VTS_02_1.VOB".into(), vob(12, 0)),
        ("AUDIO_TS/.keep".into(), Vec::new()),
    ]
}

#[test]
fn dvd_image() {
    let fs = image(Layout::Dvd, &dvd_files(0));
    let plan = plan(&fs, &VfsPath::root()).unwrap().unwrap();
    assert_eq!(plan.kind, DiscKind::Dvd);
    assert_eq!(plan.protection, None);
    let main = plan.main.unwrap();
    assert_eq!(main.id, "VTS_01");
    let parts: Vec<&str> = main.angles[0].iter().map(|s| s.path.as_str()).collect();
    assert_eq!(parts, ["/VIDEO_TS/VTS_01_1.VOB", "/VIDEO_TS/VTS_01_2.VOB"]);
    assert_eq!(main.duration_ms, 1_800_000);
    assert_eq!(main.chapters_ms, [0, 600_000, 1_500_000]);
    let streams: Vec<(StreamKind, u16, &str)> = main
        .streams
        .iter()
        .map(|s| (s.kind, s.id, s.lang.as_str()))
        .collect();
    assert_eq!(
        streams,
        [
            (StreamKind::Audio, 0x80, "en"),
            (StreamKind::Audio, 0x81, "ru"),
            (StreamKind::Subtitle, 0x20, "en"),
            (StreamKind::Subtitle, 0x21, "ru"),
            (StreamKind::Subtitle, 0x22, "uk"),
        ]
    );
    assert!(plan.extras.is_empty());
}

#[test]
fn css_dvd() {
    let fs = image(Layout::Dvd, &dvd_files(1));
    let plan = plan(&fs, &VfsPath::root()).unwrap().unwrap();
    assert_eq!(plan.protection, Some(Protection::Css));
}

#[test]
fn broken_navigation_files() {
    assert_eq!(parse_ifo(b"DVDVIDEO-VTS"), None);
    assert_eq!(parse_ifo(&[0; 4096]), None);
    assert_eq!(parse_clpi(b"HDMV"), None);
    assert_eq!(parse_clpi(&clpi_bytes(5, 9)), Some((5, 9)));
    let mut ifo = ifo_bytes(&["en"], &[], &[1000]);
    ifo.truncate(2100);
    let info = parse_ifo(&ifo).unwrap();
    assert_eq!(info.streams.len(), 1);
    assert!(info.chapters_ms.is_empty());
}
