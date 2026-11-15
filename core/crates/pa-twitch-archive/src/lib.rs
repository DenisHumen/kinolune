// SPDX-License-Identifier: MPL-2.0
//! Twitch VOD downloader (TWI-017, research 06 §4.1), full builds only.
//!
//! The whole crate is the `twitch-archive` capability of `pa-flavor`
//! (CORE-013, ADR-0041): in a store build it is an empty library, so no
//! symbol or string of the downloader reaches a store binary
//! (`tools/flavor/full-only-patterns.txt` lists `pa_twitch_archive`).
//!
//! # Pipeline
//!
//! A download is a `pa-jobs` job of type [`kinds::TWITCH_VOD_DOWNLOAD`]
//! with a `target_storage` (CORE-012): the storage gate admits it only when
//! its estimated bytes fit above the free-space reserve and inside the
//! quota, and the storage monitor pauses it when space runs out. The
//! handler, [`VodDownloader`]:
//!
//! 1. checks the remote-config `twitch.archive` flag and the native-playback
//!    decision ([`ArchiveSource::check_allowed`]); off → the job fails with
//!    `archive_disabled`;
//! 2. reads the VOD from Helix (title, owner, date, duration, thumbnail,
//!    muted ranges) and its chapters (TWI-008);
//! 3. resolves usher's master playlist through `PlaybackAccessToken`
//!    (TWI-004). Sub-only, geo-blocked or otherwise restricted VODs are
//!    **not** bypassed: usher's refusal fails the job with `usher_403` and
//!    Twitch's error code;
//! 4. picks a variant ([`Quality`]), reads its media playlist and, for a
//!    partial download, the segments covering the range ([`playlist`]);
//! 5. downloads the missing segments with 4–8 workers into
//!    `<target>/.tmp/job-<id>/seg/` (`.part`, length check, rename). The
//!    [`SegmentBitmap`] of finished segments is the job checkpoint, so a
//!    killed process resumes with only what is missing; finished files
//!    found on disk are adopted too. Muted segments (`*-muted.ts`) are
//!    downloaded as Twitch serves them and marked;
//! 6. fetches the thumbnail and the channel avatar (attachments and
//!    `-thumb.jpg`), the chat replay through [`ArchiveSource::chat_replay`]
//!    when TWI-016 provides it and the flags allow (`.chat.json.gz`);
//! 7. writes the `ffmetadata` (global `TWITCH_*` tags, chapters) and
//!    remuxes without re-encoding into MKV (default) or MP4 through the
//!    runtime-loaded LGPL FFmpeg ([`FfmpegMuxer`], `pa_probe::Ffmpeg::remux`);
//! 8. probes the result and compares its duration with Helix (or with the
//!    requested range) within [`DURATION_TOLERANCE`]; a mismatch fails the
//!    job with `duration_mismatch`;
//! 9. commits the folder `<login> [<user_id>]/<year>/<name> [v<id>]/`
//!    (video, `.info.json`, `-thumb.jpg`, `.chat.json.gz`) atomically with
//!    `StorageManager::commit_file`, group `twitch:<user_id>` for retention.
//!
//! Nothing here is compiled with a Twitch client id: GQL settings come only
//! from the signed remote config, as for playback.

#[cfg(all(pa_twitch_archive, pa_twitch_native))]
mod archive;

#[cfg(all(pa_twitch_archive, pa_twitch_native))]
pub use archive::*;
