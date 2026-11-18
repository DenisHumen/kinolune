// SPDX-License-Identifier: MPL-2.0
//! META-032: an anime folder named in Russian (kind `show` after the scan)
//! that TMDB finds by an alternative title but has no Russian translation
//! and no poster for (its details answer with the Japanese name): the
//! anime providers are asked too, the card shows the Russian title (or the
//! English one in an English UI) and the Shikimori poster. Synthetic
//! provider responses, no network.

use std::path::Path;
use std::sync::Arc;

use pa_catalog::pa_metadata::anilist::{AniListClient, AniListConfig};
use pa_catalog::pa_metadata::cache::MemoryCache;
use pa_catalog::pa_metadata::clock::ManualClock;
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::shikimori::{ShikimoriClient, ShikimoriConfig};
use pa_catalog::pa_metadata::tmdb::{TmdbClient, TmdbConfig};
use pa_catalog::{Catalog, MetadataOptions, MetadataProviders};
use pa_scanner::CancelToken;

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..120_000u32)
        .map(|i| (i as u8).wrapping_mul(29).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

fn ok(body: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: vec![],
        body: body.to_owned(),
    }
}

struct Fake;

impl Fake {
    fn respond(r: &HttpRequest, body: Option<&str>) -> HttpResponse {
        let query = r
            .query
            .iter()
            .find(|(k, _)| k == "query")
            .map_or("", |(_, v)| v.as_str());
        match r.path.as_str() {
            "/search/tv" if query == "Звёздный кот" => ok(r#"{"page":1,"results":[
                {"id":9001,"name":"Звёздный кот","original_name":"星の猫",
                 "first_air_date":"2004-04-01","original_language":"ja"}
            ]}"#),
            "/search/tv" | "/search/movie" => ok(r#"{"page":1,"results":[]}"#),
            "/tv/9001" => ok(r#"{"id":9001,"name":"星の猫","original_name":"星の猫",
                "original_language":"ja","first_air_date":"2004-04-01","status":"Ended",
                "genres":[{"id":16,"name":"Animation"}],
                "seasons":[{"season_number":1,"episode_count":2}]}"#),
            "/tv/9001/translations" => ok(r#"{"id":9001,"translations":[
                {"iso_3166_1":"US","iso_639_1":"en","data":{"name":"Star Cat"}}
            ]}"#),
            "/tv/9001/season/1" => ok(r#"{"id":1,"season_number":1,"episodes":[]}"#),
            "/animes/5001" => ok(
                r#"{"id":5001,"name":"Hoshi no Neko","russian":"Звёздный кот",
                "kind":"tv","status":"released","aired_on":"2004-04-01","myanimelist_id":5001,
                "image":{"original":"/system/animes/original/5001.jpg?1700000000",
                         "preview":"/system/animes/preview/5001.jpg"},
                "rating":"pg_13","english":["Star Cat"],"japanese":[],"synonyms":[]}"#,
            ),
            // AniList finds it by the English or the original title TMDB gave.
            "" | "/" if body.is_some_and(|b| b.contains("Star Cat") || b.contains("星の猫")) => {
                ok(r#"{"data":{"Page":{"media":[
                {"id":7001,"idMal":5001,"format":"TV","status":"FINISHED","episodes":2,"duration":24,
                 "title":{"romaji":"Hoshi no Neko","english":"Star Cat","native":"星の猫"},
                 "synonyms":[],"startDate":{"year":2004,"month":4,"day":1},
                 "coverImage":{"large":null}}
            ]}}}"#)
            }
            "" | "/" => ok(r#"{"data":{"Page":{"media":[]}}}"#),
            other => HttpResponse {
                status: 404,
                headers: vec![],
                body: format!(r#"{{"status_message":"no fixture for {other}"}}"#),
            },
        }
    }
}

struct Transport;

impl HttpTransport for Transport {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        Ok(Fake::respond(r, None))
    }

    fn post_json(&self, r: &HttpRequest, body: &str) -> Result<HttpResponse, TransportError> {
        Ok(Fake::respond(r, Some(body)))
    }
}

fn providers(language: &str) -> MetadataProviders {
    let transport: Arc<dyn HttpTransport> = Arc::new(Transport);
    let cache = Arc::new(MemoryCache::default());
    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let mut tmdb_config = TmdbConfig {
        language: language.to_owned(),
        ..Default::default()
    };
    tmdb_config.retry.max_retries = 0;
    let mut anilist_config = AniListConfig::default();
    anilist_config.retry.max_retries = 0;
    let mut shiki_config = ShikimoriConfig::default();
    shiki_config.retry.max_retries = 0;
    MetadataProviders {
        tmdb: Some(Arc::new(TmdbClient::new(
            tmdb_config,
            Keys::none().with_user_key(Some("test-key")),
            transport.clone(),
            cache.clone(),
            clock.clone(),
        ))),
        anilist: Some(Arc::new(AniListClient::new(
            anilist_config,
            transport.clone(),
            cache.clone(),
            clock.clone(),
        ))),
        shikimori: Some(Arc::new(ShikimoriClient::new(
            shiki_config,
            transport,
            cache,
            clock,
        ))),
        anime_mappings: None,
        anime_titles: None,
        extra: Default::default(),
    }
}

fn library(language: &str) -> (tempfile::TempDir, Catalog) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    for n in 1..=2u8 {
        media(
            &root,
            &format!("Звёздный кот/Звёздный кот - S01E0{n} (Озвучка AniLibria).mp4"),
            n,
        );
    }
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let report = cat
        .fetch_metadata(
            &providers(language),
            &MetadataOptions::default(),
            &CancelToken::new(),
            &mut |_, _| {},
        )
        .unwrap();
    assert_eq!(report.matched, 1, "{report:?}");
    (dir, cat)
}

#[test]
fn a_russian_anime_folder_gets_the_russian_title_and_the_shikimori_poster() {
    let (dir, cat) = library("ru-RU");
    let shows = cat.shows().unwrap();
    assert_eq!(shows.len(), 1, "{shows:?}");
    let show = &shows[0];
    assert_eq!(show.title, "Звёздный кот", "not TMDB's untranslated name");
    assert_eq!(show.tmdb_id, Some(9001));
    assert_eq!(
        show.poster_url.as_deref(),
        Some("https://shikimori.io/system/animes/original/5001.jpg")
    );
    let conn = pa_library::rusqlite::Connection::open(dir.path().join("library.db")).unwrap();
    let kind: String = conn
        .query_row("SELECT kind FROM works", [], |r| r.get(0))
        .unwrap();
    assert_eq!(kind, "anime", "the anime providers recognised the show");
}

#[test]
fn an_english_ui_gets_the_english_title_not_the_native_one() {
    let (_dir, cat) = library("en-US");
    let shows = cat.shows().unwrap();
    assert_eq!(shows[0].title, "Star Cat");
}
