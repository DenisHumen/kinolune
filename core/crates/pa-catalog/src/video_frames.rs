// SPDX-License-Identifier: MPL-2.0
//! Preview frames of the user's own videos (UI-032): one frame
//! from the file itself instead of a coloured tile, for the videos of a
//! personal library ([`Catalog::personal_frame`]) and for «Мои видео»
//! ([`Catalog::video_frame`]).
//!
//! - **Which frame.** The keyframe at about 10 % of the duration, decoded
//!   by the LGPL FFmpeg of `pa-probe` (as trickplay, PLAY-009, but one
//!   frame) at [`FRAME_WIDTH`] px. A black, mostly dark or flat frame
//!   (`frame_search::is_usable`, META-031) is skipped for the next of
//!   [`POSITIONS`]; if none is usable, the brightest that is not black is
//!   kept; a file without one gets no frame (the tile stays).
//! - **Where.** JPEG files in the app's own directory, next to the
//!   database: `frames/main/` for the main library, `frames/personal-<id>/`
//!   for each personal library — never in the shared image cache
//!   (`pa-imagecache`, LIB-013). The name is the file's content
//!   fingerprint (else its oshash, else a digest of where it lies), so a
//!   frame survives a rescan, a move and a rename and says nothing of the
//!   file's name. A file without a usable frame leaves an empty `.none`
//!   marker, so it is not decoded again.
//! - **Personal libraries (ADR-0098).** A frame is read, made and returned
//!   only while its library is unlocked, through the same check as every
//!   other call ([`Error::Denied`] for a locked library, [`Error::NotFound`]
//!   for another profile's): the check runs before the cache is read and
//!   again before a new frame is written and returned, so a library
//!   locked while a frame was being decoded gets nothing stored. The bytes
//!   go back to the app, never a path. Deleting a library (or its profile)
//!   deletes its frame folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pa_library::rusqlite::{Connection, OptionalExtension};
use pa_probe::VideoFrame;

use crate::frame_search::{FrameReader, FrameSource, is_usable};
use crate::{Catalog, Error};

/// Width of a preview frame in pixels (a 240-pt tile on a 2× screen).
pub const FRAME_WIDTH: u32 = 480;

/// JPEG quality of a stored frame.
const QUALITY: u8 = 80;

/// Where to look for a frame, as shares of the duration: about 10 % first,
/// then further in when it is black.
pub const POSITIONS: [f64; 6] = [0.10, 0.15, 0.20, 0.30, 0.40, 0.50];

/// Where to look when the duration is unknown (ms).
const POSITIONS_NO_DURATION: [u64; 4] = [10_000, 30_000, 60_000, 2_000];

/// The frame cache of a catalog: `frames/` next to its database.
fn frames_root(db: &Path) -> PathBuf {
    db.parent()
        .map_or_else(|| PathBuf::from("frames"), |d| d.join("frames"))
}

/// The folder of the frames of personal library `id`.
fn personal_dir(db: &Path, id: i64) -> PathBuf {
    frames_root(db).join(format!("personal-{id}"))
}

/// The file whose frame is asked for.
struct Target {
    source_id: i64,
    path: String,
    duration_ms: Option<u64>,
    key: String,
}

/// The file `file_id` if the connection (in whatever scope it is) lets the
/// active profile open it.
fn target(conn: &Connection, file_id: i64) -> Result<Target, Error> {
    // (source, path, size, duration, fingerprint or oshash)
    type Row = (i64, String, i64, Option<i64>, Option<String>);
    let row: Option<Row> = conn
        .query_row(
            concat!(
                "SELECT f.source_id, f.path, f.size_bytes, f.duration_ms,
                        coalesce(f.fingerprint, f.oshash)
                   FROM files f
                  WHERE f.id = ?1 AND f.missing_since_ms IS NULL AND ",
                pa_library::file_allowed!("f")
            ),
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((source_id, path, size, duration, fingerprint)) = row else {
        return Err(Error::NotFound(format!("file {file_id}")));
    };
    let key = fingerprint.unwrap_or_else(|| {
        // Not fingerprinted yet: where it lies and how big it is.
        use md4::{Digest, Md4};
        let digest = Md4::digest(format!("{source_id}\0{path}\0{size}").as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        format!("p{hex}")
    });
    Ok(Target {
        source_id,
        path,
        duration_ms: duration.and_then(|d| u64::try_from(d).ok()),
        key,
    })
}

/// Mean luma (0–255) of a frame, every 4th pixel.
fn brightness(frame: &VideoFrame) -> f64 {
    let (pixels, _) = frame.rgb.as_chunks::<3>();
    let (mut n, mut sum) = (0u64, 0u64);
    for p in pixels.iter().step_by(4) {
        n += 1;
        sum += (54 * u64::from(p[0]) + 183 * u64::from(p[1]) + 19 * u64::from(p[2])) >> 8;
    }
    if n == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let mean = sum as f64 / n as f64;
    mean
}

/// The preview frame of an open file: the first usable one of
/// [`POSITIONS`], else the brightest that is not black.
///
/// # Errors
///
/// A message when reading or decoding fails before any frame came out.
pub fn pick_frame(
    reader: &mut dyn FrameReader,
    duration_ms: Option<u64>,
) -> Result<Option<VideoFrame>, String> {
    let duration = reader.duration_ms().or(duration_ms).filter(|d| *d > 0);
    let at: Vec<u64> = match duration {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        Some(d) => POSITIONS
            .iter()
            .map(|s| (d as f64 * s).round() as u64)
            .collect(),
        None => POSITIONS_NO_DURATION.to_vec(),
    };
    let mut best: Option<(f64, VideoFrame)> = None;
    let mut last_error = None;
    for ms in at {
        match reader.frame_at(ms) {
            Ok(Some(frame)) => {
                if is_usable(&frame) {
                    return Ok(Some(frame));
                }
                let b = brightness(&frame);
                if best.as_ref().is_none_or(|(bb, _)| b > *bb) {
                    best = Some((b, frame));
                }
            }
            Ok(None) => {}
            Err(e) => last_error = Some(e),
        }
    }
    match (best, last_error) {
        (Some((b, frame)), _) if b >= 20.0 => Ok(Some(frame)),
        (None, Some(e)) => Err(e),
        _ => Ok(None),
    }
}

/// Reads a cached frame: `Some(Some(jpeg))`, `Some(None)` for a file known
/// to have none, `None` if not cached.
fn cached(dir: &Path, key: &str) -> Option<Option<Vec<u8>>> {
    if let Ok(jpeg) = std::fs::read(dir.join(format!("{key}.jpg"))) {
        return Some(Some(jpeg));
    }
    dir.join(format!("{key}.none")).exists().then_some(None)
}

/// Stores a frame (or its absence) atomically.
fn store(dir: &Path, key: &str, jpeg: Option<&[u8]>) -> Result<(), Error> {
    std::fs::create_dir_all(dir).map_err(|e| Error::Io(e.to_string()))?;
    let Some(jpeg) = jpeg else {
        return std::fs::write(dir.join(format!("{key}.none")), b"")
            .map_err(|e| Error::Io(e.to_string()));
    };
    let tmp = dir.join(format!("{key}.jpg.tmp"));
    std::fs::write(&tmp, jpeg).map_err(|e| Error::Io(e.to_string()))?;
    std::fs::rename(&tmp, dir.join(format!("{key}.jpg"))).map_err(|e| Error::Io(e.to_string()))
}

impl Catalog {
    /// Decodes the preview frame of `t` (no lock of the catalog held):
    /// `Ok(None)` if the file has none worth showing.
    fn make_frame(&self, t: &Target, frames: &dyn FrameSource) -> Result<Option<Vec<u8>>, Error> {
        let vfs: Arc<dyn pa_vfs::Vfs> = self.connect(t.source_id)?;
        let mut reader = frames.open(vfs, &t.path).map_err(Error::Io)?;
        let Some(frame) = pick_frame(reader.as_mut(), t.duration_ms).map_err(Error::Io)? else {
            return Ok(None);
        };
        crate::frame_search::encode(&frame, QUALITY)
            .map(Some)
            .map_err(Error::Io)
    }

    /// The preview frame (JPEG) of video `file_id` of personal library
    /// `id`, made on first use; `None` when the file has no frame worth
    /// showing. Only while the library is unlocked: a locked one answers
    /// [`Error::Denied`] and nothing is read, made or stored.
    ///
    /// # Errors
    ///
    /// [`Error::Denied`], [`Error::NotFound`] (another profile's library,
    /// a file not in it); [`Error::Io`] when the file cannot be read or
    /// decoded (not remembered: tried again next time).
    pub fn personal_frame(
        &self,
        id: i64,
        file_id: i64,
        frames: &dyn FrameSource,
    ) -> Result<Option<Vec<u8>>, Error> {
        let dir = personal_dir(self.path(), id);
        let t = {
            let conn = self.scoped(Some(id))?;
            let t = target(&conn, file_id)?;
            if let Some(hit) = cached(&dir, &t.key) {
                return Ok(hit);
            }
            t
        };
        let jpeg = self.make_frame(&t, frames)?;
        // Locked (or deleted) while decoding: keep nothing, show nothing.
        drop(self.scoped(Some(id))?);
        store(&dir, &t.key, jpeg.as_deref())?;
        Ok(jpeg)
    }

    /// The preview frame (JPEG) of file `file_id` of the main library
    /// («Мои видео», LIB-025), made on first use; `None` when it has no
    /// frame worth showing. A personal file is not found here.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`]; [`Error::Io`] when the file cannot be read or
    /// decoded.
    pub fn video_frame(
        &self,
        file_id: i64,
        frames: &dyn FrameSource,
    ) -> Result<Option<Vec<u8>>, Error> {
        let dir = frames_root(self.path()).join("main");
        let t = {
            let conn = self.scoped(None)?;
            let t = target(&conn, file_id)?;
            if let Some(hit) = cached(&dir, &t.key) {
                return Ok(hit);
            }
            t
        };
        let jpeg = self.make_frame(&t, frames)?;
        store(&dir, &t.key, jpeg.as_deref())?;
        Ok(jpeg)
    }

    /// Deletes the frame folders of personal libraries that no longer
    /// exist (after a library or a profile was deleted).
    pub(crate) fn prune_personal_frames(&self, conn: &Connection) {
        let root = frames_root(self.path());
        let Ok(entries) = std::fs::read_dir(&root) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(id) = name
                .to_str()
                .and_then(|n| n.strip_prefix("personal-"))
                .and_then(|n| n.parse::<i64>().ok())
            else {
                continue;
            };
            // A failed lookup keeps the folder.
            let exists = !matches!(
                conn.query_row(
                    "SELECT 1 FROM personal_libraries WHERE id = ?1",
                    [id],
                    |_| Ok(()),
                )
                .optional(),
                Ok(None)
            );
            if !exists && let Err(e) = std::fs::remove_dir_all(entry.path()) {
                tracing::warn!(
                    target: "pa_catalog::video_frames",
                    library = id,
                    error = %e,
                    "cannot delete the frames of a deleted personal library"
                );
            }
        }
    }
}
