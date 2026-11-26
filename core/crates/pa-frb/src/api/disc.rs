// SPDX-License-Identifier: MPL-2.0
//! Blu-ray and DVD folders and images for Dart (PLAY-017, ADR-0140): what
//! a disc item plays — its main title with the files of every angle, the
//! chapters and the languages of its streams, the extras — or that it is
//! protected. Files inside an image (`Film.iso/BDMV/STREAM/00010.m2ts`)
//! play through the stream proxy, which reads them from the image in
//! place. Methods of [`CoreLibrary`].

use pa_catalog::disc as d;

use crate::api::library::{CoreLibrary, LibraryError};
use crate::api::subtitles::CorePlayPart;

type Result<T> = std::result::Result<T, LibraryError>;

/// An audio or subtitle stream of a disc title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDiscStream {
    /// `audio` or `subtitle`.
    pub kind: String,
    /// Id of the stream as demuxers report it: the PID of a Blu-ray
    /// stream (`0x1100`), the substream of a DVD stream (`0x80`).
    pub id: u32,
    /// `TrueHD`, `PGS`, `AC-3`, `VobSub`.
    pub codec: String,
    /// ISO 639-2 (`eng`, Blu-ray) or 639-1 (`en`, DVD); empty if unknown.
    pub language: String,
}

/// A title of a disc: the main film or an extra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDiscTitle {
    /// Playlist (`00800`) or title set (`VTS_01`).
    pub id: String,
    pub duration_ms: i64,
    /// Files to play back to back for each angle; at least one angle.
    pub angles: Vec<Vec<CorePlayPart>>,
    /// Chapter starts from the start of the title.
    pub chapters_ms: Vec<i64>,
    /// Audio and subtitle streams in the order of the disc.
    pub streams: Vec<CoreDiscStream>,
    pub uhd: bool,
    /// 3D (MVC): plays as 2D.
    pub is_3d: bool,
}

/// What a disc item plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDiscPlayback {
    /// `bluray` or `dvd`.
    pub kind: String,
    /// `aacs`, `bdplus` or `css`: the disc is protected and is not played.
    pub protection: Option<String>,
    /// Volume name of an image, folder name of a disc folder.
    pub volume_id: String,
    /// `None`: no playable title.
    pub main: Option<CoreDiscTitle>,
    /// Other titles longer than about two minutes, longest first.
    pub extras: Vec<CoreDiscTitle>,
}

fn ms(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn part(f: d::DiscFile) -> CorePlayPart {
    CorePlayPart {
        rel_path: f.rel_path,
        local_path: f.local_path,
        size_bytes: ms(f.size),
        duration_ms: f.length_ms.map(ms),
    }
}

fn title(t: d::DiscTitleFiles) -> CoreDiscTitle {
    CoreDiscTitle {
        id: t.id,
        duration_ms: ms(t.duration_ms),
        angles: t
            .angles
            .into_iter()
            .map(|a| a.into_iter().map(part).collect())
            .collect(),
        chapters_ms: t.chapters_ms.into_iter().map(ms).collect(),
        streams: t
            .streams
            .into_iter()
            .filter_map(|s| {
                let kind = match s.kind {
                    d::StreamKind::Audio => "audio",
                    d::StreamKind::Subtitle | d::StreamKind::TextSubtitle => "subtitle",
                    d::StreamKind::Video => return None,
                };
                Some(CoreDiscStream {
                    kind: kind.to_owned(),
                    id: u32::from(s.id),
                    codec: s.codec,
                    language: s.lang,
                })
            })
            .collect(),
        uhd: t.uhd,
        is_3d: t.is_3d,
    }
}

impl From<d::DiscPlayback> for CoreDiscPlayback {
    fn from(p: d::DiscPlayback) -> Self {
        Self {
            kind: match p.kind {
                d::DiscKind::BluRay => "bluray",
                d::DiscKind::Dvd => "dvd",
            }
            .to_owned(),
            protection: p.protection.map(|x| x.code().to_owned()),
            volume_id: p.volume_id,
            main: p.main.map(title),
            extras: p.extras.into_iter().map(title).collect(),
        }
    }
}

impl CoreLibrary {
    /// What the disc of the file `rel_path` of source `source_id` plays
    /// (as in `CoreItem` and `CoreVersion`: the first clip of a disc
    /// folder or the image itself). `None` for a file that is not on a
    /// disc.
    pub fn disc_playback(
        &self,
        source_id: i64,
        rel_path: String,
    ) -> Result<Option<CoreDiscPlayback>> {
        Ok(self
            .opener()
            .disc_plan(source_id, &rel_path)?
            .map(Into::into))
    }
}

/// The files of the first angle of a disc's main title, if `rel_path` is
/// on a playable disc; for `play_parts`.
pub(crate) fn main_parts(
    library: &CoreLibrary,
    source_id: i64,
    rel_path: &str,
) -> Option<Vec<CorePlayPart>> {
    let plan = library.opener().disc_plan(source_id, rel_path).ok()??;
    if plan.protection.is_some() {
        return None;
    }
    let first = plan.main?.angles.into_iter().next()?;
    (!first.is_empty()).then(|| first.into_iter().map(part).collect())
}
