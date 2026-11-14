// SPDX-License-Identifier: MPL-2.0
//! Previews other software left next to the media, read through `pa-vfs`
//! (local folders, SMB, WebDAV alike):
//!
//! - **BIF** of Plex, Emby and Roku: `Movie.bif`, `Movie-320-10.bif`,
//!   `Movie-SD.bif`/`-HD.bif`/`-FHD.bif`.
//! - **Jellyfin trickplay** saved with the media (10.9+, "Save trickplay
//!   images next to media"): `Movie.trickplay/<width> - <cols>x<rows>/<n>.jpg`,
//!   sheets of `cols × rows` tiles, tile `i` of the film at `i × interval`.

use std::sync::{Arc, Mutex};

use pa_vfs::{ByteSource, EntryKind, Vfs, VfsPath};

use crate::{Error, bif::Bif, jpeg};

/// Thumbnail width we look for among several variants.
const PREFERRED_WIDTH: u32 = 320;
/// Interval Jellyfin uses unless configured otherwise.
pub const JELLYFIN_DEFAULT_INTERVAL_MS: u64 = 10_000;
/// Sheets kept in memory by a [`JellyfinSheets`].
const SHEET_CACHE: usize = 6;
/// Largest sidecar file read into memory.
const MAX_FILE: u64 = 256 << 20;

/// A preview found next to a media file.
#[derive(Debug)]
pub enum Sidecar {
    Bif(Bif),
    Jellyfin(JellyfinSheets),
}

/// Looks for a BIF or a Jellyfin trickplay folder next to `media` on `vfs`.
/// `duration_ms` (if known) helps to read a Jellyfin folder, which does not
/// record its interval.
///
/// # Errors
///
/// [`Error::Source`] if the folder cannot be listed or a found file cannot
/// be read; `Ok(None)` if there is nothing.
pub fn find(
    vfs: &Arc<dyn Vfs>,
    media: &VfsPath,
    duration_ms: Option<u64>,
) -> Result<Option<Sidecar>, Error> {
    let (Some(dir), Some(name)) = (media.parent(), media.file_name()) else {
        return Ok(None);
    };
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let entries = vfs.list(&dir)?;

    let mut bifs: Vec<(u8, &pa_vfs::DirEntry)> = entries
        .iter()
        .filter(|e| e.metadata.kind == EntryKind::File)
        .filter_map(|e| bif_rank(stem, &e.name).map(|r| (r, e)))
        .collect();
    bifs.sort_by_key(|(rank, e)| (*rank, e.name.clone()));
    if let Some((_, entry)) = bifs.first() {
        let data = read_all(vfs.open(&entry.path)?.as_ref())?;
        if let Ok(bif) = Bif::parse(data) {
            return Ok(Some(Sidecar::Bif(bif)));
        }
    }

    let folder = format!("{stem}.trickplay");
    if let Some(trick) = entries
        .iter()
        .find(|e| e.metadata.kind == EntryKind::Dir && e.name.eq_ignore_ascii_case(&folder))
    {
        return JellyfinSheets::open(Arc::clone(vfs), &trick.path, duration_ms);
    }
    Ok(None)
}

/// Preference of a BIF named `file` for media `stem` (lower is better), or
/// `None` if it does not belong to it.
fn bif_rank(stem: &str, file: &str) -> Option<u8> {
    let lower = file.to_lowercase();
    let base = lower.strip_suffix(".bif")?;
    let stem = stem.to_lowercase();
    let rest = base.strip_prefix(&stem)?;
    match rest {
        "" => Some(1),
        _ if rest.starts_with(&format!("-{PREFERRED_WIDTH}-")) => Some(0),
        "-hd" | "-fhd" => Some(2),
        "-sd" => Some(3),
        _ if rest.starts_with('-') => Some(4),
        _ => None,
    }
}

fn read_all(source: &dyn ByteSource) -> Result<Vec<u8>, Error> {
    let size = source.size();
    if size > MAX_FILE {
        return Err(Error::Invalid(format!("sidecar too large: {size} bytes")));
    }
    let mut data = vec![0u8; usize::try_from(size).unwrap_or(0)];
    source.read_exact_at(0, &mut data)?;
    Ok(data)
}

/// `"320 - 10x10"` → `(320, 10, 10)`.
#[must_use]
pub fn parse_resolution_dir(name: &str) -> Option<(u32, u32, u32)> {
    let (width, grid) = name.split_once('-')?;
    let (cols, rows) = grid.trim().split_once('x')?;
    let parsed = (
        width.trim().parse().ok()?,
        cols.trim().parse().ok()?,
        rows.trim().parse().ok()?,
    );
    (parsed.0 > 0 && parsed.1 > 0 && parsed.2 > 0).then_some(parsed)
}

/// A rectangle inside a sheet image, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A Jellyfin trickplay folder: JPEG sheets read on demand.
pub struct JellyfinSheets {
    vfs: Arc<dyn Vfs>,
    sheets: Vec<VfsPath>,
    cols: u32,
    rows: u32,
    tile: (u32, u32),
    interval_ms: u64,
    cache: Mutex<Vec<(usize, Arc<[u8]>)>>,
}

impl std::fmt::Debug for JellyfinSheets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JellyfinSheets")
            .field("sheets", &self.sheets.len())
            .field("grid", &(self.cols, self.rows))
            .field("tile", &self.tile)
            .field("interval_ms", &self.interval_ms)
            .finish_non_exhaustive()
    }
}

impl JellyfinSheets {
    /// Opens `<media>.trickplay` at `dir`: picks the resolution closest to
    /// 320 px, lists its sheets and reads the first one for the tile size.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if listing or reading fails; `Ok(None)` if the
    /// folder holds no usable resolution.
    pub fn open(
        vfs: Arc<dyn Vfs>,
        dir: &VfsPath,
        duration_ms: Option<u64>,
    ) -> Result<Option<Sidecar>, Error> {
        let Some((res, (width, cols, rows))) = vfs
            .list(dir)?
            .into_iter()
            .filter(|e| e.metadata.kind == EntryKind::Dir)
            .filter_map(|e| parse_resolution_dir(&e.name).map(|r| (e.path, r)))
            .min_by_key(|(_, (w, _, _))| w.abs_diff(PREFERRED_WIDTH))
        else {
            return Ok(None);
        };
        let mut sheets: Vec<(u32, VfsPath)> = vfs
            .list(&res)?
            .into_iter()
            .filter(|e| e.metadata.kind == EntryKind::File)
            .filter_map(|e| {
                let n = e.name.to_lowercase();
                let n = n.strip_suffix(".jpg")?.parse().ok()?;
                Some((n, e.path))
            })
            .collect();
        sheets.sort_by_key(|(n, _)| *n);
        // Sheets are numbered from 0 without gaps.
        let sheets: Vec<VfsPath> = sheets
            .into_iter()
            .enumerate()
            .take_while(|(i, (n, _))| *i as u32 == *n)
            .map(|(_, (_, p))| p)
            .collect();
        let Some(first) = sheets.first() else {
            return Ok(None);
        };
        let data: Arc<[u8]> = read_all(vfs.open(first)?.as_ref())?.into();
        let Some((sheet_w, sheet_h)) = jpeg::dimensions(&data) else {
            return Ok(None);
        };
        let per_sheet = u64::from(cols) * u64::from(rows);
        let tile_w = (sheet_w / cols).max(1).min(width.max(1));
        // A full first sheet gives the tile height; a single, partly filled
        // one needs the number of tiles, from the duration.
        let rows_used = if sheets.len() > 1 {
            rows
        } else {
            duration_ms.map_or(rows, |d| {
                let tiles = d.div_ceil(JELLYFIN_DEFAULT_INTERVAL_MS).clamp(1, per_sheet);
                u32::try_from(tiles.div_ceil(u64::from(cols))).unwrap_or(rows)
            })
        };
        let tile_h = (sheet_h / rows_used.max(1)).max(1);
        let interval_ms = interval(duration_ms, sheets.len() as u64 * per_sheet, per_sheet);
        Ok(Some(Sidecar::Jellyfin(Self {
            vfs,
            sheets,
            cols,
            rows,
            tile: (tile_w, tile_h),
            interval_ms,
            cache: Mutex::new(vec![(0, data)]),
        })))
    }

    /// Time between tiles.
    #[must_use]
    pub fn interval_ms(&self) -> u64 {
        self.interval_ms
    }

    /// Tiles the folder can hold (the last sheet may be partly empty).
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.sheets.len() * (self.cols * self.rows) as usize
    }

    /// The sheet and the tile for `position_ms`: `(sheet index, sheet JPEG,
    /// crop, tile time)`.
    ///
    /// # Errors
    ///
    /// [`Error::Source`] if the sheet cannot be read.
    pub fn tile_at(
        &self,
        position_ms: u64,
    ) -> Result<Option<(usize, Arc<[u8]>, Crop, u64)>, Error> {
        let per = (self.cols * self.rows) as usize;
        if self.sheets.is_empty() {
            return Ok(None);
        }
        let index = usize::try_from(position_ms / self.interval_ms)
            .unwrap_or(usize::MAX)
            .min(self.capacity() - 1);
        let sheet = index / per;
        let within = u32::try_from(index % per).unwrap_or(0);
        let data = self.sheet(sheet)?;
        let crop = Crop {
            x: (within % self.cols) * self.tile.0,
            y: (within / self.cols) * self.tile.1,
            width: self.tile.0,
            height: self.tile.1,
        };
        // Past the filled part of the last sheet: its last full row.
        let crop = match jpeg::dimensions(&data) {
            Some((_, h)) if crop.y + crop.height > h => Crop {
                y: h.saturating_sub(self.tile.1),
                x: (self.cols - 1) * self.tile.0,
                ..crop
            },
            _ => crop,
        };
        Ok(Some((sheet, data, crop, index as u64 * self.interval_ms)))
    }

    fn sheet(&self, i: usize) -> Result<Arc<[u8]>, Error> {
        {
            let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((_, d)) = cache.iter().find(|(n, _)| *n == i) {
                return Ok(Arc::clone(d));
            }
        }
        let data: Arc<[u8]> = read_all(self.vfs.open(&self.sheets[i])?.as_ref())?.into();
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(0, (i, Arc::clone(&data)));
        cache.truncate(SHEET_CACHE);
        Ok(data)
    }
}

/// Jellyfin's interval: 10 s unless the duration says the folder was made
/// with another one (then rounded to whole seconds).
fn interval(duration_ms: Option<u64>, capacity: u64, per_sheet: u64) -> u64 {
    let Some(d) = duration_ms.filter(|d| *d > 0) else {
        return JELLYFIN_DEFAULT_INTERVAL_MS;
    };
    // The tiles lie between the full sheets before the last one and all.
    let min_tiles = capacity.saturating_sub(per_sheet) + 1;
    let tiles_default = d.div_ceil(JELLYFIN_DEFAULT_INTERVAL_MS);
    if (min_tiles..=capacity).contains(&tiles_default) {
        return JELLYFIN_DEFAULT_INTERVAL_MS;
    }
    // Middle of the possible range.
    let tiles = (min_tiles + capacity) / 2;
    (d.div_ceil(tiles.max(1)).div_ceil(1000) * 1000).max(1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bif_names() {
        assert_eq!(bif_rank("Dune (2021)", "Dune (2021).bif"), Some(1));
        assert_eq!(bif_rank("Dune (2021)", "Dune (2021)-320-10.bif"), Some(0));
        assert_eq!(bif_rank("Dune (2021)", "dune (2021)-HD.BIF"), Some(2));
        assert_eq!(bif_rank("Dune (2021)", "Dune (2021)-SD.bif"), Some(3));
        assert_eq!(bif_rank("Dune (2021)", "Dune (2021)-240-5.bif"), Some(4));
        assert_eq!(bif_rank("Dune", "Dune Part Two.bif"), None);
        assert_eq!(bif_rank("Dune", "Dune.jpg"), None);
    }

    #[test]
    fn resolution_dirs() {
        assert_eq!(parse_resolution_dir("320 - 10x10"), Some((320, 10, 10)));
        assert_eq!(parse_resolution_dir("640-5x4"), Some((640, 5, 4)));
        assert_eq!(parse_resolution_dir("0 - 10x10"), None);
        assert_eq!(parse_resolution_dir("posters"), None);
    }

    #[test]
    fn jellyfin_interval() {
        // 2 h at 10 s = 720 tiles in 8 sheets of 100.
        assert_eq!(interval(Some(7_200_000), 800, 100), 10_000);
        // The same film in 3 sheets: made at a longer interval.
        assert_eq!(interval(Some(7_200_000), 300, 100), 29_000);
        assert_eq!(interval(None, 300, 100), 10_000);
    }
}
