// SPDX-License-Identifier: MPL-2.0
//! How to play an item: DirectPlay, DirectStream (remux) or Transcode
//! (HLS), from the device's capabilities; and the playback reports
//! (start, progress, stop) that keep the server's "Continue Watching",
//! "watched" and dashboard right.
//!
//! The decision is the server's: [`Client::playback_plan`] sends a device
//! profile built from [`DeviceCapabilities`] to `POST
//! /Items/{id}/PlaybackInfo`, and the answer says whether the file plays as
//! is or which stream URL to open. [`classify`] turns that answer into a
//! [`PlayMethod`].

use serde_json::{Value, json};

use crate::http::{Client, Method, encode};
use crate::model::{MediaSource, ms_to_ticks};
use crate::{Error, Result};

/// What the device's player can play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCapabilities {
    /// Video containers played as files (`mkv`, `mp4`, …).
    pub video_containers: Vec<String>,
    /// Video codecs (`h264`, `hevc`, `av1`, …).
    pub video_codecs: Vec<String>,
    /// Audio codecs inside videos and as music.
    pub audio_codecs: Vec<String>,
    /// Audio containers played as files (`mp3`, `flac`, …).
    pub audio_containers: Vec<String>,
    /// Highest bitrate the network or the user allows, bit/s; `None`: no
    /// limit.
    pub max_bitrate: Option<u64>,
    /// Audio channels the output takes.
    pub max_audio_channels: u32,
    /// Text subtitle formats the player renders from a side file.
    pub external_subtitles: Vec<String>,
}

fn list(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

impl DeviceCapabilities {
    /// The libmpv/FFmpeg player of the Kinolune clients (media_kit on
    /// Linux and Android, MPVKit on Apple): it decodes about everything,
    /// so DirectPlay is the rule and the server only transcodes for a
    /// bitrate limit or a forced transcode.
    #[must_use]
    pub fn libmpv() -> Self {
        Self {
            video_containers: list(&[
                "mkv", "webm", "mp4", "m4v", "mov", "avi", "ts", "mpegts", "m2ts", "mpg", "mpeg",
                "flv", "wmv", "asf", "ogm", "ogv", "3gp", "vob",
            ]),
            video_codecs: list(&[
                "h264",
                "hevc",
                "h265",
                "av1",
                "vp8",
                "vp9",
                "mpeg2video",
                "mpeg4",
                "msmpeg4v3",
                "vc1",
                "wmv3",
                "theora",
                "mjpeg",
            ]),
            audio_codecs: list(&[
                "aac",
                "mp3",
                "mp2",
                "ac3",
                "eac3",
                "dts",
                "truehd",
                "flac",
                "alac",
                "opus",
                "vorbis",
                "pcm_s16le",
                "pcm_s24le",
                "wmav2",
                "wmapro",
                "dca",
                "mlp",
            ]),
            audio_containers: list(&[
                "mp3", "flac", "m4a", "m4b", "aac", "ogg", "oga", "opus", "wav", "wma", "ape",
                "wv", "mka", "dsf", "aiff",
            ]),
            max_bitrate: None,
            max_audio_channels: 8,
            external_subtitles: list(&["srt", "subrip", "ass", "ssa", "vtt", "webvtt", "sub"]),
        }
    }

    /// The `DeviceProfile` JSON for `PlaybackInfo`: direct play of the
    /// listed containers and codecs, HLS (MPEG-TS, H.264 + AAC) as the
    /// transcoding target, subtitles as side files when the player renders
    /// them, burnt in otherwise.
    #[must_use]
    pub fn device_profile(&self) -> Value {
        let bitrate = self.max_bitrate.unwrap_or(200_000_000);
        let mut subtitles: Vec<Value> = self
            .external_subtitles
            .iter()
            .map(|f| json!({ "Format": f, "Method": "External" }))
            .collect();
        for f in ["pgssub", "dvdsub", "dvbsub"] {
            subtitles.push(json!({ "Format": f, "Method": "Encode" }));
        }
        json!({
            "Name": "Kinolune",
            "MaxStreamingBitrate": bitrate,
            "MaxStaticBitrate": bitrate,
            "MusicStreamingTranscodingBitrate": 320_000,
            "DirectPlayProfiles": [
                {
                    "Type": "Video",
                    "Container": self.video_containers.join(","),
                    "VideoCodec": self.video_codecs.join(","),
                    "AudioCodec": self.audio_codecs.join(","),
                },
                {
                    "Type": "Audio",
                    "Container": self.audio_containers.join(","),
                    "AudioCodec": self.audio_codecs.join(","),
                },
            ],
            "TranscodingProfiles": [
                {
                    "Type": "Video",
                    "Container": "ts",
                    "Protocol": "hls",
                    "Context": "Streaming",
                    "VideoCodec": "h264",
                    "AudioCodec": "aac,mp3,ac3",
                    "MaxAudioChannels": self.max_audio_channels.to_string(),
                    "BreakOnNonKeyFrames": true,
                    "MinSegments": 1,
                },
                {
                    "Type": "Audio",
                    "Container": "mp3",
                    "Protocol": "http",
                    "Context": "Streaming",
                    "AudioCodec": "mp3",
                    "MaxAudioChannels": "2",
                },
            ],
            "ContainerProfiles": [],
            "CodecProfiles": [],
            "SubtitleProfiles": subtitles,
        })
    }
}

/// How the item reaches the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayMethod {
    /// The original file, byte ranges as asked (through the app's proxy).
    DirectPlay,
    /// The original audio and video, repackaged by the server (HLS).
    DirectStream,
    /// Re-encoded by the server (HLS).
    Transcode,
}

impl PlayMethod {
    /// The value playback reports carry.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DirectPlay => "DirectPlay",
            Self::DirectStream => "DirectStream",
            Self::Transcode => "Transcode",
        }
    }
}

/// What to play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackRequest {
    /// The item.
    pub item_id: String,
    /// A version of it; `None`: the server's first.
    pub media_source_id: Option<String>,
    /// The device.
    pub capabilities: DeviceCapabilities,
    /// Where playback starts, ms.
    pub start_ms: i64,
    /// Audio track (stream index) to use; `None`: the default.
    pub audio_stream_index: Option<i64>,
    /// Subtitle track (stream index); `None`: the default.
    pub subtitle_stream_index: Option<i64>,
    /// Transcode even when the file could play as is (the user asked for
    /// it: "transcoding on request", research 01 §1.5).
    pub force_transcode: bool,
}

impl PlaybackRequest {
    /// Plays `item_id` from the start with `capabilities`.
    #[must_use]
    pub fn new(item_id: impl Into<String>, capabilities: DeviceCapabilities) -> Self {
        Self {
            item_id: item_id.into(),
            media_source_id: None,
            capabilities,
            start_ms: 0,
            audio_stream_index: None,
            subtitle_stream_index: None,
            force_transcode: false,
        }
    }
}

/// The answer to a [`PlaybackRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackPlan {
    /// How it plays.
    pub method: PlayMethod,
    /// The item.
    pub item_id: String,
    /// The version chosen.
    pub media_source_id: String,
    /// Id of this playback at the server; reports carry it.
    pub play_session_id: Option<String>,
    /// Path and query of the stream at the server: the original file for
    /// [`PlayMethod::DirectPlay`], the HLS playlist otherwise. Open it
    /// with [`Client::url_with_key`] (or, for DirectPlay, through the
    /// app's proxy, which keeps the token out of the player).
    pub stream_path: String,
    /// Why the server transcodes (`ContainerNotSupported`,
    /// `VideoCodecNotSupported`, …); empty for DirectPlay.
    pub transcode_reasons: Vec<String>,
    /// The version as the server described it.
    pub source: MediaSource,
}

/// Reasons a transcoding URL gives (`TranscodeReasons=a,b`).
fn reasons(url: &str) -> Vec<String> {
    let query = url.split_once('?').map_or("", |(_, q)| q);
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| k.eq_ignore_ascii_case("TranscodeReasons"))
        .map(|(_, v)| {
            v.replace("%2C", ",")
                .split(',')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Only the container stands in the way: the server can copy the streams.
const REMUX_REASONS: &[&str] = &["ContainerNotSupported", "DirectPlayError"];

/// The method for a media source of a `PlaybackInfo` answer.
#[must_use]
pub fn classify(source: &MediaSource) -> (PlayMethod, Vec<String>) {
    match source.transcoding_url.as_deref() {
        Some(url) if !source.supports_direct_play => {
            // `SupportsDirectStream` is false for an HLS answer even when
            // the server only repackages: the reasons tell.
            let why = reasons(url);
            // Emby keeps the container reasons when video copy is off
            // (bitrate limit, forced transcode) and says so in the URL.
            let lower = url.to_ascii_lowercase();
            let copy_off = lower.contains("allowvideostreamcopy=false");
            let remux = !copy_off
                && !why.is_empty()
                && why.iter().all(|r| REMUX_REASONS.contains(&r.as_str()));
            (
                if remux {
                    PlayMethod::DirectStream
                } else {
                    PlayMethod::Transcode
                },
                why,
            )
        }
        _ => (PlayMethod::DirectPlay, Vec::new()),
    }
}

/// Path and query of the original file of a media source (byte ranges).
#[must_use]
pub fn static_stream_path(item_id: &str, media_source_id: &str, audio: bool) -> String {
    format!(
        "/{}/{}/stream?static=true&mediaSourceId={}",
        if audio { "Audio" } else { "Videos" },
        encode(item_id),
        encode(media_source_id)
    )
}

/// A playback report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackReport {
    /// The item.
    pub item_id: String,
    /// The version playing.
    pub media_source_id: String,
    /// From [`PlaybackPlan::play_session_id`], if there was a plan.
    pub play_session_id: Option<String>,
    /// How it plays.
    pub method: PlayMethod,
    /// Position, ms.
    pub position_ms: i64,
    /// Paused.
    pub paused: bool,
}

impl PlaybackReport {
    fn body(&self) -> Value {
        json!({
            "ItemId": self.item_id,
            "MediaSourceId": self.media_source_id,
            "PlaySessionId": self.play_session_id,
            "PlayMethod": self.method.as_str(),
            "PositionTicks": ms_to_ticks(self.position_ms),
            "IsPaused": self.paused,
            "CanSeek": true,
        })
    }
}

impl Client {
    /// Asks the server how `req` plays on the device (`POST
    /// /Items/{id}/PlaybackInfo` with the device profile).
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for an unknown item; [`Error::Protocol`] if the
    /// server offers no way to play it.
    pub fn playback_plan(&self, user_id: &str, req: &PlaybackRequest) -> Result<PlaybackPlan> {
        let caps = &req.capabilities;
        let mut body = json!({
            "UserId": user_id,
            "DeviceProfile": caps.device_profile(),
            "StartTimeTicks": ms_to_ticks(req.start_ms),
            "AutoOpenLiveStream": true,
            "EnableDirectPlay": !req.force_transcode,
            "EnableDirectStream": !req.force_transcode,
            "EnableTranscoding": true,
            "AllowVideoStreamCopy": !req.force_transcode,
            "AllowAudioStreamCopy": !req.force_transcode,
        });
        if let Some(b) = caps.max_bitrate {
            body["MaxStreamingBitrate"] = json!(b);
        }
        if let Some(id) = &req.media_source_id {
            body["MediaSourceId"] = json!(id);
        }
        if let Some(i) = req.audio_stream_index {
            body["AudioStreamIndex"] = json!(i);
        }
        if let Some(i) = req.subtitle_stream_index {
            body["SubtitleStreamIndex"] = json!(i);
        }
        let answer: Value = self.post_json(
            &format!(
                "/Items/{}/PlaybackInfo?userId={}",
                encode(&req.item_id),
                encode(user_id)
            ),
            &body,
        )?;
        if let Some(code) = answer.get("ErrorCode").and_then(Value::as_str) {
            return Err(Error::Protocol(format!("playback refused: {code}")));
        }
        let sources: Vec<MediaSource> =
            serde_json::from_value(answer.get("MediaSources").cloned().unwrap_or(Value::Null))
                .unwrap_or_default();
        let source = req
            .media_source_id
            .as_ref()
            .and_then(|id| sources.iter().find(|s| &s.id == id))
            .or_else(|| sources.first())
            .cloned()
            .ok_or_else(|| Error::Protocol("no media source to play".into()))?;
        let play_session_id = answer
            .get("PlaySessionId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let (method, transcode_reasons) = if req.force_transcode {
            let url = source.transcoding_url.clone().unwrap_or_default();
            (PlayMethod::Transcode, reasons(&url))
        } else {
            classify(&source)
        };
        let audio = source.media_streams.iter().all(|s| s.kind != "Video");
        let stream_path = match (method, &source.transcoding_url) {
            (PlayMethod::DirectPlay, _) => static_stream_path(&req.item_id, &source.id, audio),
            (_, Some(url)) => url.clone(),
            (_, None) => return Err(Error::Protocol("no transcoding URL".into())),
        };
        Ok(PlaybackPlan {
            method,
            item_id: req.item_id.clone(),
            media_source_id: source.id.clone(),
            play_session_id,
            stream_path,
            transcode_reasons,
            source,
        })
    }

    /// Playback started.
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn report_start(&self, r: &PlaybackReport) -> Result<()> {
        self.call(Method::Post, "/Sessions/Playing", Some(&r.body()))
            .map(|_| ())
    }

    /// Position update (every few seconds, on pause and seek).
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn report_progress(&self, r: &PlaybackReport) -> Result<()> {
        let mut body = r.body();
        body["EventName"] = json!(if r.paused { "pause" } else { "timeupdate" });
        self.call(Method::Post, "/Sessions/Playing/Progress", Some(&body))
            .map(|_| ())
    }

    /// Playback ended at `r.position_ms`: the server stores the resume
    /// position (or "watched" near the end) and stops a transcode.
    ///
    /// # Errors
    ///
    /// Transport and status errors.
    pub fn report_stopped(&self, r: &PlaybackReport) -> Result<()> {
        self.call(Method::Post, "/Sessions/Playing/Stopped", Some(&r.body()))?;
        if r.method != PlayMethod::DirectPlay
            && let Some(id) = &r.play_session_id
        {
            // Best effort: the server also stops idle encodings itself.
            let _ = self.call(
                Method::Delete,
                &format!(
                    "/Videos/ActiveEncodings?deviceId={}&playSessionId={}",
                    encode(&self.device().device_id),
                    encode(id)
                ),
                None,
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(json: &str) -> MediaSource {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn classifies_answers() {
        let direct = source(r#"{"Id":"a","SupportsDirectPlay":true,"SupportsDirectStream":true}"#);
        assert_eq!(classify(&direct).0, PlayMethod::DirectPlay);
        let remux = source(
            r#"{"Id":"a","SupportsDirectPlay":false,"SupportsDirectStream":true,
            "TranscodingUrl":"/videos/a/master.m3u8?VideoCodec=h264&TranscodeReasons=ContainerNotSupported"}"#,
        );
        assert_eq!(
            classify(&remux),
            (
                PlayMethod::DirectStream,
                vec!["ContainerNotSupported".to_owned()]
            )
        );
        let transcode = source(
            r#"{"Id":"a","SupportsDirectPlay":false,"SupportsDirectStream":false,
            "TranscodingUrl":"/videos/a/master.m3u8?TranscodeReasons=VideoCodecNotSupported%2CContainerNotSupported"}"#,
        );
        let (m, why) = classify(&transcode);
        assert_eq!(m, PlayMethod::Transcode);
        assert_eq!(why, ["VideoCodecNotSupported", "ContainerNotSupported"]);
        // Emby: container reasons only, but video copy switched off.
        let emby = source(
            r#"{"Id":"mediasource_1","SupportsDirectPlay":false,
            "TranscodingUrl":"/videos/1/master.m3u8?api_key=k&TranscodeReasons=ContainerNotSupported,DirectPlayError&allowVideoStreamCopy=false"}"#,
        );
        assert_eq!(classify(&emby).0, PlayMethod::Transcode);
    }

    #[test]
    fn profile_lists_capabilities() {
        let mut caps = DeviceCapabilities::libmpv();
        caps.max_bitrate = Some(4_000_000);
        let p = caps.device_profile();
        assert_eq!(p["MaxStreamingBitrate"], 4_000_000);
        assert!(
            p["DirectPlayProfiles"][0]["VideoCodec"]
                .as_str()
                .unwrap()
                .contains("hevc")
        );
        assert_eq!(p["TranscodingProfiles"][0]["Protocol"], "hls");
    }

    #[test]
    fn static_paths() {
        assert_eq!(
            static_stream_path("i", "s", false),
            "/Videos/i/stream?static=true&mediaSourceId=s"
        );
        assert!(static_stream_path("i", "s", true).starts_with("/Audio/"));
    }
}
