// SPDX-License-Identifier: MPL-2.0
//! The stats model in its wire form (PLAY-010): the fixture is shared with
//! the Dart model of `pa_player` (`test/playback_stats_test.dart`), so both
//! sides read and write the same JSON.

use pa_playback::{AfrStatus, AudioPath, DynamicRange, NetworkFit, PlaybackStats, SourceKind};

const FIXTURE: &str = include_str!("fixtures/playback_stats.json");

#[test]
fn fixture_is_the_full_model() {
    let stats: PlaybackStats = serde_json::from_str(FIXTURE).expect("fixture parses");
    assert_eq!(stats.check(), Ok(()));
    assert_eq!(stats.dynamic_range, Some(DynamicRange::DolbyVision8));
    assert_eq!(stats.output_range, Some(DynamicRange::Sdr));
    assert_eq!(stats.audio_path, Some(AudioPath::Decoded));
    assert_eq!(stats.afr, Some(AfrStatus::Off));
    assert_eq!(stats.source, Some(SourceKind::Network));
    assert_eq!(stats.video_fps, Some(23.976));
    // Every field of the model is in the fixture, and nothing else: the
    // Dart side checks the same key set.
    let back = serde_json::to_value(&stats).expect("serialises");
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).expect("json");
    assert_eq!(back, fixture);
}

#[test]
fn unknown_fields_default_to_none() {
    let stats: PlaybackStats =
        serde_json::from_str(r#"{"video_codec":"h264","dropped_frames":1}"#).expect("parses");
    assert_eq!(stats.video_codec.as_deref(), Some("h264"));
    assert_eq!(stats.dropped_frames, 1);
    assert_eq!(stats.source, None);
    assert_eq!(stats.check(), Ok(()));
}

#[test]
fn headroom_and_transcode_hint() {
    let stats: PlaybackStats = serde_json::from_str(FIXTURE).expect("fixture parses");
    let h = stats.headroom().expect("both known");
    assert!((h - 42.0 / 18.5).abs() < 1e-9);
    assert!(!stats.transcode_advised(), "a share is never transcoded");
    let server = PlaybackStats {
        source: Some(SourceKind::Server),
        network_bps: Some(15_000_000),
        ..stats
    };
    assert!(server.transcode_advised());
}

#[test]
fn network_versus_bitrate_indicator() {
    // QA-007: the same thresholds as `NetworkFit` of the Dart player.
    assert_eq!(NetworkFit::of(2.0), NetworkFit::Enough);
    assert_eq!(NetworkFit::of(1.5), NetworkFit::Enough);
    assert_eq!(NetworkFit::of(1.49), NetworkFit::Tight);
    assert_eq!(NetworkFit::of(1.1), NetworkFit::Tight);
    assert_eq!(NetworkFit::of(1.09), NetworkFit::Short);
    let stats: PlaybackStats = serde_json::from_str(FIXTURE).expect("fixture parses");
    assert_eq!(stats.network_fit(), Some(NetworkFit::Enough));
    let slow = PlaybackStats {
        network_bps: Some(16_000_000),
        ..stats.clone()
    };
    assert_eq!(slow.network_fit(), Some(NetworkFit::Short));
    let unknown = PlaybackStats {
        network_bps: None,
        ..stats
    };
    assert_eq!(unknown.network_fit(), None);
}
