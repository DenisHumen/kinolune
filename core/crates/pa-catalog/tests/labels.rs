// SPDX-License-Identifier: MPL-2.0
//! LIB-022 end to end on a real folder: categories, tags and 1–5 star
//! ratings are saved per profile, the library queries filter and sort by
//! each of them, and nothing the active profile may not see leaks through
//! them (ADR-0055).

use std::path::Path;

use pa_catalog::{
    Catalog, Error, FolderAccess, Item, ItemKind, ItemRef, LibraryFilter, LibraryQuery,
    LibrarySort, ProfileSettings, SortField,
};
use pa_library::profiles::{DEFAULT_PROFILE, set_certifications};
use pa_model::MediaKey;
use pa_scanner::CancelToken;

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

const TOY: &str = "Kids/Toy Story (1995)/Toy.Story.1995.1080p.mkv";
const SCARY: &str = "Kids/The Ring (2002)/The.Ring.2002.1080p.mkv";
const ARRIVAL: &str = "Movies/Arrival (2016)/Arrival.2016.1080p.mkv";
const DUNE: &str = "Movies/Dune (2021)/Dune.2021.2160p.mkv";
const BLUEY: &str = "Kids/Bluey/Season 1/Bluey.S01E01.1080p.mkv";
const DARK: &str = "Shows/Dark/Season 1/Dark.S01E01.1080p.mkv";
/// A home video nothing recognizes (personal media, UI-025).
const CLIP: &str = "Home/Birthday/clip_0042.mkv";

struct Lib {
    dir: tempfile::TempDir,
    cat: Catalog,
    source: i64,
}

impl Lib {
    fn find(&self, kind: ItemKind, title: &str) -> Item {
        let items = match kind {
            ItemKind::Movie => self.cat.movies().unwrap(),
            _ => self.cat.shows().unwrap(),
        };
        items
            .into_iter()
            .find(|m| m.title == title)
            .unwrap_or_else(|| panic!("no {title}"))
    }

    fn movie(&self, title: &str) -> ItemRef {
        self.find(ItemKind::Movie, title).id
    }

    fn show(&self, title: &str) -> ItemRef {
        self.find(ItemKind::Show, title).id
    }

    fn titles(&self, kind: ItemKind, filter: LibraryFilter, sort: LibrarySort) -> Vec<String> {
        let q = LibraryQuery { kind, filter, sort };
        self.cat
            .query(&q, 0, 100)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.title)
            .collect()
    }
}

/// Toy Story (G) and The Ring (R) in `Kids/`, Arrival (PG) and Dune
/// (PG-13) outside it; the shows Bluey (TV-Y, in `Kids/`) and Dark
/// (TV-MA).
fn setup() -> Lib {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    for (i, rel) in [TOY, SCARY, ARRIVAL, DUNE, BLUEY, DARK, CLIP]
        .iter()
        .enumerate()
    {
        media(&root, rel, u8::try_from(i).unwrap() + 1);
    }
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    let source = cat
        .add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap()
        .id;
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let lib = Lib { dir, cat, source };
    let conn = pa_library::open(lib.cat.path()).unwrap();
    // The clip stays an unrecognized file: no work, no provider id.
    conn.execute(
        "DELETE FROM matches WHERE file_id = (SELECT id FROM files WHERE path = ?1)",
        [format!("/{CLIP}")],
    )
    .unwrap();
    for (title, cert) in [
        ("Toy Story", "G"),
        ("The Ring", "R"),
        ("Arrival", "PG"),
        ("Dune", "PG-13"),
    ] {
        let ItemRef::Movie(w) = lib.movie(title) else {
            unreachable!()
        };
        set_certifications(&conn, w, "tmdb", &[("US".into(), cert.into())]).unwrap();
    }
    for (title, cert) in [("Bluey", "TV-Y"), ("Dark", "TV-MA")] {
        let ItemRef::Show(w) = lib.show(title) else {
            unreachable!()
        };
        set_certifications(&conn, w, "tmdb", &[("US".into(), cert.into())]).unwrap();
    }
    lib
}

/// The `file:` key of the unrecognized clip.
fn clip_key(lib: &Lib) -> MediaKey {
    let conn = pa_library::open(lib.cat.path()).unwrap();
    let fingerprint: String = conn
        .query_row(
            "SELECT fingerprint FROM files WHERE path = ?1",
            [format!("/{CLIP}")],
            |r| r.get(0),
        )
        .unwrap();
    MediaKey::File { fingerprint }
}

fn filter() -> LibraryFilter {
    LibraryFilter::default()
}

fn by(field: SortField, descending: bool) -> LibrarySort {
    LibrarySort { field, descending }
}

#[test]
fn categories_tags_and_ratings_are_saved_per_profile() {
    let lib = setup();
    let labels = lib.cat.labels();
    let arrival = lib.movie("Arrival");
    let dune = lib.movie("Dune");
    let dark = lib.show("Dark");
    let episode = lib.cat.episodes(dark).unwrap()[0].id;

    // Categories: create, rename, delete; an item in several.
    let scifi = labels.create_category("Sci-fi").unwrap();
    let weekend = labels.create_category("Weekend").unwrap();
    let gone = labels.create_category("Old").unwrap();
    assert!(matches!(
        labels.create_category(" sci-FI "),
        Err(Error::Invalid(m)) if m == "duplicate category"
    ));
    labels.rename_category(weekend.id, "Weekend night").unwrap();
    labels.delete_category(gone.id).unwrap();
    assert!(matches!(
        labels.delete_category(gone.id),
        Err(Error::NotFound(_))
    ));
    assert!(labels.set_in_category(scifi.id, arrival, true).unwrap());
    assert!(!labels.set_in_category(scifi.id, arrival, true).unwrap());
    labels.set_in_category(scifi.id, dune, true).unwrap();
    labels.set_in_category(weekend.id, dune, true).unwrap();
    labels.set_in_category(weekend.id, episode, true).unwrap();

    // Tags, several per video, with autocomplete.
    labels
        .set_tags(dune, &["Desert".into(), "Epic".into(), "desert".into()])
        .unwrap();
    assert!(labels.add_tag(arrival, "Aliens").unwrap());
    assert!(labels.add_tag(arrival, "epic").unwrap());
    labels.add_tag(episode, "Time travel").unwrap();
    assert!(matches!(
        labels.add_tag(arrival, "  "),
        Err(Error::Invalid(_))
    ));

    // Ratings 1–5.
    assert!(labels.set_rating(dune, Some(5)).unwrap());
    labels.set_rating(arrival, Some(4)).unwrap();
    labels.set_rating(episode, Some(3)).unwrap();
    assert!(matches!(
        labels.set_rating(arrival, Some(6)),
        Err(Error::Invalid(_))
    ));

    // A restart keeps everything.
    let path = lib.cat.path().to_path_buf();
    let cat = Catalog::open(&path).unwrap();
    let labels = cat.labels();
    let cats: Vec<(String, usize)> = labels
        .categories()
        .unwrap()
        .into_iter()
        .map(|c| (c.name, c.item_count))
        .collect();
    assert_eq!(
        cats,
        [("Sci-fi".to_owned(), 2), ("Weekend night".to_owned(), 2)]
    );
    let in_dune: Vec<String> = labels
        .item_categories(dune)
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(in_dune, ["Sci-fi", "Weekend night"]);
    let weekend_items: Vec<ItemRef> = labels
        .category_items(weekend.id)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(weekend_items, [episode, dune]);
    assert_eq!(labels.item_tags(dune).unwrap(), ["Desert", "Epic"]);
    assert_eq!(labels.item_tags(arrival).unwrap(), ["Aliens", "Epic"]);
    assert_eq!(labels.item_tags(episode).unwrap(), ["Time travel"]);
    let all: Vec<(String, usize)> = labels
        .tags()
        .unwrap()
        .into_iter()
        .map(|t| (t.name, t.item_count))
        .collect();
    assert_eq!(
        all,
        [
            ("Aliens".to_owned(), 1),
            ("Desert".to_owned(), 1),
            ("Epic".to_owned(), 2),
            ("Time travel".to_owned(), 1)
        ]
    );
    let suggested: Vec<String> = labels
        .suggest_tags("e", 10)
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(suggested, ["Epic", "Aliens", "Desert", "Time travel"]);
    assert_eq!(labels.suggest_tags("e", 1).unwrap().len(), 1);
    assert_eq!(labels.tag_items("EPIC").unwrap().len(), 2);
    assert_eq!(labels.rating(dune).unwrap(), Some(5));
    assert_eq!(labels.rating(episode).unwrap(), Some(3));
    let rated: Vec<(ItemRef, u8)> = labels
        .rated_items(4)
        .unwrap()
        .into_iter()
        .map(|r| (r.item.id, r.stars))
        .collect();
    assert_eq!(rated, [(dune, 5), (arrival, 4)]);

    // Clearing.
    assert!(labels.set_rating(arrival, None).unwrap());
    assert!(labels.remove_tag(arrival, "aliens").unwrap());
    assert!(labels.suggest_tags("ali", 10).unwrap().is_empty());
    assert!(labels.set_in_category(scifi.id, arrival, false).unwrap());

    // Another profile has its own.
    let partner = cat
        .create_profile(
            &ProfileSettings {
                name: "Partner".into(),
                ..ProfileSettings::default()
            },
            None,
        )
        .unwrap();
    cat.switch_profile(partner.id, None, None).unwrap();
    assert!(labels.categories().unwrap().is_empty());
    assert!(labels.tags().unwrap().is_empty());
    assert_eq!(labels.item_tags(dune).unwrap(), Vec::<String>::new());
    assert_eq!(labels.rating(dune).unwrap(), None);
    assert!(matches!(
        labels.category_items(scifi.id),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        labels.set_in_category(scifi.id, dune, true),
        Err(Error::NotFound(_))
    ));
    // The same name is free in another profile.
    labels.create_category("Sci-fi").unwrap();
    cat.switch_profile(DEFAULT_PROFILE, None, None).unwrap();
    assert_eq!(labels.categories().unwrap().len(), 2);
}

#[test]
fn queries_filter_and_sort_by_category_tag_and_rating() {
    let lib = setup();
    let labels = lib.cat.labels();
    let toy = lib.movie("Toy Story");
    let ring = lib.movie("The Ring");
    let arrival = lib.movie("Arrival");
    let dune = lib.movie("Dune");
    let bluey = lib.show("Bluey");
    let dark = lib.show("Dark");

    let kids = labels.create_category("Kids").unwrap();
    let night = labels.create_category("Night").unwrap();
    labels.set_in_category(kids.id, toy, true).unwrap();
    labels.set_in_category(kids.id, bluey, true).unwrap();
    labels.set_in_category(night.id, ring, true).unwrap();
    labels.set_in_category(night.id, dune, true).unwrap();
    labels.set_in_category(night.id, dark, true).unwrap();
    labels.add_tag(toy, "Pixar").unwrap();
    labels.add_tag(ring, "Horror").unwrap();
    labels.add_tag(dark, "Mystery").unwrap();
    labels.add_tag(dune, "Epic").unwrap();
    labels.add_tag(arrival, "epic").unwrap();
    labels.set_rating(dune, Some(5)).unwrap();
    labels.set_rating(toy, Some(4)).unwrap();
    labels.set_rating(ring, Some(2)).unwrap();
    labels.set_rating(dark, Some(5)).unwrap();

    let movies = |f, s| lib.titles(ItemKind::Movie, f, s);
    let title = LibrarySort::default();

    // By category: OR inside the field, AND with others.
    assert_eq!(
        movies(
            LibraryFilter {
                categories: vec![night.id],
                ..filter()
            },
            title
        ),
        ["Dune", "The Ring"]
    );
    assert_eq!(
        movies(
            LibraryFilter {
                categories: vec![night.id, kids.id],
                ..filter()
            },
            title
        ),
        ["Dune", "The Ring", "Toy Story"]
    );
    assert_eq!(
        lib.titles(
            ItemKind::Show,
            LibraryFilter {
                categories: vec![kids.id],
                ..filter()
            },
            title
        ),
        ["Bluey"]
    );

    // By tag, case-insensitive.
    assert_eq!(
        movies(
            LibraryFilter {
                tags: vec!["EPIC".into()],
                ..filter()
            },
            title
        ),
        ["Arrival", "Dune"]
    );
    assert_eq!(
        movies(
            LibraryFilter {
                tags: vec!["epic".into()],
                categories: vec![night.id],
                ..filter()
            },
            title
        ),
        ["Dune"]
    );
    assert!(
        movies(
            LibraryFilter {
                tags: vec!["nope".into()],
                ..filter()
            },
            title
        )
        .is_empty()
    );

    // By rating; 0 = not rated.
    assert_eq!(
        movies(
            LibraryFilter {
                ratings: vec![4, 5],
                ..filter()
            },
            title
        ),
        ["Dune", "Toy Story"]
    );
    assert_eq!(
        movies(
            LibraryFilter {
                ratings: vec![0],
                ..filter()
            },
            title
        ),
        ["Arrival"]
    );

    // Sort by rating: missing last both ways, ties by title.
    assert_eq!(
        movies(filter(), by(SortField::UserRating, true)),
        ["Dune", "Toy Story", "The Ring", "Arrival"]
    );
    assert_eq!(
        movies(filter(), by(SortField::UserRating, false)),
        ["The Ring", "Toy Story", "Dune", "Arrival"]
    );
    // Category and tag lists are sorted by the query, too.
    assert_eq!(
        movies(
            LibraryFilter {
                categories: vec![night.id],
                ..filter()
            },
            by(SortField::UserRating, true)
        ),
        ["Dune", "The Ring"]
    );
    assert_eq!(
        movies(
            LibraryFilter {
                tags: vec!["epic".into()],
                ..filter()
            },
            by(SortField::UserRating, true)
        ),
        ["Dune", "Arrival"]
    );

    // A smart shelf keeps label filters.
    let q = LibraryQuery {
        kind: ItemKind::Movie,
        filter: LibraryFilter {
            ratings: vec![5],
            ..filter()
        },
        sort: title,
    };
    assert_eq!(LibraryQuery::from_json(&q.to_json()).unwrap(), q);
    let shelf = lib.cat.create_shelf("Best", &q, true).unwrap();
    assert_eq!(lib.cat.shelf_items(shelf.id, 0, 10).unwrap().total, 1);

    // Facets for filter chips.
    let f = lib.cat.facets(ItemKind::Movie).unwrap();
    let cats: Vec<(i64, usize)> = f.categories.iter().map(|c| (c.value, c.count)).collect();
    assert_eq!(cats, [(kids.id, 1), (night.id, 2)]);
    let tags: Vec<(&str, usize)> = f.tags.iter().map(|t| (t.value.as_str(), t.count)).collect();
    assert_eq!(tags, [("Epic", 2), ("Horror", 1), ("Pixar", 1)]);
    let ratings: Vec<(u8, usize)> = f.ratings.iter().map(|r| (r.value, r.count)).collect();
    assert_eq!(ratings, [(2, 1), (4, 1), (5, 1)]);
    let shows = lib.cat.facets(ItemKind::Show).unwrap();
    assert_eq!(shows.tags.len(), 1);
    assert_eq!(shows.ratings.len(), 1);
}

#[test]
fn hidden_items_do_not_leak_through_labels() {
    let lib = setup();
    let cat = &lib.cat;
    let labels = cat.labels();
    let toy = lib.movie("Toy Story");
    let ring = lib.movie("The Ring");
    let arrival = lib.movie("Arrival");
    let bluey = lib.show("Bluey");
    let dark = lib.show("Dark");
    let dark_ep = cat.episodes(dark).unwrap()[0].id;

    // The parent's own labels on everything.
    let parent_cat = labels.create_category("Parent only").unwrap();
    labels.set_in_category(parent_cat.id, ring, true).unwrap();
    labels.add_tag(ring, "Nightmare").unwrap();

    // A kid profile without filters yet labels things that will be hidden.
    let kid = cat
        .create_profile(
            &ProfileSettings {
                name: "Kid".into(),
                is_kid: true,
                ..ProfileSettings::default()
            },
            None,
        )
        .unwrap();
    cat.switch_profile(kid.id, None, None).unwrap();
    let mine = labels.create_category("Mine").unwrap();
    for item in [toy, ring, arrival, bluey, dark, dark_ep] {
        labels.set_in_category(mine.id, item, true).unwrap();
    }
    labels.add_tag(toy, "Funny").unwrap();
    labels.add_tag(ring, "Scary").unwrap();
    labels.add_tag(dark_ep, "Scary").unwrap();
    labels.add_tag(arrival, "Sad").unwrap();
    labels.add_tag(dark, "Spooky").unwrap();
    labels.set_rating(toy, Some(4)).unwrap();
    labels.set_rating(ring, Some(5)).unwrap();
    labels.set_rating(dark_ep, Some(5)).unwrap();

    // The parent restricts the kid: only `Kids/`, up to 12 years.
    cat.switch_profile(DEFAULT_PROFILE, None, None).unwrap();
    cat.update_profile(
        kid.id,
        &ProfileSettings {
            name: "Kid".into(),
            is_kid: true,
            max_age: Some(12),
            rating_country: "US".into(),
            hide_unrated: true,
            restrict_folders: true,
            ..ProfileSettings::default()
        },
        None,
    )
    .unwrap();
    cat.set_profile_folders(kid.id, &[FolderAccess::new(lib.source, "Kids")], None)
        .unwrap();
    cat.switch_profile(kid.id, None, None).unwrap();

    // Categories count and list only what the kid may see.
    let cats = labels.categories().unwrap();
    assert_eq!(cats.len(), 1, "the parent's category is the parent's");
    assert_eq!(cats[0].item_count, 2);
    let items: Vec<ItemRef> = labels
        .category_items(mine.id)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(items, [bluey, toy]);
    assert!(matches!(
        labels.category_items(parent_cat.id),
        Err(Error::NotFound(_))
    ));

    // Tags and autocomplete: only tags of visible items.
    let tags: Vec<String> = labels.tags().unwrap().into_iter().map(|t| t.name).collect();
    assert_eq!(tags, ["Funny"]);
    for prefix in ["s", "sc", "sad", "night", ""] {
        let s: Vec<String> = labels
            .suggest_tags(prefix, 10)
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(
            s.iter().all(|t| t == "Funny"),
            "{prefix}: {s:?} leaks hidden tags"
        );
    }
    assert!(labels.tag_items("scary").unwrap().is_empty());
    assert!(labels.tag_items("spooky").unwrap().is_empty());
    assert!(labels.tag_items("nightmare").unwrap().is_empty());

    // Ratings.
    let rated: Vec<ItemRef> = labels
        .rated_items(1)
        .unwrap()
        .into_iter()
        .map(|r| r.item.id)
        .collect();
    assert_eq!(rated, [toy]);

    // One-item calls on a hidden item answer as for a missing one.
    for item in [ring, arrival, dark, dark_ep] {
        assert!(
            matches!(labels.item_tags(item), Err(Error::NotFound(_))),
            "{item}"
        );
        assert!(matches!(labels.rating(item), Err(Error::NotFound(_))));
        assert!(matches!(
            labels.item_categories(item),
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            labels.set_rating(item, Some(1)),
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            labels.add_tag(item, "x"),
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            labels.set_in_category(mine.id, item, false),
            Err(Error::NotFound(_))
        ));
    }

    // Queries and facets.
    let q = |filter: LibraryFilter, kind| {
        cat.query(
            &LibraryQuery {
                kind,
                filter,
                sort: LibrarySort {
                    field: SortField::UserRating,
                    descending: true,
                },
            },
            0,
            50,
        )
        .unwrap()
        .items
        .into_iter()
        .map(|i| i.id)
        .collect::<Vec<_>>()
    };
    assert_eq!(
        q(
            LibraryFilter {
                categories: vec![mine.id],
                ..filter()
            },
            ItemKind::Movie
        ),
        [toy]
    );
    assert!(
        q(
            LibraryFilter {
                tags: vec!["Scary".into(), "Sad".into()],
                ..filter()
            },
            ItemKind::Movie
        )
        .is_empty()
    );
    assert!(
        q(
            LibraryFilter {
                ratings: vec![5],
                ..filter()
            },
            ItemKind::Movie
        )
        .is_empty()
    );
    assert!(
        q(
            LibraryFilter {
                categories: vec![parent_cat.id],
                ..filter()
            },
            ItemKind::Movie
        )
        .is_empty(),
        "another profile's category"
    );
    assert!(
        q(
            LibraryFilter {
                tags: vec!["Spooky".into()],
                ..filter()
            },
            ItemKind::Show
        )
        .is_empty()
    );
    assert_eq!(q(filter(), ItemKind::Movie), [toy]);
    let f = cat.facets(ItemKind::Movie).unwrap();
    assert_eq!(f.categories.len(), 1);
    assert_eq!(f.categories[0].count, 1);
    let tags: Vec<&str> = f.tags.iter().map(|t| t.value.as_str()).collect();
    assert_eq!(tags, ["Funny"]);
    let ratings: Vec<u8> = f.ratings.iter().map(|r| r.value).collect();
    assert_eq!(ratings, [4]);
    let shows = cat.facets(ItemKind::Show).unwrap();
    assert!(shows.tags.is_empty(), "{:?}", shows.tags);

    // Back on the parent profile everything the kid labeled is the kid's,
    // and the parent's own labels are intact.
    cat.switch_profile(DEFAULT_PROFILE, None, None).unwrap();
    let tags: Vec<String> = labels.tags().unwrap().into_iter().map(|t| t.name).collect();
    assert_eq!(tags, ["Nightmare"]);
    drop(lib.dir);
}

#[test]
fn unrecognized_files_take_labels_by_key() {
    let lib = setup();
    let cat = &lib.cat;
    let labels = cat.labels();
    let clip = clip_key(&lib);
    assert!(
        cat.movies().unwrap().iter().all(|m| m.title != "clip 0042"),
        "the clip is no catalog item"
    );

    let family = labels.create_category("Family").unwrap();
    assert!(labels.set_in_category(family.id, clip.clone(), true).unwrap());
    labels.add_tag(clip.clone(), "Birthday").unwrap();
    labels.set_rating(clip.clone(), Some(5)).unwrap();
    assert_eq!(labels.item_tags(clip.clone()).unwrap(), ["Birthday"]);
    assert_eq!(labels.rating(clip.clone()).unwrap(), Some(5));
    assert_eq!(labels.item_categories(clip.clone()).unwrap()[0].item_count, 1);
    assert!(labels.category_items(family.id).unwrap().is_empty());
    let files = labels.category_files(family.id).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].key, clip);
    assert_eq!(files[0].rel_path, CLIP);
    assert_eq!(files[0].source_id, lib.source);
    assert_eq!(labels.tag_files("birthday").unwrap(), files);
    assert_eq!(labels.tags().unwrap()[0].item_count, 1);

    // The file key of a recognized file labels its item.
    let conn = pa_library::open(cat.path()).unwrap();
    let toy_fp: String = conn
        .query_row(
            "SELECT fingerprint FROM files WHERE path = ?1",
            [format!("/{TOY}")],
            |r| r.get(0),
        )
        .unwrap();
    let toy_key = MediaKey::File {
        fingerprint: toy_fp,
    };
    labels.add_tag(toy_key, "Pixar").unwrap();
    assert_eq!(labels.item_tags(lib.movie("Toy Story")).unwrap(), ["Pixar"]);

    // A key of no file here.
    let ghost = MediaKey::File {
        fingerprint: "00ff".into(),
    };
    assert!(matches!(
        labels.add_tag(ghost, "x"),
        Err(Error::NotFound(_))
    ));

    // A profile restricted to `Kids/` does not see the clip or its labels.
    let kid = cat
        .create_profile(
            &ProfileSettings {
                name: "Kid".into(),
                ..ProfileSettings::default()
            },
            None,
        )
        .unwrap();
    cat.switch_profile(kid.id, None, None).unwrap();
    labels.add_tag(clip.clone(), "Party").unwrap();
    cat.switch_profile(DEFAULT_PROFILE, None, None).unwrap();
    cat.update_profile(
        kid.id,
        &ProfileSettings {
            name: "Kid".into(),
            restrict_folders: true,
            ..ProfileSettings::default()
        },
        None,
    )
    .unwrap();
    cat.set_profile_folders(kid.id, &[FolderAccess::new(lib.source, "Kids")], None)
        .unwrap();
    cat.switch_profile(kid.id, None, None).unwrap();
    assert!(labels.tags().unwrap().is_empty());
    assert!(labels.suggest_tags("p", 10).unwrap().is_empty());
    assert!(labels.tag_files("party").unwrap().is_empty());
    assert!(matches!(
        labels.item_tags(clip.clone()),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        labels.set_rating(clip, Some(1)),
        Err(Error::NotFound(_))
    ));
}
