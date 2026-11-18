// SPDX-License-Identifier: MPL-2.0
//! UI-012 end to end: «Исправить матч» by a pasted link. A season of 24
//! episodes nobody recognised waits in the manual queue; the user pastes
//! its TheTVDB link, takes the TMDB series it names, and the fix covers all
//! 24 episodes at once and teaches the folder a rule (`FolderRule`), so a
//! 25th file joins the series offline. Also: a result of another kind
//! switches the work's type, anime ids resolve through the anime database,
//! and the manual search finds anime there. Synthetic provider responses
//! (no real TMDB data).

use std::path::Path;
use std::sync::{Arc, Mutex};

use pa_catalog::pa_anime_mappings::AnimeIds;
use pa_catalog::pa_anime_mappings::offline_db::OfflineEntry;
use pa_catalog::pa_metadata::cache::{MemoryCache, ProviderCache};
use pa_catalog::pa_metadata::clock::ManualClock;
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::tmdb::{TmdbClient, TmdbConfig};
use pa_catalog::{AnimeTitles, Catalog, ItemRef, MetadataOptions, MetadataProviders};
use pa_library::rusqlite::Connection;
use pa_scanner::CancelToken;

const FOLDER: &str = "Anime/Frieren";
const DANDADAN: &str = "Anime/Frieren/[Other] Dandadan - 07 (1080p) [BBBB0007].mkv";

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

fn episode(n: u32) -> String {
    format!("{FOLDER}/[Grp] Sousou no Frieren - {n:02} (1080p) [AAAA{n:04}].mkv")
}

fn ok(body: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: vec![],
        body: body.to_owned(),
    }
}

/// Synthetic TMDB: title searches find nothing; TheTVDB 424536 is the
/// series 900024 (28 episodes in season 1); 700001 is a film.
#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<String>>,
}

impl HttpTransport for Fake {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.calls.lock().unwrap().push(r.path.clone());
        let source = r
            .query
            .iter()
            .find(|(k, _)| k == "external_source")
            .map_or("", |(_, v)| v.as_str());
        Ok(match r.path.as_str() {
            "/search/tv" | "/search/movie" | "/search/multi" => {
                ok(r#"{"page":1,"results":[]}"#)
            }
            "/find/424536" if source == "tvdb_id" => ok(r#"{"movie_results":[],"tv_results":[
                {"id":900024,"name":"Frieren","original_name":"Sousou no Frieren",
                 "first_air_date":"2023-09-29","poster_path":"/frieren.jpg"}]}"#),
            "/tv/900024" => ok(r#"{"id":900024,"name":"Frieren","original_name":"Sousou no Frieren",
                "original_language":"ja","first_air_date":"2023-09-29","status":"Ended",
                "poster_path":"/frieren.jpg",
                "seasons":[{"season_number":1,"episode_count":28}]}"#),
            "/movie/700001" => ok(r#"{"id":700001,"title":"Pilot Story","release_date":"2019-05-01"}"#),
            "/tv/700001" => HttpResponse {
                status: 404,
                headers: vec![],
                body: r#"{"status_code":34,"status_message":"not found"}"#.to_owned(),
            },
            other => HttpResponse {
                status: 404,
                headers: vec![],
                body: format!(r#"{{"status_message":"no fixture for {other}"}}"#),
            },
        })
    }
}

/// Fails on any call, and remembers it.
#[derive(Default)]
struct Offline {
    calls: Mutex<Vec<String>>,
}

impl HttpTransport for Offline {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.calls.lock().unwrap().push(r.url());
        Err(TransportError("offline".into()))
    }

    fn post_json(&self, r: &HttpRequest, _body: &str) -> Result<HttpResponse, TransportError> {
        self.calls.lock().unwrap().push(r.url());
        Err(TransportError("offline".into()))
    }
}

fn providers(transport: Arc<dyn HttpTransport>) -> MetadataProviders {
    let cache: Arc<dyn ProviderCache> = Arc::new(MemoryCache::default());
    let mut config = TmdbConfig::default();
    config.retry.max_retries = 0;
    MetadataProviders {
        tmdb: Some(Arc::new(TmdbClient::new(
            config,
            Keys::none().with_user_key(Some("test-key")),
            transport,
            cache,
            Arc::new(ManualClock::at_unix(1_800_000_000)),
        ))),
        anilist: None,
        shikimori: None,
        anime_mappings: None,
        anime_titles: None,
    }
}

/// The anime database with Frieren (AniDB 17617, AniList 154587, MAL
/// 52991).
fn anime_titles() -> Arc<AnimeTitles> {
    Arc::new(AnimeTitles::new(vec![OfflineEntry {
        ids: AnimeIds {
            anidb: Some(17617),
            anilist: Some(154_587),
            mal: Some(52991),
            ..Default::default()
        },
        title: "Sousou no Frieren".to_owned(),
        kind: "TV".to_owned(),
        episodes: Some(28),
        year: Some(2023),
        synonyms: vec!["Frieren: Beyond Journey's End".to_owned()],
    }]))
}

struct Lib {
    dir: tempfile::TempDir,
    cat: Catalog,
}

impl Lib {
    fn new(files: &[String]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("media");
        for (i, f) in files.iter().enumerate() {
            media(&root, f, u8::try_from(i % 250).unwrap() + 1);
        }
        let cat = Catalog::open(dir.path().join("library.db")).unwrap();
        cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
            .unwrap();
        let lib = Self { dir, cat };
        lib.scan();
        lib
    }

    fn root(&self) -> std::path::PathBuf {
        self.dir.path().join("media")
    }

    fn scan(&self) {
        self.cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    }

    fn run(&self, p: &MetadataProviders) {
        self.cat
            .fetch_metadata(
                p,
                &MetadataOptions::default(),
                &CancelToken::new(),
                &mut |_, _| {},
            )
            .unwrap();
    }

    fn db(&self) -> Connection {
        Connection::open(self.cat.path()).unwrap()
    }

    /// `(work, state, decided_by)` of a file.
    fn file(&self, rel: &str) -> (i64, String, String) {
        self.db()
            .query_row(
                "SELECT m.work_id, m.state, m.decided_by FROM files f
                 JOIN matches m ON m.file_id = f.id AND m.state <> 'rejected'
                 WHERE f.path = ?1",
                [format!("/{rel}")],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    fn kind(&self, work: i64) -> String {
        self.db()
            .query_row("SELECT kind FROM works WHERE id = ?1", [work], |r| r.get(0))
            .unwrap()
    }
}

#[test]
fn a_pasted_link_fixes_all_24_episodes_and_creates_the_folder_rule() {
    let mut files: Vec<String> = (1..=24).map(episode).collect();
    files.push(DANDADAN.to_owned());
    let lib = Lib::new(&files);
    let fake = Arc::new(Fake::default());
    let p = providers(fake.clone());
    lib.run(&p);

    // The 24 episodes are one work in the manual queue.
    let (work, _, _) = lib.file(&episode(1));
    for n in 1..=24 {
        assert_eq!(lib.file(&episode(n)).0, work, "episode {n}");
    }
    let entry = lib
        .cat
        .review_item(ItemRef::Show(work))
        .unwrap()
        .expect("in the inbox");
    assert_eq!(entry.files, 24);
    let dandadan = lib.file(DANDADAN).0;
    assert_ne!(dandadan, work);

    // The user pastes the TheTVDB link; TMDB `/find` names the series.
    let found = lib
        .cat
        .review_lookup(&p, work, " https://thetvdb.com/?tab=series&id=424536 ")
        .unwrap();
    assert_eq!(found.source.as_deref(), Some("tvdb"));
    let c = &found.candidates[0];
    assert_eq!(
        (c.provider.as_str(), c.ext_kind.as_str(), c.ext_id.as_str()),
        ("tmdb", "tv", "900024")
    );
    assert_eq!(c.title, "Frieren");
    assert_eq!(c.year, Some(2023));
    assert_eq!(c.signals.tier, "id");
    assert!(c.poster_url.as_deref().unwrap().ends_with("/frieren.jpg"));
    // Nothing is written by a lookup.
    assert!(lib.cat.review_item(ItemRef::Show(work)).unwrap().is_some());

    // One step: every episode takes it and the folder learns the rule.
    let report = lib
        .cat
        .review_apply(work, &c.provider, &c.ext_kind, &c.ext_id)
        .unwrap();
    assert_eq!(report.files, 24, "{report:?}");
    assert_eq!(report.folders, [FOLDER]);
    let target = report.work_id;
    for n in 1..=24 {
        let (w, state, _) = lib.file(&episode(n));
        assert_eq!((w, state.as_str()), (target, "user_set"), "episode {n}");
    }
    let rules = lib.cat.folder_rules(ItemRef::Show(target)).unwrap();
    assert_eq!(rules.len(), 1, "{rules:?}");
    assert_eq!(rules[0].folder, FOLDER);
    assert_eq!(rules[0].work_id, target);
    // The other show of the folder keeps its own recognition.
    assert_eq!(lib.file(DANDADAN).0, dandadan);
    assert!(lib.cat.review_item(ItemRef::Show(target)).unwrap().is_none());
    assert_eq!(lib.cat.fix_report(target).unwrap(), report);

    // A 25th episode arrives while the providers are offline: the rule
    // takes it into the series.
    media(&lib.root(), &episode(25), 25);
    lib.scan();
    let offline = Arc::new(Offline::default());
    lib.run(&providers(offline));
    let (w, state, decided_by) = lib.file(&episode(25));
    assert_eq!(
        (w, state.as_str(), decided_by.as_str()),
        (target, "user_set", "rule")
    );
    assert_eq!(lib.cat.fix_report(target).unwrap().files, 25);
}

#[test]
fn a_result_of_another_kind_switches_the_type() {
    let file = "Movies/Pilot/Pilot Story (2019) 1080p.mkv".to_owned();
    let lib = Lib::new(std::slice::from_ref(&file));
    let p = providers(Arc::new(Fake::default()));
    let (work, _, _) = lib.file(&file);
    assert_eq!(lib.kind(work), "movie");

    // Bare digits: a TMDB id of the work's kind.
    let found = lib.cat.review_lookup(&p, work, "700001").unwrap();
    assert_eq!(found.source.as_deref(), Some("tmdb"));
    assert_eq!(found.candidates.len(), 1);
    assert_eq!(found.candidates[0].ext_kind, "movie");
    assert_eq!(found.candidates[0].title, "Pilot Story");

    // Not an id: nothing understood, no request.
    let none = lib.cat.review_lookup(&p, work, "Pilot Story").unwrap();
    assert_eq!(none.source, None);
    assert!(none.candidates.is_empty());

    // The user takes a TMDB series for the «movie»: it becomes a series.
    let report = lib.cat.review_apply(work, "tmdb", "tv", "900024").unwrap();
    assert_eq!(report.kind, "show");
    assert_eq!(report.files, 1);
    assert_eq!(lib.kind(report.work_id), "show");
    assert_eq!(lib.file(&file).1, "user_set");
}

#[test]
fn anime_ids_resolve_through_the_anime_database() {
    let files: Vec<String> = (1..=3).map(episode).collect();
    let lib = Lib::new(&files);
    let mut p = providers(Arc::new(Fake::default()));
    p.anime_titles = Some(anime_titles());
    let (work, _, _) = lib.file(&episode(1));

    // MAL, Shikimori (a MAL id) and AniDB name the same AniList entry.
    for text in [
        "https://myanimelist.net/anime/52991/Sousou_no_Frieren",
        "https://shikimori.one/animes/z52991-sousou-no-frieren",
        "anidb 17617",
    ] {
        let found = lib.cat.review_lookup(&p, work, text).unwrap();
        let c = found
            .candidates
            .iter()
            .find(|c| c.provider == "anilist")
            .unwrap_or_else(|| panic!("{text}: {found:?}"));
        assert_eq!(c.ext_id, "154587", "{text}");
        assert_eq!(c.title, "Sousou no Frieren");
        assert_eq!(c.year, Some(2023));
    }

    // The manual search finds it in the anime database (TMDB knows
    // nothing).
    let found = lib
        .cat
        .review_search(&p, work, "Sousou no Frieren", None)
        .unwrap();
    assert!(
        found
            .iter()
            .any(|c| c.provider == "anilist" && c.ext_id == "154587"),
        "{found:?}"
    );

    // Taking the AniList entry makes the work an anime.
    let report = lib.cat.review_apply(work, "anilist", "", "154587").unwrap();
    assert_eq!(report.kind, "anime");
    assert_eq!(report.files, 3);
    assert_eq!(report.folders, [FOLDER]);
}
