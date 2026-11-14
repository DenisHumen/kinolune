// SPDX-License-Identifier: MPL-2.0
//! The preview service: one [`Session`] per media file, filled in the
//! background from the best available origin.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use pa_probe::Ffmpeg;
use pa_vfs::{ByteSource, Vfs, VfsPath};

use crate::bif::Bif;
use crate::cache::{TrickplayCache, Usage};
use crate::generate::{self, GenerateOptions, LiveFrames};
use crate::sidecar::{self, Crop, JellyfinSheets, Sidecar};
use crate::Error;

/// A media file to preview.
#[derive(Debug, Clone)]
pub struct Media {
    /// Identity of the file's content for the cache: e.g. source, path,
    /// size and modification time. A changed file must get a new key.
    pub key: String,
    /// The file.
    pub source: Arc<dyn ByteSource>,
    /// Where to look for previews of other software (BIF, Jellyfin):
    /// the file's source and its path there.
    pub sidecar: Option<(Arc<dyn Vfs>, VfsPath)>,
    /// Known duration (from the library), for Jellyfin folders.
    pub duration_ms: Option<u64>,
}

/// Where a session's previews come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A BIF next to the media (Plex, Emby, Roku).
    SidecarBif,
    /// A Jellyfin trickplay folder next to the media.
    Jellyfin,
    /// Our cache, generated earlier.
    Cache,
    /// Generated now.
    Generated,
}

/// State of a [`Session`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Looking for existing previews.
    Searching,
    /// Generating: `done` of `total` thumbnails; previews are already
    /// served from the finished ones.
    Generating { done: usize, total: usize },
    /// Complete.
    Ready(Origin),
    /// No previews: generation is off, impossible (no FFmpeg, no video) or
    /// failed. Frames generated before a failure are still served.
    Unavailable(String),
}

/// One preview image for a position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// A JPEG: the thumbnail itself, or a sheet of tiles to crop.
    pub image: Arc<[u8]>,
    /// The tile inside `image`; `None` for the whole image.
    pub crop: Option<Crop>,
    /// Identifies `image` within the session (equal ids, equal bytes), so a
    /// UI can keep a decoded sheet.
    pub image_id: u64,
    /// Time of the thumbnail (the keyframe or the tile's slot).
    pub position_ms: u64,
}

enum Content {
    None,
    Live(Arc<LiveFrames>),
    Bif(Arc<Bif>),
    Jellyfin(Arc<JellyfinSheets>),
}

/// Previews of one media file. Cheap to query from any thread.
pub struct Session {
    key: String,
    status: Mutex<Status>,
    changed: Condvar,
    content: RwLock<Content>,
    cancel: Arc<AtomicBool>,
    users: AtomicUsize,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl Session {
    fn new(key: String) -> Self {
        Self {
            key,
            status: Mutex::new(Status::Searching),
            changed: Condvar::new(),
            content: RwLock::new(Content::None),
            cancel: Arc::new(AtomicBool::new(false)),
            users: AtomicUsize::new(1),
        }
    }

    /// Current state; while generating, with live progress.
    #[must_use]
    pub fn status(&self) -> Status {
        let status = self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Status::Generating { .. } = status
            && let Content::Live(live) = &*self.content()
        {
            return Status::Generating {
                done: live.filled(),
                total: live.total(),
            };
        }
        status
    }

    fn set_status(&self, status: Status) {
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = status;
        self.changed.notify_all();
    }

    fn content(&self) -> std::sync::RwLockReadGuard<'_, Content> {
        self.content
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set_content(&self, content: Content) {
        *self
            .content
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = content;
    }

    /// Time between previews, once known.
    #[must_use]
    pub fn interval_ms(&self) -> Option<u64> {
        match &*self.content() {
            Content::None => None,
            Content::Live(l) => Some(l.interval_ms()),
            Content::Bif(b) => Some(b.interval_ms()),
            Content::Jellyfin(j) => Some(j.interval_ms()),
        }
    }

    /// The preview for `position_ms`, or `None` if there is none (yet).
    /// While generating it is the closest finished thumbnail. Fast for
    /// generated, cached and BIF previews (memory); a Jellyfin sheet not
    /// read yet is read from the source first.
    #[must_use]
    pub fn frame_at(&self, position_ms: u64) -> Option<Frame> {
        let content = self.content();
        match &*content {
            Content::None => None,
            Content::Live(live) => {
                let (i, image, pos) = live.nearest(position_ms)?;
                Some(Frame {
                    image,
                    crop: None,
                    image_id: i as u64,
                    position_ms: pos,
                })
            }
            Content::Bif(bif) => {
                let i = bif.index_at(position_ms)?;
                let (jpeg, pos) = bif.frame(i)?;
                Some(Frame {
                    image: jpeg.into(),
                    crop: None,
                    image_id: i as u64,
                    position_ms: pos,
                })
            }
            Content::Jellyfin(sheets) => {
                let sheets = Arc::clone(sheets);
                drop(content);
                let (sheet, image, crop, pos) = sheets.tile_at(position_ms).ok()??;
                Some(Frame {
                    image,
                    crop: Some(crop),
                    image_id: sheet as u64,
                    position_ms: pos,
                })
            }
        }
    }

    /// Waits until the session is no longer searching or generating, at
    /// most `timeout`; the status then.
    #[must_use]
    pub fn wait(&self, timeout: Duration) -> Status {
        let deadline = Instant::now() + timeout;
        let mut status = self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while matches!(*status, Status::Searching | Status::Generating { .. }) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            status = self
                .changed
                .wait_timeout(status, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        drop(status);
        self.status()
    }

    /// Stops a running generation (nothing is cached then).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// A user of the session is done with it; the last one stops a running
    /// generation (see [`Trickplay::prepare`]).
    pub fn release(&self) {
        if self.users.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.cancel();
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Settings of a [`Trickplay`] service.
#[derive(Debug, Clone)]
pub struct TrickplayOptions {
    /// Generate previews when nothing exists (clients: a user option;
    /// the server: always, as a background job).
    pub generate: bool,
    /// How to generate.
    pub generation: GenerateOptions,
    /// Size limit of the disk cache.
    pub cache_max_bytes: u64,
}

impl Default for TrickplayOptions {
    fn default() -> Self {
        Self {
            generate: true,
            generation: GenerateOptions::default(),
            cache_max_bytes: crate::cache::DEFAULT_MAX_BYTES,
        }
    }
}

/// See the crate docs.
pub struct Trickplay {
    cache: Arc<TrickplayCache>,
    options: Mutex<TrickplayOptions>,
    ffmpeg: Option<&'static Ffmpeg>,
    sessions: Mutex<Vec<Weak<Session>>>,
}

impl std::fmt::Debug for Trickplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Trickplay")
            .field("cache", &self.cache.dir())
            .finish_non_exhaustive()
    }
}

impl Trickplay {
    /// Opens the service with its cache in `cache_dir`, generating with the
    /// process-wide FFmpeg ([`Ffmpeg::shared`]) when needed.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the cache directory cannot be created.
    pub fn open(cache_dir: impl Into<std::path::PathBuf>, options: TrickplayOptions) -> Result<Self, Error> {
        Self::with_ffmpeg(cache_dir, options, None)
    }

    /// As [`open`](Self::open) with a given FFmpeg (`None`: the shared one,
    /// loaded on first generation).
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the cache directory cannot be created.
    pub fn with_ffmpeg(
        cache_dir: impl Into<std::path::PathBuf>,
        options: TrickplayOptions,
        ffmpeg: Option<&'static Ffmpeg>,
    ) -> Result<Self, Error> {
        let cache = TrickplayCache::open(cache_dir, options.cache_max_bytes)?;
        Ok(Self {
            cache: Arc::new(cache),
            options: Mutex::new(options),
            ffmpeg,
            sessions: Mutex::new(Vec::new()),
        })
    }

    fn options(&self) -> TrickplayOptions {
        self.options
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Turns generation on or off for sessions prepared from now on.
    pub fn set_generate(&self, generate: bool) {
        self.options
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .generate = generate;
    }

    /// The disk cache.
    #[must_use]
    pub fn cache(&self) -> &TrickplayCache {
        &self.cache
    }

    /// Cache size.
    #[must_use]
    pub fn usage(&self) -> Usage {
        self.cache.usage()
    }

    /// Starts (or joins) the session of `media` and returns at once; the
    /// work runs on a background thread: a sidecar BIF or Jellyfin folder,
    /// else our cache, else generation (if enabled), whose result is
    /// cached. Every caller must [`Session::release`] it when done; the
    /// last release stops an unfinished generation.
    #[must_use]
    pub fn prepare(&self, media: Media) -> Arc<Session> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        sessions.retain(|w| w.strong_count() > 0);
        if let Some(existing) = sessions
            .iter()
            .filter_map(Weak::upgrade)
            .find(|s| s.key == media.key && !s.is_cancelled())
        {
            existing.users.fetch_add(1, Ordering::AcqRel);
            return existing;
        }
        let session = Arc::new(Session::new(media.key.clone()));
        sessions.push(Arc::downgrade(&session));
        drop(sessions);

        let worker = Arc::clone(&session);
        let cache = Arc::clone(&self.cache);
        let options = self.options();
        let ffmpeg = self.ffmpeg;
        let spawned = std::thread::Builder::new()
            .name("trickplay".into())
            .spawn(move || fill(&worker, &media, &cache, &options, ffmpeg));
        if let Err(e) = spawned {
            session.set_status(Status::Unavailable(format!("no thread: {e}")));
        }
        session
    }

    /// Prepares `media` and waits for the result, at most `timeout`: for a
    /// background job (the server generates ahead of playback).
    #[must_use]
    pub fn ensure(&self, media: Media, timeout: Duration) -> Status {
        let session = self.prepare(media);
        let status = session.wait(timeout);
        session.release();
        status
    }
}

fn fill(
    session: &Session,
    media: &Media,
    cache: &TrickplayCache,
    options: &TrickplayOptions,
    ffmpeg: Option<&'static Ffmpeg>,
) {
    if let Some((vfs, path)) = &media.sidecar {
        match sidecar::find(vfs, path, media.duration_ms) {
            Ok(Some(Sidecar::Bif(bif))) => {
                session.set_content(Content::Bif(Arc::new(bif)));
                return session.set_status(Status::Ready(Origin::SidecarBif));
            }
            Ok(Some(Sidecar::Jellyfin(sheets))) => {
                session.set_content(Content::Jellyfin(Arc::new(sheets)));
                return session.set_status(Status::Ready(Origin::Jellyfin));
            }
            // A folder that cannot be listed does not stop us.
            Ok(None) | Err(_) => {}
        }
    }
    if let Some(bif) = cache.load(&media.key).and_then(|d| Bif::parse(d).ok()) {
        session.set_content(Content::Bif(Arc::new(bif)));
        return session.set_status(Status::Ready(Origin::Cache));
    }
    if !options.generate {
        return session.set_status(Status::Unavailable("generation is off".into()));
    }
    let ffmpeg = match ffmpeg.map_or_else(Ffmpeg::shared, Ok) {
        Ok(f) => f,
        Err(e) => return session.set_status(Status::Unavailable(e.to_string())),
    };
    let mut opts = options.generation.clone();
    opts.cancel = Some(Arc::clone(&session.cancel));
    let live = match generate::prepare(ffmpeg, &media.source, &opts) {
        Ok(live) => live,
        Err(e) => return session.set_status(Status::Unavailable(e.to_string())),
    };
    session.set_content(Content::Live(Arc::clone(&live)));
    session.set_status(Status::Generating {
        done: 0,
        total: live.total(),
    });
    match generate::run(ffmpeg, &media.source, &opts, &live) {
        Ok(_) => {
            // A failed write only costs a later regeneration.
            let _ = cache.store(&media.key, &live.to_bif());
            session.set_status(Status::Ready(Origin::Generated));
        }
        Err(e) => session.set_status(Status::Unavailable(e.to_string())),
    }
}
