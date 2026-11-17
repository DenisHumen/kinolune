// SPDX-License-Identifier: MPL-2.0
//! The trailer of a movie or show page (UI-036, ADR-0100) for Dart.
//!
//! [`CoreLibrary::trailer`] picks it with `pa-trailers`: the user's own
//! trailer file, else the media server's trailer (Dart asks
//! `CoreJellyfin::server_trailer` and passes the URL in), else — with
//! internet trailers on — a TMDB video (YouTube or Vimeo) for the official
//! embedded player. TMDB is asked only when it is the last resort; the
//! answer is cached with the other provider responses.

use std::path::PathBuf;
use std::sync::Arc;

use pa_catalog as c;
use pa_catalog::pa_metadata as m;

use crate::api::details::{CoreExtra, CoreFileRef};
use crate::api::library::{CoreLibrary, LibraryError, item_ref};

type Result<T> = std::result::Result<T, LibraryError>;

/// What [`CoreLibrary::trailer`] needs besides the item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTrailerRequest {
    /// UI language (`ru`, `en-US`): the first language a video is wanted
    /// in; then the title's original language, then English.
    pub ui_language: String,
    /// Internet trailers (TMDB, YouTube, Vimeo) are allowed.
    pub internet: bool,
    /// The media server's trailer stream URL, if it has one
    /// (`CoreJellyfin::server_trailer`).
    pub server_url: Option<String>,
    /// The user's own TMDB key; `None` uses the key built into the app.
    pub tmdb_api_key: Option<String>,
    /// Directory of the provider response cache (as for metadata runs).
    pub cache_dir: String,
}

/// The trailer a page plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTrailer {
    /// `file`, `server`, `youtube` or `vimeo`.
    pub source: String,
    /// `file`: the extra to play.
    pub extra: Option<CoreExtra>,
    /// `server`: the stream URL (contains a token: do not log it);
    /// `youtube`/`vimeo`: the page of the video at its site (opens the
    /// site's app where no embedded player is possible).
    pub url: Option<String>,
    /// `youtube`/`vimeo`: the validated video id for the embedded player.
    pub video_key: Option<String>,
    pub name: String,
    /// ISO 639-1, if known.
    pub language: Option<String>,
    /// `trailer`, `teaser` or `clip`.
    pub video_type: String,
}

impl CoreLibrary {
    /// The trailer of movie or show `item_id`, or `None` (then the page
    /// shows its slowly moving backdrop). Blocks on TMDB when it is asked;
    /// a TMDB failure is "no trailer", not an error.
    pub fn trailer(
        &self,
        item_id: String,
        request: CoreTrailerRequest,
    ) -> Result<Option<CoreTrailer>> {
        let item = item_ref(&item_id)?;
        let catalog = self.catalog();
        let details = catalog.details(item)?;
        let types: Vec<&str> = details
            .extras
            .iter()
            .map(|e| e.extra_type.as_str())
            .collect();
        let choice =
            pa_trailers::choose(&types, request.server_url.clone(), request.internet, || {
                online(&catalog, item, &details, &request)
            });
        Ok(choice.map(|choice| match choice {
            pa_trailers::Choice::Own(i) => {
                let e = details.extras[i].clone();
                CoreTrailer {
                    source: "file".into(),
                    name: e.title.clone(),
                    video_type: e.extra_type.clone(),
                    extra: Some(CoreExtra {
                        file: CoreFileRef::from(e.file),
                        extra_type: e.extra_type,
                        title: e.title,
                        size_bytes: e.size_bytes,
                        duration_ms: e.duration_ms,
                    }),
                    url: None,
                    video_key: None,
                    language: None,
                }
            }
            pa_trailers::Choice::Server(url) => CoreTrailer {
                source: "server".into(),
                extra: None,
                url: Some(url),
                video_key: None,
                name: String::new(),
                language: None,
                video_type: "trailer".into(),
            },
            pa_trailers::Choice::Online(t) => CoreTrailer {
                source: t.site.as_str().into(),
                extra: None,
                url: Some(t.watch_url()),
                video_key: Some(t.key),
                name: t.name,
                language: t.language,
                video_type: t.video_type.as_str().into(),
            },
        }))
    }
}

/// The TMDB video of `item`, if it has a TMDB id and TMDB answers.
fn online(
    catalog: &c::Catalog,
    item: c::ItemRef,
    details: &c::ItemDetails,
    request: &CoreTrailerRequest,
) -> Option<pa_trailers::OnlineTrailer> {
    let id = details.external_ids.iter().find(|x| x.provider == "tmdb")?;
    let kind = match id.kind.as_str() {
        "movie" => m::tmdb::MediaKind::Movie,
        "tv" => m::tmdb::MediaKind::Tv,
        _ => return None,
    };
    let tmdb_id: u64 = id.id.trim().parse().ok()?;
    let original = catalog.original_language(item).ok().flatten();
    let languages = pa_trailers::preferred_languages(&request.ui_language, original.as_deref());

    let cache_dir = PathBuf::from(&request.cache_dir);
    std::fs::create_dir_all(&cache_dir).ok()?;
    let cache = m::cache::SqliteCache::open(&cache_dir.join("provider-cache.sqlite")).ok()?;
    let key = request
        .tmdb_api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty());
    let client = m::tmdb::TmdbClient::live(m::tmdb::TmdbConfig::default(), key, Arc::new(cache));
    // No key, offline, rate limited: the page shows its backdrop.
    let body = client
        .videos(
            kind,
            tmdb_id,
            &pa_trailers::include_video_language(&languages),
        )
        .ok()?;
    let videos = pa_trailers::parse_tmdb_videos(&body).ok()?;
    pa_trailers::pick_online(&videos, &languages)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(root: &std::path::Path, rel: &str, seed: u8) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let data: Vec<u8> = (0..150_000u32)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect();
        std::fs::write(path, data).unwrap();
    }

    fn request(dir: &std::path::Path, server: Option<&str>) -> CoreTrailerRequest {
        CoreTrailerRequest {
            ui_language: "ru-RU".into(),
            internet: true,
            server_url: server.map(str::to_owned),
            tmdb_api_key: None,
            cache_dir: dir.join("cache").to_string_lossy().into(),
        }
    }

    /// Own file first, then the server; without a TMDB id nothing is
    /// fetched (no network in this test).
    #[test]
    fn own_trailer_file_then_server_trailer() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("media");
        media(&root, "Movies/Arrival (2016)/Arrival.2016.mkv", 1);
        media(
            &root,
            "Movies/Arrival (2016)/Trailers/Arrival Trailer.mkv",
            2,
        );
        media(&root, "Movies/Dune (2021)/Dune.2021.mkv", 3);
        let lib = CoreLibrary::open(dir.path().join("l.db").to_string_lossy().into()).unwrap();
        let catalog = lib.catalog();
        catalog
            .add_source(None, "local", "Media", root.to_str().unwrap(), None)
            .unwrap();
        catalog
            .scan(None, &c::CancelToken::new(), &mut |_| {})
            .unwrap();
        let movies = catalog.movies().unwrap();
        let id = |title: &str| {
            movies
                .iter()
                .find(|m| m.title == title)
                .unwrap_or_else(|| panic!("{title} in {movies:?}"))
                .id
                .to_string()
        };

        let own = lib
            .trailer(id("Arrival"), request(dir.path(), Some("http://srv/t")))
            .unwrap()
            .expect("own trailer");
        assert_eq!(own.source, "file");
        let extra = own.extra.unwrap();
        assert_eq!(extra.title, "Arrival Trailer");
        assert_eq!(extra.extra_type, "trailer");
        assert!(extra.file.rel_path.ends_with("Trailers/Arrival Trailer.mkv"));

        let server = lib
            .trailer(id("Dune"), request(dir.path(), Some("http://srv/t")))
            .unwrap()
            .expect("server trailer");
        assert_eq!(server.source, "server");
        assert_eq!(server.url.as_deref(), Some("http://srv/t"));

        // No own file, no server, no TMDB id: none (and no request).
        assert_eq!(
            lib.trailer(id("Dune"), request(dir.path(), None)).unwrap(),
            None
        );
    }
}
