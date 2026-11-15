// SPDX-License-Identifier: MPL-2.0
//! LIB-023 benchmark: a large network library is recognised end to end
//! without a stall.
//!
//! A Samba share (Docker, LAN-like latency through netem) holds ~200 groups
//! that mirror a real home library: films as BDRemux/2160p files of 5–60 GB,
//! western series by season, anime folders with dozens of fansub episodes,
//! Russian names and `NN Title.mkv` episodes. Files are sparse: a small real
//! Matroska header (title duration, HEVC and AC-3 tracks) followed by a
//! sparse tail of the real size, so the disk holds almost nothing while
//! every read path (listing, OSHash, probe) sees realistic sizes.
//!
//! The run is the app's: scan (list, folders, probe, fingerprint, local
//! recognition) through `pa-sources`/libsmb2, the provider metadata run
//! against synthetic TMDB, AniList and Shikimori with realistic latencies and
//! their rate limits (AniList's degraded 30 per minute with 429 and
//! `Retry-After`), the posters through `pa-imagecache` as the grid loads
//! them, then the probes left for later. Every step is timed, and so is the
//! moment titles and posters of the whole library are in; a watchdog fails
//! the run when nothing moves for `PA_BENCH_STALL_SECS` (default 120 s).
//!
//! Run with `tests/large_library/run.sh` (starts Samba, sets the variables
//! read here).
#![cfg(feature = "server-tests")]

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pa_catalog::pa_metadata::anilist::{AniListClient, AniListConfig};
use pa_catalog::pa_metadata::cache::MemoryCache;
use pa_catalog::pa_metadata::clock::SystemClock;
use pa_catalog::pa_metadata::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use pa_catalog::pa_metadata::key::Keys;
use pa_catalog::pa_metadata::shikimori::{ShikimoriClient, ShikimoriConfig};
use pa_catalog::pa_metadata::tmdb::{TmdbClient, TmdbConfig};
use pa_catalog::{
    Catalog, MetadataOptions, MetadataProviders, MetadataStage, ScanStep, SourcesConnector,
};
use pa_scanner::CancelToken;
use pa_secrets::{MemoryStore, SecretString};
use pa_sources::{NewSource, SourceKind, SourceManager};

fn var(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: use tests/large_library/run.sh"))
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

// --- Fixture -----------------------------------------------------------------

/// EBML element: id bytes, 8-byte size, payload.
fn el(id: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.push(0x01);
    v.extend_from_slice(&(payload.len() as u64).to_be_bytes()[1..]);
    v.extend_from_slice(payload);
    v
}

fn uint(id: &[u8], n: u64) -> Vec<u8> {
    el(id, &n.to_be_bytes())
}

fn float(id: &[u8], f: f64) -> Vec<u8> {
    el(id, &f.to_be_bytes())
}

/// A Matroska header of a file of `size` bytes lasting `duration`: EBML
/// header, Segment (sized to the file), Info with the duration, an HEVC
/// 2160p and a Russian AC-3 5.1 track. The rest of the file is zeros.
fn mkv_header(size: u64, duration: Duration, seed: u64) -> Vec<u8> {
    let ebml = el(
        &[0x1A, 0x45, 0xDF, 0xA3],
        &[
            uint(&[0x42, 0x86], 1),
            uint(&[0x42, 0xF7], 1),
            uint(&[0x42, 0xF2], 4),
            uint(&[0x42, 0xF3], 8),
            el(&[0x42, 0x82], b"matroska"),
            uint(&[0x42, 0x87], 4),
            uint(&[0x42, 0x85], 2),
        ]
        .concat(),
    );
    let info = el(
        &[0x15, 0x49, 0xA9, 0x66],
        &[
            uint(&[0x2A, 0xD7, 0xB1], 1_000_000),
            float(&[0x44, 0x89], duration.as_millis() as f64),
            el(&[0x4D, 0x80], b"pa-bench"),
            el(&[0x57, 0x41], format!("pa-bench {seed}").as_bytes()),
        ]
        .concat(),
    );
    let video = el(
        &[0xAE],
        &[
            uint(&[0xD7], 1),
            uint(&[0x73, 0xC5], seed * 2 + 1),
            uint(&[0x83], 1),
            el(&[0x86], b"V_MPEGH/ISO/HEVC"),
            el(
                &[0xE0],
                &[uint(&[0xB0], 3840), uint(&[0xBA], 2160)].concat(),
            ),
        ]
        .concat(),
    );
    let audio = el(
        &[0xAE],
        &[
            uint(&[0xD7], 2),
            uint(&[0x73, 0xC5], seed * 2 + 2),
            uint(&[0x83], 2),
            el(&[0x86], b"A_AC3"),
            el(&[0x22, 0xB5, 0x9C], b"rus"),
            el(
                &[0xE1],
                &[float(&[0xB5], 48_000.0), uint(&[0x9F], 6)].concat(),
            ),
        ]
        .concat(),
    );
    let tracks = el(&[0x16, 0x54, 0xAE, 0x6B], &[video, audio].concat());
    let body = [info, tracks].concat();
    let seg_size = size - ebml.len() as u64 - 12;
    let mut segment = vec![0x18, 0x53, 0x80, 0x67, 0x01];
    segment.extend_from_slice(&seg_size.to_be_bytes()[1..]);
    [ebml, segment, body].concat()
}

/// Writes a sparse media file: the header, then a hole up to `size`.
fn sparse_file(root: &Path, rel: &str, size: u64, duration: Duration, seed: u64) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, mkv_header(size, duration, seed)).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(size)
        .unwrap();
}

const GB: u64 = 1_000_000_000;

const WORDS: &[&str] = &[
    "Silent", "Harbor", "Crimson", "Echo", "Northern", "Lights", "Iron", "Garden", "Hollow",
    "Crown", "Midnight", "Express", "Glass", "River", "Paper", "Moon", "Stone", "Orchard",
    "Winter", "Signal", "Golden", "Hour", "Last", "Frontier", "Broken", "Compass", "Velvet",
    "Storm", "Distant", "Shore", "Electric", "Dreams", "Hidden", "Valley", "Burning", "Bridge",
];

const RU_WORDS: &[&str] = &[
    "Тихая",
    "Гавань",
    "Красная",
    "Река",
    "Северный",
    "Ветер",
    "Железный",
    "Сад",
    "Последний",
    "Рубеж",
    "Ночной",
    "Экспресс",
    "Стеклянная",
    "Луна",
    "Каменный",
    "Остров",
    "Зимний",
    "Сигнал",
];

const ROMAJI: &[&str] = &[
    "Hoshi", "no", "Kakera", "Sora", "Tsubasa", "Yume", "Kaze", "Hikari", "Mirai", "Kage", "Tsuki",
    "Umi", "Hana", "Yoru", "Tabi", "Kokoro", "Ryuu", "Mori",
];

fn pick(words: &[&str], n: usize, k: usize) -> String {
    (0..k)
        .map(|i| words[(n * 7 + i * 13 + n / 3) % words.len()])
        .collect::<Vec<_>>()
        .join(" ")
}

/// One group of the fixture, as the provider mock knows it.
#[derive(Debug, Clone)]
struct Known {
    title: String,
    year: u32,
    tv: bool,
    anime: bool,
    /// TMDB knows the title (some anime it does not).
    on_tmdb: bool,
}

/// Seeds the library under `root` (once; a marker file keeps the seed).
/// Returns the groups and the number of media files.
fn seed_library(root: &Path) -> (Vec<Known>, usize) {
    let mut known = Vec::new();
    let mut files = Vec::new();
    let mut seed = 1u64;
    let mut add =
        |files: &mut Vec<(String, u64, Duration, u64)>, rel: String, size: u64, d: Duration| {
            seed += 1;
            // Distinct sizes: the OSHash of a sparse file is its size plus the
            // header words.
            files.push((rel, size + seed * 4096, d, seed));
        };
    // Films: 70 Latin titles, 25 Russian, one with two versions.
    for n in 0..95 {
        let ru = n >= 70;
        let title = if ru {
            pick(RU_WORDS, n, 2)
        } else {
            pick(WORDS, n, 2 + n % 2)
        };
        let title = format!("{title} {}", n + 1);
        let year = 1975 + (n as u32 * 7) % 50;
        let size = (5 + (n as u64 * 11) % 56) * GB;
        let d = Duration::from_secs(5400 + (n as u64 * 97) % 3600);
        let rel = match n % 4 {
            0 => format!(
                "Movies/{title} ({year})/{}.{year}.2160p.UHD.BDRemux.HEVC.mkv",
                title.replace(' ', ".")
            ),
            1 => format!("Movies/{title} ({year})/{title} ({year}) BDRemux 1080p.mkv"),
            2 => format!(
                "Movies/{}.{year}.2160p.WEB-DL.DV.HDR.mkv",
                title.replace(' ', ".")
            ),
            _ => format!("Фильмы/{title} ({year})/{title}.{year}.BDRip.1080p.mkv"),
        };
        add(&mut files, rel, size, d);
        if n == 3 {
            add(
                &mut files,
                format!(
                    "Movies/{title} ({year})/{}.{year}.1080p.BluRay.mkv",
                    title.replace(' ', ".")
                ),
                12 * GB,
                d,
            );
        }
        known.push(Known {
            title,
            year,
            tv: false,
            anime: false,
            on_tmdb: true,
        });
    }
    // Western series: 35 shows, 1–3 seasons of 8–12 episodes.
    for n in 0..35 {
        let title = format!("{} {}", pick(WORDS, n + 40, 2), n + 1);
        let year = 2005 + (n as u32 * 3) % 18;
        let seasons = 1 + n % 3;
        for s in 1..=seasons {
            for e in 1..=(8 + (n + s) % 5) {
                let rel = format!(
                    "Series/{title}/Season {s:02}/{}.S{s:02}E{e:02}.1080p.WEB-DL.mkv",
                    title.replace(' ', ".")
                );
                add(
                    &mut files,
                    rel,
                    (2 + (e as u64) % 3) * GB,
                    Duration::from_secs(2700),
                );
            }
        }
        known.push(Known {
            title,
            year,
            tv: true,
            anime: false,
            on_tmdb: true,
        });
    }
    // Anime: 70 series of 12–26 episodes, fansub names; some with Russian
    // folders, some as `NN Title.mkv`; a fifth unknown to TMDB.
    for n in 0..70 {
        let title = format!("{} {}", pick(ROMAJI, n, 3), n + 1);
        let year = 2000 + (n as u32 * 5) % 25;
        let eps = 12 + (n * 5) % 15;
        for e in 1..=eps {
            let rel = match n % 7 {
                5 => format!(
                    "Аниме/{title} ({year})/{e:02} {}.mkv",
                    pick(ROMAJI, n + e, 2)
                ),
                6 => format!("Anime/{title}/[Anilibria] {title} - {e:02} [WEBRip 1080p].mkv"),
                _ => format!(
                    "Anime/{title}/[SubsPlease] {title} - {e:02} (1080p) [{:08X}].mkv",
                    n * 1000 + e
                ),
            };
            add(
                &mut files,
                rel,
                (1 + (e as u64) % 3) * GB / 2 * 3,
                Duration::from_secs(1420),
            );
        }
        known.push(Known {
            title,
            year,
            tv: true,
            anime: true,
            on_tmdb: n % 5 != 4,
        });
    }
    let marker = root.join(".pa-bench-seeded");
    if !marker.exists() {
        for (rel, size, d, seed) in &files {
            sparse_file(root, rel, *size, *d, *seed);
        }
        std::fs::write(&marker, files.len().to_string()).unwrap();
    }
    (known, files.len())
}

// --- Providers -----------------------------------------------------------------

/// Sliding-window limit of a mock provider: more than `max` requests in
/// `window` is a 429.
struct Window {
    max: usize,
    window: Duration,
    sent: VecDeque<Instant>,
}

impl Window {
    fn new(max: usize, window: Duration) -> Self {
        Self {
            max,
            window,
            sent: VecDeque::new(),
        }
    }

    /// `Ok(remaining)` or `Err(retry after)`.
    fn admit(&mut self, now: Instant) -> Result<usize, Duration> {
        while self
            .sent
            .front()
            .is_some_and(|t| now.duration_since(*t) >= self.window)
        {
            self.sent.pop_front();
        }
        if self.sent.len() >= self.max {
            let first = *self.sent.front().unwrap();
            return Err(self.window.saturating_sub(now.duration_since(first)));
        }
        self.sent.push_back(now);
        Ok(self.max - self.sent.len())
    }
}

#[derive(Default)]
struct Stats {
    requests: HashMap<&'static str, u64>,
    throttled: HashMap<&'static str, u64>,
}

/// Synthetic TMDB, AniList and Shikimori over the fixture's titles.
struct MockProviders {
    known: Vec<Known>,
    tmdb_ids: Mutex<HashMap<u64, usize>>,
    anilist_rate: Mutex<Window>,
    shikimori_rate: Mutex<Window>,
    shikimori_minute: Mutex<Window>,
    tmdb_rate: Mutex<Window>,
    stats: Mutex<Stats>,
    latency: [Duration; 3],
}

fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

impl MockProviders {
    fn new(known: Vec<Known>) -> Self {
        Self {
            known,
            tmdb_ids: Mutex::default(),
            // AniList's degraded mode: 30 a minute, then 429 with Retry-After.
            anilist_rate: Mutex::new(Window::new(30, Duration::from_secs(60))),
            shikimori_rate: Mutex::new(Window::new(5, Duration::from_secs(1))),
            shikimori_minute: Mutex::new(Window::new(90, Duration::from_secs(60))),
            tmdb_rate: Mutex::new(Window::new(50, Duration::from_secs(1))),
            stats: Mutex::default(),
            latency: [
                Duration::from_millis(env_u64("PA_BENCH_TMDB_MS", 80)),
                Duration::from_millis(env_u64("PA_BENCH_ANILIST_MS", 250)),
                Duration::from_millis(env_u64("PA_BENCH_SHIKIMORI_MS", 200)),
            ],
        }
    }

    fn count(&self, provider: &'static str, throttled: bool) {
        let mut s = self.stats.lock().unwrap();
        *s.requests.entry(provider).or_default() += 1;
        if throttled {
            *s.throttled.entry(provider).or_default() += 1;
        }
    }

    fn find(&self, query: &str) -> Option<usize> {
        let q = fold(query);
        self.known.iter().position(|k| fold(&k.title) == q)
    }

    fn tmdb_id(&self, i: usize) -> u64 {
        let id = 1000 + i as u64;
        self.tmdb_ids.lock().unwrap().insert(id, i);
        id
    }

    fn ok(body: String, headers: Vec<(String, String)>) -> HttpResponse {
        HttpResponse {
            status: 200,
            headers,
            body,
        }
    }

    fn not_found() -> HttpResponse {
        HttpResponse {
            status: 404,
            headers: vec![],
            body: r#"{"status_code":34,"status_message":"not found"}"#.into(),
        }
    }

    fn too_many(retry_after: Duration) -> HttpResponse {
        HttpResponse {
            status: 429,
            headers: vec![(
                "retry-after".into(),
                retry_after.as_secs().max(1).to_string(),
            )],
            body: r#"{"errors":[{"message":"Too Many Requests.","status":429}]}"#.into(),
        }
    }

    fn tmdb(&self, r: &HttpRequest) -> HttpResponse {
        std::thread::sleep(self.latency[0]);
        if let Err(wait) = self.tmdb_rate.lock().unwrap().admit(Instant::now()) {
            self.count("tmdb", true);
            return Self::too_many(wait);
        }
        self.count("tmdb", false);
        let param = |k: &str| r.query.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        let path = r.path.as_str();
        let empty = r#"{"page":1,"results":[],"total_pages":0,"total_results":0}"#.to_owned();
        if path == "/search/movie" || path == "/search/tv" {
            let tv = path == "/search/tv";
            let Some(i) = param("query").and_then(|q| self.find(&q)) else {
                return Self::ok(empty, vec![]);
            };
            let k = &self.known[i];
            let year = param("year").or_else(|| param("first_air_date_year"));
            if k.tv != tv || !k.on_tmdb || year.is_some_and(|y| y != k.year.to_string()) {
                return Self::ok(empty, vec![]);
            }
            let id = self.tmdb_id(i);
            let t = json_str(&k.title);
            let body = if tv {
                format!(
                    r#"{{"page":1,"results":[{{"id":{id},"name":{t},"original_name":{t},"first_air_date":"{}-04-01","popularity":30,"original_language":"{}"}}],"total_pages":1,"total_results":1}}"#,
                    k.year,
                    if k.anime { "ja" } else { "en" }
                )
            } else {
                format!(
                    r#"{{"page":1,"results":[{{"id":{id},"title":{t},"original_title":{t},"release_date":"{}-04-01","popularity":30,"original_language":"en"}}],"total_pages":1,"total_results":1}}"#,
                    k.year
                )
            };
            return Self::ok(body, vec![]);
        }
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let Some(id) = parts.get(1).and_then(|s| s.parse::<u64>().ok()) else {
            return Self::not_found();
        };
        let Some(i) = self.tmdb_ids.lock().unwrap().get(&id).copied() else {
            return Self::not_found();
        };
        let k = &self.known[i];
        let t = json_str(&k.title);
        match (parts[0], parts.get(2).copied(), parts.get(3).copied()) {
            ("movie", None, _) => Self::ok(
                format!(
                    r#"{{"id":{id},"title":{t},"original_title":{t},"original_language":"en","overview":"Synthetic.","release_date":"{}-04-01","status":"Released","runtime":120,"poster_path":"/p{id}.jpg","backdrop_path":"/b{id}.jpg","vote_average":7.1,"vote_count":900,"genres":[{{"id":18,"name":"драма"}}]}}"#,
                    k.year
                ),
                vec![],
            ),
            ("tv", None, _) => Self::ok(
                format!(
                    r#"{{"id":{id},"name":{t},"original_name":{t},"original_language":"{}","overview":"Synthetic.","first_air_date":"{}-04-01","status":"Ended","episode_run_time":[24],"poster_path":"/p{id}.jpg","backdrop_path":"/b{id}.jpg","number_of_seasons":3,"number_of_episodes":90,"seasons":[{{"season_number":1,"episode_count":30}},{{"season_number":2,"episode_count":30}},{{"season_number":3,"episode_count":30}}]}}"#,
                    if k.anime { "ja" } else { "en" },
                    k.year
                ),
                vec![],
            ),
            ("tv", Some("season"), Some(s)) => {
                let s: u32 = s.parse().unwrap_or(1);
                let eps: Vec<String> = (1..=30)
                    .map(|e| {
                        format!(
                            r#"{{"id":{},"season_number":{s},"episode_number":{e},"name":"Episode {e}","still_path":"/s{id}-{s}-{e}.jpg"}}"#,
                            id * 1000 + u64::from(s) * 100 + e
                        )
                    })
                    .collect();
                Self::ok(
                    format!(
                        r#"{{"id":{id},"season_number":{s},"episodes":[{}]}}"#,
                        eps.join(",")
                    ),
                    vec![],
                )
            }
            (_, Some("translations"), _) => {
                Self::ok(format!(r#"{{"id":{id},"translations":[]}}"#), vec![])
            }
            (_, Some("credits"), _) => {
                Self::ok(format!(r#"{{"id":{id},"cast":[],"crew":[]}}"#), vec![])
            }
            (_, Some("release_dates" | "content_ratings"), _) => {
                Self::ok(format!(r#"{{"id":{id},"results":[]}}"#), vec![])
            }
            _ => Self::not_found(),
        }
    }

    fn anilist_media(&self, i: usize) -> String {
        let k = &self.known[i];
        format!(
            r#"{{"id":{},"idMal":{},"format":"TV","status":"FINISHED","episodes":24,"duration":24,"title":{{"romaji":{},"english":null,"native":null}},"synonyms":[],"startDate":{{"year":{},"month":4,"day":1}},"description":"Synthetic.","genres":["Drama"],"coverImage":{{"large":"https://img.test/anilist/{i}.jpg"}}}}"#,
            50_000 + i,
            60_000 + i,
            json_str(&k.title),
            k.year
        )
    }

    fn anilist(&self, body: &str) -> HttpResponse {
        std::thread::sleep(self.latency[1]);
        let remaining = match self.anilist_rate.lock().unwrap().admit(Instant::now()) {
            Ok(r) => r,
            Err(wait) => {
                self.count("anilist", true);
                return Self::too_many(wait);
            }
        };
        self.count("anilist", false);
        let headers = vec![
            ("x-ratelimit-limit".to_owned(), "30".to_owned()),
            ("x-ratelimit-remaining".to_owned(), remaining.to_string()),
        ];
        let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let vars = &v["variables"];
        if let Some(q) = vars["search"].as_str() {
            let media: Vec<String> = self
                .find(q)
                .filter(|&i| self.known[i].anime)
                .map(|i| self.anilist_media(i))
                .into_iter()
                .collect();
            return Self::ok(
                format!(r#"{{"data":{{"Page":{{"media":[{}]}}}}}}"#, media.join(",")),
                headers,
            );
        }
        let i = vars["id"]
            .as_u64()
            .map(|id| id as usize)
            .and_then(|id| id.checked_sub(50_000))
            .or_else(|| {
                vars["idMal"]
                    .as_u64()
                    .and_then(|m| (m as usize).checked_sub(60_000))
            });
        match i.filter(|&i| i < self.known.len()) {
            Some(i) => Self::ok(
                format!(r#"{{"data":{{"Media":{}}}}}"#, self.anilist_media(i)),
                headers,
            ),
            None => Self::ok(
                r#"{"data":{"Media":null},"errors":[{"message":"Not Found.","status":404}]}"#
                    .into(),
                headers,
            ),
        }
    }

    fn shikimori(&self, r: &HttpRequest) -> HttpResponse {
        std::thread::sleep(self.latency[2]);
        let now = Instant::now();
        let second = self.shikimori_rate.lock().unwrap().admit(now);
        let minute = second.and_then(|_| self.shikimori_minute.lock().unwrap().admit(now));
        if let Err(wait) = minute {
            self.count("shikimori", true);
            return Self::too_many(wait);
        }
        self.count("shikimori", false);
        let mal = r
            .path
            .strip_prefix("/animes/")
            .and_then(|s| s.parse::<usize>().ok())
            .and_then(|m| m.checked_sub(60_000))
            .filter(|&i| i < self.known.len());
        match mal {
            Some(i) => Self::ok(
                format!(
                    r#"{{"id":{},"name":{},"russian":{},"kind":"tv","status":"released","aired_on":"{}-04-01","myanimelist_id":{},"description":"Синтетика.","rating":"pg_13","english":[],"japanese":[],"synonyms":[]}}"#,
                    60_000 + i,
                    json_str(&self.known[i].title),
                    json_str(&format!("Русское {}", self.known[i].title)),
                    self.known[i].year,
                    60_000 + i
                ),
                vec![],
            ),
            None => Self::not_found(),
        }
    }
}

struct Transport(Arc<MockProviders>, &'static str);

impl HttpTransport for Transport {
    fn get(&self, r: &HttpRequest) -> Result<HttpResponse, TransportError> {
        Ok(match self.1 {
            "tmdb" => self.0.tmdb(r),
            _ => self.0.shikimori(r),
        })
    }

    fn post_json(&self, _r: &HttpRequest, body: &str) -> Result<HttpResponse, TransportError> {
        Ok(self.0.anilist(body))
    }
}

fn providers(mock: &Arc<MockProviders>) -> MetadataProviders {
    let cache = Arc::new(MemoryCache::default());
    let clock = Arc::new(SystemClock::default());
    MetadataProviders {
        tmdb: Some(Arc::new(TmdbClient::new(
            TmdbConfig::default(),
            Keys::none().with_user_key(Some("bench-key")),
            Arc::new(Transport(mock.clone(), "tmdb")),
            cache.clone(),
            clock.clone(),
        ))),
        anilist: Some(Arc::new(AniListClient::new(
            AniListConfig::default(),
            Arc::new(Transport(mock.clone(), "anilist")),
            cache.clone(),
            clock.clone(),
        ))),
        shikimori: Some(Arc::new(ShikimoriClient::new(
            ShikimoriConfig::default(),
            Arc::new(Transport(mock.clone(), "shikimori")),
            cache,
            clock,
        ))),
        anime_mappings: None,
        anime_titles: None,
    }
}

// --- Images --------------------------------------------------------------------

struct ImageServer {
    latency: Duration,
    served: AtomicU64,
}

impl pa_imagecache::ImageFetcher for ImageServer {
    fn fetch(&self, _url: &str) -> Result<pa_imagecache::Fetched, pa_imagecache::FetchError> {
        std::thread::sleep(self.latency);
        self.served.fetch_add(1, Ordering::Relaxed);
        // A tiny JPEG: SOI, APP0 JFIF, EOI.
        let mut body = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        body.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        body.extend(std::iter::repeat_n(0u8, 20_000));
        body.extend_from_slice(&[0xFF, 0xD9]);
        Ok(pa_imagecache::Fetched { status: 200, body })
    }
}

// --- Watchdog and timing ---------------------------------------------------------

/// Fails the run when nothing moves for too long.
struct Watchdog {
    last: Arc<Mutex<(Instant, String)>>,
    stalled: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}

impl Watchdog {
    fn start(limit: Duration, cancel: CancelToken) -> Self {
        let last = Arc::new(Mutex::new((Instant::now(), "start".to_owned())));
        let stalled = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        {
            let (last, stalled, done) = (last.clone(), stalled.clone(), done.clone());
            std::thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(500));
                    let (at, what) = last.lock().unwrap().clone();
                    if at.elapsed() > limit {
                        eprintln!("STALL: nothing moved for {:?}; last: {what}", at.elapsed());
                        stalled.store(true, Ordering::Relaxed);
                        cancel.cancel();
                        return;
                    }
                }
            });
        }
        Self {
            last,
            stalled,
            done,
        }
    }

    fn tick(&self, what: String) {
        *self.last.lock().unwrap() = (Instant::now(), what);
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct Phases(Vec<(String, Duration)>);

impl Phases {
    fn add(&mut self, name: &str, d: Duration) {
        eprintln!("[bench] {name}: {:.1} s", d.as_secs_f64());
        self.0.push((name.to_owned(), d));
    }
}

#[test]
fn large_smb_library_is_recognised_without_a_stall() {
    let data = PathBuf::from(var("PA_BENCH_DATA"));
    let t = Instant::now();
    let (known, file_count) = seed_library(&data);
    eprintln!(
        "[bench] fixture: {} groups, {file_count} files ({:.1} s)",
        known.len(),
        t.elapsed().as_secs_f64()
    );

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let cat = Arc::new(Catalog::open(&db).unwrap());
    let manager = Arc::new(SourceManager::open(&db, Arc::new(MemoryStore::new())).unwrap());
    cat.set_connector(Arc::new(SourcesConnector::new(Arc::clone(&manager))));
    let source = manager
        .create(
            NewSource::new(SourceKind::Smb, "NAS", var("PA_BENCH_SMB")).with_login(
                var("PA_BENCH_USER"),
                SecretString::new(var("PA_BENCH_PASS")),
            ),
        )
        .unwrap()
        .id;

    let stall = Duration::from_secs(env_u64("PA_BENCH_STALL_SECS", 120));
    let cancel = CancelToken::new();
    let dog = Watchdog::start(stall, cancel.clone());
    let mut phases = Phases::default();
    let total = Instant::now();

    // Scan: every update is a sign of life.
    let mut errors = Vec::new();
    let mut steps: Vec<(ScanStep, Instant, Instant)> = Vec::new();
    let step = |s: ScanStep, steps: &mut Vec<(ScanStep, Instant, Instant)>| {
        let now = Instant::now();
        match steps.last_mut() {
            Some(last) if last.0 == s => last.2 = now,
            _ => steps.push((s, now, now)),
        }
    };
    let scan_start = Instant::now();
    cat.scan(Some(source), &cancel, &mut |u| {
        step(u.step, &mut steps);
        dog.tick(format!(
            "scan {:?} {}/{:?} {:?}",
            u.step, u.processed, u.total, u.current_path
        ));
        errors.extend(u.error);
    })
    .unwrap();
    phases.add(
        "scan (list, folders, probe, fingerprint, recognise)",
        scan_start.elapsed(),
    );
    assert!(!dog.stalled.load(Ordering::Relaxed), "the scan stalled");
    let shows = cat.shows().unwrap().len();
    let movies = cat.movies().unwrap().len();
    eprintln!(
        "[bench] recognised: {movies} films, {shows} series; {} errors",
        errors.len()
    );
    for e in errors.iter().take(10) {
        eprintln!("[bench]   error: {e}");
    }

    // Metadata: titles of every work first, then anime.
    let mock = Arc::new(MockProviders::new(known.clone()));
    let providers = providers(&mock);
    let meta_start = Instant::now();
    let mut titles_done = None;
    let mut longest_wait = Duration::ZERO;
    let report = cat
        .fetch_metadata_with(&providers, &MetadataOptions::default(), &cancel, &mut |p| {
            if titles_done.is_none() && p.stage == MetadataStage::Anime {
                titles_done = Some(meta_start.elapsed());
            }
            for w in &p.waiting {
                longest_wait = longest_wait.max(w.wait);
            }
            dog.tick(format!(
                "metadata {:?} {}/{} waiting {:?}",
                p.stage, p.done, p.total, p.waiting
            ));
        })
        .unwrap();
    let meta = meta_start.elapsed();
    let titles = titles_done.unwrap_or(meta);
    phases.add("metadata: titles and posters (local, TMDB)", titles);
    phases.add("metadata: anime (AniList, Shikimori)", meta - titles);
    eprintln!("[bench] metadata report: {report:?}");
    eprintln!(
        "[bench] longest provider wait shown: {:.0} s",
        longest_wait.as_secs_f64()
    );
    {
        let s = mock.stats.lock().unwrap();
        eprintln!(
            "[bench] provider requests: {:?}; 429: {:?}",
            s.requests, s.throttled
        );
    }
    assert!(
        !dog.stalled.load(Ordering::Relaxed),
        "the metadata run stalled"
    );

    // Posters as the grid loads them: six at a time.
    let posters: Vec<String> = {
        let conn = pa_library::open(&db).unwrap();
        let mut stmt = conn
            .prepare("SELECT url FROM artwork WHERE kind = 'poster' AND work_id IS NOT NULL")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let images = Arc::new(
        pa_imagecache::ImageCache::open(
            dir.path().join("images"),
            pa_imagecache::CacheOptions::default(),
            Arc::new(ImageServer {
                latency: Duration::from_millis(env_u64("PA_BENCH_IMAGE_MS", 60)),
                served: AtomicU64::new(0),
            }),
        )
        .unwrap(),
    );
    let img_start = Instant::now();
    let queue = Arc::new(Mutex::new(posters.clone()));
    std::thread::scope(|s| {
        for _ in 0..6 {
            let (queue, images) = (queue.clone(), images.clone());
            s.spawn(move || {
                loop {
                    let Some(url) = queue.lock().unwrap().pop() else {
                        break;
                    };
                    let _ = images.get(&url, Some(342));
                }
            });
        }
    });
    phases.add("posters (image cache)", img_start.elapsed());
    dog.tick("posters".into());
    let visible = scan_start.elapsed() - meta + titles;
    eprintln!(
        "[bench] titles and posters of the library in: {:.1} s after the scan started",
        visible.as_secs_f64()
    );

    // The probes left for later.
    let deferred_start = Instant::now();
    let mut deferred = (0, 0);
    cat.probe_deferred(&cancel, &mut |u| {
        deferred = (u.processed, u.total.unwrap_or(0));
        dog.tick(format!("deferred probe {}/{:?}", u.processed, u.total));
        errors.extend(u.error);
    })
    .unwrap();
    phases.add("deferred probes (later episodes)", deferred_start.elapsed());
    eprintln!("[bench] deferred probes: {}/{}", deferred.0, deferred.1);
    assert!(
        !dog.stalled.load(Ordering::Relaxed),
        "the deferred probe stalled"
    );
    for (s, from, to) in &steps {
        eprintln!(
            "[bench]   scan step {s:?}: {:.1} s",
            to.duration_since(*from).as_secs_f64()
        );
    }

    let elapsed = total.elapsed();
    eprintln!("[bench] posters: {}", posters.len());
    let unprobed: i64 = pa_library::open(&db)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM files WHERE probed_at_ms IS NULL AND missing_since_ms IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!("[bench] files left unprobed: {unprobed}");
    eprintln!(
        "[bench] TOTAL after the share was reachable: {:.1} s",
        elapsed.as_secs_f64()
    );
    for (name, d) in &phases.0 {
        eprintln!("[bench] | {name} | {:.1} s |", d.as_secs_f64());
    }
    assert!(movies + shows > 0, "nothing was recognised");
    assert!(
        !report.stopped,
        "the metadata run stopped early: {:?}",
        report.error
    );
    assert!(!posters.is_empty(), "no posters");
    // Episodes named `NN Title.mkv` become films of their own until the
    // parser learns them (not LIB-023), so not every work is matched; but no
    // fetch may fail and every group the providers know is found.
    assert_eq!(report.failed, 0, "{report:?}");
    assert!(report.matched >= 180, "{report:?}");
    assert_eq!(unprobed, 0, "every file probed in the end");
    // The budget is for titles and posters of the whole library, the part
    // a person waits for.
    let budget = env_u64("PA_BENCH_MAX_SECS", 0);
    if budget > 0 {
        assert!(
            visible <= Duration::from_secs(budget),
            "titles and posters took {visible:?}, budget {budget} s"
        );
    }
}
