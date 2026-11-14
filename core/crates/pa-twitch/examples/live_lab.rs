// SPDX-License-Identifier: MPL-2.0
//! Live lab for the player test of TWI-006 (full flavor only).
//!
//! Serves the MPEG-TS segments of a directory (`*.ts`, sorted by name, made
//! by FFmpeg) as a fake Twitch-style low-latency live stream on
//! `127.0.0.1`, puts the live proxy in front of it and prints one JSON line:
//!
//! ```json
//! {"proxy":"http://127.0.0.1:…/<token>/live.m3u8","stats":"…/stats.json",
//!  "origin":"…/live/index.m3u8","t0_ms":…,"first_sequence":1000}
//! ```
//!
//! Then it reads commands from stdin, one per line: `served` prints the
//! segments the origin finished sending as a JSON array; EOF stops.
//!
//! ```text
//! live_lab <dir> [--segment-secs 2] [--ads 6,7,8] [--mode low|stable]
//! ```

#[cfg(not(pa_twitch_native))]
fn main() {
    eprintln!("live_lab needs the full flavor: --features testing,pa-flavor/full");
    std::process::exit(2);
}

#[cfg(pa_twitch_native)]
fn main() {
    use std::io::BufRead;
    use std::sync::Arc;

    use pa_twitch::playback::live::{LiveMode, LiveProxy, ReqwestUpstream};
    use pa_twitch::testing::{FakeLiveOptions, FakeLiveOrigin, FakeLiveSegment};

    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("segments dir");
    let mut segment_secs = 2.0f64;
    let mut ads: Vec<usize> = Vec::new();
    let mut mode = LiveMode::LowLatency;
    while let Some(flag) = args.next() {
        let value = args.next().expect("flag value");
        match flag.as_str() {
            "--segment-secs" => segment_secs = value.parse().expect("seconds"),
            "--ads" => {
                ads = value
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.parse().expect("index"))
                    .collect()
            }
            "--mode" => {
                mode = match value.as_str() {
                    "stable" => LiveMode::Stable,
                    _ => LiveMode::LowLatency,
                }
            }
            other => panic!("unknown flag {other}"),
        }
    }
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "ts"))
        .collect();
    files.sort();
    let segments: Vec<FakeLiveSegment> = files
        .iter()
        .enumerate()
        .map(|(k, p)| FakeLiveSegment {
            bytes: std::fs::read(p).expect("read segment"),
            duration: segment_secs,
            ad: ads.contains(&k),
        })
        .collect();
    assert!(!segments.is_empty(), "no .ts files in {dir}");
    let options = FakeLiveOptions::default();
    let origin = FakeLiveOrigin::start(segments, options).expect("origin");
    let upstream = Arc::new(ReqwestUpstream::new().expect("client"));
    let proxy = LiveProxy::start(upstream, &origin.playlist_url(), mode).expect("proxy");
    println!(
        "{}",
        serde_json::json!({
            "proxy": proxy.playlist_url(),
            "stats": proxy.stats_url(),
            "origin": origin.playlist_url(),
            "t0_ms": origin.t0_ms(),
            "first_sequence": options.first_sequence,
        })
    );
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim() == "served" {
            let served: Vec<_> = origin
                .served()
                .iter()
                .map(|s| serde_json::json!({"sequence": s.sequence, "bytes": s.bytes, "ad": s.ad}))
                .collect();
            println!("{}", serde_json::Value::Array(served));
        }
    }
}
