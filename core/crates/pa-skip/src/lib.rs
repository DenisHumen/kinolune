// SPDX-License-Identifier: MPL-2.0
//! Intro and credits detection for series (PLAY-011).
//!
//! - **Intro**: the audio the episodes of a season share near their start.
//!   Each file's intro window (the first 25 %, at most 10 min) gets an
//!   audio [`fingerprint`] — our own implementation of the Haitsma–Kalker
//!   band-energy fingerprint, no Chromaprint (LGPL) — and every episode
//!   is [`matching`] against its neighbours; the [`season`] agrees on the
//!   stretch most of them share, at different offsets in each file (cold
//!   opens differ).
//! - **Credits**: the last long run of black frames and light text on a
//!   dark background in the outro window ([`credits`]), else the ending
//!   song the episodes share (the same audio matching on the outro window).
//! - **Chapters** named "Opening", "OP", "Credits", "ED", "Recap",
//!   "Preview"… ([`chapters`]) are used as they are and win over both.
//! - **Imported markers** (PLAY-015, [`import`]): Plex and Jellyfin
//!   markers and EDL files next to the video win over all of the above
//!   (only a user correction beats them).
//!
//! [`analyze::analyze_file`] reads one file through the LGPL FFmpeg that
//! `pa-probe` loads at run time (audio decoded to mono in Rust, keyframe
//! thumbnails as for trickplay). [`job::analyze_season`] is the background
//! job over a library: it stores the per-file analysis (fingerprints
//! included, so a new episode is matched without reading the others again)
//! and the segments in `pa-library` (`skip_analysis`, `skip_segments`),
//! from which playback takes them (`pa_library::skip::effective_segments`).

pub mod analyze;
pub mod chapters;
pub mod credits;
pub mod fingerprint;
pub mod import;
pub mod job;
pub mod matching;
pub mod season;

pub use analyze::{AnalyzeOptions, FileAnalysis, Found, analyze_file, season_segments};
pub use import::{ImportReport, import_markers, import_season};
pub use job::{ANALYSIS_VERSION, JOB_KIND, Opener, SeasonReport, analyze_season};

/// Error of the library job.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("library: {0}")]
    Library(#[from] pa_library::Error),
    #[error("analysis cancelled")]
    Cancelled,
    /// Imported markers that do not parse.
    #[error("markers: {0}")]
    Import(String),
}
