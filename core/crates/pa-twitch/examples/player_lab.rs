// SPDX-License-Identifier: MPL-2.0
//! Player lab for the manual checks of TWI-014 (full flavor only).
//!
//! Stands in for usher: serves made-up master playlists whose variants
//! point at local media, so the app's native Twitch player runs end to end
//! without Twitch (build the app with
//! `--dart-define=KINOLUNE_TWITCH_LAB=http://127.0.0.1:<port>`):
//!
//! - `/live/<login>/master.m3u8`: a live stream of the fake low-latency
//!   origin of TWI-006 (`FakeLiveOrigin`: prefetch segments, an ad break
//!   announced by `twitch-stitched-ad`). Variants `720p60 (source)` and
//!   `480p30` (same media: this is a lab) and `audio_only` when an audio
//!   directory is given.
//! - `/vod/<id>/master.m3u8` and `/vod/<id>/index.m3u8`: a VOD of the same
//!   segments (`#EXT-X-ENDLIST`).
//!
//! ```text
//! player_lab <video dir> [--audio <dir>] [--port 0] [--segment-secs 2]
//!            [--ads 20,21,22] [--live-from 0]
//!            [--vod <dir> --vod-audio <dir> --vod-secs 6]
//! ```
//!
//! `--vod` and `--vod-audio` give the VOD its own segments (a long
//! continuous recording, so the chapters and resume positions of the
//! made-up channels fit on its timeline); by default it is the live media.
//!
//! Prints one JSON line with the lab URL and the origin ports (for
//! `adb reverse` on an emulator), then serves until stdin closes.
//! All names and ids are made up; nothing here talks to Twitch.

#[cfg(not(pa_twitch_native))]
fn main() {
    eprintln!("player_lab needs the full flavor: --features testing,pa-flavor/full");
    std::process::exit(2);
}

#[cfg(pa_twitch_native)]
fn main() {
    lab::main();
}

#[cfg(pa_twitch_native)]
mod lab {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use pa_twitch::testing::{FakeLiveOptions, FakeLiveOrigin, FakeLiveSegment};

    struct Media {
        video: Vec<Vec<u8>>,
        vod: Vec<Vec<u8>>,
        vod_audio: Vec<Vec<u8>>,
        vod_secs: f64,
        live: String,
        live_audio: Option<String>,
        base: String,
    }

    fn segments(dir: &Path) -> Vec<Vec<u8>> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .expect("read dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "ts"))
            .collect();
        files.sort();
        files
            .iter()
            .map(|p| std::fs::read(p).expect("read segment"))
            .collect()
    }

    fn origin(segs: &[Vec<u8>], from: usize, secs: f64, ads: &[usize]) -> FakeLiveOrigin {
        let list = segs
            .iter()
            .enumerate()
            .skip(from)
            .map(|(k, b)| FakeLiveSegment {
                bytes: b.clone(),
                duration: secs,
                ad: ads.contains(&k),
            })
            .collect();
        FakeLiveOrigin::start(list, FakeLiveOptions::default()).expect("origin")
    }

    pub fn main() {
        let mut args = std::env::args().skip(1);
        let dir = PathBuf::from(args.next().expect("video segments dir"));
        let mut audio_dir: Option<PathBuf> = None;
        let mut port = 0u16;
        let mut secs = 2.0f64;
        let mut ads: Vec<usize> = Vec::new();
        let mut from = 0usize;
        let mut vod_dir: Option<PathBuf> = None;
        let mut vod_audio_dir: Option<PathBuf> = None;
        let mut vod_secs: Option<f64> = None;
        while let Some(flag) = args.next() {
            let value = args.next().expect("flag value");
            match flag.as_str() {
                "--audio" => audio_dir = Some(PathBuf::from(value)),
                "--port" => port = value.parse().expect("port"),
                "--segment-secs" => secs = value.parse().expect("seconds"),
                "--ads" => {
                    ads = value
                        .split(',')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.parse().expect("index"))
                        .collect()
                }
                "--live-from" => from = value.parse().expect("index"),
                "--vod" => vod_dir = Some(PathBuf::from(value)),
                "--vod-audio" => vod_audio_dir = Some(PathBuf::from(value)),
                "--vod-secs" => vod_secs = Some(value.parse().expect("seconds")),
                other => panic!("unknown flag {other}"),
            }
        }
        let video = segments(&dir);
        assert!(!video.is_empty(), "no .ts files in {}", dir.display());
        let audio = audio_dir.as_deref().map(segments).unwrap_or_default();
        let live = origin(&video, from, secs, &ads);
        let live_audio = (!audio.is_empty()).then(|| origin(&audio, from, secs, &ads));
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).expect("bind");
        let addr = listener.local_addr().expect("addr");
        let port_of = |url: &str| -> u16 {
            url.trim_start_matches("http://127.0.0.1:")
                .split('/')
                .next()
                .and_then(|p| p.parse().ok())
                .unwrap_or_default()
        };
        let mut ports = vec![addr.port(), port_of(&live.playlist_url())];
        if let Some(a) = &live_audio {
            ports.push(port_of(&a.playlist_url()));
        }
        let vod = vod_dir
            .as_deref()
            .map(segments)
            .unwrap_or_else(|| video.clone());
        let vod_audio = vod_audio_dir.as_deref().map(segments).unwrap_or(audio);
        let media = Arc::new(Media {
            vod,
            vod_audio,
            vod_secs: vod_secs.unwrap_or(secs),
            video,
            live: live.playlist_url(),
            live_audio: live_audio.as_ref().map(FakeLiveOrigin::playlist_url),
            base: format!("http://{addr}"),
        });
        println!(
            "{}",
            serde_json::json!({
                "lab": media.base,
                "live_origin": media.live,
                "ports": ports,
                "segments": media.video.len(),
            })
        );
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let m = media.clone();
                std::thread::spawn(move || {
                    let _ = serve(&m, stream);
                });
            }
        });
        // Serve until stdin closes; keep the origins alive.
        let mut sink = String::new();
        let _ = std::io::stdin().read_to_string(&mut sink);
        drop((live, live_audio));
    }

    fn master(variants: &[(&str, &str, Option<&str>, u64, &str)]) -> String {
        let mut out = String::from("#EXTM3U\n");
        for (group, name, resolution, bandwidth, url) in variants {
            out.push_str(&format!(
                "#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"{group}\",NAME=\"{name}\",AUTOSELECT=YES,DEFAULT=YES\n"
            ));
            let res = resolution
                .map(|r| format!(",RESOLUTION={r}"))
                .unwrap_or_default();
            let codecs = if *group == "audio_only" {
                "mp4a.40.2"
            } else {
                "avc1.4D401F,mp4a.40.2"
            };
            out.push_str(&format!(
                "#EXT-X-STREAM-INF:BANDWIDTH={bandwidth}{res},CODECS=\"{codecs}\",VIDEO=\"{group}\"\n{url}\n"
            ));
        }
        out
    }

    fn vod_playlist(count: usize, secs: f64, prefix: &str) -> String {
        let mut out = format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{}\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-MEDIA-SEQUENCE:0\n",
            secs.ceil() as u64
        );
        for k in 0..count {
            out.push_str(&format!("#EXTINF:{secs:.3},\n{prefix}/{k}.ts\n"));
        }
        out.push_str("#EXT-X-ENDLIST\n");
        out
    }

    fn serve(m: &Media, stream: TcpStream) -> std::io::Result<()> {
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
                break;
            }
        }
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let (status, kind, body): (&str, &str, Vec<u8>) = match parts.as_slice() {
            ["live", _, "master.m3u8"] => {
                let mut v = vec![
                    (
                        "chunked",
                        "720p60 (source)",
                        Some("1280x720"),
                        3_000_000,
                        m.live.as_str(),
                    ),
                    (
                        "480p30",
                        "480p30",
                        Some("854x480"),
                        1_400_000,
                        m.live.as_str(),
                    ),
                ];
                if let Some(a) = &m.live_audio {
                    v.push(("audio_only", "audio_only", None, 160_000, a.as_str()));
                }
                (
                    "200 OK",
                    "application/vnd.apple.mpegurl",
                    master(&v).into_bytes(),
                )
            }
            ["vod", id, "master.m3u8"] => {
                let video = format!("{}/vod/{id}/index.m3u8", m.base);
                let audio = format!("{}/vod/{id}/audio.m3u8", m.base);
                let mut v = vec![
                    (
                        "chunked",
                        "720p60 (source)",
                        Some("1280x720"),
                        3_000_000,
                        video.as_str(),
                    ),
                    (
                        "480p30",
                        "480p30",
                        Some("854x480"),
                        1_400_000,
                        video.as_str(),
                    ),
                ];
                if !m.vod_audio.is_empty() {
                    v.push(("audio_only", "audio_only", None, 160_000, audio.as_str()));
                }
                (
                    "200 OK",
                    "application/vnd.apple.mpegurl",
                    master(&v).into_bytes(),
                )
            }
            ["vod", _, "index.m3u8"] => (
                "200 OK",
                "application/vnd.apple.mpegurl",
                vod_playlist(m.vod.len(), m.vod_secs, "v").into_bytes(),
            ),
            ["vod", _, "audio.m3u8"] => (
                "200 OK",
                "application/vnd.apple.mpegurl",
                vod_playlist(m.vod_audio.len(), m.vod_secs, "a").into_bytes(),
            ),
            ["vod", _, dir @ ("v" | "a"), file] => {
                let list = if *dir == "v" { &m.vod } else { &m.vod_audio };
                match file
                    .strip_suffix(".ts")
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|n| list.get(n))
                {
                    Some(bytes) => ("200 OK", "video/mp2t", bytes.clone()),
                    None => ("404 Not Found", "text/plain", b"no segment".to_vec()),
                }
            }
            _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
        };
        let mut out = stream;
        write!(
            out,
            "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        out.write_all(&body)?;
        out.flush()
    }
}
