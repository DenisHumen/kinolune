// SPDX-License-Identifier: MPL-2.0
//! Writes small disc images for tests and tools (PLAY-017).
//!
//! [`ImageBuilder`] lays out a UDF 1.02 image with an ISO 9660 bridge (as
//! on a DVD) or a UDF 2.50 image with a metadata partition (as on a
//! Blu-ray), or plain ISO 9660. Only what a reader needs is filled in;
//! descriptor tags carry valid checksums and CRCs, so other readers open
//! the images too. Files are held in memory: keep them small.

use std::collections::BTreeMap;

use super::SECTOR;

const S: usize = SECTOR as usize;

/// File system(s) of a built image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// UDF 1.02 and ISO 9660 over the same files (a DVD-Video disc).
    Dvd,
    /// UDF 2.50 with a metadata partition (a Blu-ray disc).
    BluRay,
    /// ISO 9660 only.
    Iso9660,
}

/// Collects files and writes an image with [`ImageBuilder::build`].
#[derive(Debug, Clone)]
pub struct ImageBuilder {
    layout: Layout,
    volume_id: String,
    files: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Default)]
struct Dir {
    dirs: BTreeMap<String, Dir>,
    files: BTreeMap<String, usize>,
}

/// Where each entry went.
#[derive(Debug, Default, Clone, Copy)]
struct Place {
    /// Block of the file entry (in the metadata space).
    fe: u32,
    /// First block and byte length of the data (directory data in the
    /// metadata space, file data in the physical partition).
    data: u32,
    len: u64,
}

impl ImageBuilder {
    /// An empty image of `layout` named `volume_id`.
    #[must_use]
    pub fn new(layout: Layout, volume_id: &str) -> Self {
        Self {
            layout,
            volume_id: volume_id.to_owned(),
            files: BTreeMap::new(),
        }
    }

    /// Adds a file at `path` (`BDMV/STREAM/00001.m2ts`); parent folders
    /// are created.
    #[must_use]
    pub fn file(mut self, path: &str, data: impl Into<Vec<u8>>) -> Self {
        self.files
            .insert(path.trim_matches('/').to_owned(), data.into());
        self
    }

    /// The image.
    ///
    /// # Panics
    ///
    /// If a file is larger than 4 GiB or the image would exceed 2^32
    /// sectors (not for test images).
    #[must_use]
    pub fn build(&self) -> Vec<u8> {
        let contents: Vec<&Vec<u8>> = self.files.values().collect();
        let mut root = Dir::default();
        for (i, path) in self.files.keys().enumerate() {
            let mut dir = &mut root;
            let mut parts: Vec<&str> = path.split('/').collect();
            let name = parts.pop().expect("a name");
            for p in parts {
                dir = dir.dirs.entry(p.to_owned()).or_default();
            }
            dir.files.insert(name.to_owned(), i);
        }
        let udf = self.layout != Layout::Iso9660;
        let v250 = self.layout == Layout::BluRay;

        // Metadata space: FSD, then per directory its entry and data, per
        // file its entry. Directory data size depends only on the names.
        let mut meta_blocks: u32 = 1;
        let mut dir_places: Vec<Place> = Vec::new();
        let mut file_places: Vec<Place> = vec![Place::default(); contents.len()];
        fn plan(
            d: &Dir,
            meta: &mut u32,
            dirs: &mut Vec<Place>,
            files: &mut [Place],
            contents: &[&Vec<u8>],
        ) {
            let len = fids_len(d) as u64;
            dirs.push(Place {
                fe: *meta,
                data: *meta + 1,
                len,
            });
            *meta += 1 + u32::try_from(len.div_ceil(SECTOR)).expect("small");
            for i in d.files.values() {
                files[*i] = Place {
                    fe: *meta,
                    data: 0,
                    len: contents[*i].len() as u64,
                };
                *meta += 1;
            }
            for sub in d.dirs.values() {
                plan(sub, meta, dirs, files, contents);
            }
        }
        if udf {
            plan(
                &root,
                &mut meta_blocks,
                &mut dir_places,
                &mut file_places,
                &contents,
            );
        }
        // Physical partition: for UDF 2.50 the metadata file entry and its
        // data come first; file data follows.
        let meta_base: u32 = if v250 { 1 } else { 0 };
        let mut phys = if udf { meta_base + meta_blocks } else { 0 };
        for (i, p) in file_places.iter_mut().enumerate() {
            p.data = phys;
            p.len = contents[i].len() as u64;
            phys += u32::try_from(p.len.div_ceil(SECTOR)).expect("small");
        }
        let part_start: u32 = 257;
        let part_len = phys;
        // ISO 9660 directories after the partition.
        let mut iso_dirs: Vec<(u32, u32)> = Vec::new();
        let mut next = part_start + part_len;
        let iso = self.layout != Layout::BluRay;
        if iso {
            fn plan_iso(d: &Dir, next: &mut u32, out: &mut Vec<(u32, u32)>) {
                let n = u32::try_from(iso_dir_len(d).div_ceil(S)).expect("small");
                out.push((*next, n));
                *next += n;
                for sub in d.dirs.values() {
                    plan_iso(sub, next, out);
                }
            }
            plan_iso(&root, &mut next, &mut iso_dirs);
        }
        let total = next + 1;
        let mut img = vec![0u8; total as usize * S];
        let sec = |lba: u32| -> std::ops::Range<usize> {
            let a = lba as usize * S;
            a..a + S
        };

        // Volume recognition.
        let mut vrs = 16;
        if iso {
            let r = sec(16);
            self.iso_pvd(&mut img[r], total, iso_dirs[0]);
            let r = sec(17);
            img[r.start..r.start + 7].copy_from_slice(b"\xFFCD001\x01");
            vrs = 18;
        }
        if udf {
            for (i, id) in [
                &b"BEA01"[..],
                if v250 { b"NSR03" } else { b"NSR02" },
                b"TEA01",
            ]
            .iter()
            .enumerate()
            {
                let r = sec(vrs + i as u32);
                img[r.start + 1..r.start + 6].copy_from_slice(id);
                img[r.start + 6] = 1;
            }
            self.udf_vds(&mut img, part_start, part_len, v250);
            // Partition contents.
            let phys_at = |lbn: u32| (part_start + lbn) as usize * S;
            let meta_at = |lbn: u32| phys_at(meta_base + lbn);
            let meta_part: u16 = if v250 { 1 } else { 0 };
            if v250 {
                // Metadata file: one extent over the metadata space.
                let fe = file_entry(
                    true,
                    0,
                    250,
                    u64::from(meta_blocks) * SECTOR,
                    &short_ad(u64::from(meta_blocks) * SECTOR, meta_base),
                    0,
                );
                img[phys_at(0)..phys_at(0) + fe.len()].copy_from_slice(&fe);
            }
            // File Set Descriptor.
            let fsd = fsd(&self.volume_id, dir_places[0].fe, meta_part);
            img[meta_at(0)..meta_at(0) + fsd.len()].copy_from_slice(&fsd);
            // Directories and files.
            let mut di = 0;
            fn write(
                d: &Dir,
                parent: usize,
                di: &mut usize,
                dirs: &[Place],
                files: &[Place],
                ctx: &mut Ctx<'_>,
            ) {
                let me = *di;
                *di += 1;
                let place = dirs[me];
                let fids = fids(
                    d,
                    dirs[parent].fe,
                    place.data,
                    me,
                    dirs,
                    files,
                    ctx.meta_part,
                );
                let ads = if ctx.v250 {
                    long_ad(place.len, place.data, ctx.meta_part)
                } else {
                    short_ad(place.len, place.data)
                };
                let fe = file_entry(
                    ctx.v250,
                    place.fe,
                    4,
                    place.len,
                    &ads,
                    if ctx.v250 { 1 } else { 0 },
                );
                let at = (ctx.meta_at)(place.fe);
                ctx.img[at..at + fe.len()].copy_from_slice(&fe);
                let at = (ctx.meta_at)(place.data);
                ctx.img[at..at + fids.len()].copy_from_slice(&fids);
                for i in d.files.values() {
                    let p = files[*i];
                    let ads = if p.len == 0 {
                        Vec::new()
                    } else if ctx.v250 {
                        long_ad(p.len, p.data, 0)
                    } else {
                        short_ad(p.len, p.data)
                    };
                    let fe =
                        file_entry(ctx.v250, p.fe, 5, p.len, &ads, if ctx.v250 { 1 } else { 0 });
                    let at = (ctx.meta_at)(p.fe);
                    ctx.img[at..at + fe.len()].copy_from_slice(&fe);
                }
                // Sub-directories are numbered in the order `plan` used:
                // after this directory, depth first.
                for sub in d.dirs.values() {
                    write(sub, me, di, dirs, files, ctx);
                }
            }
            let mut ctx = Ctx {
                img: &mut img,
                meta_at: &meta_at,
                v250,
                meta_part,
            };
            write(&root, 0, &mut di, &dir_places, &file_places, &mut ctx);
        }
        // File data.
        for (i, p) in file_places.iter().enumerate() {
            let at = (part_start + p.data) as usize * S;
            img[at..at + contents[i].len()].copy_from_slice(contents[i]);
        }
        if iso {
            let mut k = 0;
            write_iso_dirs(
                &root,
                &mut img,
                &iso_dirs,
                &mut k,
                0,
                &file_places,
                part_start,
            );
        }
        // Closing anchor, as on real media.
        if udf {
            let last = total - 1;
            let a = avdp(last);
            img[last as usize * S..last as usize * S + a.len()].copy_from_slice(&a);
        }
        img
    }

    fn iso_pvd(&self, b: &mut [u8], total: u32, root: (u32, u32)) {
        b[..7].copy_from_slice(b"\x01CD001\x01");
        b[8..72].fill(b' ');
        let v = self.volume_id.as_bytes();
        let n = v.len().min(32);
        b[40..40 + n].copy_from_slice(&v[..n]);
        both32(&mut b[80..88], total);
        both16(&mut b[120..124], 1);
        both16(&mut b[124..128], 1);
        both16(&mut b[128..132], 2048);
        let rec = iso_record(&[0], root.0, root.1 * 2048, true);
        b[156..156 + rec.len()].copy_from_slice(&rec);
        b[881] = 1;
    }

    fn udf_vds(&self, img: &mut [u8], part_start: u32, part_len: u32, v250: bool) {
        let rev: u16 = if v250 { 0x0250 } else { 0x0102 };
        let at = |lba: u32| lba as usize * S;
        // Primary Volume Descriptor.
        let mut pvd = vec![0u8; 512];
        put_dstring(&mut pvd[24..56], &self.volume_id);
        finish_tag(&mut pvd, 1, 32);
        img[at(32)..at(32) + 512].copy_from_slice(&pvd);
        // Partition Descriptor.
        let mut pd = vec![0u8; 512];
        pd[20..22].copy_from_slice(&1u16.to_le_bytes());
        pd[22..24].copy_from_slice(&0u16.to_le_bytes());
        regid(&mut pd[24..56], if v250 { b"+NSR03" } else { b"+NSR02" }, 0);
        pd[184..188].copy_from_slice(&1u32.to_le_bytes());
        pd[188..192].copy_from_slice(&part_start.to_le_bytes());
        pd[192..196].copy_from_slice(&part_len.to_le_bytes());
        finish_tag(&mut pd, 5, 33);
        img[at(33)..at(33) + 512].copy_from_slice(&pd);
        // Logical Volume Descriptor.
        let mut lvd = vec![0u8; 512];
        lvd[20] = 0; // CS0 character set
        put_dstring(&mut lvd[84..212], &self.volume_id);
        lvd[212..216].copy_from_slice(&2048u32.to_le_bytes());
        regid(&mut lvd[216..248], b"*OSTA UDF Compliant", rev);
        // FSD long_ad: one block at 0 of the metadata (or only) partition.
        lvd[248..252].copy_from_slice(&2048u32.to_le_bytes());
        lvd[256..258].copy_from_slice(&u16::from(v250).to_le_bytes());
        let mut maps = vec![1u8, 6, 1, 0, 0, 0];
        if v250 {
            let mut m = vec![0u8; 64];
            m[0] = 2;
            m[1] = 64;
            regid(&mut m[4..36], b"*UDF Metadata Partition", rev);
            m[36..38].copy_from_slice(&1u16.to_le_bytes());
            m[38..40].copy_from_slice(&0u16.to_le_bytes());
            m[40..44].copy_from_slice(&0u32.to_le_bytes()); // metadata file
            m[44..48].copy_from_slice(&u32::MAX.to_le_bytes()); // no mirror
            m[48..52].copy_from_slice(&u32::MAX.to_le_bytes()); // no bitmap
            m[52..56].copy_from_slice(&32u32.to_le_bytes());
            m[56..58].copy_from_slice(&1u16.to_le_bytes());
            maps.extend_from_slice(&m);
        }
        let n_maps: u32 = if v250 { 2 } else { 1 };
        lvd[264..268].copy_from_slice(&u32::try_from(maps.len()).expect("small").to_le_bytes());
        lvd[268..272].copy_from_slice(&n_maps.to_le_bytes());
        lvd[440..440 + maps.len()].copy_from_slice(&maps);
        lvd.truncate(440 + maps.len());
        finish_tag(&mut lvd, 6, 34);
        img[at(34)..at(34) + lvd.len()].copy_from_slice(&lvd);
        // Terminating Descriptor.
        let mut td = vec![0u8; 512];
        finish_tag(&mut td, 8, 35);
        img[at(35)..at(35) + 512].copy_from_slice(&td);
        let a = avdp(256);
        img[at(256)..at(256) + a.len()].copy_from_slice(&a);
    }
}

struct Ctx<'a> {
    img: &'a mut Vec<u8>,
    meta_at: &'a dyn Fn(u32) -> usize,
    v250: bool,
    meta_part: u16,
}

fn both16(b: &mut [u8], v: u16) {
    b[..2].copy_from_slice(&v.to_le_bytes());
    b[2..4].copy_from_slice(&v.to_be_bytes());
}

fn both32(b: &mut [u8], v: u32) {
    b[..4].copy_from_slice(&v.to_le_bytes());
    b[4..8].copy_from_slice(&v.to_be_bytes());
}

fn iso_record(name: &[u8], extent: u32, len: u32, dir: bool) -> Vec<u8> {
    let mut r = vec![0u8; 33 + name.len() + usize::from(name.len().is_multiple_of(2))];
    r[0] = u8::try_from(r.len()).expect("short name");
    both32(&mut r[2..10], extent);
    both32(&mut r[10..18], len);
    r[25] = if dir { 2 } else { 0 };
    both16(&mut r[28..32], 1);
    r[32] = u8::try_from(name.len()).expect("short name");
    r[33..33 + name.len()].copy_from_slice(name);
    r
}

fn iso_dir_records(d: &Dir) -> Vec<(Vec<u8>, bool)> {
    let mut out = Vec::new();
    let mut names: Vec<(String, bool)> = d
        .dirs
        .keys()
        .map(|n| (n.clone(), true))
        .chain(d.files.keys().map(|n| (format!("{n};1"), false)))
        .collect();
    names.sort();
    for (n, is_dir) in names {
        out.push((n.into_bytes(), is_dir));
    }
    out
}

fn iso_dir_len(d: &Dir) -> usize {
    let mut at = 0;
    let lens = [34usize, 34].into_iter().chain(
        iso_dir_records(d)
            .into_iter()
            .map(|(n, _)| 33 + n.len() + usize::from(n.len().is_multiple_of(2))),
    );
    for l in lens {
        if at % S + l > S {
            at = at.div_ceil(S) * S;
        }
        at += l;
    }
    at.max(1)
}

fn write_iso_dirs(
    d: &Dir,
    img: &mut [u8],
    places: &[(u32, u32)],
    k: &mut usize,
    parent: usize,
    files: &[Place],
    part_start: u32,
) {
    let me = *k;
    *k += 1;
    let (extent, n) = places[me];
    let mut recs = vec![
        iso_record(&[0], extent, n * 2048, true),
        iso_record(&[1], places[parent].0, places[parent].1 * 2048, true),
    ];
    // Sub-directory numbers follow `plan_iso`: depth first after this one.
    let mut sub_index = Vec::new();
    let mut next = *k;
    for sub in d.dirs.values() {
        sub_index.push(next);
        next += count_dirs(sub);
    }
    for (name, is_dir) in iso_dir_records(d) {
        let text = String::from_utf8(name.clone()).expect("utf-8");
        if is_dir {
            let pos = d.dirs.keys().position(|k| *k == text).expect("dir");
            let (e, l) = places[sub_index[pos]];
            recs.push(iso_record(&name, e, l * 2048, true));
        } else {
            let i = d.files[text.trim_end_matches(";1")];
            let p = files[i];
            let e = if p.len == 0 { 0 } else { part_start + p.data };
            recs.push(iso_record(
                &name,
                e,
                u32::try_from(p.len).expect("< 4 GiB"),
                false,
            ));
        }
    }
    let base = extent as usize * S;
    let mut at = 0;
    for r in recs {
        if at % S + r.len() > S {
            at = at.div_ceil(S) * S;
        }
        img[base + at..base + at + r.len()].copy_from_slice(&r);
        at += r.len();
    }
    for sub in d.dirs.values() {
        write_iso_dirs(sub, img, places, k, me, files, part_start);
    }
}

fn count_dirs(d: &Dir) -> usize {
    1 + d.dirs.values().map(count_dirs).sum::<usize>()
}

/// CRC-ITU-T (polynomial 0x1021, initial 0) of descriptor bodies.
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Fills the 16-byte tag of descriptor `d` (id, version, CRC, location).
fn finish_tag(d: &mut [u8], id: u16, location: u32) {
    d[0..2].copy_from_slice(&id.to_le_bytes());
    d[2..4].copy_from_slice(&3u16.to_le_bytes());
    let crc = crc16(&d[16..]);
    let len = u16::try_from(d.len() - 16).expect("small");
    d[8..10].copy_from_slice(&crc.to_le_bytes());
    d[10..12].copy_from_slice(&len.to_le_bytes());
    d[12..16].copy_from_slice(&location.to_le_bytes());
    d[4] = 0;
    let sum = d[..16].iter().fold(0u8, |a, x| a.wrapping_add(*x));
    d[4] = sum;
}

fn regid(b: &mut [u8], ident: &[u8], suffix: u16) {
    b[1..1 + ident.len()].copy_from_slice(ident);
    b[24..26].copy_from_slice(&suffix.to_le_bytes());
}

fn cs0(s: &str) -> Vec<u8> {
    if s.chars().all(|c| u32::from(c) < 256) {
        let mut v = vec![8u8];
        v.extend(
            s.chars()
                .map(|c| u8::try_from(u32::from(c)).expect("latin-1")),
        );
        v
    } else {
        let mut v = vec![16u8];
        for u in s.encode_utf16() {
            v.extend_from_slice(&u.to_be_bytes());
        }
        v
    }
}

fn put_dstring(b: &mut [u8], s: &str) {
    let mut v = cs0(s);
    v.truncate(b.len() - 1);
    b[..v.len()].copy_from_slice(&v);
    let last = b.len() - 1;
    b[last] = u8::try_from(v.len()).expect("short");
}

fn avdp(location: u32) -> Vec<u8> {
    let mut a = vec![0u8; 512];
    a[16..20].copy_from_slice(&(4 * 2048u32).to_le_bytes());
    a[20..24].copy_from_slice(&32u32.to_le_bytes());
    a[24..28].copy_from_slice(&(4 * 2048u32).to_le_bytes());
    a[28..32].copy_from_slice(&32u32.to_le_bytes());
    finish_tag(&mut a, 2, location);
    a
}

fn fsd(volume_id: &str, root_fe: u32, part: u16) -> Vec<u8> {
    let mut f = vec![0u8; 512];
    f[28..30].copy_from_slice(&3u16.to_le_bytes());
    f[30..32].copy_from_slice(&3u16.to_le_bytes());
    f[32..36].copy_from_slice(&1u32.to_le_bytes());
    f[36..40].copy_from_slice(&1u32.to_le_bytes());
    put_dstring(&mut f[112..240], volume_id);
    put_dstring(&mut f[304..336], volume_id);
    f[400..404].copy_from_slice(&2048u32.to_le_bytes());
    f[404..408].copy_from_slice(&root_fe.to_le_bytes());
    f[408..410].copy_from_slice(&part.to_le_bytes());
    regid(&mut f[416..448], b"*OSTA UDF Compliant", 0x0102);
    finish_tag(&mut f, 256, 0);
    f
}

fn short_ad(len: u64, lbn: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut left = len;
    let mut at = lbn;
    // At most 2^30 - 2048 bytes per descriptor.
    let max = (1u64 << 30) - SECTOR;
    while left > 0 {
        let n = left.min(max);
        out.extend_from_slice(&u32::try_from(n).expect("fits").to_le_bytes());
        out.extend_from_slice(&at.to_le_bytes());
        left -= n;
        at += u32::try_from(n / SECTOR).expect("fits");
    }
    out
}

fn long_ad(len: u64, lbn: u32, part: u16) -> Vec<u8> {
    short_ad(len, lbn)
        .as_chunks::<8>()
        .0
        .iter()
        .flat_map(|c| {
            let mut v = c.to_vec();
            v.extend_from_slice(&part.to_le_bytes());
            v.extend_from_slice(&[0; 6]);
            v
        })
        .collect()
}

/// A File Entry (UDF 1.02) or an Extended File Entry (2.50) with
/// allocation descriptors `ads` (short if `ad_type` is 0, long if 1).
fn file_entry(
    extended: bool,
    location: u32,
    file_type: u8,
    size: u64,
    ads: &[u8],
    ad_type: u16,
) -> Vec<u8> {
    let (len_at, ad_at) = if extended { (208, 216) } else { (168, 176) };
    let mut e = vec![0u8; ad_at + ads.len()];
    // ICB tag: strategy 4, one entry.
    e[20..22].copy_from_slice(&4u16.to_le_bytes());
    e[24..26].copy_from_slice(&1u16.to_le_bytes());
    e[27] = file_type;
    e[34..36].copy_from_slice(&ad_type.to_le_bytes());
    e[36..40].copy_from_slice(&u32::MAX.to_le_bytes());
    e[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    e[44..48].copy_from_slice(&0x14A5u32.to_le_bytes());
    e[48..50].copy_from_slice(&1u16.to_le_bytes());
    e[56..64].copy_from_slice(&size.to_le_bytes());
    let blocks = size.div_ceil(SECTOR);
    if extended {
        e[64..72].copy_from_slice(&size.to_le_bytes());
        e[72..80].copy_from_slice(&blocks.to_le_bytes());
        e[128..132].copy_from_slice(&1u32.to_le_bytes());
    } else {
        e[64..72].copy_from_slice(&blocks.to_le_bytes());
        e[108..112].copy_from_slice(&1u32.to_le_bytes());
    }
    e[len_at + 4..len_at + 8]
        .copy_from_slice(&u32::try_from(ads.len()).expect("small").to_le_bytes());
    e[ad_at..].copy_from_slice(ads);
    finish_tag(&mut e, if extended { 266 } else { 261 }, location);
    e
}

fn fid(name: &str, chars: u8, icb: u32, part: u16, location: u32) -> Vec<u8> {
    let fi = if name.is_empty() {
        Vec::new()
    } else {
        cs0(name)
    };
    let len = (38 + fi.len() + 3) & !3;
    let mut f = vec![0u8; len];
    f[16..18].copy_from_slice(&1u16.to_le_bytes());
    f[18] = chars;
    f[19] = u8::try_from(fi.len()).expect("short name");
    f[20..24].copy_from_slice(&2048u32.to_le_bytes());
    f[24..28].copy_from_slice(&icb.to_le_bytes());
    f[28..30].copy_from_slice(&part.to_le_bytes());
    f[38..38 + fi.len()].copy_from_slice(&fi);
    finish_tag(&mut f, 257, location);
    f
}

fn fids_len(d: &Dir) -> usize {
    let one = |n: &str| (38 + if n.is_empty() { 0 } else { cs0(n).len() } + 3) & !3;
    one("")
        + d.dirs
            .keys()
            .chain(d.files.keys())
            .map(|n| one(n))
            .sum::<usize>()
}

#[allow(clippy::too_many_arguments)]
fn fids(
    d: &Dir,
    parent_fe: u32,
    location: u32,
    me: usize,
    dirs: &[Place],
    files: &[Place],
    part: u16,
) -> Vec<u8> {
    let mut out = fid("", 0x0A, parent_fe, part, location);
    let mut next = me + 1;
    for (name, sub) in &d.dirs {
        out.extend(fid(name, 0x02, dirs[next].fe, part, location));
        next += count_dirs(sub);
    }
    for (name, i) in &d.files {
        out.extend(fid(name, 0, files[*i].fe, part, location));
    }
    out
}
