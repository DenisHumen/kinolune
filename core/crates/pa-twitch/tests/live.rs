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
//!
//! TWI-025: the live DVR window over a generated sliding playlist (rewind
//! 60 s and back to live; the window grows past Twitch's own).
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

// ---- TWI-025: live DVR ----------------------------------------------------

fn dvr_segments(count: usize, duration: f64) -> Vec<FakeLiveSegment> {
    (0..count)
        .map(|k| FakeLiveSegment {
            bytes: (0..2048 + k * 13).map(|i| (i * 17 + k * 3) as u8).collect(),
            duration,
            ad: false,
        })
        .collect()
}

async fn playlist(client: &reqwest::Client, proxy: &LiveProxy) -> MediaPlaylist {
    let (status, text) = get(client, &proxy.playlist_url()).await;
    assert_eq!(status, 200);
    MediaPlaylist::parse(std::str::from_utf8(&text).unwrap()).unwrap()
}

async fn stats(client: &reqwest::Client, proxy: &LiveProxy) -> serde_json::Value {
    let (status, body) = get(client, &format!("{}?ahead=0&rate=1", proxy.stats_url())).await;
    assert_eq!(status, 200);
    serde_json::from_slice(&body).unwrap()
}

/// A generated live stream with a sliding 80 s window: the player rewinds
/// 60 s, gets exactly the segment that was on air then, and goes back to
/// live.
#[tokio::test]
async fn dvr_seek_sixty_seconds_back_and_back_to_live() {
    let source = dvr_segments(60, 2.0);
    let options = FakeLiveOptions {
        window: 40,
        prefetch: 2,
        preroll: 45,
        first_sequence: 1000,
    };
    let origin = FakeLiveOrigin::start(source.clone(), options).unwrap();
    let upstream = Arc::new(ReqwestUpstream::new().unwrap());
    let proxy = LiveProxy::start(upstream, &origin.playlist_url(), LiveMode::LowLatency).unwrap();
    let client = reqwest::Client::new();
    let base = base(&proxy.playlist_url()).to_owned();

    // Watching live: the newest complete segment.
    let live = playlist(&client, &proxy).await;
    let newest = live.complete().last().unwrap().clone();
    let (status, _) = get(&client, &format!("{base}{}", newest.uri)).await;
    assert_eq!(status, 200);
    let playhead = stats(&client, &proxy).await["playhead_ms"]
        .as_i64()
        .unwrap();
    assert_eq!(playhead, newest.end_date_time().unwrap());

    // 60 s back.
    let (status, body) = get(&client, &format!("{}?by=-60", proxy.seek_url())).await;
    assert_eq!(status, 200);
    let seek: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(seek["live"], false);
    let at = seek["at_ms"].as_i64().unwrap();
    assert_eq!(at, playhead - 60_000);

    // FFmpeg (`live_start_index=-2`) starts at the segment on air then.
    let p = playlist(&client, &proxy).await;
    let start = &p.segments[p.segments.len() - 2];
    let begin = start.program_date_time.unwrap();
    assert!(begin <= at && at < begin + 2000, "{begin} vs {at}");
    let (status, bytes) = get(&client, &format!("{base}{}", start.uri)).await;
    assert_eq!(status, 200);
    let k = (start.sequence - origin.sequence(0)) as usize;
    assert_eq!(bytes, source[k].bytes, "rewound segment altered");
    // Then the whole window again, up to the edge.
    let p = playlist(&client, &proxy).await;
    assert!(p.segments.last().unwrap().sequence > newest.sequence);
    let json = stats(&client, &proxy).await;
    assert_eq!(json["dvr"]["active"], true);
    assert_eq!(json["rate"], 1.0);
    assert_eq!(json["should_jump"], false);
    let behind = json["dvr"]["behind_seconds"].as_f64().unwrap();
    assert!((58.0..66.0).contains(&behind), "behind {behind}");

    // "В эфир".
    let (status, _) = get(&client, &proxy.reset_url()).await;
    assert_eq!(status, 200);
    let raw = MediaPlaylist::parse(&origin.playlist_now()).unwrap();
    let p = playlist(&client, &proxy).await;
    assert!(p.segments.last().unwrap().sequence >= raw.segments.last().unwrap().sequence);
    let json = stats(&client, &proxy).await;
    assert_eq!(json["dvr"]["active"], false);
    assert!(json["dvr"]["window_seconds"].as_f64().unwrap() >= 78.0);
}

/// Twitch lists only the last few segments; while the player is away
/// (paused, or playing the VOD from the start) the proxy keeps reading the
/// playlist itself, and the DVR window grows past Twitch's window.
#[tokio::test]
async fn dvr_window_grows_past_the_twitch_window() {
    let options = FakeLiveOptions {
        window: 4,
        prefetch: 2,
        preroll: 4,
        first_sequence: 1000,
    };
    let origin = FakeLiveOrigin::start(dvr_segments(40, 0.5), options).unwrap();
    let upstream = Arc::new(ReqwestUpstream::new().unwrap());
    let proxy = LiveProxy::start(upstream, &origin.playlist_url(), LiveMode::LowLatency).unwrap();
    let client = reqwest::Client::new();
    let first = playlist(&client, &proxy).await;
    assert_eq!(first.media_sequence, 1000);
    tokio::time::sleep(Duration::from_millis(4500)).await;
    let raw = MediaPlaylist::parse(&origin.playlist_now()).unwrap();
    assert!(raw.media_sequence > 1005, "{}", raw.media_sequence);
    let p = playlist(&client, &proxy).await;
    assert_eq!(p.media_sequence, 1000);
    let seqs: Vec<u64> = p.segments.iter().map(|s| s.sequence).collect();
    let last = *seqs.last().unwrap();
    assert_eq!(seqs, (1000..=last).collect::<Vec<_>>(), "gap in the window");
    assert!(p.segments.iter().all(|s| s.program_date_time.is_some()));
}

/// Twitch since 2026-11 (BUG-035): an fMP4/CMAF media playlist with
/// `https://` init segments (`#EXT-X-MAP`) and segment URIs without an
/// extension, a stitched ad with its own init segment. Our FFmpeg has no
/// TLS, so the player must get only loopback URLs: init segments through
/// `m/<id>.mp4`, segments as `video/mp4`.
mod fmp4 {
    use std::collections::HashMap;
    use std::sync::Arc;

    use pa_twitch::BoxFuture;
    use pa_twitch::playback::PlaybackError;
    use pa_twitch::playback::live::{
        LiveMode, LiveProxy, LiveUpstream, MediaPlaylist, SegmentBody,
    };

    const EDGE: &str = "https://edge.example.invalid/v1/segment";

    struct Upstream {
        playlist: String,
        bodies: HashMap<String, Vec<u8>>,
    }

    struct Body(Option<Vec<u8>>);

    impl SegmentBody for Body {
        fn content_length(&self) -> Option<u64> {
            self.0.as_ref().map(|b| b.len() as u64)
        }

        fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, PlaybackError>> {
            Box::pin(async move { Ok(self.0.take()) })
        }
    }

    impl LiveUpstream for Upstream {
        fn playlist<'a>(&'a self, _url: &'a str) -> BoxFuture<'a, Result<String, PlaybackError>> {
            Box::pin(async move { Ok(self.playlist.clone()) })
        }

        fn segment<'a>(
            &'a self,
            url: &'a str,
        ) -> BoxFuture<'a, Result<Box<dyn SegmentBody>, PlaybackError>> {
            Box::pin(async move {
                match self.bodies.get(url) {
                    Some(b) => Ok(Box::new(Body(Some(b.clone()))) as Box<dyn SegmentBody>),
                    None => Err(PlaybackError::NotFound),
                }
            })
        }
    }

    fn upstream() -> Upstream {
        let playlist = format!(
            "#EXTM3U
#EXT-X-VERSION:6
#EXT-X-TARGETDURATION:5
#EXT-X-MEDIA-SEQUENCE:0
#EXT-X-DATERANGE:ID=\"stitched-ad-1\",CLASS=\"twitch-stitched-ad\",START-DATE=\"2026-11-27T08:00:00.000Z\",DURATION=2.000
#EXT-X-DISCONTINUITY
#EXT-X-MAP:URI=\"{EDGE}/AdInit\"
#EXT-X-PROGRAM-DATE-TIME:2026-11-27T08:00:00.000Z
#EXTINF:2.000,Amazon|1
{EDGE}/Ad0
#EXT-X-DISCONTINUITY
#EXT-X-MAP:URI=\"{EDGE}/LiveInit\"
#EXT-X-PROGRAM-DATE-TIME:2026-11-27T08:00:02.000Z
#EXTINF:2.000,live
{EDGE}/Live1
#EXT-X-PROGRAM-DATE-TIME:2026-11-27T08:00:04.000Z
#EXTINF:2.000,live
{EDGE}/Live2
#EXT-X-TWITCH-PREFETCH:{EDGE}/Live3
"
        );
        let bodies = ["AdInit", "Ad0", "LiveInit", "Live1", "Live2", "Live3"]
            .iter()
            .map(|n| (format!("{EDGE}/{n}"), format!("bytes of {n}").into_bytes()))
            .collect();
        Upstream { playlist, bodies }
    }

    async fn get(client: &reqwest::Client, url: &str) -> (u16, String, Vec<u8>) {
        let r = client.get(url).send().await.unwrap();
        let status = r.status().as_u16();
        let mime = r
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap().to_owned())
            .unwrap_or_default();
        (status, mime, r.bytes().await.unwrap().to_vec())
    }

    #[tokio::test]
    async fn init_segments_and_fmp4_segments_go_through_the_proxy() {
        // Installs the rustls crypto provider the test client needs.
        pa_twitch::playback::live::ReqwestUpstream::new().unwrap();
        let proxy = LiveProxy::start(
            Arc::new(upstream()),
            "https://playlist.example.invalid/v1/playlist/x.m3u8",
            LiveMode::LowLatency,
        )
        .unwrap();
        let client = reqwest::Client::new();
        let url = proxy.playlist_url();
        let base = &url[..url.rfind('/').unwrap() + 1];
        let (status, _, text) = get(&client, &url).await;
        assert_eq!(status, 200);
        let text = String::from_utf8(text).unwrap();
        assert!(!text.contains("https://"), "upstream URL leaked:\n{text}");
        let maps: Vec<&str> = text
            .lines()
            .filter_map(|l| l.strip_prefix("#EXT-X-MAP:URI=\""))
            .map(|l| l.trim_end_matches('"'))
            .collect();
        assert_eq!(maps, ["m/0.mp4", "m/1.mp4"], "{text}");
        for (uri, name) in maps.iter().zip(["AdInit", "LiveInit"]) {
            let (status, mime, body) = get(&client, &format!("{base}{uri}")).await;
            assert_eq!((status, mime.as_str()), (200, "video/mp4"));
            assert_eq!(body, format!("bytes of {name}").into_bytes());
        }
        let playlist = MediaPlaylist::parse(&text).unwrap();
        let names = ["Ad0", "Live1", "Live2", "Live3"];
        assert_eq!(playlist.segments.len(), names.len());
        for (s, name) in playlist.segments.iter().zip(names) {
            assert!(s.uri.ends_with(".mp4"), "{}", s.uri);
            let (status, mime, body) = get(&client, &format!("{base}{}", s.uri)).await;
            assert_eq!((status, mime.as_str()), (200, "video/mp4"));
            assert_eq!(body, format!("bytes of {name}").into_bytes());
        }
        // The ad is labelled as before and played in full.
        assert_eq!(proxy.report(0.0, 1.0).totals.ad_segments, 1);
        // A second reading keeps the same init segment ids.
        let (_, _, again) = get(&client, &url).await;
        assert!(String::from_utf8(again).unwrap().contains("URI=\"m/1.mp4\""));
    }
}
