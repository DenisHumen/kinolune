// SPDX-License-Identifier: MPL-2.0
//! BUG-009: the owner's titles are recognized on their own — on a fresh
//! library and on a library whose decisions an older build made.
//!
//! The library database outlives the app (it stays when the app is removed
//! and installed again). A build before META-033 compared the English or
//! romaji names of the files with the Russian and original titles of TMDB
//! only, so most of these works were `not_found` or `auto_review`, and the
//! current build never asked again: a `not_found` work only after a week,
//! an `auto_review` one never (its TMDB id was read back on the monthly
//! refresh with the decision kept). Now a decision of an older scorer
//! version is made again on the next run; user decisions, «not media» and
//! real ambiguity stay.
//!
//! BUG-011: a film named by the title before its subtitle (`Агент 327
//! (2017)` → `Агент 327: Операция «Парикмахерская»`, the only result of
//! that year) is recognized on its own too; two films of a franchise that
//! share the head and the year are real ambiguity.
//!
//! TMDB is synthetic and shaped like the real `ru-RU` answers (Russian
//! `title`/`name`, Japanese or Chinese original, English and romaji only in
//! `/translations` and `/alternative_titles`); an «old build» is the same
//! TMDB without those two lists, as such a build never asked for them.

use std::path::Path;
use std::sync::{Arc, Mutex};

use pa_catalog::pa_metadata::cache::MemoryCache;
use pa_catalog::pa_metadata::clock::ManualClock;
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::tmdb::{TmdbClient, TmdbConfig};
use pa_catalog::{Catalog, MetadataOptions, MetadataProviders, ReviewState};
use pa_library::rusqlite::Connection;
use pa_scanner::CancelToken;
use serde_json::{Value, json};

fn media(root: &Path, rel: &str, seed: u32) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31))
        .collect();
    data[..4].copy_from_slice(&seed.to_le_bytes());
    std::fs::write(path, data).unwrap();
}

/// One TMDB work.
struct Work {
    id: u64,
    tv: bool,
    ru: &'static str,
    original: &'static str,
    year: u32,
    language: &'static str,
    /// English and romaji titles: translations and alternative titles.
    other: &'static [&'static str],
    /// TV: regular episodes of season 1.
    episodes: u32,
}

const fn movie(
    id: u64,
    ru: &'static str,
    original: &'static str,
    year: u32,
    language: &'static str,
    other: &'static [&'static str],
) -> Work {
    Work {
        id,
        tv: false,
        ru,
        original,
        year,
        language,
        other,
        episodes: 0,
    }
}

const fn tv(
    id: u64,
    ru: &'static str,
    original: &'static str,
    year: u32,
    other: &'static [&'static str],
    episodes: u32,
) -> Work {
    Work {
        id,
        tv: true,
        ru,
        original,
        year,
        language: "ja",
        other,
        episodes,
    }
}

/// The owner's report: what the files are named and the work they are.
/// Movies are one file (in a folder of its own when the name has a year in
/// brackets), series a folder of two episodes.
const CORPUS: &[(&str, Work)] = &[
    (
        // BUG-011: the live QA-016 run of 2026-11-20.
        "Агент 327 (2017)",
        movie(
            457_784,
            "Агент 327: Операция «Парикмахерская»",
            "Agent 327: Operatie Barbershop",
            2017,
            "nl",
            &["Agent 327: Operation Barbershop"],
        ),
    ),
    (
        "Princess Mononoke 1997",
        movie(
            128,
            "Принцесса Мононоке",
            "もののけ姫",
            1997,
            "ja",
            &["Princess Mononoke", "Mononoke-hime"],
        ),
    ),
    (
        "Millennium Actress 2001",
        // TMDB dates it by the 2002 release; the festival year is in the name.
        movie(
            33320,
            "Актриса тысячелетия",
            "千年女優",
            2002,
            "ja",
            &["Millennium Actress", "Sennen Joyuu"],
        ),
    ),
    (
        "Pale Cocoon",
        movie(
            20683,
            "Бледный кокон",
            "ペイル・コクーン",
            2005,
            "ja",
            &["Pale Cocoon"],
        ),
    ),
    (
        "Steamboy 2004",
        movie(8953, "Стимбой", "スチームボーイ", 2004, "ja", &["Steamboy"]),
    ),
    (
        "The Legend of Hei 2 2025",
        // No Russian title yet: TMDB answers with the Chinese one.
        movie(
            1_144_107,
            "罗小黑战记 2",
            "罗小黑战记 2",
            2025,
            "zh",
            &["The Legend of Hei 2", "The Legend of Hei II"],
        ),
    ),
    (
        "Boogiepop and Others",
        tv(
            84163,
            "Бугипоп никогда не смеётся",
            "ブギーポップは笑わない",
            2019,
            &["Boogiepop and Others", "Boogiepop wa Warawanai"],
            18,
        ),
    ),
    (
        "Darker than Black",
        tv(
            31718,
            "Темнее черного",
            "DARKER THAN BLACK -黒の契約者-",
            2007,
            &[
                "Darker than Black",
                "Darker than Black: Kuro no Keiyakusha",
                "Darker than Black: Ryuusei no Gemini",
            ],
            25,
        ),
    ),
    (
        "Darling in the Franxx",
        tv(
            76121,
            "Любимый во Франксе",
            "ダーリン・イン・ザ・フランキス",
            2018,
            &["DARLING in the FRANXX"],
            24,
        ),
    ),
    (
        "Desert Punk",
        tv(
            29616,
            "Пустынная крыса",
            "砂ぼうず",
            2004,
            &["Desert Punk", "Sunabouzu"],
            24,
        ),
    ),
    (
        "NieA under 7",
        tv(
            28223,
            "Ния под семёркой",
            "NieA_7",
            2000,
            &["NieA under 7", "NieA_7"],
            13,
        ),
    ),
    (
        "No Guns Life",
        tv(
            91270,
            "Жизнь без оружия",
            "ノー・ガンズ・ライフ",
            2019,
            &["No Guns Life"],
            24,
        ),
    ),
    (
        "Psycho-Pass",
        tv(
            43865,
            "Психопаспорт",
            "PSYCHO-PASS サイコパス",
            2012,
            &["Psycho-Pass"],
            22,
        ),
    ),
    (
        "Steins;Gate 0",
        tv(
            78102,
            "Врата Штейна 0",
            "シュタインズ・ゲート ゼロ",
            2018,
            &["Steins;Gate 0", "Steins;Gate Zero"],
            23,
        ),
    ),
    (
        "Tenjou Tenge",
        tv(
            34732,
            "Небо и земля",
            "天上天下",
            2004,
            &["Tenjho Tenge", "Tenjou Tenge"],
            24,
        ),
    ),
    (
        "Youjo Senki",
        tv(
            69346,
            "Военная хроника маленькой девочки",
            "幼女戦記",
            2017,
            &["Saga of Tanya the Evil", "Youjo Senki"],
            12,
        ),
    ),
];

/// The second season, named as its own title: TMDB has it inside the
/// series, so it joins `Darker than Black`.
const GEMINI: &str = "Darker than Black Ryuusei no Gemini";

/// Works TMDB returns for many queries (a page is never a single result).
const DECOYS: &[Work] = &[
    movie(1, "Обезьяны", "Monkeys", 1995, "en", &["Monkeys"]),
    tv(2, "Чёрное", "Black", 2007, &["Black"], 10),
];

/// Two films of one name and no year in the file: a real tie that only
/// the user can break.
const DUNES: &[Work] = &[
    movie(841, "Дюна", "Dune", 1984, "en", &["Dune"]),
    movie(438_631, "Дюна", "Dune", 2021, "en", &["Dune"]),
];

/// Two films of one franchise and one year that share the title before
/// the subtitle (synthetic): the file named by that title alone is a real
/// tie (BUG-011).
const SAGA: &str = "Сага о Кинолуне (2017)";
const SAGAS: &[Work] = &[
    movie(
        900_001,
        "Сага о Кинолуне: Начало",
        "Kinolune Saga: The Beginning",
        2017,
        "en",
        &["Kinolune Saga: The Beginning"],
    ),
    movie(
        900_002,
        "Сага о Кинолуне: Возвращение",
        "Kinolune Saga: The Return",
        2017,
        "en",
        &["Kinolune Saga: The Return"],
    ),
];

/// What TMDB would return for a default file name: it must never be asked.
const CLIP: Work = movie(77, "Клип", "Clip", 2012, "sr", &["Clip"]);

fn all_works() -> impl Iterator<Item = &'static Work> {
    CORPUS
        .iter()
        .map(|(_, w)| w)
        .chain(DECOYS)
        .chain(DUNES)
        .chain(SAGAS)
        .chain(std::iter::once(&CLIP))
}

fn fold(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

impl Work {
    fn titles(&self) -> impl Iterator<Item = &str> {
        [self.ru, self.original]
            .into_iter()
            .chain(self.other.iter().copied())
    }

    fn date(&self) -> String {
        format!("{}-04-01", self.year)
    }

    /// A search result: Russian and original titles only.
    fn result(&self) -> Value {
        if self.tv {
            json!({"id": self.id, "name": self.ru, "original_name": self.original,
                   "first_air_date": self.date(), "original_language": self.language,
                   "poster_path": format!("/{}.jpg", self.id)})
        } else {
            json!({"id": self.id, "title": self.ru, "original_title": self.original,
                   "release_date": self.date(), "original_language": self.language,
                   "poster_path": format!("/{}.jpg", self.id)})
        }
    }

    fn details(&self) -> Value {
        let mut d = self.result();
        d["genres"] = json!([{"id": 16, "name": "мультфильм"}]);
        d["status"] = json!("Released");
        if self.tv {
            d["episode_run_time"] = json!([24]);
            d["seasons"] = json!([{"season_number": 1, "episode_count": self.episodes}]);
        } else {
            d["runtime"] = json!(100);
        }
        d
    }

    fn translations(&self) -> Value {
        let key = if self.tv { "name" } else { "title" };
        let mut list = vec![json!({"iso_3166_1": "RU", "iso_639_1": "ru", "data": {key: self.ru}})];
        if let Some(en) = self.other.first() {
            list.push(json!({"iso_3166_1": "US", "iso_639_1": "en", "data": {key: en}}));
        }
        json!({"id": self.id, "translations": list})
    }

    fn alternative_titles(&self) -> Value {
        let list: Vec<Value> = self
            .other
            .iter()
            .map(|t| json!({"iso_3166_1": "JP", "title": t}))
            .collect();
        if self.tv {
            json!({"id": self.id, "results": list})
        } else {
            json!({"id": self.id, "titles": list})
        }
    }
}

/// Synthetic TMDB; `old` answers like it would to a build before META-033,
/// which never asked for titles in other languages.
#[derive(Default)]
struct FakeTmdb {
    old: Mutex<bool>,
    searches: Mutex<Vec<String>>,
}

fn ok(body: &Value) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: vec![],
        body: body.to_string(),
    }
}

fn not_found() -> HttpResponse {
    HttpResponse {
        status: 404,
        headers: vec![],
        body: r#"{"status_code":34,"status_message":"not found"}"#.to_owned(),
    }
}

impl FakeTmdb {
    fn respond(&self, r: &HttpRequest) -> HttpResponse {
        let parts: Vec<&str> = r.path.trim_matches('/').split('/').collect();
        match parts.as_slice() {
            ["search", kind] => {
                let query = r
                    .query
                    .iter()
                    .find(|(k, _)| k == "query")
                    .map_or("", |(_, v)| v.as_str());
                self.searches.lock().unwrap().push(query.to_owned());
                let q = fold(query);
                let tv = *kind == "tv";
                // TMDB finds a title by its first words too (`Агент 327` →
                // `Агент 327: Операция «Парикмахерская»`).
                let mut results: Vec<Value> = all_works()
                    .filter(|w| w.tv == tv && w.titles().any(|t| fold(t).starts_with(&q)))
                    .map(Work::result)
                    .collect();
                if !results.is_empty() {
                    // A page always has more than the right work.
                    results.extend(DECOYS.iter().filter(|w| w.tv == tv).map(Work::result));
                }
                ok(&json!({"page": 1, "results": results, "total_pages": 1}))
            }
            [kind, id, rest @ ..] if *kind == "movie" || *kind == "tv" => {
                let tv = *kind == "tv";
                let Some(w) = all_works().find(|w| w.tv == tv && id.parse() == Ok(w.id)) else {
                    return not_found();
                };
                let old = *self.old.lock().unwrap();
                match rest {
                    [] => ok(&w.details()),
                    ["translations"] if !old => ok(&w.translations()),
                    ["alternative_titles"] if !old => ok(&w.alternative_titles()),
                    _ => not_found(),
                }
            }
            _ => not_found(),
        }
    }
}

struct Transport(Arc<FakeTmdb>);

impl HttpTransport for Transport {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        Ok(self.0.respond(r))
    }
}

fn providers(fake: &Arc<FakeTmdb>) -> MetadataProviders {
    let mut config = TmdbConfig::default();
    config.retry.max_retries = 0;
    MetadataProviders {
        tmdb: Some(Arc::new(TmdbClient::new(
            config,
            Keys::none().with_user_key(Some("test-key")),
            Arc::new(Transport(fake.clone())),
            // A fresh cache per run: an old build's answers are not reused.
            Arc::new(MemoryCache::default()),
            Arc::new(ManualClock::at_unix(1_800_000_000)),
        ))),
        anilist: None,
        shikimori: None,
        anime_mappings: None,
        anime_titles: None,
        extra: Default::default(),
    }
}

struct Library {
    dir: tempfile::TempDir,
    cat: Catalog,
}

fn library() -> Library {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    let mut seed = 1;
    for (name, work) in CORPUS {
        if work.tv {
            for e in 1..=2 {
                media(&root, &format!("Anime/{name}/{name} - {e:02}.mkv"), seed);
                seed += 1;
            }
        } else if name.ends_with(')') {
            media(&root, &format!("Movies/{name}/{name}.mkv"), seed);
            seed += 1;
        } else {
            media(&root, &format!("Movies/{name}.mkv"), seed);
            seed += 1;
        }
    }
    for e in 1..=2 {
        media(
            &root,
            &format!("Anime/{GEMINI}/{GEMINI} - {e:02}.mkv"),
            seed,
        );
        seed += 1;
    }
    media(&root, &format!("Movies/{SAGA}/{SAGA}.mkv"), seed);
    seed += 1;
    media(&root, "Movies/Dune.mkv", seed);
    media(&root, "Movies/Zzyzx Qwerty 2003.mkv", seed + 1);
    media(&root, "Downloads/New folder (2)/clip.mp4", seed + 2);
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    Library { dir, cat }
}

impl Library {
    fn db(&self) -> Connection {
        Connection::open(self.dir.path().join("library.db")).unwrap()
    }

    fn run(&self, fake: &Arc<FakeTmdb>) -> pa_catalog::MetadataReport {
        self.cat
            .fetch_metadata(
                &providers(fake),
                &MetadataOptions::default(),
                &CancelToken::new(),
                &mut |_, _| {},
            )
            .unwrap()
    }

    /// `(TMDB id, every live match is auto)` of the work a file of `name`
    /// belongs to; `(None, false)` for a file without a work.
    fn decision(&self, name: &str) -> (Option<u64>, bool) {
        let db = self.db();
        let Ok(work) = db.query_row(
            "SELECT m.work_id FROM matches m JOIN files f ON f.id = m.file_id
             WHERE f.path LIKE '%' || ?1 || '%' AND m.state <> 'rejected' LIMIT 1",
            [name],
            |r| r.get::<_, i64>(0),
        ) else {
            return (None, false);
        };
        let tmdb: Option<String> = db
            .query_row(
                "SELECT ext_id FROM external_ids WHERE work_id = ?1 AND provider = 'tmdb'",
                [work],
                |r| r.get(0),
            )
            .ok();
        let auto: bool = db
            .query_row(
                "SELECT NOT EXISTS (SELECT 1 FROM matches WHERE work_id = ?1
                                      AND state NOT IN ('auto', 'rejected'))
                    AND EXISTS (SELECT 1 FROM work_metadata WHERE work_id = ?1
                                  AND outcome = 'matched')",
                [work],
                |r| r.get(0),
            )
            .unwrap();
        (tmdb.and_then(|t| t.parse().ok()), auto)
    }

    /// Works of the report decided right and silently (`auto`).
    fn auto_rate(&self) -> usize {
        CORPUS
            .iter()
            .filter(|(name, w)| self.decision(name) == (Some(w.id), true))
            .count()
    }

    fn work_of(&self, name: &str) -> i64 {
        self.db()
            .query_row(
                "SELECT m.work_id FROM matches m JOIN files f ON f.id = m.file_id
                 WHERE f.path LIKE '%' || ?1 || '%' AND m.state <> 'rejected' LIMIT 1",
                [name],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// The content kind of the file ending in `name`.
    fn kind(&self, name: &str) -> Option<String> {
        self.db()
            .query_row(
                "SELECT k.kind FROM file_content_kinds k JOIN files f ON f.id = k.file_id
                 WHERE f.path LIKE '%' || ?1",
                [name],
                |r| r.get(0),
            )
            .ok()
    }

    /// Makes the stored decisions look as an older build left them: its
    /// scorer version, nothing on the matches of works it did not find, and
    /// «Мои видео» for an untitled film it did not find (LIB-025) without
    /// the scorer it was decided under.
    fn as_older_build(&self) {
        let db = self.db();
        db.execute(
            "UPDATE file_content_kinds
                SET reasons_json = replace(reasons_json, ',\"' || ?1 || '\"', '')",
            [pa_catalog::metadata::MODEL_VERSION],
        )
        .unwrap();
        db.execute(
            "UPDATE matches SET model_version = 'scorer_v1' WHERE decided_by = 'scorer'",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE matches SET model_version = NULL WHERE decided_by = 'rule'",
            [],
        )
        .unwrap();
    }
}

#[test]
fn a_fresh_library_recognizes_the_report_on_its_own() {
    let lib = library();
    let fake = Arc::new(FakeTmdb::default());
    lib.run(&fake);
    for (name, w) in CORPUS {
        assert_eq!(lib.decision(name), (Some(w.id), true), "{name}");
    }
    assert_eq!(lib.decision(GEMINI), (Some(31718), true), "{GEMINI}");
    eprintln!("fresh library: {}/{} auto", lib.auto_rate(), CORPUS.len());

    // Only real ambiguity reaches the user: two films «Dune», two films of
    // a franchise sharing the head of their titles and a name TMDB does
    // not know.
    let inbox = lib.cat.review_items(100).unwrap();
    let mut waiting: Vec<(&str, ReviewState)> = inbox
        .iter()
        .map(|i| (i.local_title.as_str(), i.state))
        .collect();
    waiting.sort_unstable_by_key(|w| w.0);
    assert_eq!(
        waiting,
        [
            ("Dune", ReviewState::Check),
            ("Zzyzx Qwerty", ReviewState::Manual),
            ("Сага о Кинолуне", ReviewState::Check),
        ]
    );
    // BUG-005: a default file name never reaches TMDB.
    assert!(
        fake.searches
            .lock()
            .unwrap()
            .iter()
            .all(|q| !q.to_lowercase().contains("clip"))
    );
    assert!(lib.cat.movies().unwrap().iter().all(|m| m.title != "clip"));
    // The tie is not decided for the user.
    assert!(!lib.decision(SAGA).1, "{SAGA}");
}

#[test]
fn decisions_of_an_older_build_are_made_again_on_the_next_run() {
    let lib = library();
    let fake = Arc::new(FakeTmdb::default());

    // An older build: English and romaji names against Russian and
    // original titles only.
    *fake.old.lock().unwrap() = true;
    lib.run(&fake);
    lib.as_older_build();
    let before = lib.auto_rate();
    let inbox_before = lib.cat.review_count().unwrap();
    assert!(before < CORPUS.len() / 2, "{before}");
    assert!(inbox_before.total() >= 10, "{inbox_before:?}");
    // A film without a year that nothing was found for is «Мои видео».
    assert_eq!(lib.kind("Pale Cocoon.mkv").as_deref(), Some("own_video"));

    // The user decided two of them before the update: a result of their
    // own choice (a decoy, on purpose) and «not a series».
    let chosen = lib.work_of("Steamboy");
    let chosen = lib.cat.review_choose(chosen, "tmdb", "movie", "1").unwrap();
    let not_media = lib.work_of("Tenjou Tenge");
    lib.cat.review_not_media(not_media).unwrap();

    // The current build, first scan and run after the update: everything
    // an older scorer decided is decided again, without the user.
    *fake.old.lock().unwrap() = false;
    fake.searches.lock().unwrap().clear();
    lib.cat
        .scan(None, &CancelToken::new(), &mut |_| {})
        .unwrap();
    assert_eq!(lib.kind("Pale Cocoon.mkv").as_deref(), Some("video"));
    let report = lib.run(&fake);
    assert!(
        fake.searches
            .lock()
            .unwrap()
            .iter()
            .all(|q| q != "Tenjou Tenge")
    );
    eprintln!("first run after the update: {report:?}");
    // Only `Zzyzx Qwerty`, which TMDB does not know.
    assert_eq!(report.not_found, 1, "{report:?}");
    let after = lib.auto_rate();
    eprintln!(
        "library of an older build: {before}/{n} auto before, {after}/{n} after",
        n = CORPUS.len()
    );
    for (name, w) in CORPUS {
        if ["Steamboy 2004", "Tenjou Tenge"].contains(name) {
            continue;
        }
        assert_eq!(lib.decision(name), (Some(w.id), true), "{name}");
    }
    assert_eq!(after, CORPUS.len() - 2);

    // The user's decisions stay.
    assert_eq!(lib.decision("Steamboy").0, Some(1));
    let db = lib.db();
    let user: String = db
        .query_row(
            "SELECT group_concat(DISTINCT state) FROM matches WHERE work_id = ?1",
            [chosen],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(user, "user_set");
    let not_media_asked: bool = db
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM work_metadata WHERE work_id = ?1)
                 OR EXISTS (SELECT 1 FROM external_ids WHERE work_id = ?1)",
            [not_media],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!not_media_asked);

    // Real ambiguity stays with the user; nothing became a movie «clip».
    let inbox: Vec<String> = lib
        .cat
        .review_items(100)
        .unwrap()
        .into_iter()
        .map(|i| i.local_title)
        .collect();
    assert!(inbox.contains(&"Dune".to_owned()), "{inbox:?}");
    assert!(inbox.contains(&"Zzyzx Qwerty".to_owned()), "{inbox:?}");
    assert!(inbox.contains(&"Сага о Кинолуне".to_owned()), "{inbox:?}");
    assert_eq!(inbox.len(), 3, "{inbox:?}");
    assert!(
        fake.searches
            .lock()
            .unwrap()
            .iter()
            .all(|q| !q.to_lowercase().contains("clip"))
    );

    // Decided once: the next run takes nothing (no loop over works no
    // provider knows, ties or user decisions).
    let again = lib.run(&fake);
    assert_eq!(again.works, 0, "{again:?}");
}
