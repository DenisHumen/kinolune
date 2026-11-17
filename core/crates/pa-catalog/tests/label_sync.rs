// SPDX-License-Identifier: MPL-2.0
//! SYNC-013 done-when: two replicas — each its own catalog over its own
//! copy of the same files — edit categories, tags and ratings offline on
//! both sides; after they exchange their ops both have the same labels.
//! A category made on both with the same name merges, the newest write of
//! each label wins, deletes and removals stay, and nothing of a personal
//! library (UI-025) leaves the device.

use std::collections::BTreeMap;
use std::path::Path;

use pa_catalog::{Catalog, ItemKind, ItemRef, Lock};
use pa_scanner::CancelToken;
use pa_sync::{Op, OpBody};

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

const FILES: [&str; 6] = [
    "Movies/Toy Story (1995)/Toy.Story.1995.1080p.mkv",
    "Movies/Arrival (2016)/Arrival.2016.1080p.mkv",
    "Movies/Dune (2021)/Dune.2021.2160p.mkv",
    "Shows/Bluey/Season 1/Bluey.S01E01.1080p.mkv",
    "Shows/Dark/Season 1/Dark.S01E01.1080p.mkv",
    "Home/Birthday/clip_0042.mkv",
];

/// TMDB ids of the shows: their work refs (`tmdb:tv:…`) name them across
/// devices.
const SHOW_IDS: [(&str, &str); 2] = [("Bluey", "82728"), ("Dark", "70523")];

struct Device {
    _dir: tempfile::TempDir,
    cat: Catalog,
}

impl Device {
    /// A device with its own library of the same files. `tmdb_dune`: this
    /// one also knows Dune's TMDB id (the other knows it by its file key).
    fn new(name: &str, tmdb_dune: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("media");
        for (i, rel) in FILES.iter().enumerate() {
            media(&root, rel, u8::try_from(i).unwrap() + 1);
        }
        let cat = Catalog::open(dir.path().join("library.db")).unwrap();
        cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
            .unwrap();
        cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
        let dev = Self { _dir: dir, cat };
        let conn = pa_library::open(dev.cat.path()).unwrap();
        conn.execute(
            "DELETE FROM matches WHERE file_id = (SELECT id FROM files WHERE path = ?1)",
            [format!("/{}", FILES[5])],
        )
        .unwrap();
        for (title, id) in SHOW_IDS {
            let ItemRef::Show(w) = dev.item(ItemKind::Show, title) else {
                unreachable!()
            };
            conn.execute(
                "INSERT INTO external_ids (work_id, provider, ext_kind, ext_id)
                 VALUES (?1, 'tmdb', 'tv', ?2)",
                pa_library::rusqlite::params![w, id],
            )
            .unwrap();
        }
        if tmdb_dune {
            let ItemRef::Movie(w) = dev.item(ItemKind::Movie, "Dune") else {
                unreachable!()
            };
            conn.execute(
                "INSERT INTO external_ids (work_id, provider, ext_kind, ext_id)
                 VALUES (?1, 'tmdb', 'movie', '438631')",
                [w],
            )
            .unwrap();
        }
        dev.cat.enable_sync(name).unwrap();
        dev
    }

    fn item(&self, kind: ItemKind, title: &str) -> ItemRef {
        let items = match kind {
            ItemKind::Movie => self.cat.movies().unwrap(),
            _ => self.cat.shows().unwrap(),
        };
        items
            .into_iter()
            .find(|m| m.title == title)
            .unwrap_or_else(|| panic!("no {title}"))
            .id
    }

    fn movie(&self, title: &str) -> ItemRef {
        self.item(ItemKind::Movie, title)
    }

    fn show(&self, title: &str) -> ItemRef {
        self.item(ItemKind::Show, title)
    }

    fn category(&self, name: &str) -> i64 {
        self.cat
            .labels()
            .categories()
            .unwrap()
            .into_iter()
            .find(|c| c.name.to_lowercase() == name.to_lowercase())
            .unwrap_or_else(|| panic!("no category {name}"))
            .id
    }

    /// Every label of the main library: categories with their items (by
    /// lower-case name), tags with their items, ratings.
    fn labels(&self) -> Labels {
        let labels = self.cat.labels();
        let titles = |items: Vec<pa_catalog::Item>| -> Vec<String> {
            let mut t: Vec<String> = items.into_iter().map(|i| i.title).collect();
            t.sort();
            t
        };
        let categories = labels
            .categories()
            .unwrap()
            .into_iter()
            .map(|c| {
                (
                    c.name.to_lowercase(),
                    titles(labels.category_items(c.id).unwrap()),
                )
            })
            .collect();
        let tags = labels
            .tags()
            .unwrap()
            .into_iter()
            .map(|t| {
                (
                    t.name.to_lowercase(),
                    titles(labels.tag_items(&t.name).unwrap()),
                )
            })
            .collect();
        let ratings = labels
            .rated_items(1)
            .unwrap()
            .into_iter()
            .map(|r| (r.item.title, r.stars))
            .collect();
        Labels {
            categories,
            tags,
            ratings,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Labels {
    categories: BTreeMap<String, Vec<String>>,
    tags: BTreeMap<String, Vec<String>>,
    ratings: BTreeMap<String, u8>,
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

/// Sends what each side has queued to the other until both are quiet, as
/// the LAN transport does. Returns every op that went over.
fn exchange(a: &Device, b: &Device) -> Vec<Op> {
    let mut sent = Vec::new();
    for _ in 0..10 {
        // A label change or sync start reconciles; here, every round.
        a.cat.sync_labels().unwrap();
        b.cat.sync_labels().unwrap();
        let from_a = a.cat.pending_ops(10_000).unwrap();
        let from_b = b.cat.pending_ops(10_000).unwrap();
        if from_a.is_empty() && from_b.is_empty() {
            return sent;
        }
        for (from, to, ops) in [(a, b, &from_a), (b, a, &from_b)] {
            for op in ops {
                to.cat.apply_remote(op).unwrap();
                from.cat.ack_op(op.op_id).unwrap();
            }
        }
        sent.extend(from_a);
        sent.extend(from_b);
    }
    panic!("the replicas never went quiet");
}

#[test]
fn categories_tags_and_ratings_converge_after_offline_edits_on_both() {
    let phone = Device::new("phone", true);
    let tv = Device::new("tv", false);

    // Offline, phone: two categories, tags, ratings.
    let p = phone.cat.labels();
    let watch = p.create_category("To watch").unwrap().id;
    p.set_in_category(watch, phone.movie("Dune"), true).unwrap();
    p.set_in_category(watch, phone.movie("Arrival"), true)
        .unwrap();
    let family = p.create_category("Family").unwrap().id;
    p.set_in_category(family, phone.movie("Toy Story"), true)
        .unwrap();
    p.add_tag(phone.movie("Dune"), "Sci-Fi").unwrap();
    p.add_tag(phone.show("Dark"), "Thriller").unwrap();
    p.set_rating(phone.movie("Dune"), Some(5)).unwrap();
    p.set_rating(phone.movie("Arrival"), Some(3)).unwrap();

    // Offline, TV: the same category under another case, its own ones, the
    // same tag spelled otherwise, a conflicting rating made later.
    let t = tv.cat.labels();
    let watch_tv = t.create_category("to watch").unwrap().id;
    t.set_in_category(watch_tv, tv.movie("Toy Story"), true)
        .unwrap();
    let kids = t.create_category("Kids").unwrap().id;
    t.set_in_category(kids, tv.show("Bluey"), true).unwrap();
    t.add_tag(tv.movie("Dune"), "sci-fi").unwrap();
    t.add_tag(tv.movie("Arrival"), "Aliens").unwrap();
    t.set_rating(tv.movie("Dune"), Some(4)).unwrap();
    t.set_rating(tv.movie("Toy Story"), Some(2)).unwrap();

    let ops = exchange(&phone, &tv);
    assert!(
        ops.iter().any(|o| matches!(
            &o.body,
            OpBody::CategoryItem { target, .. } if target == "tmdb:movie:438631"
        )),
        "Dune goes by its TMDB key from the phone"
    );
    assert!(
        ops.iter().any(|o| matches!(
            &o.body,
            OpBody::TagSet { target, .. } if target == "tmdb:tv:70523"
        )),
        "a show goes by its work ref"
    );
    assert!(
        ops.iter()
            .any(|o| matches!(o.body, OpBody::Rating { rating: Some(10), .. })),
        "five stars are rating 10"
    );

    let first = phone.labels();
    assert_eq!(first, tv.labels());
    assert_eq!(
        first.categories,
        BTreeMap::from([
            ("family".into(), names(&["Toy Story"])),
            ("kids".into(), names(&["Bluey"])),
            ("to watch".into(), names(&["Arrival", "Dune", "Toy Story"])),
        ])
    );
    assert_eq!(
        first.tags,
        BTreeMap::from([
            ("aliens".into(), names(&["Arrival"])),
            ("sci-fi".into(), names(&["Dune"])),
            ("thriller".into(), names(&["Dark"])),
        ])
    );
    assert_eq!(
        first.ratings,
        BTreeMap::from([
            ("Arrival".into(), 3),
            ("Dune".into(), 4),
            ("Toy Story".into(), 2),
        ])
    );
    // One category id on both sides after the merge.
    assert_eq!(
        phone.cat.labels().categories().unwrap().len(),
        tv.cat.labels().categories().unwrap().len()
    );

    // Offline again, both sides change the same things.
    let p = phone.cat.labels();
    p.rename_category(phone.category("Family"), "Family night")
        .unwrap();
    p.set_in_category(phone.category("To watch"), phone.movie("Arrival"), false)
        .unwrap();
    p.remove_tag(phone.movie("Dune"), "SCI-FI").unwrap();
    p.set_rating(phone.movie("Arrival"), None).unwrap();
    let t = tv.cat.labels();
    t.delete_category(tv.category("Kids")).unwrap();
    t.set_in_category(tv.category("To watch"), tv.show("Dark"), true)
        .unwrap();
    t.add_tag(tv.movie("Toy Story"), "Pixar").unwrap();
    t.set_rating(tv.movie("Arrival"), Some(5)).unwrap();

    exchange(&phone, &tv);
    let second = phone.labels();
    assert_eq!(second, tv.labels());
    assert_eq!(
        second.categories,
        BTreeMap::from([
            ("family night".into(), names(&["Toy Story"])),
            ("to watch".into(), names(&["Dark", "Dune", "Toy Story"])),
        ])
    );
    assert_eq!(
        second.tags,
        BTreeMap::from([
            ("aliens".into(), names(&["Arrival"])),
            ("pixar".into(), names(&["Toy Story"])),
            ("thriller".into(), names(&["Dark"])),
        ])
    );
    assert_eq!(
        second.ratings,
        BTreeMap::from([
            ("Arrival".into(), 5),
            ("Dune".into(), 4),
            ("Toy Story".into(), 2),
        ])
    );

    // Nothing echoes: quiet on both sides.
    assert_eq!(phone.cat.sync_labels().unwrap(), 0);
    assert_eq!(tv.cat.sync_labels().unwrap(), 0);

    // Replayed old ops (a journal pulled again) change nothing.
    for op in &ops {
        tv.cat.apply_remote(op).unwrap();
        phone.cat.apply_remote(op).unwrap();
    }
    assert_eq!(phone.labels(), second);
    assert_eq!(tv.labels(), second);
}

#[test]
fn labels_of_items_a_device_lacks_wait_for_them() {
    let phone = Device::new("phone", true);
    let tv = Device::new("tv", false);
    // The TV has no Dune yet (another source, not scanned).
    {
        let conn = pa_library::open(tv.cat.path()).unwrap();
        conn.execute("DELETE FROM works WHERE title = 'Dune'", [])
            .unwrap();
        conn.execute(
            "DELETE FROM files WHERE path = ?1",
            [format!("/{}", FILES[2])],
        )
        .unwrap();
    }
    let p = phone.cat.labels();
    let c = p.create_category("Epic").unwrap().id;
    p.set_in_category(c, phone.movie("Dune"), true).unwrap();
    p.add_tag(phone.movie("Dune"), "Desert").unwrap();
    p.set_rating(phone.movie("Dune"), Some(5)).unwrap();
    exchange(&phone, &tv);
    let on_tv = tv.labels();
    assert_eq!(on_tv.categories["epic"], Vec::<String>::new());
    assert!(on_tv.tags.is_empty());
    assert!(on_tv.ratings.is_empty());

    // Dune shows up on the TV: the labels that waited for it apply at the
    // next sync, and nothing goes back.
    let dir = tempfile::tempdir().unwrap();
    media(dir.path(), FILES[2], 3);
    let src = tv
        .cat
        .add_source(None, "local", "More", dir.path().to_str().unwrap(), None)
        .unwrap()
        .id;
    tv.cat.scan(Some(src), &CancelToken::new(), &mut |_| {}).unwrap();
    assert_eq!(tv.cat.sync_labels().unwrap(), 0);
    let on_tv = tv.labels();
    assert_eq!(on_tv.categories["epic"], names(&["Dune"]));
    assert_eq!(on_tv.tags["desert"], names(&["Dune"]));
    assert_eq!(on_tv.ratings["Dune"], 5);
    assert_eq!(phone.labels(), on_tv);
}

#[test]
fn personal_libraries_never_sync() {
    let phone = Device::new("phone", true);
    let tv = Device::new("tv", false);
    let source = phone.cat.sources().unwrap()[0].id;
    let vault = phone
        .cat
        .create_personal_library("Семья", &Lock::None)
        .unwrap()
        .id;
    phone.cat.add_personal_folder(vault, source, "Home").unwrap();
    let clip = phone
        .cat
        .personal_items(vault, &Default::default(), Default::default())
        .unwrap()
        .into_iter()
        .find_map(|i| i.key)
        .expect("the clip is fingerprinted");
    let personal = phone.cat.personal_labels(vault);
    let c = personal.create_category("Секретное").unwrap().id;
    personal.set_in_category(c, clip.clone(), true).unwrap();
    personal.add_tag(clip.clone(), "Birthday").unwrap();
    personal.set_rating(clip.clone(), Some(5)).unwrap();
    // A main-library label still goes.
    phone
        .cat
        .labels()
        .add_tag(phone.movie("Arrival"), "Aliens")
        .unwrap();

    let ops = exchange(&phone, &tv);
    let clip_key = clip.to_string();
    assert!(!ops.is_empty());
    for op in &ops {
        let text = pa_sync::outbox::encode(op).unwrap();
        assert!(!text.contains("Секретное"), "{text}");
        assert!(!text.contains("Birthday"), "{text}");
        assert!(!text.contains(&clip_key), "{text}");
    }
    assert_eq!(
        tv.labels().tags,
        BTreeMap::from([("aliens".into(), names(&["Arrival"]))])
    );
    assert!(tv.labels().categories.is_empty());
}
