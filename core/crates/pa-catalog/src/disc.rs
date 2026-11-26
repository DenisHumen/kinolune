// SPDX-License-Identifier: MPL-2.0
//! Playing Blu-ray and DVD folders and images (PLAY-017, ADR-0140).
//!
//! An item of a disc points at the first clip of its main title
//! (`Film/BDMV/STREAM/00010.m2ts`, LIB-021) or at the image itself
//! (`Film.iso`). [`SourceOpener::disc_plan`] reads the disc
//! ([`pa_scanner::disc::plan`]) through the source, and for an image
//! through [`IsoFs`] over the image file, and returns the files to play
//! as paths of the source: a file in an image is
//! `Film.iso/BDMV/STREAM/00010.m2ts`, which the stream proxy serves from
//! inside the image ([`SourceOpener::open_file`]). Nothing is unpacked.

use std::sync::Arc;

use pa_scanner::disc::{DiscPlan, DiscSegment, DiscTitle};
pub use pa_scanner::disc::{DiscKind, DiscStream, Protection};
pub use pa_scanner::mpls::StreamKind;
use pa_vfs::VfsPath;
use pa_vfs::iso::{IsoFs, is_image_name};

use crate::items::{local_path, relative};
use crate::{Error, SourceOpener};

/// One file of a disc title, or a part of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscFile {
    /// Path relative to the source root, like
    /// [`Item::rel_path`](crate::Item::rel_path); inside an image:
    /// `Film.iso/BDMV/STREAM/00010.m2ts`.
    pub rel_path: String,
    /// Absolute path for files of a local folder; `None` for network
    /// sources and files inside images (the app plays them through the
    /// stream proxy).
    pub local_path: Option<String>,
    pub size: u64,
    /// Start in the file and length (`None`: whole file).
    pub start_ms: Option<u64>,
    pub length_ms: Option<u64>,
}

/// A title of a disc with the files of each angle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscTitleFiles {
    /// Playlist (`00800`) or title set (`VTS_01`).
    pub id: String,
    pub duration_ms: u64,
    /// Files to play for each angle (at least one angle).
    pub angles: Vec<Vec<DiscFile>>,
    pub chapters_ms: Vec<u64>,
    pub streams: Vec<DiscStream>,
    pub uhd: bool,
    pub is_3d: bool,
}

/// What a disc item plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscPlayback {
    pub kind: DiscKind,
    /// Copy protection: the disc is not played (ADR-0139).
    pub protection: Option<Protection>,
    /// Volume name of an image (`MY_MOVIE`); the folder name of a disc
    /// folder.
    pub volume_id: String,
    /// `None`: no playable title.
    pub main: Option<DiscTitleFiles>,
    pub extras: Vec<DiscTitleFiles>,
}

/// Where the disc of a file is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscRoot {
    /// A disc image file.
    Image(VfsPath),
    /// The folder holding `BDMV` or `VIDEO_TS`.
    Folder(VfsPath),
}

/// The disc a file belongs to: the image itself, or the folder above its
/// `BDMV` or `VIDEO_TS`; `None` for other files.
#[must_use]
pub fn disc_root(path: &VfsPath) -> Option<DiscRoot> {
    if path.file_name().is_some_and(is_image_name) {
        return Some(DiscRoot::Image(path.clone()));
    }
    let mut root = VfsPath::root();
    for name in path.components() {
        if name.eq_ignore_ascii_case("BDMV") || name.eq_ignore_ascii_case("VIDEO_TS") {
            return Some(DiscRoot::Folder(root));
        }
        root = root.join(name).ok()?;
    }
    None
}

fn title_files(
    t: DiscTitle,
    rel: &dyn Fn(&VfsPath) -> String,
    local: &dyn Fn(&str) -> Option<String>,
) -> DiscTitleFiles {
    let file = |s: DiscSegment| {
        let rel_path = rel(&s.path);
        DiscFile {
            local_path: local(&rel_path),
            rel_path,
            size: s.size,
            start_ms: s.start_ms,
            length_ms: s.length_ms,
        }
    };
    DiscTitleFiles {
        id: t.id,
        duration_ms: t.duration_ms,
        angles: t
            .angles
            .into_iter()
            .map(|a| a.into_iter().map(file).collect())
            .collect(),
        chapters_ms: t.chapters_ms,
        streams: t.streams,
        uhd: t.uhd,
        is_3d: t.is_3d,
    }
}

fn playback(
    plan: DiscPlan,
    volume_id: String,
    rel: &dyn Fn(&VfsPath) -> String,
    local: &dyn Fn(&str) -> Option<String>,
) -> DiscPlayback {
    DiscPlayback {
        kind: plan.kind,
        protection: plan.protection,
        volume_id,
        main: plan.main.map(|t| title_files(t, rel, local)),
        extras: plan
            .extras
            .into_iter()
            .map(|t| title_files(t, rel, local))
            .collect(),
    }
}

impl SourceOpener {
    /// What the disc of the file `rel_path` of source `source_id` plays
    /// (an [`Item::rel_path`](crate::Item::rel_path) or a version's): its
    /// main title, extras, angles, chapters and streams, or its
    /// protection. `None` for a file that is not on a disc, or a folder
    /// without `BDMV` or `VIDEO_TS`.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for a file the active profile may not open;
    /// [`Error::Io`] when the source or image cannot be read.
    pub fn disc_plan(&self, source_id: i64, rel_path: &str) -> Result<Option<DiscPlayback>, Error> {
        let path = VfsPath::parse(rel_path)?;
        let Some(root) = disc_root(&path) else {
            return Ok(None);
        };
        if !self.catalog().may_open(source_id, path.as_str())? {
            return Err(Error::NotFound(rel_path.to_owned()));
        }
        let source = crate::sources::get(&self.catalog().lock(), source_id)?
            .ok_or_else(|| Error::NotFound(format!("source {source_id}")))?;
        match root {
            DiscRoot::Image(image) => {
                let fs = self.image(source_id, &image)?;
                let Some(plan) = pa_scanner::disc::plan(fs.as_ref(), &VfsPath::root())? else {
                    return Ok(None);
                };
                let rel = |p: &VfsPath| relative(&format!("{}{}", image.as_str(), p.as_str()));
                Ok(Some(playback(
                    plan,
                    fs.volume_id().to_owned(),
                    &rel,
                    &|_| None,
                )))
            }
            DiscRoot::Folder(folder) => {
                let vfs = self.vfs(source_id)?;
                let Some(plan) = pa_scanner::disc::plan(vfs.as_ref(), &folder)? else {
                    return Ok(None);
                };
                let rel = |p: &VfsPath| relative(p.as_str());
                let local = |r: &str| {
                    source
                        .local_root
                        .as_ref()
                        .and_then(|_| local_path(&source.uri, r))
                };
                Ok(Some(playback(
                    plan,
                    folder.file_name().unwrap_or_default().to_owned(),
                    &rel,
                    &local,
                )))
            }
        }
    }

    /// The image `image` of source `id` opened as a file system; the
    /// last few are kept, so the proxy reads clip after clip without
    /// reading the image's tree again.
    pub fn image(&self, id: i64, image: &VfsPath) -> pa_vfs::Result<Arc<IsoFs>> {
        if let Some(fs) = self.cached_image(id, image) {
            return Ok(fs);
        }
        let file = self.open_file(id, image)?;
        let fs = Arc::new(IsoFs::open(file)?);
        self.keep_image(id, image, Arc::clone(&fs));
        Ok(fs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots() {
        let p = |s: &str| VfsPath::parse(s).unwrap();
        assert_eq!(
            disc_root(&p("/m/Film (2020)/BDMV/STREAM/00010.m2ts")),
            Some(DiscRoot::Folder(p("/m/Film (2020)")))
        );
        assert_eq!(
            disc_root(&p("/Film/video_ts/VTS_01_1.VOB")),
            Some(DiscRoot::Folder(p("/Film")))
        );
        assert_eq!(
            disc_root(&p("/m/Film.ISO")),
            Some(DiscRoot::Image(p("/m/Film.ISO")))
        );
        assert_eq!(disc_root(&p("/m/Film.mkv")), None);
        assert_eq!(
            disc_root(&p("/BDMV/STREAM/1.m2ts")),
            Some(DiscRoot::Folder(VfsPath::root()))
        );
    }
}
