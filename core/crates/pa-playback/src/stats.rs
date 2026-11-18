// SPDX-License-Identifier: MPL-2.0
//! The stats overlay model (PLAY-010, ADR-NNNN): one record every engine
//! fills — the input stream, the output, the decoder path, frames, the
//! buffer against the network, AFR and the audio path.
//!
//! Every field added by PLAY-010 is optional: an engine fills what it can
//! measure and leaves the rest `None`, so engines that predate the model
//! (Media3, the Apple engines) keep compiling and pass the contract. What
//! an engine fills must be consistent: [`PlaybackStats::check`] holds the
//! rules, and the contract case `stats` runs them on every engine.
//!
//! The JSON form (snake_case, `null` for unknown) is the wire format: the
//! Dart model of `pa_player` reads and writes the same JSON
//! (`tests/fixtures/playback_stats.json` is shared by both test suites),
//! and the diagnostic report (CORE-008) carries it as `playback.json`.

use serde::{Deserialize, Serialize};

use crate::caps::DynamicRange;

/// Where the bytes come from; drives the adaptive buffer and the
/// transcode hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A file on this device.
    Local,
    /// A share read through the core (SMB, WebDAV, SFTP, FTP) — the
    /// loopback proxy.
    Network,
    /// A media server stream (Jellyfin, Emby, Plex, own server): the server
    /// could transcode.
    Server,
    /// A live stream (Twitch): the buffer is kept short on purpose.
    Live,
}

/// How the sound leaves the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioPath {
    /// The compressed stream goes to the receiver as is (bitstream).
    Passthrough,
    /// The player decodes a compressed stream to PCM.
    Decoded,
    /// The source is already PCM (LPCM, WAV, FLAC is "decoded").
    Lpcm,
}

/// Automatic frame rate matching of the display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfrStatus {
    /// Turned off in the settings (or not implemented by the engine).
    Off,
    /// The display was switched to the frame rate of the video.
    Switched,
    /// The display already ran at a matching rate.
    Matched,
    /// The display has no matching mode.
    Unsupported,
}

/// Technical numbers for the stats overlay (PLAY-010).
///
/// The first seven fields are the PLAY-003 contract; the rest came with
/// PLAY-010 and are optional for every engine.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackStats {
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    /// Hardware decoding is active.
    pub hw_decoding: Option<bool>,
    /// Audio is sent as a bitstream (passthrough).
    pub audio_passthrough: Option<bool>,
    /// Frames dropped on output (late for the display), since open.
    pub dropped_frames: u64,
    /// Bitrate of the stream (video + audio), bits per second.
    pub bitrate_bps: Option<u64>,
    /// Media buffered ahead of the playhead.
    pub buffered_ms: u64,

    /// Engine name and version ("libmpv 0.40.0", "Media3 1.6").
    pub engine: Option<String>,
    /// Container format ("matroska", "mp4").
    pub container: Option<String>,
    /// Codec profile ("Main 10", "High").
    pub video_profile: Option<String>,
    pub video_width: Option<u32>,
    pub video_height: Option<u32>,
    pub video_fps: Option<f64>,
    pub video_bit_depth: Option<u8>,
    /// Dynamic range of the input stream.
    pub dynamic_range: Option<DynamicRange>,
    /// HDR10 static metadata, cd/m².
    pub max_cll: Option<u32>,
    pub max_fall: Option<u32>,
    /// Dolby Vision profile (5, 7, 8).
    pub dv_profile: Option<u8>,
    /// Hardware decoder in use ("vaapi", "mediacodec", "videotoolbox").
    pub hw_decoder: Option<String>,
    /// Frames the decoder dropped (could not keep up), since open.
    pub decoder_dropped_frames: Option<u64>,
    /// What goes to the display.
    pub output_width: Option<u32>,
    pub output_height: Option<u32>,
    /// Refresh rate of the display.
    pub output_fps: Option<f64>,
    /// Dynamic range sent to the display (`Sdr` after tone mapping).
    pub output_range: Option<DynamicRange>,
    pub afr: Option<AfrStatus>,
    pub audio_path: Option<AudioPath>,
    pub audio_channels: Option<u32>,
    pub audio_output_channels: Option<u32>,
    pub audio_sample_rate: Option<u32>,
    pub source: Option<SourceKind>,
    /// Measured read speed of the source, bits per second.
    pub network_bps: Option<u64>,
    /// Read-ahead the adaptive buffer aims for.
    pub buffer_target_ms: Option<u64>,
    /// Playback stops to refill the buffer since open.
    pub stalls: Option<u32>,
}

/// Network speed per stream bitrate below which a server stream is better
/// transcoded (the buffer drains while playing).
pub const TRANSCODE_HEADROOM: f64 = 1.2;

/// Stalls after which a server stream is better transcoded, whatever the
/// measured speed.
pub const TRANSCODE_STALLS: u32 = 3;

impl PlaybackStats {
    /// Network speed per stream bitrate (2.0: twice as fast as needed), if
    /// both are known.
    pub fn headroom(&self) -> Option<f64> {
        match (self.network_bps, self.bitrate_bps) {
            (Some(n), Some(b)) if b > 0 => Some(n as f64 / b as f64),
            _ => None,
        }
    }

    /// The source is a server and it does not keep up: suggest a
    /// transcoded (lower bitrate) stream. Only servers can transcode; a
    /// share gets a bigger buffer instead.
    pub fn transcode_advised(&self) -> bool {
        if self.source != Some(SourceKind::Server) {
            return false;
        }
        self.headroom().is_some_and(|h| h < TRANSCODE_HEADROOM)
            || self.stalls.unwrap_or(0) >= TRANSCODE_STALLS
    }

    /// The consistency rules of the model; `Err` names the broken one.
    /// Unknown (`None`) fields never break a rule.
    pub fn check(&self) -> Result<(), String> {
        fn pair<T>(a: &Option<T>, b: &Option<T>) -> bool {
            a.is_some() == b.is_some()
        }
        fn positive(v: Option<f64>) -> bool {
            v.is_none_or(|v| v.is_finite() && v > 0.0)
        }
        if !pair(&self.video_width, &self.video_height) {
            return Err("video_width and video_height come together".into());
        }
        if !pair(&self.output_width, &self.output_height) {
            return Err("output_width and output_height come together".into());
        }
        if [self.video_width, self.video_height, self.output_width]
            .into_iter()
            .chain([self.output_height])
            .any(|v| v == Some(0))
        {
            return Err("sizes are positive".into());
        }
        if !positive(self.video_fps) || !positive(self.output_fps) {
            return Err("frame rates are positive and finite".into());
        }
        if self.hw_decoder.is_some() && self.hw_decoding != Some(true) {
            return Err("hw_decoder needs hw_decoding = true".into());
        }
        if let (Some(path), Some(pass)) = (self.audio_path, self.audio_passthrough)
            && (path == AudioPath::Passthrough) != pass
        {
            return Err("audio_path and audio_passthrough agree".into());
        }
        let hdr = self.dynamic_range.is_some_and(|r| r != DynamicRange::Sdr);
        if (self.max_cll.is_some() || self.max_fall.is_some()) && self.dynamic_range.is_some() && !hdr
        {
            return Err("MaxCLL/MaxFALL only for HDR".into());
        }
        if self.dv_profile.is_some()
            && self
                .dynamic_range
                .is_some_and(|r| !r.is_dolby_vision())
        {
            return Err("dv_profile only for Dolby Vision".into());
        }
        if self.network_bps.is_some() && self.source == Some(SourceKind::Local) {
            return Err("no network speed for a local file".into());
        }
        if let (Some(t), Some(b)) = (self.buffer_target_ms, self.bitrate_bps)
            && t == 0
            && b > 0
        {
            return Err("buffer target is positive".into());
        }
        Ok(())
    }

    /// Checks that `later` (read after `self`, same item) only counts up.
    pub fn check_progress(&self, later: &Self) -> Result<(), String> {
        if later.dropped_frames < self.dropped_frames {
            return Err("dropped_frames never goes down".into());
        }
        if let (Some(a), Some(b)) = (self.decoder_dropped_frames, later.decoder_dropped_frames)
            && b < a
        {
            return Err("decoder_dropped_frames never goes down".into());
        }
        if let (Some(a), Some(b)) = (self.stalls, later.stalls)
            && b < a
        {
            return Err("stalls never go down".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(network: u64, bitrate: u64, stalls: u32) -> PlaybackStats {
        PlaybackStats {
            source: Some(SourceKind::Server),
            network_bps: Some(network),
            bitrate_bps: Some(bitrate),
            stalls: Some(stalls),
            ..Default::default()
        }
    }

    #[test]
    fn transcode_only_for_a_slow_server() {
        assert!(server(10_000_000, 20_000_000, 0).transcode_advised());
        assert!(!server(40_000_000, 20_000_000, 0).transcode_advised());
        assert!(server(40_000_000, 20_000_000, 3).transcode_advised());
        let share = PlaybackStats {
            source: Some(SourceKind::Network),
            ..server(1, 20_000_000, 9)
        };
        assert!(!share.transcode_advised());
    }

    #[test]
    fn rules() {
        assert!(PlaybackStats::default().check().is_ok());
        let bad = PlaybackStats {
            hw_decoder: Some("vaapi".into()),
            ..Default::default()
        };
        assert!(bad.check().is_err());
        let bad = PlaybackStats {
            audio_path: Some(AudioPath::Decoded),
            audio_passthrough: Some(true),
            ..Default::default()
        };
        assert!(bad.check().is_err());
        let bad = PlaybackStats {
            dynamic_range: Some(DynamicRange::Hdr10),
            dv_profile: Some(8),
            ..Default::default()
        };
        assert!(bad.check().is_err());
        let bad = PlaybackStats {
            video_width: Some(1920),
            ..Default::default()
        };
        assert!(bad.check().is_err());
        let bad = PlaybackStats {
            output_fps: Some(f64::NAN),
            ..Default::default()
        };
        assert!(bad.check().is_err());
        let a = PlaybackStats {
            dropped_frames: 3,
            ..Default::default()
        };
        assert!(a.check_progress(&PlaybackStats::default()).is_err());
    }
}
