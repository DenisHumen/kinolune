// SPDX-License-Identifier: MPL-2.0
//! LIB-025 end to end on a real folder: one source holds movies, a show,
//! phone clips, an album and an audiobook, and the user picks nothing.
//! Phone clips land in «Мои видео» and never reach a provider; audiobooks
//! are marked on their tracks; «Это не то» on a folder flips its kind,
//! survives a rescan and flips back.

use std::path::Path;
use std::sync::{Arc, Mutex};

use pa_catalog::pa_metadata::cache::MemoryCache;
use pa_catalog::pa_metadata::clock::ManualClock;
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::tmdb::{TmdbClient, TmdbConfig};
use pa_catalog::{Catalog, ContentKind, MetadataOptions, MetadataProviders};
use pa_scanner::CancelToken;

fn media(root: &Path, rel: &str, seed: u8) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..150_000u32)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    std::fs::write(path, data).unwrap();
}

const DUNE: &str = "Movies/Dune (2021)/Dune.2021.2160p.mkv";
const ARRIVAL: &str = "Movies/Arrival (2016)/Arrival.2016.1080p.mkv";
const DARK1: &str = "Shows/Dark/Season 1/Dark.S01E01.1080p.mkv";
const CLIP1: &str = "Phone/DCIM/Camera/VID_20240612_183000.mp4";
const CLIP2: &str = "Phone/DCIM/Camera/VID_20240613_101500.mp4";
/// No camera name of its own: the folder of clips says what it is.
const CLIP3: &str = "Phone/DCIM/Camera/Birthday.mp4";
const SONG1: &str = "Music/Band/2019 - Album/01 - Song.mp3";
const SONG2: &str = "Music/Band/2019 - Album/02 - Other Song.mp3";
const BOOK1: &str = "Аудиокниги/Автор - Книга (читает Чтец)/01.mp3";
const BOOK2: &str = "Аудиокниги/Автор - Книга (читает Чтец)/02.mp3";

/// Records every provider request and answers "nothing found".
#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<String>>,
}

impl HttpTransport for Recorder {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let q = r
            .query
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        self.calls.lock().unwrap().push(format!("{} {q}", r.path));
        Ok(HttpResponse {
            status: 200,
            headers: vec![],
            body: r#"{"page":1,"results":[],"total_pages":0,"total_results":0}"#.into(),
        })
    }

    fn post_json(&self, r: &HttpRequest, body: &str) -> Result<HttpResponse, TransportError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{} {body}", r.path));
        Ok(HttpResponse {
            status: 200,
            headers: vec![],
            body: r#"{"data":{"Page":{"media":[]}}}"#.into(),
        })
    }
}

fn tmdb(rec: &Arc<Recorder>) -> MetadataProviders {
    let transport: Arc<dyn HttpTransport> = rec.clone();
    let mut config = TmdbConfig::default();
    config.retry.max_retries = 0;
    MetadataProviders {
        tmdb: Some(Arc::new(TmdbClient::new(
            config,
            Keys::none().with_user_key(Some("test-key")),
            transport,
            Arc::new(MemoryCache::default()),
            Arc::new(ManualClock::at_unix(1_800_000_000)),
        ))),
        anilist: None,
        shikimori: None,
        anime_mappings: None,
        anime_titles: None,
    }
}

struct Lib {
    _dir: tempfile::TempDir,
    cat: Catalog,
    source: i64,
}

impl Lib {
    fn scan(&self) {
        self.cat
            .scan(None, &CancelToken::new(), &mut |_| {})
            .unwrap();
    }

    fn movie_titles(&self) -> Vec<String> {
        let mut t: Vec<String> = self
            .cat
            .movies()
            .unwrap()
            .into_iter()
            .map(|m| m.title)
            .collect();
        t.sort();
        t
    }

    fn own(&self) -> Vec<String> {
        let mut t: Vec<String> = self
            .cat
            .own_videos()
            .unwrap()
            .into_iter()
            .map(|v| v.rel_path)
            .collect();
        t.sort();
        t
    }

    fn audiobooks(&self) -> Vec<String> {
        let mut t: Vec<String> = self
            .cat
            .music_tracks()
            .unwrap()
            .into_iter()
            .filter(|t| t.audiobook)
            .map(|t| t.item.rel_path)
            .collect();
        t.sort();
        t
    }

    fn kind(&self, rel: &str) -> Option<ContentKind> {
        self.cat.content_kind(self.source, rel).unwrap()
    }
}

fn setup() -> Lib {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("media");
    for (i, rel) in [
        DUNE, ARRIVAL, DARK1, CLIP1, CLIP2, CLIP3, SONG1, SONG2, BOOK1, BOOK2,
    ]
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
    let lib = Lib {
        _dir: dir,
        cat,
        source,
    };
    lib.scan();
    lib
}

#[test]
fn one_source_holds_everything_and_the_user_picks_nothing() {
    let lib = setup();
    assert_eq!(lib.kind(DUNE), Some(ContentKind::Video));
    assert_eq!(lib.kind(DARK1), Some(ContentKind::Video));
    for clip in [CLIP1, CLIP2, CLIP3] {
        assert_eq!(lib.kind(clip), Some(ContentKind::OwnVideo), "{clip}");
    }
    assert_eq!(lib.kind(SONG1), Some(ContentKind::Music));
    assert_eq!(lib.kind(BOOK1), Some(ContentKind::Audiobook));

    // Movies and shows as before; the clips are «Мои видео», not items.
    assert_eq!(lib.movie_titles(), ["Arrival", "Dune"]);
    assert_eq!(lib.cat.shows().unwrap().len(), 1);
    assert_eq!(lib.own(), [CLIP3, CLIP1, CLIP2]);
    let own = lib.cat.own_videos().unwrap();
    assert!(own.iter().all(|v| v.local_path.is_some()));
    assert!(own.iter().any(|v| v.title == "Birthday"));

    // Audiobooks are tracks of the music library, marked.
    assert_eq!(lib.audiobooks(), [BOOK1, BOOK2]);
    assert_eq!(lib.cat.music_tracks().unwrap().len(), 4);

    // The metadata run never sends a clip to a provider.
    let rec = Arc::new(Recorder::default());
    lib.cat
        .fetch_metadata(
            &tmdb(&rec),
            &MetadataOptions::default(),
            &CancelToken::new(),
            &mut |_, _| {},
        )
        .unwrap();
    let calls = rec.calls.lock().unwrap().clone();
    assert!(calls.iter().any(|c| c.contains("Dune")), "{calls:?}");
    for word in ["VID", "Birthday", "20240612", "Camera"] {
        assert!(
            calls.iter().all(|c| !c.contains(word)),
            "{word} went to a provider: {calls:?}"
        );
    }
}

#[test]
fn not_this_on_a_folder_flips_its_kind_and_is_remembered() {
    let lib = setup();
    // A movie folder that is the user's own footage.
    assert_eq!(
        lib.cat.not_this(lib.source, "Movies/Arrival (2016)").unwrap(),
        Some(ContentKind::OwnVideo)
    );
    assert_eq!(lib.movie_titles(), ["Dune"]);
    assert!(lib.own().contains(&ARRIVAL.to_owned()));
    let kinds = lib.cat.folder_kinds(lib.source, "Movies").unwrap();
    let arrival = kinds
        .iter()
        .find(|k| k.path == "Movies/Arrival (2016)")
        .unwrap();
    assert_eq!(arrival.kind, ContentKind::OwnVideo);
    assert!(arrival.overridden);
    let dune = kinds.iter().find(|k| k.path == "Movies/Dune (2021)").unwrap();
    assert_eq!(dune.kind, ContentKind::Video);
    assert!(!dune.overridden);

    // Remembered: a rescan keeps it.
    lib.scan();
    assert_eq!(lib.kind(ARRIVAL), Some(ContentKind::OwnVideo));
    assert_eq!(lib.movie_titles(), ["Dune"]);

    // The same action again brings back what was detected.
    assert_eq!(
        lib.cat.not_this(lib.source, "Movies/Arrival (2016)").unwrap(),
        Some(ContentKind::Video)
    );
    assert_eq!(lib.movie_titles(), ["Arrival", "Dune"]);
    assert!(!lib.own().contains(&ARRIVAL.to_owned()));
    assert!(
        lib.cat
            .folder_kinds(lib.source, "Movies")
            .unwrap()
            .iter()
            .all(|k| !k.overridden)
    );

    // Clips the classifier got wrong: «Это не то» makes them videos, which
    // are recognized right away.
    assert_eq!(
        lib.cat.not_this(lib.source, "Phone").unwrap(),
        Some(ContentKind::Video)
    );
    assert!(lib.own().is_empty());
    assert_eq!(lib.kind(CLIP3), Some(ContentKind::Video));
    assert!(lib.movie_titles().iter().any(|t| t == "Birthday"));
    assert_eq!(
        lib.cat.not_this(lib.source, "Phone").unwrap(),
        Some(ContentKind::OwnVideo)
    );
    assert_eq!(lib.own(), [CLIP3, CLIP1, CLIP2]);
    assert_eq!(lib.movie_titles(), ["Arrival", "Dune"]);

    // Audio: an album that is a book, and back.
    assert_eq!(
        lib.cat.not_this(lib.source, "Music/Band").unwrap(),
        Some(ContentKind::Audiobook)
    );
    assert_eq!(lib.audiobooks(), [SONG1, SONG2, BOOK1, BOOK2]);
    assert_eq!(
        lib.cat.not_this(lib.source, "Music/Band").unwrap(),
        Some(ContentKind::Music)
    );
    assert_eq!(lib.audiobooks(), [BOOK1, BOOK2]);

    // A folder without media says so.
    assert_eq!(lib.cat.not_this(lib.source, "Nowhere").unwrap(), None);
}
