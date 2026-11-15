// SPDX-License-Identifier: MPL-2.0
//! Music metadata (META-021) end to end on a synthetic collection: 200
//! made-up albums with made-up MBIDs, a fake MusicBrainz and Cover Art
//! Archive (no network, no real data), the library afterwards.
//!
//! Done-when of META-021: the collection matches by MBID on at least 95 %.
//! The collection mixes how albums are tagged — release MBIDs (Picard),
//! release-group MBIDs only, and plain tags (title, album artist, track
//! numbers, durations) with editions of the same album, same-title albums
//! of other artists, missing files and compilations as decoys.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pa_catalog::music_metadata::{MusicProviders, MusicMetadataOptions};
use pa_catalog::pa_metadata::cache::MemoryCache;
use pa_catalog::pa_metadata::clock::{Clock, ManualClock};
use pa_catalog::pa_metadata::coverart::{CoverArtClient, CoverArtConfig};
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::musicbrainz::{MusicBrainzClient, MusicBrainzConfig};
use pa_catalog::Catalog;
use pa_library::music::{self, AudioInfo, TrackTags};
use pa_library::rusqlite::Connection;
use pa_scanner::CancelToken;
use serde_json::{Value, json};

const ALBUMS: usize = 200;

/// Deterministic made-up MBID of entity `kind` number `n`.
fn id(kind: u32, n: usize) -> String {
    let n = n as u64;
    format!(
        "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
        0x5000_0000 + kind,
        (n >> 16) & 0xffff,
        n & 0xfff,
        kind & 0xfff,
        0x00ab_cdef_0000 + n
    )
}

const RELEASE: u32 = 1;
const GROUP: u32 = 2;
const ARTIST: u32 = 3;
const DECOY: u32 = 4;
const OTHER: u32 = 5;

/// Small deterministic PRNG (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const WORDS: &[&str] = &[
    "Северный",
    "ветер",
    "Glass",
    "Harbor",
    "Lumen",
    "Тихий",
    "Orbit",
    "Paper",
    "Moth",
    "Река",
    "Static",
    "Velvet",
    "Ember",
    "Дом",
    "Signal",
    "Hollow",
];

/// One album of the synthetic world.
#[derive(Debug, Clone)]
struct World {
    title: String,
    artist: String,
    artist_n: usize,
    year: i64,
    /// Durations by disc.
    discs: Vec<Vec<u64>>,
    compilation: bool,
}

fn world() -> Vec<World> {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    (0..ALBUMS)
        .map(|i| {
            let artist_n = i % 70;
            let compilation = i % 23 == 5;
            // Every tenth title is reused by another artist.
            let base = if i % 10 == 3 { i - 1 } else { i };
            let title = format!(
                "{} {} {}",
                WORDS[base % WORDS.len()],
                WORDS[(base / 3 + 5) % WORDS.len()],
                base
            );
            let disc_count = if i.is_multiple_of(17) { 2 } else { 1 };
            let discs = (0..disc_count)
                .map(|_| {
                    let n = 6 + rng.below(11) as usize;
                    (0..n).map(|_| 120_000 + rng.below(300_000)).collect()
                })
                .collect();
            World {
                title,
                artist: if compilation {
                    "Various Artists".into()
                } else {
                    format!("Synth Artist {artist_n}")
                },
                artist_n,
                year: 1970 + (i % 50) as i64,
                discs,
                compilation,
            }
        })
        .collect()
}

fn credit(w: &World) -> Value {
    let artist_id = if w.compilation {
        id(ARTIST, 999)
    } else {
        id(ARTIST, w.artist_n)
    };
    json!([{"name": w.artist, "joinphrase": "",
            "artist": {"id": artist_id, "name": w.artist, "sort-name": w.artist}}])
}

fn media(discs: &[Vec<u64>], with_tracks: bool) -> Value {
    Value::Array(
        discs
            .iter()
            .enumerate()
            .map(|(d, tracks)| {
                let mut m = json!({"position": d + 1, "format": "Digital Media",
                                   "track-count": tracks.len()});
                if with_tracks {
                    m["tracks"] = Value::Array(
                        tracks
                            .iter()
                            .enumerate()
                            .map(|(t, len)| {
                                json!({"position": t + 1, "title": format!("Track {}", t + 1),
                                       "length": len, "recording": {"id": id(9, d * 100 + t)}})
                            })
                            .collect(),
                    );
                }
                m
            })
            .collect(),
    )
}

/// A release JSON: `kind` is RELEASE (the real one), DECOY (deluxe or a
/// reissue with other durations) or OTHER (another artist, same title).
fn release_json(i: usize, w: &World, kind: u32, discs: &[Vec<u64>], tracks: bool) -> Value {
    let group = if kind == OTHER { id(OTHER + 10, i) } else { id(GROUP, i) };
    let mut r = json!({
        "id": id(kind, i),
        "title": w.title,
        "status": "Official",
        "date": format!("{}-03-01", w.year + i64::from(kind == DECOY) * 10),
        "country": "XW",
        "artist-credit": credit(w),
        "release-group": {"id": group, "title": w.title, "primary-type": "Album",
                          "secondary-types": if w.compilation { json!(["Compilation"]) } else { json!([]) },
                          "first-release-date": w.year.to_string()},
        "media": media(discs, tracks),
        "label-info": [{"label": {"id": id(7, i), "name": "Made-up Records"}}],
        "track-count": discs.iter().map(Vec::len).sum::<usize>(),
        "score": 100,
    });
    if kind == OTHER {
        r["artist-credit"] = json!([{"name": "Someone Else", "joinphrase": "",
            "artist": {"id": id(ARTIST, 900 + i), "name": "Someone Else", "sort-name": "Else, Someone"}}]);
    }
    r
}

/// The decoy of album `i`: a deluxe edition (+4 tracks) or a reissue with
/// the same track count but other durations.
fn decoy_discs(i: usize, w: &World) -> Vec<Vec<u64>> {
    let mut discs = w.discs.clone();
    if i.is_multiple_of(2) {
        discs[0].extend([200_000, 210_000, 220_000, 230_000]);
    } else {
        for d in &mut discs {
            for len in d.iter_mut() {
                *len += 25_000;
            }
        }
    }
    discs
}

/// Fake MusicBrainz and Cover Art Archive over the synthetic world.
struct Fake {
    world: Vec<World>,
    releases: HashMap<String, (usize, u32)>,
    clock: Arc<ManualClock>,
    /// `(host, path, monotonic time)` of every request.
    log: Mutex<Vec<(String, String, Duration)>>,
}

fn ok(v: &Value) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: vec![],
        body: v.to_string(),
    }
}

fn not_found() -> HttpResponse {
    HttpResponse {
        status: 404,
        headers: vec![],
        body: r#"{"error":"Not Found"}"#.into(),
    }
}

fn phrases(query: &str) -> Vec<(String, String)> {
    // `release:"x" AND artist:"y"`; escapes are not needed by the fixture.
    let mut out = Vec::new();
    let mut rest = query;
    while let Some(colon) = rest.find(":\"") {
        let field = rest[..colon].rsplit(' ').next().unwrap_or("").to_owned();
        let after = &rest[colon + 2..];
        let end = after.find('"').unwrap_or(after.len());
        out.push((field, after[..end].to_owned()));
        rest = &after[(end + 1).min(after.len())..];
    }
    out
}

impl Fake {
    fn new(clock: Arc<ManualClock>) -> Self {
        let world = world();
        let mut releases = HashMap::new();
        for i in 0..ALBUMS {
            releases.insert(id(RELEASE, i), (i, RELEASE));
            if i % 3 != 2 {
                releases.insert(id(DECOY, i), (i, DECOY));
            }
            if i % 10 == 2 {
                releases.insert(id(OTHER, i), (i, OTHER));
            }
        }
        Self {
            world,
            releases,
            clock,
            log: Mutex::new(Vec::new()),
        }
    }

    fn release(&self, rid: &str, tracks: bool) -> Option<Value> {
        let &(i, kind) = self.releases.get(rid)?;
        let w = &self.world[i];
        let discs = if kind == DECOY {
            decoy_discs(i, w)
        } else {
            w.discs.clone()
        };
        Some(release_json(i, w, kind, &discs, tracks))
    }

    fn group(&self, gid: &str) -> Option<Value> {
        let i = (0..ALBUMS).find(|&i| id(GROUP, i) == gid)?;
        let w = &self.world[i];
        Some(json!({"id": gid, "title": w.title, "primary-type": "Album",
                    "first-release-date": w.year.to_string(),
                    "artist-credit": credit(w),
                    "genres": [{"name": if i.is_multiple_of(2) { "hip hop" } else { "alternative rock" }, "count": 4},
                               {"name": "electronic", "count": 1}]}))
    }

    fn search(&self, query: &str) -> Value {
        let p = phrases(query);
        let title = p.iter().find(|(f, _)| f == "release").map(|(_, v)| v.to_lowercase());
        let artist = p.iter().find(|(f, _)| f == "artist").map(|(_, v)| v.to_lowercase());
        let mut found: Vec<Value> = Vec::new();
        let mut ids: Vec<&String> = self.releases.keys().collect();
        ids.sort();
        for rid in ids {
            let r = self.release(rid, false).unwrap();
            let rt = r["title"].as_str().unwrap().to_lowercase();
            let ra = r["artist-credit"][0]["name"].as_str().unwrap().to_lowercase();
            // Fuzzy like MusicBrainz: the title matches, the artist may not.
            if title.as_deref() == Some(rt.as_str()) {
                let mut r = r;
                r["score"] = json!(if artist.as_deref() == Some(ra.as_str()) { 100 } else { 70 });
                found.push(r);
            }
        }
        json!({"count": found.len(), "offset": 0, "releases": found})
    }

    fn artist(&self, aid: &str) -> Option<Value> {
        let n = (0..70).find(|&n| id(ARTIST, n) == aid)?;
        Some(json!({"id": aid, "name": format!("Synth Artist {n}"),
                    "sort-name": format!("Artist {n}, Synth"),
                    "type": if n.is_multiple_of(2) { "Group" } else { "Person" }, "country": "XW",
                    "life-span": {"begin": "1990", "end": null, "ended": false},
                    "disambiguation": "made up",
                    "genres": [{"name": "post-rock", "count": 2}]}))
    }
}

impl HttpTransport for Fake {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.log
            .lock()
            .unwrap()
            .push((r.base.clone(), r.path.clone(), self.clock.monotonic()));
        assert!(
            r.headers
                .iter()
                .any(|(k, v)| k == "user-agent" && v.starts_with("Kinolune/") && !v.contains('@')),
            "every request carries the project User-Agent"
        );
        let mut parts = r.path.trim_start_matches('/').splitn(2, '/');
        let (entity, rid) = (parts.next().unwrap_or(""), parts.next());
        if r.base.contains("coverartarchive") {
            let i = (0..ALBUMS).find(|&i| Some(id(GROUP, i).as_str()) == rid);
            return Ok(match i {
                Some(i) if i.is_multiple_of(2) => ok(&json!({"images": [{"front": true, "types": ["Front"],
                    "approved": true, "image": "https://coverartarchive.org/x.jpg"}],
                    "release": "https://musicbrainz.org/release/x"})),
                _ => not_found(),
            });
        }
        let query = r
            .query
            .iter()
            .find(|(k, _)| k == "query")
            .map(|(_, v)| v.as_str());
        Ok(match (entity, rid, query) {
            ("release", None, Some(q)) => ok(&self.search(q)),
            ("release", Some(rid), _) => self.release(rid, true).map_or_else(not_found, |v| ok(&v)),
            ("release-group", Some(gid), _) => self.group(gid).map_or_else(not_found, |v| ok(&v)),
            ("artist", Some(aid), _) => self.artist(aid).map_or_else(not_found, |v| ok(&v)),
            ("artist", None, Some(_)) => ok(&json!({"artists": []})),
            _ => not_found(),
        })
    }
}

/// How album `i` is tagged in the local library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tagging {
    ReleaseMbid,
    GroupMbid,
    Plain,
}

fn tagging(i: usize) -> Tagging {
    match i % 10 {
        0..=3 => Tagging::ReleaseMbid,
        4 => Tagging::GroupMbid,
        _ => Tagging::Plain,
    }
}

/// Writes the local library: one file per track with tags as a tagger
/// would leave them; some albums miss a file, some have no year.
fn build_library(db: &Connection, world: &[World]) {
    db.execute(
        "INSERT INTO sources (id, kind, name, uri, created_at_ms) VALUES (1, 'local', 'Music', 'file:///m', 0)",
        [],
    )
    .unwrap();
    let mut tags_by_path: HashMap<String, TrackTags> = HashMap::new();
    let mut file_id = 0;
    for (i, w) in world.iter().enumerate() {
        let how = tagging(i);
        for (d, tracks) in w.discs.iter().enumerate() {
            for (t, len) in tracks.iter().enumerate() {
                // Every 7th plain album misses its last file.
                if how == Tagging::Plain && i.is_multiple_of(7) && d == 0 && t + 1 == tracks.len() {
                    continue;
                }
                file_id += 1;
                let path = format!("/{}/{}/{}-{:02}.flac", w.artist, w.title, d + 1, t + 1);
                db.execute(
                    "INSERT INTO files (id, source_id, path, size_bytes, added_at_ms) VALUES (?1, 1, ?2, 10, 0)",
                    pa_library::rusqlite::params![file_id, path],
                )
                .unwrap();
                // Durations as decoded: within a second of MusicBrainz.
                let jitter = (i as u64 * 7 + t as u64) % 1500;
                let tags = TrackTags {
                    title: Some(format!("Track {}", t + 1)),
                    artists: vec![w.artist.clone()],
                    album_artists: if w.compilation { vec![] } else { vec![w.artist.clone()] },
                    album: Some(w.title.clone()),
                    compilation: w.compilation,
                    track_number: Some(t as u32 + 1),
                    track_total: i.is_multiple_of(3).then_some(tracks.len() as u32),
                    disc_number: Some(d as u32 + 1),
                    disc_total: Some(w.discs.len() as u32),
                    year: (i % 11 != 4).then_some(w.year as u16),
                    // Half of the albums carry no genre tag; the others a
                    // messy one.
                    genres: if i.is_multiple_of(2) { vec![] } else { vec!["Hip Hop/Rap".into()] },
                    release_mbid: (how == Tagging::ReleaseMbid).then(|| id(RELEASE, i).to_uppercase()),
                    release_group_mbid: (how != Tagging::Plain).then(|| id(GROUP, i)),
                    audio: AudioInfo {
                        codec: Some("flac".into()),
                        duration_ms: Some(len + jitter),
                        ..AudioInfo::default()
                    },
                    ..TrackTags::default()
                };
                tags_by_path.insert(path, tags);
            }
        }
    }
    let mut albums = Vec::new();
    for file in music::stale_files(db, 1).unwrap() {
        let put = music::put_track(db, &file, &tags_by_path[&file.path], 1).unwrap();
        albums.extend(put.album_id);
    }
    albums.sort_unstable();
    albums.dedup();
    for a in albums {
        music::refresh_album(db, a, 1).unwrap();
    }
}

fn providers(fake: &Arc<Fake>, clock: &Arc<ManualClock>) -> MusicProviders {
    let cache = Arc::new(MemoryCache::default());
    MusicProviders {
        musicbrainz: Some(Arc::new(MusicBrainzClient::new(
            MusicBrainzConfig::default(),
            fake.clone(),
            cache.clone(),
            clock.clone(),
        ))),
        cover_art: Some(Arc::new(CoverArtClient::new(
            CoverArtConfig::default(),
            fake.clone(),
            cache,
            clock.clone(),
        ))),
    }
}

/// `(album title, album artist)` → `(release, release group)` matched.
fn matches(db: &Connection) -> HashMap<(String, String), (Option<String>, Option<String>)> {
    let mut stmt = db
        .prepare(
            "SELECT a.title,
                    (SELECT ar.name FROM music_album_artists x JOIN music_artists ar ON ar.id = x.artist_id
                     WHERE x.album_id = a.id ORDER BY x.position LIMIT 1),
                    m.release_mbid, m.release_group_mbid
             FROM music_albums a JOIN music_album_metadata m ON m.album_id = a.id
             WHERE m.outcome = 'matched'",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok((
            (r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default()),
            (r.get(2)?, r.get(3)?),
        ))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

#[test]
fn synthetic_collection_matches_by_mbid() {
    let dir = tempfile::tempdir().unwrap();
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    let db = Connection::open(dir.path().join("library.db")).unwrap();
    let world = world();
    build_library(&db, &world);

    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let fake = Arc::new(Fake::new(clock.clone()));
    let providers = providers(&fake, &clock);
    let mut seen = Vec::new();
    let report = cat
        .fetch_music_metadata(
            &providers,
            &MusicMetadataOptions::default(),
            &CancelToken::new(),
            &mut |done, total| seen.push((done, total)),
        )
        .unwrap();
    assert!(!report.stopped, "{report:?}");
    assert_eq!(report.albums, ALBUMS, "{report:?}");
    assert_eq!(seen.last().map(|(d, t)| d == t), Some(true));

    // Done-when: at least 95 % matched to the right MBIDs.
    let found = matches(&db);
    let mut correct = 0;
    let mut wrong = Vec::new();
    for (i, w) in world.iter().enumerate() {
        let artist = if w.compilation {
            "Various Artists".to_owned()
        } else {
            w.artist.clone()
        };
        let want_release = (tagging(i) != Tagging::GroupMbid).then(|| id(RELEASE, i));
        let got = found.get(&(w.title.clone(), artist));
        if got == Some(&(want_release.clone(), Some(id(GROUP, i)))) {
            correct += 1;
        } else {
            wrong.push((i, tagging(i), got.cloned()));
        }
    }
    let rate = f64::from(correct) / ALBUMS as f64;
    eprintln!("META-021: {correct}/{ALBUMS} albums matched by MBID ({:.1} %)", rate * 100.0);
    assert!(rate >= 0.95, "matched {correct}/{ALBUMS}; wrong: {wrong:?}");

    // MusicBrainz saw at most one request per second, Cover Art Archive too.
    let log = fake.log.lock().unwrap().clone();
    for host in ["musicbrainz", "coverartarchive"] {
        let times: Vec<Duration> = log
            .iter()
            .filter(|(b, _, _)| b.contains(host))
            .map(|(_, _, t)| *t)
            .collect();
        assert!(times.len() > 1, "{host}");
        for pair in times.windows(2) {
            assert!(
                pair[1] - pair[0] >= Duration::from_millis(999),
                "{host}: requests {:?} apart",
                pair[1] - pair[0]
            );
        }
    }

    // Covers, genres and album data reach the music views.
    let tracks = cat.music_tracks().unwrap();
    assert!(!tracks.is_empty());
    let cover = "https://coverartarchive.org/release-group/";
    for t in &tracks {
        let artist = t.album_artists.first().cloned().unwrap_or_default();
        let Some(i) = world.iter().position(|w| {
            Some(&w.title) == t.item.album.as_ref()
                && (w.artist == artist || (w.compilation && artist == "Various Artists"))
        }) else {
            panic!("unknown album of {t:?}");
        };
        if found.values().all(|(_, g)| g.as_deref() != Some(id(GROUP, i).as_str())) {
            continue; // one of the few albums not matched
        }
        if i.is_multiple_of(2) {
            assert_eq!(
                t.cover_url.as_deref(),
                Some(format!("{cover}{}/front-500", id(GROUP, i)).as_str())
            );
            assert_eq!(t.album_genres, ["Hip-Hop", "Electronic"]);
            // No genre tag: the album's genres.
            assert_eq!(t.genres, ["Hip-Hop", "Electronic"]);
        } else {
            assert_eq!(t.cover_url, None);
            assert_eq!(t.album_genres, ["Alternative Rock", "Electronic"]);
            // Tag genres, normalized.
            assert_eq!(t.genres, ["Hip-Hop", "Rap"]);
        }
        assert_eq!(t.album_type.as_deref(), Some("Album"));
        assert_eq!(t.original_year, Some(world[i].year));
    }
    assert!(report.covers > 0);

    // Album artists got MusicBrainz data through the album credits.
    let artists = cat.music_artist_infos().unwrap();
    assert!(artists.len() >= 60, "{} artists", artists.len());
    let a = artists
        .iter()
        .find(|a| a.name == "Synth Artist 4")
        .expect("artist 4");
    assert_eq!(a.mbid, id(ARTIST, 4));
    assert_eq!(a.kind.as_deref(), Some("Group"));
    assert_eq!(a.genres, ["Post-Rock"]);
    assert_eq!(a.begin_date.as_deref(), Some("1990"));
    assert!(artists.iter().all(|a| a.name != "Various Artists"));

    // A second run asks for nothing: everything is fresh.
    let before = fake.log.lock().unwrap().len();
    let again = cat
        .fetch_music_metadata(
            &providers,
            &MusicMetadataOptions::default(),
            &CancelToken::new(),
            &mut |_, _| {},
        )
        .unwrap();
    assert_eq!((again.albums, again.artists), (0, 0), "{again:?}");
    assert_eq!(fake.log.lock().unwrap().len(), before);

    // A reset asks again (answers now come from the response cache).
    cat.reset_music_metadata().unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM music_album_metadata", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn offline_marks_albums_failed_and_stops() {
    struct Offline;
    impl HttpTransport for Offline {
        fn get(&self, _r: &HttpRequest) -> Result<HttpResponse, TransportError> {
            Err(TransportError("offline".into()))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    let db = Connection::open(dir.path().join("library.db")).unwrap();
    build_library(&db, &world()[..20]);
    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let cache = Arc::new(MemoryCache::default());
    let providers = MusicProviders {
        musicbrainz: Some(Arc::new(MusicBrainzClient::new(
            MusicBrainzConfig::default(),
            Arc::new(Offline),
            cache,
            clock.clone(),
        ))),
        cover_art: None,
    };
    let report = cat
        .fetch_music_metadata(
            &providers,
            &MusicMetadataOptions::default(),
            &CancelToken::new(),
            &mut |_, _| {},
        )
        .unwrap();
    assert!(report.stopped);
    assert_eq!(report.failed, 5, "{report:?}");
    assert!(report.error.is_some());
    let failed: i64 = db
        .query_row(
            "SELECT count(*) FROM music_album_metadata WHERE outcome = 'failed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(failed, 5);
    // Failed albums are retried on a later run.
    std::thread::sleep(Duration::from_millis(5));
    assert_eq!(
        cat.fetch_music_metadata(
            &providers,
            &MusicMetadataOptions {
                retry_failed_after_ms: 0,
                ..Default::default()
            },
            &CancelToken::new(),
            &mut |_, _| {},
        )
        .unwrap()
        .albums,
        20
    );
}

#[test]
fn cancelled_run_stops_and_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let cat = Catalog::open(dir.path().join("library.db")).unwrap();
    let db = Connection::open(dir.path().join("library.db")).unwrap();
    build_library(&db, &world()[..10]);
    let clock = Arc::new(ManualClock::at_unix(1_800_000_000));
    let fake = Arc::new(Fake::new(clock.clone()));
    let providers = providers(&fake, &clock);
    let cancel = CancelToken::new();
    let report = cat
        .fetch_music_metadata(&providers, &MusicMetadataOptions::default(), &cancel, &mut |done, _| {
            if done == 3 {
                cancel.cancel();
            }
        })
        .unwrap();
    assert!(report.stopped);
    let rest = cat
        .fetch_music_metadata(
            &providers,
            &MusicMetadataOptions::default(),
            &CancelToken::new(),
            &mut |_, _| {},
        )
        .unwrap();
    assert_eq!(rest.albums, 10 - 3, "{rest:?}");
    assert!(!rest.stopped);
}
