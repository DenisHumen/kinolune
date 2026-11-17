// SPDX-License-Identifier: MPL-2.0
//! Subtitles from the internet (PLAY-008, ADR о PLAY-008): search
//! OpenSubtitles for a video file of the library, rank what it offers and
//! keep the one the user picks.
//!
//! **What is asked.** The file's OSHash (`files.oshash`, LIB-004: it *is*
//! the OpenSubtitles moviehash) and, if the file is recognised, the ids of
//! its work: IMDb/TMDB of a movie, or IMDb/TMDB of the show with the
//! season and episode number of an episode. Languages: the profile's
//! subtitle languages (PLAY-004) plus English. If the ids find nothing,
//! the hash alone is tried once more. Searching spends no quota.
//!
//! **Ranking** ([`rank`]): subtitles made for this very file (moviehash
//! match: in sync with it) first, then by the place of their language in
//! the profile's list (other languages after), then human translations
//! before machine/AI ones, then by downloads. Releases split over several
//! CDs are left out: they do not fit one file.
//!
//! **Where a download goes** ([`Catalog::save_subtitle`]):
//! - next to the video, as `<video name>.<lang>[.forced][.sdh].<ext>`
//!   (`.2`, `.3` before the extension if the name is taken), when the
//!   source can be written (NET-023 [`pa_vfs::files::writability`]); the
//!   folder is then refreshed ([`Catalog::refresh_after_write`]), so the
//!   file is an ordinary sidecar from now on, on every device;
//! - otherwise (a read-only share, a media server, a write that failed)
//!   into the app's cache, `<cache>/<file fingerprint>/<name>`: attached
//!   to the file's content, so it survives a rename or a move of the
//!   video. [`Catalog::cached_subtitles`] lists them for the player.
//!
//! The file is kept **byte for byte**: an ASS file stays ASS with its
//! styles (PLAY-006 renders them), nothing is converted.

use std::cmp::Reverse;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use pa_library::rusqlite::{OptionalExtension, params};
use pa_metadata::opensubtitles::{
    self as os, OpenSubtitlesClient, OpenSubtitlesError, SearchQuery, Subtitle,
};
use pa_scanner::CancelToken;
use pa_vfs::VfsPath;
use pa_vfs::files::{self, Conflict};

use crate::items::{local_path, relative};
use crate::subtitles::Sidecar;
use crate::{Catalog, Error, sources};

/// Extensions of subtitle files that are kept as they come.
const EXTENSIONS: &[&str] = &["ass", "ssa", "srt", "vtt", "sub", "smi"];

/// Most numbered names (`.2` … `.N`) tried before giving up.
const MAX_NAME_TRIES: u32 = 50;

/// What the search asks about a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubtitleTarget {
    /// OSHash (16 hex digits), once the scan computed it.
    pub oshash: Option<String>,
    /// `movie` or `episode` when the file is recognised.
    pub kind: Option<String>,
    /// IMDb id of the movie (`tt…`).
    pub imdb_id: Option<String>,
    /// TMDB id of the movie.
    pub tmdb_id: Option<u64>,
    /// IMDb id of the show of an episode.
    pub parent_imdb_id: Option<String>,
    /// TMDB id of the show of an episode.
    pub parent_tmdb_id: Option<u64>,
    /// Season of an episode.
    pub season: Option<u32>,
    /// Episode number.
    pub episode: Option<u32>,
}

impl SubtitleTarget {
    fn has_ids(&self) -> bool {
        self.imdb_id.is_some()
            || self.tmdb_id.is_some()
            || ((self.parent_imdb_id.is_some() || self.parent_tmdb_id.is_some())
                && self.season.is_some()
                && self.episode.is_some())
    }

    fn query(&self, languages: &[String], with_ids: bool) -> SearchQuery {
        let mut q = SearchQuery {
            moviehash: self.oshash.clone(),
            languages: languages.to_vec(),
            ..SearchQuery::default()
        };
        if with_ids {
            q.kind.clone_from(&self.kind);
            if self.kind.as_deref() == Some("episode") {
                // The show's TMDB id is enough and more reliable than IMDb.
                q.parent_tmdb_id = self.parent_tmdb_id;
                if q.parent_tmdb_id.is_none() {
                    q.parent_imdb_id.clone_from(&self.parent_imdb_id);
                }
                q.season_number = self.season;
                q.episode_number = self.episode;
            } else {
                q.imdb_id.clone_from(&self.imdb_id);
                if q.imdb_id.is_none() {
                    q.tmdb_id = self.tmdb_id;
                }
            }
        }
        q
    }
}

/// One subtitle the user can pick, ranked by [`rank`].
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleOffer {
    /// What [`Catalog::download_subtitle`] takes.
    pub file_id: u64,
    /// OpenSubtitles subtitle id.
    pub subtitle_id: String,
    /// ISO 639-2/B code (`rus`), or the provider's code if unknown.
    pub language: Option<String>,
    /// The provider's language code (`ru`, `pt-br`).
    pub provider_language: Option<String>,
    /// Release name.
    pub release: String,
    /// Downloads so far.
    pub download_count: u64,
    /// Made for this very file (moviehash match).
    pub hash_match: bool,
    /// SDH / hearing impaired.
    pub hearing_impaired: bool,
    /// Only the foreign-language lines.
    pub forced: bool,
    /// Machine or AI translation.
    pub machine_translated: bool,
    /// From a trusted uploader.
    pub trusted: bool,
    /// Frame rate it was timed for.
    pub fps: Option<f64>,
    /// Uploader name.
    pub uploader: Option<String>,
}

impl SubtitleOffer {
    fn from_subtitle(s: Subtitle) -> Option<Self> {
        let file_id = s.single_file()?.file_id;
        let provider_language = s.language.clone().filter(|l| !l.is_empty());
        Some(Self {
            file_id,
            language: provider_language
                .as_deref()
                .map(|l| os::iso639_2(l).map_or_else(|| l.to_ascii_lowercase(), str::to_owned)),
            provider_language,
            release: s
                .release
                .clone()
                .or_else(|| s.files.first().and_then(|f| f.file_name.clone()))
                .unwrap_or_default(),
            download_count: s.download_count,
            hash_match: s.moviehash_match,
            hearing_impaired: s.hearing_impaired,
            forced: s.foreign_parts_only,
            machine_translated: s.machine_translated || s.ai_translated,
            trusted: s.from_trusted,
            fps: s.fps,
            uploader: s.uploader.and_then(|u| u.name),
            subtitle_id: s.subtitle_id,
        })
    }
}

/// Orders `subtitles` for the user (see the module docs). `languages`:
/// ISO 639-2/B codes of the profile, best first.
#[must_use]
pub fn rank(subtitles: Vec<Subtitle>, languages: &[String]) -> Vec<SubtitleOffer> {
    let mut offers: Vec<SubtitleOffer> = subtitles
        .into_iter()
        .filter_map(SubtitleOffer::from_subtitle)
        .collect();
    let place = |o: &SubtitleOffer| {
        o.language
            .as_deref()
            .and_then(|l| languages.iter().position(|p| p.eq_ignore_ascii_case(l)))
            .unwrap_or(languages.len())
    };
    offers.sort_by_key(|o| {
        (
            !o.hash_match,
            place(o),
            o.machine_translated,
            Reverse(o.download_count),
        )
    });
    // The same file can come twice (two pages, two queries).
    let mut seen = std::collections::HashSet::new();
    offers.retain(|o| seen.insert(o.file_id));
    offers
}

/// Why a search or a download did not give a subtitle.
#[derive(Debug, thiserror::Error)]
pub enum SubtitleSearchError {
    /// The library failed (unknown file, database).
    #[error(transparent)]
    Library(#[from] Error),
    /// OpenSubtitles failed or refused.
    #[error(transparent)]
    Provider(#[from] OpenSubtitlesError),
}

/// A downloaded subtitle and where it went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSubtitle {
    /// The subtitle as the player loads it, like the other sidecars:
    /// `rel_path` in the source for a file next to the video (network
    /// files play through the stream proxy), `local_path` for a local
    /// source or the cache.
    pub sidecar: Sidecar,
    /// `true`: written next to the video; `false`: in the app's cache.
    pub next_to_video: bool,
}

/// The file's row: id, OSHash, source URI.
type FileRow = (i64, Option<String>, String);

impl Catalog {
    /// What the subtitle search asks about the file `rel_path` of source
    /// `source_id` (as in [`Item::rel_path`](crate::Item::rel_path)).
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for a file the library does not know.
    pub fn subtitle_target(&self, source_id: i64, rel_path: &str) -> Result<SubtitleTarget, Error> {
        let conn = self.lock();
        let path = format!("/{}", rel_path.trim_start_matches('/'));
        let (file_id, oshash, _) = file_row(&conn, source_id, &path)?;
        let mut target = SubtitleTarget {
            oshash,
            ..SubtitleTarget::default()
        };
        let matched: Option<(i64, String, Option<i64>)> = conn
            .query_row(
                "SELECT m.work_id, w.kind,
                        (SELECT me.episode_id FROM match_episodes me
                          WHERE me.match_id = m.id ORDER BY me.position LIMIT 1)
                 FROM matches m JOIN works w ON w.id = m.work_id
                 WHERE m.file_id = ?1 AND m.state <> 'rejected'",
                [file_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((work_id, work_kind, episode_id)) = matched else {
            return Ok(target);
        };
        let ext = |provider: &str, kind: &str| -> Result<Option<String>, Error> {
            Ok(conn
                .query_row(
                    "SELECT ext_id FROM external_ids
                     WHERE work_id = ?1 AND provider = ?2 AND ext_kind = ?3",
                    params![work_id, provider, kind],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        };
        let imdb = ext("imdb", "")?;
        if work_kind == "movie" {
            target.kind = Some("movie".to_owned());
            target.imdb_id = imdb;
            target.tmdb_id = ext("tmdb", "movie")?.and_then(|id| id.trim().parse().ok());
            return Ok(target);
        }
        // A show or an anime series: the episode's numbers in TMDB order.
        let numbers: Option<(Option<i64>, Option<i64>)> = match episode_id {
            Some(id) => conn
                .query_row(
                    "SELECT season_number, episode_number FROM episodes WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?,
            None => None,
        };
        let tmdb_tv = ext("tmdb", "tv")?.and_then(|id| id.trim().parse().ok());
        if let Some((Some(season), Some(episode))) = numbers {
            target.kind = Some("episode".to_owned());
            target.parent_imdb_id = imdb;
            target.parent_tmdb_id = tmdb_tv;
            target.season = u32::try_from(season).ok();
            target.episode = u32::try_from(episode).ok();
        } else if let Some(movie_id) = ext("tmdb", "movie")? {
            // An anime film filed as a work of kind `anime`.
            target.kind = Some("movie".to_owned());
            target.imdb_id = imdb;
            target.tmdb_id = movie_id.trim().parse().ok();
        }
        Ok(target)
    }

    /// Searches OpenSubtitles for the file `rel_path` of source
    /// `source_id` and ranks the offers ([`rank`]). `languages`: the
    /// profile's subtitle languages (ISO 639 codes, best first); English
    /// is always asked for too. Spends no download quota.
    ///
    /// Empty when the file has neither an OSHash nor ids yet.
    ///
    /// # Errors
    ///
    /// An unknown file, and the provider's errors
    /// ([`OpenSubtitlesError::NoKey`] without a key: nothing is sent).
    pub fn search_subtitles(
        &self,
        client: &OpenSubtitlesClient,
        source_id: i64,
        rel_path: &str,
        languages: &[String],
    ) -> Result<Vec<SubtitleOffer>, SubtitleSearchError> {
        let target = self.subtitle_target(source_id, rel_path)?;
        let profile: Vec<String> = languages
            .iter()
            .filter_map(|l| {
                os::search_codes(l)
                    .first()
                    .and_then(|c| os::iso639_2(c))
                    .map(str::to_owned)
            })
            .collect();
        let mut codes: Vec<String> = Vec::new();
        for l in profile.iter().map(String::as_str).chain(["eng"]) {
            for c in os::search_codes(l) {
                if !codes.iter().any(|x| x == c) {
                    codes.push(c.to_owned());
                }
            }
        }
        let mut found = Vec::new();
        if target.has_ids() {
            found = client.search(&target.query(&codes, true))?;
        }
        if found.is_empty() && target.oshash.is_some() {
            found = client.search(&target.query(&codes, false))?;
        }
        Ok(rank(found, &profile))
    }

    /// Downloads `offer` (one download of the quota) and keeps it for the
    /// file `rel_path` of source `source_id`: next to the video if the
    /// source can be written, else in `cache_dir` (see the module docs).
    ///
    /// # Errors
    ///
    /// An unknown file, the provider's errors (the quota used up:
    /// [`OpenSubtitlesError::QuotaExhausted`]), and a cache that cannot be
    /// written.
    pub fn download_subtitle(
        &self,
        client: &OpenSubtitlesClient,
        source_id: i64,
        rel_path: &str,
        offer: &SubtitleOffer,
        cache_dir: &Path,
    ) -> Result<SavedSubtitle, SubtitleSearchError> {
        let link = client.download(offer.file_id)?;
        let bytes = client.fetch(&link)?;
        let ext = subtitle_extension(link.file_name.as_deref(), &bytes);
        Ok(self.save_subtitle(source_id, rel_path, offer, ext, &bytes, cache_dir)?)
    }

    /// Keeps a downloaded subtitle file (`bytes`, extension `ext`) for the
    /// file `rel_path` of source `source_id`: next to it when the source
    /// can be written, else in `cache_dir`.
    ///
    /// # Errors
    ///
    /// An unknown file, or the cache cannot be written.
    pub fn save_subtitle(
        &self,
        source_id: i64,
        rel_path: &str,
        offer: &SubtitleOffer,
        ext: &str,
        bytes: &[u8],
        cache_dir: &Path,
    ) -> Result<SavedSubtitle, Error> {
        let video = VfsPath::parse(&format!("/{}", rel_path.trim_start_matches('/')))
            .map_err(|e| Error::Invalid(e.to_string()))?;
        let (_, oshash, uri) = {
            let conn = self.lock();
            file_row(&conn, source_id, video.as_str())?
        };
        let stem = video
            .file_name()
            .map(|n| n.rsplit_once('.').map_or(n, |(s, _)| s).to_owned())
            .unwrap_or_else(|| "subtitle".to_owned());
        let tail = name_tail(offer);
        match self.write_next_to(source_id, &video, &stem, &tail, ext, bytes) {
            Ok(Some(path)) => {
                let rel = relative(path.as_str());
                let sidecar = sidecar_of(offer, rel.clone(), local_path(&uri, &rel));
                return Ok(SavedSubtitle {
                    sidecar,
                    next_to_video: true,
                });
            }
            Ok(None) => {}
            Err(e) => {
                tracing::info!(
                    target: "pa_catalog::subtitle_search",
                    error = %e,
                    "subtitle not written next to the video; keeping it in the cache"
                );
            }
        }
        let dir = cache_dir.join(cache_key(oshash.as_deref(), source_id, video.as_str()));
        std::fs::create_dir_all(&dir).map_err(|e| Error::Io(e.to_string()))?;
        let path = free_local_name(&dir, &stem, &tail, ext);
        std::fs::write(&path, bytes).map_err(|e| Error::Io(e.to_string()))?;
        let local = path.to_string_lossy().into_owned();
        Ok(SavedSubtitle {
            sidecar: sidecar_of(offer, local.clone(), Some(local)),
            next_to_video: false,
        })
    }

    /// Writes the subtitle next to the video; `Ok(None)` when the source
    /// cannot be written.
    fn write_next_to(
        &self,
        source_id: i64,
        video: &VfsPath,
        stem: &str,
        tail: &str,
        ext: &str,
        bytes: &[u8],
    ) -> Result<Option<VfsPath>, Error> {
        let source = {
            let conn = self.lock();
            sources::get(&conn, source_id)?
                .ok_or_else(|| Error::NotFound(format!("source {source_id}")))?
        };
        let vfs = self.connector().connect(&source)?;
        if !files::writability(vfs.as_ref())? {
            return Ok(None);
        }
        let dir = video.parent().unwrap_or_else(VfsPath::root);
        let mut name = None;
        for n in 1..=MAX_NAME_TRIES {
            let candidate = numbered(stem, tail, ext, n);
            match vfs.stat(&dir.join(&candidate)?) {
                Err(e) if e.kind() == pa_vfs::ErrorKind::NotFound => {
                    name = Some(candidate);
                    break;
                }
                Err(e) => return Err(e.into()),
                Ok(_) => {}
            }
        }
        let name = name.ok_or_else(|| Error::Invalid("no free subtitle name".to_owned()))?;
        let outcome = files::upload(
            vfs.as_ref(),
            &dir,
            &name,
            &mut Cursor::new(bytes.to_vec()),
            Conflict::Skip,
            &mut |_: u64, _: u64| true,
        )?;
        if outcome.skipped {
            return Ok(None);
        }
        drop(vfs);
        // The new file becomes an ordinary sidecar (group_assets). A
        // failure here is fixed by the next scan; the player gets the path.
        if let Err(e) = self.refresh_after_write(source_id, &outcome.touched, &CancelToken::new()) {
            tracing::warn!(
                target: "pa_catalog::subtitle_search",
                error = %e,
                "library refresh after a subtitle download failed"
            );
        }
        Ok(Some(outcome.path))
    }

    /// Subtitles of the file `rel_path` of source `source_id` that were
    /// downloaded into `cache_dir` (its source could not be written), by
    /// the file's fingerprint. Empty when there are none.
    ///
    /// # Errors
    ///
    /// An unknown file.
    pub fn cached_subtitles(
        &self,
        source_id: i64,
        rel_path: &str,
        cache_dir: &Path,
    ) -> Result<Vec<Sidecar>, Error> {
        let path = format!("/{}", rel_path.trim_start_matches('/'));
        let (_, oshash, _) = {
            let conn = self.lock();
            file_row(&conn, source_id, &path)?
        };
        let dir = cache_dir.join(cache_key(oshash.as_deref(), source_id, &path));
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };
        let mut out: Vec<Sidecar> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let (stem, ext) = name.rsplit_once('.')?;
                if !EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
                    return None;
                }
                let info = pa_scanner::structure::subtitle_info(stem);
                let local = e.path().to_string_lossy().into_owned();
                Some(Sidecar {
                    rel_path: local.clone(),
                    local_path: Some(local),
                    lang: info.lang,
                    forced: info.forced,
                    hearing_impaired: info.hearing_impaired,
                    default: false,
                })
            })
            .collect();
        out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        Ok(out)
    }
}

fn file_row(
    conn: &pa_library::rusqlite::Connection,
    source_id: i64,
    path: &str,
) -> Result<FileRow, Error> {
    conn.query_row(
        "SELECT f.id, f.oshash, s.uri FROM files f JOIN sources s ON s.id = f.source_id
         WHERE f.source_id = ?1 AND f.path = ?2",
        params![source_id, path],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()?
    .ok_or_else(|| Error::NotFound(format!("file {source_id}:{path}")))
}

fn sidecar_of(offer: &SubtitleOffer, rel_path: String, local_path: Option<String>) -> Sidecar {
    Sidecar {
        rel_path,
        local_path,
        lang: offer.language.clone(),
        forced: offer.forced,
        hearing_impaired: offer.hearing_impaired,
        default: false,
    }
}

/// `.ru`, `.ru.forced`, `.en.sdh` — what the scanner reads back as the
/// language and flags (`pa_scanner::structure::subtitle_info`).
fn name_tail(offer: &SubtitleOffer) -> String {
    let mut tail = String::new();
    let lang = offer
        .provider_language
        .as_deref()
        .and_then(|l| l.split('-').next())
        .filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphabetic()));
    if let Some(l) = lang {
        tail.push('.');
        tail.push_str(&l.to_ascii_lowercase());
    }
    if offer.forced {
        tail.push_str(".forced");
    }
    if offer.hearing_impaired {
        tail.push_str(".sdh");
    }
    tail
}

/// `Movie.ru.ass`, then `Movie.ru.2.ass`, `Movie.ru.3.ass`.
fn numbered(stem: &str, tail: &str, ext: &str, n: u32) -> String {
    if n <= 1 {
        format!("{stem}{tail}.{ext}")
    } else {
        format!("{stem}{tail}.{n}.{ext}")
    }
}

fn free_local_name(dir: &Path, stem: &str, tail: &str, ext: &str) -> PathBuf {
    (1..=MAX_NAME_TRIES)
        .map(|n| dir.join(numbered(stem, tail, ext, n)))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(numbered(stem, tail, ext, MAX_NAME_TRIES + 1)))
}

/// Cache folder of a file: its OSHash; a file not hashed yet gets a key
/// of its place (FNV-1a of source and path).
fn cache_key(oshash: Option<&str>, source_id: i64, path: &str) -> String {
    if let Some(h) = oshash.filter(|h| !h.is_empty()) {
        return h.to_owned();
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in format!("{source_id}:{path}").bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("p{hash:016x}")
}

/// The extension a downloaded file is kept with: the one of the name the
/// provider gives if it is a subtitle format, else what the content says.
#[must_use]
pub fn subtitle_extension(file_name: Option<&str>, bytes: &[u8]) -> &'static str {
    if let Some(ext) = file_name
        .and_then(|n| n.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase())
        && let Some(known) = EXTENSIONS.iter().find(|e| **e == ext)
    {
        return known;
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("[Script Info]") {
        if head.contains("v4.00+") || head.contains("[V4+ Styles]") {
            "ass"
        } else {
            "ssa"
        }
    } else if head.starts_with("WEBVTT") {
        "vtt"
    } else {
        "srt"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pa_metadata::opensubtitles::SubtitleFile;

    fn sub(id: u64, lang: &str, downloads: u64, hash: bool, machine: bool) -> Subtitle {
        serde_json::from_value(serde_json::json!({
            "subtitle_id": id.to_string(),
            "language": lang,
            "download_count": downloads,
            "moviehash_match": hash,
            "machine_translated": machine,
            "files": [{"file_id": id}],
        }))
        .unwrap()
    }

    #[test]
    fn ranks_hash_then_language_then_human_then_downloads() {
        let mut cds = sub(9, "ru", 1, true, false);
        cds.files.push(SubtitleFile {
            file_id: 10,
            cd_number: Some(2),
            file_name: None,
        });
        let subs = vec![
            sub(1, "en", 5000, false, false),
            sub(2, "ru", 10, false, false),
            sub(3, "ru", 9000, false, true),
            sub(4, "en", 3, true, false),
            sub(5, "ru", 2, true, false),
            sub(6, "fr", 99999, false, false),
            sub(7, "ru", 500, false, false),
            cds,
        ];
        let langs = vec!["rus".to_owned(), "eng".to_owned()];
        let order: Vec<u64> = rank(subs, &langs).iter().map(|o| o.file_id).collect();
        assert_eq!(order, [5, 4, 7, 2, 3, 1, 6]);
    }

    #[test]
    fn extension_from_name_or_content() {
        assert_eq!(subtitle_extension(Some("x.ASS"), b""), "ass");
        assert_eq!(
            subtitle_extension(Some("noext"), b"[Script Info]\nScriptType: v4.00+\n"),
            "ass"
        );
        assert_eq!(
            subtitle_extension(None, b"\xef\xbb\xbf[Script Info]\nScriptType: v4.00\n"),
            "ssa"
        );
        assert_eq!(subtitle_extension(None, b"WEBVTT\n\n"), "vtt");
        assert_eq!(
            subtitle_extension(Some("a.zip"), b"1\n00:00:01,000 --> "),
            "srt"
        );
    }

    #[test]
    fn names_carry_language_and_flags() {
        let mut offer = SubtitleOffer::from_subtitle(sub(1, "pt-br", 1, false, false)).unwrap();
        assert_eq!(offer.language.as_deref(), Some("por"));
        assert_eq!(name_tail(&offer), ".pt");
        offer.forced = true;
        offer.hearing_impaired = true;
        assert_eq!(name_tail(&offer), ".pt.forced.sdh");
        assert_eq!(numbered("Movie", ".ru", "ass", 1), "Movie.ru.ass");
        assert_eq!(numbered("Movie", ".ru", "ass", 3), "Movie.ru.3.ass");
        let info = pa_scanner::structure::subtitle_info(".ru.3");
        assert_eq!(info.lang.as_deref(), Some("rus"));
    }
}
