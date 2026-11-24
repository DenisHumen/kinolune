// SPDX-License-Identifier: MPL-2.0
//! Subtitles from OpenSubtitles (PLAY-008) for a file of the library, on
//! recorded (synthetic) responses: no network. The search asks by the
//! file's OSHash and its IMDb id, ranks the offers, and the download is
//! kept byte for byte (an ASS file keeps its styles): next to the video on
//! a writable source, in the app's cache on a read-only one.

use std::path::Path;
use std::sync::Arc;

use pa_catalog::Catalog;
use pa_catalog::pa_metadata::clock::ManualClock;
use pa_catalog::pa_metadata::http::HttpResponse;
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::opensubtitles::{
    OpenSubtitlesClient, OpenSubtitlesConfig, OpenSubtitlesError,
};
use pa_catalog::pa_metadata::replay::ReplayTransport;
use pa_scanner::CancelToken;

const SAMPLE_ASS: &str = include_str!("../../pa-metadata/tests/fixtures/opensubtitles/sample.ass");
const VIDEO: &str = "Movies/Lunar Harbor (2019)/Lunar.Harbor.2019.1080p.WEB-DL.mkv";
const LINK_PATH: &str = "/download/SYNTHETIC7002/subfile/Lunar.Harbor.2019.1080p.WEB-DL.ass";

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

fn ok(body: String) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: vec![],
        body,
    }
}

fn subtitle(id: u64, lang: &str, downloads: u64, hash: bool) -> serde_json::Value {
    serde_json::json!({
        "id": id.to_string(),
        "type": "subtitle",
        "attributes": {
            "subtitle_id": id.to_string(),
            "language": lang,
            "download_count": downloads,
            "hearing_impaired": false,
            "foreign_parts_only": false,
            "ai_translated": false,
            "machine_translated": false,
            "from_trusted": false,
            "moviehash_match": hash,
            "fps": 23.976,
            "release": format!("Lunar.Harbor.2019.{id}"),
            "files": [{"file_id": id, "cd_number": 1, "file_name": "x"}],
        }
    })
}

/// The OpenSubtitles answers for the file with OSHash `hash`.
fn transport(hash: &str) -> Arc<ReplayTransport> {
    let t = Arc::new(ReplayTransport::default());
    let page = serde_json::json!({
        "total_pages": 1, "page": 1,
        "data": [
            subtitle(7001, "en", 5000, false),
            subtitle(7003, "ru", 900, false),
            subtitle(7002, "ru", 30, true),
        ],
    });
    t.push(
        &format!("/subtitles?imdb_id=900001&languages=en%2Cru&moviehash={hash}&type=movie"),
        ok(page.to_string()),
    );
    t.push(
        r#"/download {"file_id":7002}"#,
        ok(serde_json::json!({
            "link": format!("https://www.opensubtitles.com{LINK_PATH}"),
            "file_name": "Lunar.Harbor.2019.1080p.WEB-DL.ass",
            "requests": 3,
            "remaining": 2,
            "reset_time_utc": "2026-11-18T20:00:00.000Z",
        })
        .to_string()),
    );
    t.push(LINK_PATH, ok(SAMPLE_ASS.to_owned()));
    t.push(
        r#"/download {"file_id":7003}"#,
        HttpResponse {
            status: 406,
            headers: vec![],
            body: r#"{"remaining":-1,"reset_time_utc":"2026-11-18T03:00:00.000Z"}"#.into(),
        },
    );
    t
}

fn client(t: &Arc<ReplayTransport>) -> OpenSubtitlesClient {
    OpenSubtitlesClient::new(
        OpenSubtitlesConfig::default(),
        Keys::none().with_project_key(Some("project-key-1")),
        None,
        Arc::clone(t) as _,
        Arc::new(ManualClock::at_unix(1_800_000_000)),
    )
}

/// A library with the movie, recognised locally, given an IMDb id as the
/// metadata step would.
fn library(dir: &Path) -> (Catalog, i64, std::path::PathBuf) {
    let root = dir.join("media");
    media(&root, VIDEO, 1);
    let db = dir.join("db/library.db");
    let cat = Catalog::open(&db).unwrap();
    cat.add_source(None, "local", "Media", root.to_str().unwrap(), None)
        .unwrap();
    cat.scan(None, &CancelToken::new(), &mut |_| {}).unwrap();
    let source = cat.sources().unwrap()[0].id;
    let conn = pa_library::open(&db).unwrap();
    let n = conn
        .execute(
            "INSERT INTO external_ids (work_id, provider, ext_kind, ext_id)
             SELECT work_id, 'imdb', '', 'tt0900001' FROM matches WHERE state <> 'rejected'",
            [],
        )
        .unwrap();
    assert_eq!(n, 1, "the movie is matched to a work");
    (cat, source, root)
}

#[test]
fn search_ranks_and_download_lands_next_to_the_video() {
    let dir = tempfile::tempdir().unwrap();
    let (cat, source, root) = library(dir.path());
    let target = cat.subtitle_target(source, VIDEO).unwrap();
    let hash = target.oshash.clone().expect("the scan hashed the file");
    assert_eq!(target.kind.as_deref(), Some("movie"));
    assert_eq!(target.imdb_id.as_deref(), Some("tt0900001"));

    let t = transport(&hash);
    let os = client(&t);
    let offers = cat
        .search_subtitles(&os, source, VIDEO, &["rus".to_owned()])
        .unwrap();
    let order: Vec<u64> = offers.iter().map(|o| o.file_id).collect();
    // In sync with the file first, then Russian, then the rest.
    assert_eq!(order, [7002, 7003, 7001]);
    assert!(offers[0].hash_match);
    assert_eq!(offers[0].language.as_deref(), Some("rus"));

    let cache = dir.path().join("cache/subtitles");
    let saved = cat
        .download_subtitle(&os, source, VIDEO, &offers[0], &cache)
        .unwrap();
    assert!(saved.next_to_video);
    assert_eq!(
        saved.sidecar.rel_path,
        "Movies/Lunar Harbor (2019)/Lunar.Harbor.2019.1080p.WEB-DL.ru.ass"
    );
    let written = std::fs::read(root.join(&saved.sidecar.rel_path)).unwrap();
    assert_eq!(written, SAMPLE_ASS.as_bytes(), "ASS kept byte for byte");
    let text = String::from_utf8(written).unwrap();
    assert!(text.contains("[V4+ Styles]") && text.contains("Style: Sign,Georgia"));
    assert_eq!(os.quota().unwrap().remaining, Some(2));
    assert!(
        !cache.exists(),
        "nothing in the cache for a writable source"
    );

    // The library knows it as a sidecar now, with its language.
    let subs = cat.sidecar_subtitles(source, VIDEO).unwrap();
    let sub = subs
        .iter()
        .find(|s| s.rel_path == saved.sidecar.rel_path)
        .expect("refreshed after the write");
    assert_eq!(sub.lang.as_deref(), Some("rus"));

    // A second copy does not overwrite the first.
    let again = cat
        .download_subtitle(&os, source, VIDEO, &offers[0], &cache)
        .unwrap();
    assert_eq!(
        again.sidecar.rel_path,
        "Movies/Lunar Harbor (2019)/Lunar.Harbor.2019.1080p.WEB-DL.ru.2.ass"
    );

    // The quota used up is an error, and nothing is written.
    let err = cat
        .download_subtitle(&os, source, VIDEO, &offers[1], &cache)
        .unwrap_err();
    assert!(matches!(
        err,
        pa_catalog::SubtitleSearchError::Provider(OpenSubtitlesError::QuotaExhausted { .. })
    ));
    assert_eq!(os.quota().unwrap().remaining, Some(0));
}

#[cfg(unix)]
#[test]
fn read_only_source_keeps_the_download_in_the_cache() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let (cat, source, root) = library(dir.path());
    let hash = cat.subtitle_target(source, VIDEO).unwrap().oshash.unwrap();
    let t = transport(&hash);
    let os = client(&t);
    let offers = cat
        .search_subtitles(&os, source, VIDEO, &["ru".to_owned(), "en".to_owned()])
        .unwrap();

    let folders = [
        root.clone(),
        root.join("Movies"),
        root.join("Movies/Lunar Harbor (2019)"),
    ];
    let set = |mode: u32| {
        for f in &folders {
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(mode)).unwrap();
        }
    };
    set(0o555);
    let cache = dir.path().join("cache/subtitles");
    let saved = cat.download_subtitle(&os, source, VIDEO, &offers[0], &cache);
    set(0o755);
    let saved = saved.unwrap();
    assert!(!saved.next_to_video);
    let path = saved.sidecar.local_path.clone().unwrap();
    assert!(Path::new(&path).starts_with(cache.join(&hash)), "{path}");
    assert!(path.ends_with("Lunar.Harbor.2019.1080p.WEB-DL.ru.ass"));
    assert_eq!(std::fs::read(&path).unwrap(), SAMPLE_ASS.as_bytes());
    assert!(
        !root
            .join("Movies/Lunar Harbor (2019)/Lunar.Harbor.2019.1080p.WEB-DL.ru.ass")
            .exists()
    );

    // Attached to the file: listed for it, with language from the name.
    let cached = cat.cached_subtitles(source, VIDEO, &cache).unwrap();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].local_path.as_deref(), Some(path.as_str()));
    assert_eq!(cached[0].lang.as_deref(), Some("rus"));
}

#[test]
fn a_file_without_hash_or_ids_is_not_searched() {
    let dir = tempfile::tempdir().unwrap();
    let (cat, source, _) = library(dir.path());
    let t = Arc::new(ReplayTransport::default());
    let os = client(&t);
    assert!(cat.subtitle_target(source, "Movies/missing.mkv").is_err());
    // An unknown file is an error, not a request.
    assert!(
        cat.search_subtitles(&os, source, "Movies/missing.mkv", &[])
            .is_err()
    );
    assert!(t.requests().is_empty());
}

// ---------------------------------------------------------------------------
// Subdl as a second provider (PLAY-016)

mod subdl_provider {
    use super::*;
    use pa_catalog::SubtitleProviders;
    use pa_catalog::pa_metadata::http::{
        BytesResponse, HttpRequest, HttpTransport, TransportError,
    };
    use pa_catalog::pa_metadata::subdl::{SubdlClient, SubdlConfig, SubdlError};

    const CASSETTE: &str = include_str!("../../pa-metadata/tests/fixtures/subdl/cassette.jsonl");
    const SUBDL_ASS: &[u8] = include_bytes!("../../pa-metadata/tests/fixtures/subdl/sample.ass");
    const MOVIE_ZIP: &[u8] = include_bytes!("../../pa-metadata/tests/fixtures/subdl/movie.zip");

    /// Subdl searches from its cassette, the movie's ZIP as bytes.
    struct Server(ReplayTransport);

    impl HttpTransport for Server {
        fn get(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
            self.0.get(request)
        }

        fn get_bytes(&self, request: &HttpRequest) -> Result<BytesResponse, TransportError> {
            let found = request.path == "/subtitle/900101-900201.zip";
            Ok(BytesResponse {
                status: if found { 200 } else { 404 },
                headers: Vec::new(),
                body: if found {
                    MOVIE_ZIP.to_vec()
                } else {
                    Vec::new()
                },
            })
        }
    }

    fn subdl(key: Option<&str>) -> SubdlClient {
        SubdlClient::new(
            SubdlConfig::default(),
            Keys::none().with_project_key(key),
            Arc::new(Server(ReplayTransport::from_jsonl(CASSETTE).unwrap())),
            Arc::new(ManualClock::at_unix(1_800_000_000)),
        )
    }

    fn summary(offers: &[pa_catalog::SubtitleOffer]) -> Vec<String> {
        offers
            .iter()
            .map(|o| format!("{}:{}", o.provider, o.language.as_deref().unwrap_or("?")))
            .collect()
    }

    #[test]
    fn offers_of_both_merge_and_a_subdl_zip_lands_next_to_the_video() {
        let dir = tempfile::tempdir().unwrap();
        let (cat, source, root) = library(dir.path());
        let hash = cat.subtitle_target(source, VIDEO).unwrap().oshash.unwrap();
        let t = transport(&hash);
        let os = client(&t);
        let sd = subdl(Some("subdl-project-key"));
        let found = cat
            .search_subtitles_from(
                SubtitleProviders {
                    opensubtitles: Some(&os),
                    subdl: Some(&sd),
                },
                source,
                VIDEO,
                &["rus".to_owned()],
            )
            .unwrap();
        assert!(found.failures.is_empty(), "{:?}", found.failures);
        assert_eq!(found.asked, 2);
        // In sync with the file, then Russian by downloads, then English,
        // then the rest — one ranking across both providers.
        assert_eq!(
            summary(&found.offers),
            [
                "opensubtitles:rus",
                "opensubtitles:rus",
                "subdl:rus",
                "opensubtitles:eng",
                "subdl:eng",
                "subdl:por",
            ]
        );
        let ru = &found.offers[2];
        assert_eq!(
            ru.download_url.as_deref(),
            Some("/subtitle/900101-900201.zip")
        );
        assert_eq!(ru.release, "Lunar.Harbor.2019.1080p.BluRay");
        assert!(found.offers[4].hearing_impaired);

        let cache = dir.path().join("cache/subtitles");
        let saved = cat
            .download_subdl_subtitle(&sd, source, VIDEO, ru, &cache)
            .unwrap();
        assert!(saved.next_to_video);
        assert_eq!(
            saved.sidecar.rel_path,
            "Movies/Lunar Harbor (2019)/Lunar.Harbor.2019.1080p.WEB-DL.ru.ass"
        );
        let written = std::fs::read(root.join(&saved.sidecar.rel_path)).unwrap();
        assert_eq!(written, SUBDL_ASS, "unzipped, ASS kept byte for byte");
        assert_eq!(saved.sidecar.lang.as_deref(), Some("rus"));

        // An archive that is not there is Subdl's error, nothing written.
        let gone = found.offers[4].clone();
        assert!(matches!(
            cat.download_subdl_subtitle(&sd, source, VIDEO, &gone, &cache),
            Err(pa_catalog::SubtitleSearchError::Subdl(SubdlError::Http {
                status: 404,
                ..
            }))
        ));
    }

    #[test]
    fn a_failing_provider_keeps_the_others_offers() {
        let dir = tempfile::tempdir().unwrap();
        let (cat, source, _) = library(dir.path());
        let hash = cat.subtitle_target(source, VIDEO).unwrap().oshash.unwrap();

        // OpenSubtitles unreachable (nothing recorded): Subdl's offers stay.
        let broken = Arc::new(ReplayTransport::default());
        let os = client(&broken);
        let sd = subdl(Some("subdl-project-key"));
        let found = cat
            .search_subtitles_from(
                SubtitleProviders {
                    opensubtitles: Some(&os),
                    subdl: Some(&sd),
                },
                source,
                VIDEO,
                &["rus".to_owned()],
            )
            .unwrap();
        assert_eq!(found.offers.len(), 3);
        assert!(found.offers.iter().all(|o| o.provider == "subdl"));
        assert!(matches!(
            found.failures.as_slice(),
            [pa_catalog::SubtitleSearchError::Provider(_)]
        ));

        // No Subdl key: nothing is sent to it, OpenSubtitles' offers stay.
        let t = transport(&hash);
        let os = client(&t);
        let keyless = subdl(None);
        let found = cat
            .search_subtitles_from(
                SubtitleProviders {
                    opensubtitles: Some(&os),
                    subdl: Some(&keyless),
                },
                source,
                VIDEO,
                &["rus".to_owned()],
            )
            .unwrap();
        assert_eq!(found.offers.len(), 3);
        assert!(found.offers.iter().all(|o| o.provider == "opensubtitles"));
        assert!(matches!(
            found.failures.as_slice(),
            [pa_catalog::SubtitleSearchError::Subdl(SubdlError::NoKey)]
        ));
    }
}
