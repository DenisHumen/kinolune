// SPDX-License-Identifier: MPL-2.0
//! TWI-006: low-latency live through the loopback proxy, ads as is.
//!
//! A fake Twitch-style origin (made-up names, `127.0.0.1`) publishes
//! segments in real time with prefetch segments and a stitched ad break.
//! A client that reads the proxy like FFmpeg's HLS demuxer does (start near
//! the end of the playlist, read segments in order, reload when out) must
//! receive every segment, the ad segments byte for byte included. The
//! player-level check with libmpv is
//! `clients/flutter/packages/pa_player/test/live_libmpv_test.dart`.
#![cfg(all(pa_twitch_native, feature = "reqwest"))]

use std::sync::Arc;
use std::time::{Duration, Instant};

use pa_twitch::playback::live::{LiveMode, LiveProxy, MediaPlaylist, ReqwestUpstream};
use pa_twitch::testing::{FakeLiveOptions, FakeLiveOrigin, FakeLiveSegment};

const SEGMENT: f64 = 0.5;
const COUNT: usize = 16;
const ADS: std::ops::Range<usize> = 7..10;

fn segments() -> Vec<FakeLiveSegment> {
    (0..COUNT)
        .map(|k| FakeLiveSegment {
            // Distinct bytes per segment, a few KiB.
            bytes: (0..4096 + k * 97).map(|i| (i * 31 + k * 7) as u8).collect(),
            duration: SEGMENT,
            ad: ADS.contains(&k),
        })
        .collect()
}

async fn get(client: &reqwest::Client, url: &str) -> (u16, Vec<u8>) {
    let r = client.get(url).send().await.unwrap();
    let status = r.status().as_u16();
    (status, r.bytes().await.unwrap().to_vec())
}

fn base(url: &str) -> &str {
    &url[..url.rfind('/').unwrap() + 1]
}

/// Reads the proxy like FFmpeg's HLS demuxer: starts `start_from_end`
/// segments before the end of the first playlist, then reads segments in
/// sequence order, reloading the playlist when it has read them all, until
/// `#EXT-X-ENDLIST`. Returns `(sequence, bytes, latency right after the
/// segment arrived)`.
async fn play(proxy: &LiveProxy, start_from_end: usize) -> Vec<(u64, Vec<u8>, Option<f64>)> {
    let client = reqwest::Client::new();
    let url = proxy.playlist_url();
    let mut next: Option<u64> = None;
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(Instant::now() < deadline, "live playback did not finish");
        let (status, text) = get(&client, &url).await;
        assert_eq!(status, 200);
        let text = String::from_utf8(text).unwrap();
        let playlist = MediaPlaylist::parse(&text).unwrap();
        let first = *next.get_or_insert_with(|| {
            let n = playlist.segments.len();
            playlist.segments[n.saturating_sub(start_from_end)].sequence
        });
        let mut fetched = false;
        for s in playlist.segments.iter().filter(|s| s.sequence >= first) {
            let (status, bytes) = get(&client, &format!("{}{}", base(&url), s.uri)).await;
            assert_eq!(status, 200, "segment {}", s.sequence);
            let latency = proxy.report(0.0, 1.0).latency;
            got.push((s.sequence, bytes, latency));
            next = Some(s.sequence + 1);
            fetched = true;
        }
        if playlist.ended && !fetched {
            return got;
        }
        if !fetched {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

#[tokio::test]
async fn low_latency_keeps_every_segment_ads_included() {
    let source = segments();
    let origin = FakeLiveOrigin::start(source.clone(), FakeLiveOptions::default()).unwrap();
    let upstream = Arc::new(ReqwestUpstream::new().unwrap());
    let proxy = LiveProxy::start(upstream, &origin.playlist_url(), LiveMode::LowLatency).unwrap();

    // The rendered playlist: prefetch segments are plain segments now, the
    // ad marker is passed through.
    let client = reqwest::Client::new();
    let (_, first) = get(&client, &proxy.playlist_url()).await;
    let first = String::from_utf8(first).unwrap();
    assert!(!first.contains("TWITCH-PREFETCH"));
    assert!(first.contains("#EXTINF:0.500,prefetch"));
    assert!(
        !first.contains("/live/seg/"),
        "upstream URLs must be mapped"
    );

    // Start at the first prefetch segment (FFmpeg `live_start_index=-2`).
    let got = play(&proxy, 2).await;
    let start = got[0].0;
    let seqs: Vec<u64> = got.iter().map(|g| g.0).collect();
    let expected: Vec<u64> = (start..origin.sequence(COUNT)).collect();
    assert_eq!(
        seqs, expected,
        "segments must arrive in order, none missing"
    );
    // Every segment, the ad segments included, byte for byte.
    for (seq, bytes, _) in &got {
        let k = (seq - origin.sequence(0)) as usize;
        assert_eq!(bytes, &source[k].bytes, "segment {seq} altered");
    }
    let ad_seqs: Vec<u64> = ADS.map(|k| origin.sequence(k)).collect();
    for seq in &ad_seqs {
        assert!(seqs.contains(seq), "ad segment {seq} was dropped");
    }
    let served_ads = origin.served().iter().filter(|s| s.ad).count();
    assert_eq!(served_ads, ADS.len());

    let report = proxy.report(0.0, 1.0);
    assert_eq!(report.totals.ad_segments, ADS.len() as u64);
    let ad_bytes: u64 = ADS.map(|k| source[k].bytes.len() as u64).sum();
    assert_eq!(report.totals.ad_bytes, ad_bytes);
    assert!(report.totals.prefetch_segments > 0, "{report:?}");
    assert!(report.delivered.iter().filter(|d| d.ad).count() == ADS.len());

    // Low latency: once a segment has arrived, the media handed to the
    // player is at most about one segment behind the wall clock.
    let worst = got
        .iter()
        .skip(1)
        .filter_map(|g| g.2)
        .fold(0.0f64, f64::max);
    assert!(worst < 1.5, "latency without player buffer {worst} s");
}

#[tokio::test]
async fn stable_mode_waits_for_complete_segments() {
    let origin = FakeLiveOrigin::start(segments(), FakeLiveOptions::default()).unwrap();
    let upstream = Arc::new(ReqwestUpstream::new().unwrap());
    let proxy = LiveProxy::start(upstream, &origin.playlist_url(), LiveMode::Stable).unwrap();
    let client = reqwest::Client::new();
    let (status, text) = get(&client, &proxy.playlist_url()).await;
    assert_eq!(status, 200);
    let p = MediaPlaylist::parse(std::str::from_utf8(&text).unwrap()).unwrap();
    assert!(
        p.segments
            .iter()
            .all(|s| !s.prefetch && s.title != "prefetch")
    );
    let raw = MediaPlaylist::parse(&origin.playlist_now()).unwrap();
    assert_eq!(p.segments.len(), raw.complete().count());

    // Switching to low latency adds the prefetch segments.
    proxy.set_mode(LiveMode::LowLatency);
    let (_, text) = get(&client, &proxy.playlist_url()).await;
    assert!(std::str::from_utf8(&text).unwrap().contains(",prefetch\n"));
}

#[tokio::test]
async fn token_and_stats() {
    let origin = FakeLiveOrigin::start(segments(), FakeLiveOptions::default()).unwrap();
    let upstream = Arc::new(ReqwestUpstream::new().unwrap());
    let proxy = LiveProxy::start(upstream, &origin.playlist_url(), LiveMode::LowLatency).unwrap();
    let client = reqwest::Client::new();
    let (status, _) = get(
        &client,
        &format!("http://{}/wrong/live.m3u8", proxy.local_addr()),
    )
    .await;
    assert_eq!(status, 403);
    let (status, _) = get(&client, &proxy.playlist_url()).await;
    assert_eq!(status, 200);
    let (status, body) = get(&client, &format!("{}?ahead=1.5&rate=1", proxy.stats_url())).await;
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["mode"], "low_latency");
    assert_eq!(json["low_latency"]["prefetch"], 2);
    assert_eq!(json["seek_allowed"], true);
    assert!(json["edge_ms"].as_i64().unwrap() > origin.t0_ms());
}
