// SPDX-License-Identifier: MPL-2.0
//! LIB-012 end to end on a real folder: per-profile state is isolated, and
//! a kid profile sees nothing outside its white list and rating limit
//! through any API (lists, queries, search, "Continue watching",
//! favorites, collections, the file browser, details and playback).

use std::path::Path;

use pa_catalog::{
    Catalog, Error, FolderAccess, Item, ItemKind, ItemRef, LibraryFilter, LibraryQuery,
    LibrarySort, ProfileSettings,
};
use pa_library::profiles::{DEFAULT_PROFILE, set_certifications};
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
const HOME: &str = "Kids/Holiday Film (2019)/Holiday.Film.2019.mkv";
const ARRIVAL: &str = "Movies/Arrival (2016)/Arrival.2016.1080p.mkv";
const BLUEY: &str = "Kids/Bluey/Season 1/Bluey.S01E01.1080p.mkv";
const DARK: &str = "Shows/Dark/Season 1/Dark.S01E01.1080p.mkv";

const NOW: i64 = 1_763_000_000_000;

struct Lib {
    _dir: tempfile::TempDir,
    cat: Catalog,
    source: i64,
}

impl Lib {
    fn movie(&self, title: &str) -> Item {
        self.cat
            .movies()
            .unwrap()
            .into_iter()
            .find(|m| m.title == title)
            .unwrap_or_else(|| panic!("no {title}"))
    }

    fn show(&self, title: &str) -> Item {
        self.cat
            .shows()
            .unwrap()
            .into_iter()
            .find(|m| m.title == title)
            .unwrap_or_else(|| panic!("no {title}"))
    }

    fn titles(&self, kind: ItemKind) -> Vec<String> {
        let items = match kind {
            ItemKind::Movie => self.cat.movies().unwrap(),
            _ => self.cat.shows().unwrap(),
        };
        items.into_iter().map(|i| i.title).collect()
    }
}

/// Toy Story (G), The Ring (R) and an unrated film in `Kids/`, Arrival (PG)
/// outside it; the shows Bluey (TV-Y, in `Kids/`) and Dark (TV-MA).
fn setup() -> Lib {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    for (i, rel) in [TOY, SCARY, HOME, ARRIVAL, BLUEY, DARK].iter().enumerate() {
        media(&root, rel, u8::try_from(i).unwrap() + 1);
    }
    let poster = root.join("Kids/Toy Story (1995)/poster.jpg");
    std::fs::write(poster, b"\x89PNG\r\n\x1a\n").unwrap();
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    let source = cat
        .add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap()
        .id;
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let lib = Lib {
        _dir: dir,
        cat,
        source,
    };
    // TMDB-like ratings, written by another connection as the metadata run
    // would.
    let conn = pa_library::open(lib.cat.path()).unwrap();
    for (title, cert) in [("Toy Story", "G"), ("The Ring", "R"), ("Arrival", "PG")] {
        let ItemRef::Movie(w) = lib.movie(title).id else {
            unreachable!()
        };
        set_certifications(&conn, w, "tmdb", &[("US".into(), cert.into())]).unwrap();
    }
    for (title, cert) in [("Bluey", "TV-Y"), ("Dark", "TV-MA")] {
        let ItemRef::Show(w) = lib.show(title).id else {
            unreachable!()
        };
        set_certifications(&conn, w, "tmdb", &[("US".into(), cert.into())]).unwrap();
    }
    lib
}

fn kid_settings() -> ProfileSettings {
    ProfileSettings {
        name: "Kid".into(),
        avatar: Some("color:2".into()),
        is_kid: true,
        max_age: Some(12),
        rating_country: "US".into(),
        hide_unrated: true,
        hide_artwork: true,
        restrict_folders: true,
    }
}

#[test]
fn existing_state_belongs_to_the_default_profile_and_profiles_are_isolated() {
    let lib = setup();
    let cat = &lib.cat;
    assert_eq!(cat.active_profile().unwrap().id, DEFAULT_PROFILE);
    let arrival = lib.movie("Arrival").id;
    let toy = lib.movie("Toy Story").id;
    assert!(
        cat.save_progress(arrival, 600_000, 6_000_000, NOW, false)
            .unwrap()
    );
    cat.set_favorite(toy, true).unwrap();
    let mut prefs = cat.view_prefs("movies").unwrap();
    prefs.sort.descending = true;
    cat.set_view_prefs(&prefs).unwrap();
    let shelf = cat
        .create_shelf("All", &LibraryQuery::new(ItemKind::Movie), true)
        .unwrap();

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
    assert_eq!(cat.active_profile().unwrap().id, partner.id);
    assert_eq!(cat.progress(arrival).unwrap(), None);
    assert!(cat.continue_watching(10).unwrap().is_empty());
    assert!(cat.favorites(false).unwrap().is_empty());
    assert!(!cat.view_prefs("movies").unwrap().sort.descending);
    assert!(cat.shelves(false).unwrap().is_empty());
    assert!(
        cat.shelf_items(shelf.id, 0, 10).is_err(),
        "another profile's shelf"
    );
    assert_eq!(cat.sync_profile().unwrap(), partner.sync_id);
    // Unrestricted: the whole library.
    assert_eq!(lib.titles(ItemKind::Movie).len(), 4);

    cat.save_progress(toy, 1_200_000, 6_000_000, NOW + 1, false)
        .unwrap();
    cat.set_favorite(arrival, true).unwrap();

    cat.switch_profile(DEFAULT_PROFILE, None, None).unwrap();
    assert_eq!(cat.sync_profile().unwrap(), "default");
    let cont: Vec<ItemRef> = cat
        .continue_watching(10)
        .unwrap()
        .into_iter()
        .map(|c| c.item.id)
        .collect();
    assert_eq!(cont, [arrival]);
    let favs: Vec<ItemRef> = cat
        .favorites(false)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(favs, [toy]);
    assert!(cat.view_prefs("movies").unwrap().sort.descending);
    assert_eq!(cat.progress(toy).unwrap(), None);

    // The choice survives a restart.
    cat.switch_profile(partner.id, None, None).unwrap();
    let path = cat.path().to_path_buf();
    let reopened = Catalog::open(&path).unwrap();
    assert_eq!(reopened.active_profile().unwrap().id, partner.id);
}

#[test]
fn kid_profile_sees_nothing_outside_its_filters_through_any_api() {
    let lib = setup();
    let cat = &lib.cat;
    let toy = lib.movie("Toy Story");
    let ring = lib.movie("The Ring").id;
    let arrival = lib.movie("Arrival").id;
    let holiday = lib.movie("Holiday Film").id;
    let bluey = lib.show("Bluey").id;
    let dark = lib.show("Dark").id;
    assert_eq!(
        toy.local_poster.as_deref(),
        Some("Kids/Toy Story (1995)/poster.jpg")
    );

    // A kid profile without filters yet: some progress, favorites and a
    // collection, also of what will be hidden.
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
    for (n, item) in [toy.id, ring, arrival, holiday].into_iter().enumerate() {
        let at = NOW + i64::try_from(n).unwrap();
        assert!(
            cat.save_progress(item, 600_000, 6_000_000, at, false)
                .unwrap()
        );
        cat.set_favorite(item, true).unwrap();
    }
    let col = cat.create_collection("Mine").unwrap();
    cat.add_to_collection(col.id, ring).unwrap();
    let mixed = cat.create_collection("Mixed").unwrap();
    cat.add_to_collection(mixed.id, ring).unwrap();
    cat.add_to_collection(mixed.id, toy.id).unwrap();
    let playlist = cat.create_playlist("Evening").unwrap();
    cat.add_to_playlist(playlist.id, ring).unwrap();
    cat.add_to_playlist(playlist.id, toy.id).unwrap();

    // The parent sets the filters and a PIN on their own profile.
    cat.switch_profile(DEFAULT_PROFILE, None, None).unwrap();
    cat.set_profile_pin(DEFAULT_PROFILE, Some("2468"), None)
        .unwrap();
    cat.update_profile(kid.id, &kid_settings(), None).unwrap();
    cat.set_profile_folders(kid.id, &[FolderAccess::new(lib.source, "Kids")], None)
        .unwrap();
    cat.switch_profile(kid.id, None, None).unwrap();

    // Lists.
    assert_eq!(lib.titles(ItemKind::Movie), ["Toy Story"]);
    assert_eq!(lib.titles(ItemKind::Show), ["Bluey"]);
    let shown = cat.movies().unwrap();
    assert_eq!(shown[0].local_poster, None, "artwork hidden");
    assert_eq!(shown[0].poster_url, None);
    for kind in [ItemKind::Movie, ItemKind::Show, ItemKind::Episode] {
        for item in cat.recently_added(kind, 50).unwrap() {
            assert!(
                item.id == toy.id || item.show_id == Some(bluey) || item.id == bluey,
                "{item:?}"
            );
        }
    }
    assert!(cat.episodes(dark).unwrap().is_empty());
    assert_eq!(cat.episodes(bluey).unwrap().len(), 1);

    // Queries, facets and the alphabet (LIB-009).
    let q = LibraryQuery {
        kind: ItemKind::Movie,
        filter: LibraryFilter::default(),
        sort: LibrarySort::default(),
    };
    let page = cat.query(&q, 0, 50).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].id, toy.id);
    let letters: usize = cat
        .alphabet_index(&q)
        .unwrap()
        .iter()
        .map(|a| a.count)
        .sum();
    assert_eq!(letters, 1);
    let shelf = cat.create_shelf("Movies", &q, true).unwrap();
    assert_eq!(cat.shelf_items(shelf.id, 0, 50).unwrap().total, 1);

    // Search.
    for text in ["Ring", "Arrival", "Holiday", "Dark"] {
        assert!(cat.search(text, 20).unwrap().is_empty(), "{text}");
        assert!(cat.search_grouped(text, 20).unwrap().is_empty(), "{text}");
    }
    assert_eq!(cat.search("Toy", 20).unwrap().len(), 1);

    // "Continue watching", favorites, collections, playlists.
    let cont: Vec<ItemRef> = cat
        .continue_watching(20)
        .unwrap()
        .into_iter()
        .map(|c| c.item.id)
        .collect();
    assert_eq!(cont, [toy.id]);
    let favs: Vec<ItemRef> = cat
        .favorites(false)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(favs, [toy.id]);
    let cols: Vec<String> = cat
        .collections()
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(
        cols,
        ["Mixed"],
        "a collection of hidden items only is hidden"
    );
    let in_mixed: Vec<ItemRef> = cat
        .collection_items(mixed.id)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(in_mixed, [toy.id]);
    let in_list: Vec<ItemRef> = cat
        .playlist_items(playlist.id)
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(in_list, [toy.id]);

    // Direct access by id.
    for hidden in [ring, arrival, holiday, dark] {
        assert_eq!(cat.item(hidden).unwrap(), None, "{hidden}");
        assert!(
            matches!(cat.details(hidden), Err(Error::NotFound(_))),
            "{hidden}"
        );
        assert!(cat.progress(hidden).is_err(), "{hidden}");
        assert!(cat.genres(hidden).is_err(), "{hidden}");
        assert!(cat.set_favorite(hidden, true).is_err(), "{hidden}");
        assert!(cat.resume_point(hidden).is_err(), "{hidden}");
    }
    for hidden in [ring, arrival, holiday] {
        assert!(cat.versions(hidden).is_err(), "{hidden}");
    }
    assert!(cat.show_episodes(dark, None).is_err());
    assert!(cat.details(toy.id).is_ok());
    assert_eq!(cat.versions(toy.id).unwrap().len(), 1);

    // Files: the browser, playback and the stream proxy.
    let sources = cat.sources().unwrap();
    assert_eq!(sources.len(), 1);
    let root: Vec<String> = cat
        .browse(lib.source, "", LibrarySort::default(), true)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(root, ["Kids"]);
    let kids: Vec<String> = cat
        .browse(lib.source, "Kids", LibrarySort::default(), true)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(kids, ["Bluey", "Toy Story (1995)"]);
    assert!(
        cat.browse(lib.source, "Movies", LibrarySort::default(), true)
            .unwrap()
            .is_empty()
    );
    for hidden in [SCARY, HOME, ARRIVAL, DARK] {
        assert!(
            cat.play_parts(lib.source, hidden).unwrap().is_empty(),
            "{hidden}"
        );
        assert!(!cat.may_open(lib.source, hidden).unwrap(), "{hidden}");
    }
    assert_eq!(cat.play_parts(lib.source, TOY).unwrap().len(), 1);
    assert!(cat.may_open(lib.source, TOY).unwrap());
    assert!(
        cat.may_open(lib.source, "Kids/Toy Story (1995)/poster.jpg")
            .unwrap()
    );
    assert!(
        !cat.may_open(lib.source, "Movies/Arrival (2016)/poster.jpg")
            .unwrap()
    );

    // Leaving the kid profile needs the parent's PIN.
    assert!(matches!(
        cat.switch_profile(DEFAULT_PROFILE, None, None),
        Err(Error::Denied(_))
    ));
    assert!(matches!(
        cat.switch_profile(DEFAULT_PROFILE, Some("1111"), None),
        Err(Error::Denied(_))
    ));
    assert_eq!(cat.active_profile().unwrap().id, kid.id);
    cat.switch_profile(DEFAULT_PROFILE, Some("2468"), None)
        .unwrap();
    assert_eq!(lib.titles(ItemKind::Movie).len(), 4);
    assert_eq!(lib.movie("Toy Story").local_poster, toy.local_poster);
}

#[test]
fn rating_policy_and_parent_ratings() {
    let lib = setup();
    let cat = &lib.cat;
    let kid = cat
        .create_profile(
            &ProfileSettings {
                restrict_folders: false,
                hide_artwork: false,
                ..kid_settings()
            },
            None,
        )
        .unwrap();
    // The parent rates the home video for everyone and Arrival 16+.
    let holiday = lib.movie("Holiday Film").id;
    let arrival = lib.movie("Arrival").id;
    cat.set_user_certification(arrival, Some(16), None).unwrap();
    assert_eq!(cat.certifications(arrival).unwrap()[0].provider, "user");

    cat.switch_profile(kid.id, None, None).unwrap();
    // No folder list: every folder; R, 16+ and unrated hidden.
    assert_eq!(lib.titles(ItemKind::Movie), ["Toy Story"]);
    assert_eq!(lib.titles(ItemKind::Show), ["Bluey"]);

    cat.switch_profile(DEFAULT_PROFILE, None, None).unwrap();
    cat.set_user_certification(holiday, Some(0), None).unwrap();
    cat.set_user_certification(arrival, None, None).unwrap();
    cat.switch_profile(kid.id, None, None).unwrap();
    assert_eq!(
        lib.titles(ItemKind::Movie),
        ["Arrival", "Holiday Film", "Toy Story"]
    );

    // "No rating = hidden" off: unrated video shows.
    cat.switch_profile(DEFAULT_PROFILE, None, None).unwrap();
    cat.set_user_certification(holiday, None, None).unwrap();
    cat.update_profile(
        kid.id,
        &ProfileSettings {
            hide_unrated: false,
            restrict_folders: false,
            ..kid_settings()
        },
        None,
    )
    .unwrap();
    cat.switch_profile(kid.id, None, None).unwrap();
    assert_eq!(
        lib.titles(ItemKind::Movie),
        ["Arrival", "Holiday Film", "Toy Story"]
    );
    // A stricter limit.
    let mut s = cat.active_profile().unwrap();
    s.max_age = Some(6);
    cat.update_profile(kid.id, &ProfileSettings::of(&s), None)
        .unwrap();
    assert_eq!(lib.titles(ItemKind::Movie), ["Holiday Film", "Toy Story"]);
    // An empty white list with the restriction on shows nothing.
    cat.update_profile(kid.id, &kid_settings(), None).unwrap();
    assert!(cat.movies().unwrap().is_empty());
    assert!(cat.sources().unwrap().is_empty());
}
