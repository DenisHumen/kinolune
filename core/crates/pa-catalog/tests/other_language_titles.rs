// SPDX-License-Identifier: MPL-2.0
//! META-033: works named in English or romaji match TMDB results that a
//! metadata run in `ru-RU` receives with only Russian and original titles.
//!
//! The responses are synthetic but shaped like the real ones the
//! maintainer saw with `language=ru-RU`: the right work is the first
//! result, its `title`/`name` is Russian, its original title is often
//! Japanese, and the English or romaji title is only in `/translations` or
//! `/alternative_titles`.

use std::cell::RefCell;
use std::collections::HashMap;

use pa_catalog::metadata::{ACCEPT, AUTO, REVIEW, WorkQuery, match_search, match_search_titled};
use pa_metadata::tmdb::{MediaKind, SearchResult};
use pa_recognition::parse::parse;

/// One TMDB work: id, kind, Russian title, original title, year, titles
/// from translations and alternative titles.
struct Tmdb {
    id: u64,
    kind: MediaKind,
    ru: &'static str,
    original: &'static str,
    year: u32,
    other: &'static [&'static str],
}

impl Tmdb {
    fn result(&self) -> SearchResult {
        let date = format!("{}-04-01", self.year);
        let tv = self.kind == MediaKind::Tv;
        serde_json::from_value(serde_json::json!({
            "id": self.id,
            "media_type": self.kind.as_str(),
            (if tv { "name" } else { "title" }): self.ru,
            (if tv { "original_name" } else { "original_title" }): self.original,
            (if tv { "first_air_date" } else { "release_date" }): date,
            "original_language": "ja",
            "poster_path": format!("/{}.jpg", self.id),
        }))
        .unwrap()
    }
}

const fn movie(
    id: u64,
    ru: &'static str,
    original: &'static str,
    year: u32,
    other: &'static [&'static str],
) -> Tmdb {
    Tmdb {
        id,
        kind: MediaKind::Movie,
        ru,
        original,
        year,
        other,
    }
}

const fn tv(
    id: u64,
    ru: &'static str,
    original: &'static str,
    year: u32,
    other: &'static [&'static str],
) -> Tmdb {
    Tmdb {
        id,
        kind: MediaKind::Tv,
        ru,
        original,
        year,
        other,
    }
}

/// A decoy that TMDB also returns for many queries.
const DECOY_MOVIE: Tmdb = movie(1, "Обезьяны", "Monkeys", 1995, &["Monkeys"]);
const DECOY_TV: Tmdb = tv(2, "Чёрное", "Black", 2007, &["Black"]);

/// A film named `X Movie` whose TMDB title is `X: The Movie`: below `auto`.
const REVIEW_ONLY: &[&str] = &["Movies/Psycho-Pass Movie.mkv"];

/// `(path, expected work)`.
fn cases() -> Vec<(&'static str, Tmdb)> {
    vec![
        (
            "Movies/12 Monkeys [Двенадцать обезьян] (1995) BDRip 1080p.mkv",
            movie(
                63,
                "12 обезьян",
                "Twelve Monkeys",
                1995,
                &["Twelve Monkeys", "12 Monkeys"],
            ),
        ),
        (
            "Movies/Princess Mononoke 1997.mkv",
            movie(
                128,
                "Принцесса Мононоке",
                "もののけ姫",
                1997,
                &["Princess Mononoke", "Mononoke-hime"],
            ),
        ),
        (
            "Movies/Steamboy 2004.mkv",
            movie(8953, "Стимбой", "スチームボーイ", 2004, &["Steamboy"]),
        ),
        (
            "Movies/Millennium Actress 2001.mkv",
            movie(
                33320,
                "Актриса тысячелетия",
                "千年女優",
                2001,
                &["Millennium Actress", "Sennen Joyuu"],
            ),
        ),
        (
            "Movies/Cowboy Bebop Knockin' on Heaven's Door.mp4",
            movie(
                11299,
                "Ковбой Бибоп: Достучаться до небес",
                "カウボーイビバップ 天国の扉",
                2001,
                &[
                    "Cowboy Bebop: The Movie",
                    "Cowboy Bebop: Knockin' on Heaven's Door",
                ],
            ),
        ),
        (
            "Movies/Escaflowne The Movie (2000).mp4",
            movie(
                37933,
                "Эскафлон",
                "エスカフローネ",
                2000,
                &["Escaflowne: The Movie", "Escaflowne"],
            ),
        ),
        (
            "Movies/Psycho-Pass Movie.mkv",
            movie(
                330326,
                "Психопаспорт: Фильм",
                "劇場版 PSYCHO-PASS サイコパス",
                2015,
                &["Psycho-Pass: The Movie"],
            ),
        ),
        (
            "Movies/Steins Gate The Movie.mkv",
            movie(
                212162,
                "Врата Штейна: Нагрузочная область дежавю",
                "劇場版 STEINS;GATE 負荷領域のデジャヴ",
                2013,
                &[
                    "Steins;Gate: The Movie − Load Region of Déjà Vu",
                    "Gekijouban Steins;Gate: Fuka Ryouiki no Déjà vu",
                ],
            ),
        ),
        (
            "Darker than Black",
            tv(
                30981,
                "Темнее черного",
                "DARKER THAN BLACK -黒の契約者-",
                2007,
                &["Darker than Black"],
            ),
        ),
        (
            "Darling in the Franxx",
            tv(
                75214,
                "Милый во Франксе",
                "ダーリン・イン・ザ・フランキス",
                2018,
                &["DARLING in the FRANXX"],
            ),
        ),
        (
            "Psycho-pass",
            tv(
                45845,
                "Психопаспорт",
                "PSYCHO-PASS サイコパス",
                2012,
                &["Psycho-Pass"],
            ),
        ),
        (
            "Steins;Gate 0",
            tv(
                79062,
                "Врата Штейна 0",
                "シュタインズ・ゲート ゼロ",
                2018,
                &["Steins;Gate 0"],
            ),
        ),
        (
            "Youjo Senki",
            tv(
                69849,
                "Военная хроника маленькой девочки",
                "幼女戦記",
                2017,
                &["Saga of Tanya the Evil", "Youjo Senki"],
            ),
        ),
        (
            "Great Teacher Onizuka",
            tv(
                31724,
                "Крутой учитель Онидзука",
                "GTO",
                1999,
                &["Great Teacher Onizuka"],
            ),
        ),
        (
            "NieA Under 7 Remux",
            tv(
                42882,
                "Ниа Андер 7",
                "ニア アンダーセブン",
                2000,
                &["NieA_7", "NieA Under 7"],
            ),
        ),
        (
            "No Guns Life",
            tv(
                90937,
                "Без оружия — нет жизни",
                "ノー・ガンズ・ライフ",
                2019,
                &["No Guns Life"],
            ),
        ),
        (
            "Tenjou Tenge",
            tv(
                26453,
                "Тэндзё Тэнгэ",
                "天上天下",
                2004,
                &["Tenjho Tenge", "Tenjou Tenge"],
            ),
        ),
        (
            "Desert Punk",
            tv(
                26209,
                "Пустынный панк",
                "砂ぼうず",
                2004,
                &["Desert Punk", "Sunabouzu"],
            ),
        ),
        (
            "Boogiepop and Others",
            tv(
                83097,
                "Бугипоп никогда не смеётся",
                "ブギーポップは笑わない",
                2019,
                &["Boogiepop and Others"],
            ),
        ),
    ]
}

/// The work a library would query for `path`: a movie file, or the first
/// episode `Folder/Folder - 01.mkv` of a series folder, through the parser.
fn query(path: &str, kind: MediaKind) -> WorkQuery {
    let file = match kind {
        MediaKind::Movie => path.to_owned(),
        MediaKind::Tv => format!("{path}/{path} - 01.mkv"),
    };
    let p = parse(&file);
    let b = &p.best().parsed;
    WorkQuery {
        kind: if kind == MediaKind::Movie {
            "movie"
        } else {
            "show"
        }
        .into(),
        year: b.year.map(i64::from),
        titles: b.title.iter().chain(&b.alt_titles).cloned().collect(),
        ..Default::default()
    }
}

#[test]
fn english_and_romaji_titles_match_works_searched_in_russian() {
    for (path, expected) in cases() {
        let decoy = match expected.kind {
            MediaKind::Movie => DECOY_MOVIE,
            MediaKind::Tv => DECOY_TV,
        };
        let page = || vec![expected.result(), decoy.result()];
        let other: HashMap<u64, Vec<String>> = [&expected, &decoy]
            .iter()
            .map(|t| (t.id, t.other.iter().map(|s| (*s).to_owned()).collect()))
            .collect();
        let work = query(path, expected.kind);
        let asked = RefCell::new(0);
        let m = match_search_titled::<()>(
            &work,
            &mut |kind, _, _| {
                assert_eq!(kind, expected.kind);
                Ok(page())
            },
            &mut |_, _| Ok(None),
            &mut |_, id| {
                *asked.borrow_mut() += 1;
                Ok(other.get(&id).cloned().unwrap_or_default())
            },
        )
        .unwrap()
        .unwrap_or_else(|| panic!("{path}: no match for {work:?}"));
        assert_eq!(m.result.id, expected.id, "{path}: {m:?}");
        // The display title stays Russian (the metadata language).
        assert_eq!(m.result.title.as_deref(), Some(expected.ru), "{path}");
        // `X Movie` against `X: The Movie − Subtitle`: matched with its
        // poster, but marked «please check» (`auto_review`).
        let min = if REVIEW_ONLY.contains(&path) {
            REVIEW
        } else {
            AUTO
        };
        assert!(m.score >= min, "{path}: {m:?}");
        eprintln!("{path}: {:.3} {}", m.score, m.tier);
        assert!(
            *asked.borrow() <= 2,
            "{path}: titles of the first results only"
        );

        // Before META-033 (Russian and original titles only) the same
        // page did not reach `auto`.
        let before = match_search::<()>(&work, &mut |_, _, _| Ok(page())).unwrap();
        assert!(
            before
                .as_ref()
                .is_none_or(|b| b.score < AUTO || b.result.id != expected.id),
            "{path}: {before:?}"
        );
    }
}

#[test]
fn a_well_ranked_page_asks_for_no_other_titles() {
    let work = WorkQuery {
        kind: "movie".into(),
        year: Some(1995),
        titles: vec!["Twelve Monkeys".into()],
        ..Default::default()
    };
    let r = movie(63, "12 обезьян", "Twelve Monkeys", 1995, &[]).result();
    let m = match_search_titled::<()>(
        &work,
        &mut |_, _, _| Ok(vec![r.clone()]),
        &mut |_, _| Ok(None),
        &mut |_, _| panic!("no extra requests for an `auto` match"),
    )
    .unwrap()
    .unwrap();
    assert!(m.score >= AUTO);
}

#[test]
fn a_weak_match_is_kept_for_review_instead_of_dropped() {
    // Nothing ranks `ACCEPT`, but the provider's best answer is shown
    // (title, poster) as `auto_review`.
    let work = WorkQuery {
        kind: "movie".into(),
        year: Some(2013),
        titles: vec!["Steins Gate Load Region".into()],
        ..Default::default()
    };
    let r = movie(
        212162,
        "Врата Штейна: Нагрузочная область дежавю",
        "劇場版 STEINS;GATE 負荷領域のデジャヴ",
        2013,
        &[],
    )
    .result();
    let m = match_search_titled::<()>(
        &work,
        &mut |_, _, _| Ok(vec![r.clone()]),
        &mut |_, _| Ok(None),
        &mut |_, _| {
            Ok(vec![
                "Steins;Gate: The Movie − Load Region of Déjà Vu".into(),
            ])
        },
    )
    .unwrap();
    let m = m.expect("kept for review");
    assert_eq!(m.result.id, 212162);
    assert!(m.score >= REVIEW && m.score < ACCEPT, "{m:?}");
    // An unrelated page is still rejected.
    let unrelated = movie(5, "Буря", "The Tempest", 2010, &[]).result();
    let m = match_search_titled::<()>(
        &work,
        &mut |_, _, _| Ok(vec![unrelated.clone()]),
        &mut |_, _| Ok(None),
        &mut |_, _| Ok(vec!["The Tempest".into()]),
    )
    .unwrap();
    assert!(m.is_none(), "{m:?}");
}
