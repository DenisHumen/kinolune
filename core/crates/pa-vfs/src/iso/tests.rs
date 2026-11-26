// SPDX-License-Identifier: MPL-2.0

use std::sync::Arc;

use super::build::{ImageBuilder, Layout};
use super::*;

#[derive(Debug)]
struct Mem(Vec<u8>);

impl ByteSource for Mem {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let start = usize::try_from(offset).unwrap().min(self.0.len());
        let n = buf.len().min(self.0.len() - start);
        buf[..n].copy_from_slice(&self.0[start..start + n]);
        Ok(n)
    }
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from((i * 31 + usize::from(seed)) % 251).unwrap())
        .collect()
}

fn sample(layout: Layout) -> (Vec<u8>, Vec<(&'static str, Vec<u8>)>) {
    let files = vec![
        ("BDMV/index.bdmv", b"INDX0200".to_vec()),
        ("BDMV/PLAYLIST/00800.mpls", pattern(300, 1)),
        ("BDMV/STREAM/00001.m2ts", pattern(3 * 2048 + 77, 2)),
        ("BDMV/STREAM/00002.m2ts", pattern(2048, 3)),
        ("BDMV/STREAM/empty.m2ts", Vec::new()),
        ("CERTIFICATE/id.bdmv", pattern(10, 4)),
        ("Фильм.txt", "привет".as_bytes().to_vec()),
    ];
    let mut b = ImageBuilder::new(layout, "MY_MOVIE");
    for (p, d) in &files {
        b = b.file(p, d.clone());
    }
    (b.build(), files)
}

fn open(img: Vec<u8>) -> IsoFs {
    IsoFs::open(Arc::new(Mem(img))).unwrap()
}

fn read_all(fs: &IsoFs, path: &str) -> Vec<u8> {
    let f = fs.open(&VfsPath::parse(path).unwrap()).unwrap();
    let mut out = vec![0u8; usize::try_from(f.size()).unwrap()];
    // Odd chunk sizes cross piece and sector boundaries.
    let mut at = 0;
    while at < out.len() {
        let end = (at + 1000).min(out.len());
        let n = f.read_at(at as u64, &mut out[at..end]).unwrap();
        assert!(n > 0);
        at += n;
    }
    assert_eq!(f.read_at(out.len() as u64, &mut [0; 4]).unwrap(), 0);
    out
}

fn check(fs: &IsoFs, files: &[(&str, Vec<u8>)]) {
    for (path, data) in files {
        assert_eq!(&read_all(fs, path), data, "{path}");
        let meta = fs.stat(&VfsPath::parse(path).unwrap()).unwrap();
        assert!(meta.is_file());
        assert_eq!(meta.size, data.len() as u64);
    }
    let root: Vec<String> = fs
        .list(&VfsPath::root())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(root.contains(&"BDMV".to_owned()), "{root:?}");
    assert!(root.contains(&"Фильм.txt".to_owned()), "{root:?}");
    let stream = fs.list(&VfsPath::parse("/BDMV/STREAM").unwrap()).unwrap();
    assert_eq!(stream.len(), 3);
    assert!(stream.iter().all(|e| e.metadata.is_file()));
    assert_eq!(stream[0].path.as_str(), "/BDMV/STREAM/00001.m2ts");
    assert!(fs.stat(&VfsPath::parse("/BDMV").unwrap()).unwrap().is_dir());
    let err = fs.open(&VfsPath::parse("/BDMV").unwrap()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::IsADirectory);
    let err = fs.stat(&VfsPath::parse("/BDMV/nope").unwrap()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    let err = fs
        .list(&VfsPath::parse("/BDMV/index.bdmv").unwrap())
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotADirectory);
}

#[test]
fn bluray_udf_250_with_metadata_partition() {
    let (img, files) = sample(Layout::BluRay);
    let fs = open(img);
    assert_eq!(fs.format(), IsoFormat::Udf { revision: 0x0250 });
    assert_eq!(fs.volume_id(), "MY_MOVIE");
    check(&fs, &files);
}

#[test]
fn dvd_udf_102_bridge() {
    let (img, files) = sample(Layout::Dvd);
    let fs = open(img.clone());
    assert_eq!(fs.format(), IsoFormat::Udf { revision: 0x0102 });
    check(&fs, &files);
    // The ISO 9660 side of the bridge lists the same files.
    let (joliet, volume, tree) = iso9660::read(&Mem(img)).unwrap().unwrap();
    assert!(!joliet);
    assert_eq!(volume, "MY_MOVIE");
    assert_eq!(tree.nodes.len(), 5 + 7);
}

#[test]
fn plain_iso9660() {
    let (img, files) = sample(Layout::Iso9660);
    let fs = open(img);
    assert_eq!(fs.format(), IsoFormat::Iso9660 { joliet: false });
    check(&fs, &files);
}

#[test]
fn names_match_ignoring_case() {
    let fs = open(sample(Layout::BluRay).0);
    let a = read_all(&fs, "/bdmv/stream/00001.M2TS");
    assert_eq!(a, read_all(&fs, "/BDMV/STREAM/00001.m2ts"));
}

#[test]
fn not_an_image() {
    let err = IsoFs::open(Arc::new(Mem(vec![0u8; 100 * 2048]))).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    let err = IsoFs::open(Arc::new(Mem(b"hello".to_vec()))).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}

#[test]
fn broken_images_fail_without_panics() {
    for layout in [Layout::BluRay, Layout::Dvd, Layout::Iso9660] {
        let (img, _) = sample(layout);
        // Cut short anywhere.
        for len in (0..img.len()).step_by(4096 + 13) {
            let _ = IsoFs::open(Arc::new(Mem(img[..len].to_vec())));
        }
        // Flip bytes of the descriptors, entries and directories.
        let sectors = img.len() / 2048;
        for s in 16..sectors.min(300) {
            for off in [
                0usize, 4, 12, 20, 25, 27, 34, 56, 168, 172, 208, 212, 252, 268, 404, 440,
            ] {
                let mut bad = img.clone();
                let at = s * 2048 + off;
                bad[at] ^= 0xA5;
                bad[at + 1] = bad[at + 1].wrapping_add(0x40);
                if let Ok(fs) = IsoFs::open(Arc::new(Mem(bad))) {
                    walk(&fs, &VfsPath::root());
                }
            }
        }
    }
}

fn walk(fs: &IsoFs, dir: &VfsPath) {
    let Ok(entries) = fs.list(dir) else { return };
    for e in entries {
        if e.metadata.is_dir() {
            walk(fs, &e.path);
        } else if let Ok(f) = fs.open(&e.path) {
            let mut buf = vec![0u8; 4096];
            let _ = f.read_at(0, &mut buf);
            let _ = f.read_at(f.size().saturating_sub(10), &mut buf);
        }
    }
}

#[test]
fn image_paths() {
    let p = |s: &str| VfsPath::parse(s).unwrap();
    let (outer, inner) = split_image_path(&p("/m/Film.ISO/BDMV/index.bdmv")).unwrap();
    assert_eq!(outer.as_str(), "/m/Film.ISO");
    assert_eq!(inner.as_str(), "/BDMV/index.bdmv");
    assert!(split_image_path(&p("/m/Film.iso")).is_none());
    assert!(split_image_path(&p("/m/Film.mkv")).is_none());
    assert!(split_image_path(&p("/m/.iso/a")).is_none());
    assert!(is_image_name("a.iso") && !is_image_name("iso") && !is_image_name("a.isox"));
}

/// Cross-check with other tools: `PA_ISO_DUMP=<dir>` writes the sample
/// images there (to open with `7z l`, `udfinfo`); `PA_ISO_EXTERNAL=<dir>`
/// opens every `.iso` made by other tools (genisoimage, mkudffs) there.
#[test]
fn other_tools() {
    if let Some(dir) = std::env::var_os("PA_ISO_DUMP") {
        let dir = std::path::PathBuf::from(dir);
        for (name, layout) in [
            ("bluray.iso", Layout::BluRay),
            ("dvd.iso", Layout::Dvd),
            ("plain.iso", Layout::Iso9660),
        ] {
            std::fs::write(dir.join(name), sample(layout).0).unwrap();
        }
    }
    if let Some(dir) = std::env::var_os("PA_ISO_EXTERNAL") {
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            if path.extension().is_some_and(|x| x == "iso") {
                let fs = open(std::fs::read(&path).unwrap());
                let mut names = Vec::new();
                list_all(&fs, &VfsPath::root(), &mut names);
                println!(
                    "{}: {:?} {:?} {names:?}",
                    path.display(),
                    fs.format(),
                    fs.volume_id()
                );
            }
        }
    }
}

fn list_all(fs: &IsoFs, dir: &VfsPath, out: &mut Vec<String>) {
    for e in fs.list(dir).unwrap() {
        if e.metadata.is_dir() {
            list_all(fs, &e.path, out);
        } else {
            let data = read_all(fs, e.path.as_str());
            out.push(format!("{}={}", e.path.as_str(), data.len()));
        }
    }
}
