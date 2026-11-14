// SPDX-License-Identifier: MPL-2.0
//! A fake Twitch-style low-latency HLS origin for tests (TWI-006).
//!
//! Serves a sliding live media playlist over plain HTTP on
//! `127.0.0.1:<random port>` from segments given in memory, in real time:
//!
//! - segment `k` is "encoded" from `t0 + Σ d[..k]` to `t0 + Σ d[..=k]`;
//!   complete segments are listed with `#EXT-X-PROGRAM-DATE-TIME`, the next
//!   ones as `#EXT-X-TWITCH-PREFETCH`;
//! - a prefetch segment is streamed with chunked encoding while it is being
//!   "encoded" (bytes in proportion to elapsed time), as Twitch does;
//! - runs of ad segments are titled `Amazon|…` and announced by an
//!   `#EXT-X-DATERANGE` of class `twitch-stitched-ad`.
//!
//! All names and ids are made up; nothing here talks to Twitch.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::playback::live::format_date_time;

/// One segment of the fake stream.
#[derive(Debug, Clone)]
pub struct FakeLiveSegment {
    /// Segment bytes (an MPEG-TS file in player tests, anything otherwise).
    pub bytes: Vec<u8>,
    /// Duration, seconds.
    pub duration: f64,
    /// Part of an ad break.
    pub ad: bool,
}

/// Shape of the fake stream.
#[derive(Debug, Clone, Copy)]
pub struct FakeLiveOptions {
    /// Complete segments listed in the playlist.
    pub window: usize,
    /// Prefetch segments listed after them.
    pub prefetch: usize,
    /// Segments already complete when the origin starts.
    pub preroll: usize,
    /// Media sequence number of the first segment.
    pub first_sequence: u64,
}

impl Default for FakeLiveOptions {
    fn default() -> Self {
        Self {
            window: 6,
            prefetch: 2,
            preroll: 3,
            first_sequence: 1000,
        }
    }
}

/// A segment download the origin finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServedSegment {
    /// Media sequence number.
    pub sequence: u64,
    /// Bytes sent (always the whole segment).
    pub bytes: u64,
    /// An ad segment.
    pub ad: bool,
}

struct Origin {
    segments: Vec<FakeLiveSegment>,
    starts: Vec<i64>,
    options: FakeLiveOptions,
    t0: i64,
    addr: SocketAddr,
    served: Mutex<Vec<ServedSegment>>,
    stop: AtomicBool,
}

/// The running origin; dropping it stops the server.
pub struct FakeLiveOrigin {
    origin: Arc<Origin>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn ms(secs: f64) -> i64 {
    (secs * 1000.0).round() as i64
}

impl FakeLiveOrigin {
    /// Starts serving `segments`; the first `options.preroll` are complete
    /// at once, the rest follow in real time.
    pub fn start(segments: Vec<FakeLiveSegment>, options: FakeLiveOptions) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let addr = listener.local_addr()?;
        let mut starts = Vec::with_capacity(segments.len() + 1);
        let mut at = 0i64;
        for s in &segments {
            starts.push(at);
            at += ms(s.duration);
        }
        starts.push(at);
        let preroll = options.preroll.min(segments.len());
        let t0 = now_ms() - starts[preroll];
        let origin = Arc::new(Origin {
            segments,
            starts,
            options,
            t0,
            addr,
            served: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
        });
        listener.set_nonblocking(true)?;
        let o = origin.clone();
        let thread = std::thread::Builder::new()
            .name("fake-live-origin".into())
            .spawn(move || {
                while !o.stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let o = o.clone();
                            let _ = std::thread::Builder::new()
                                .name("fake-live-conn".into())
                                .spawn(move || {
                                    let _ = o.serve(stream);
                                });
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(10)),
                    }
                }
            })?;
        Ok(Self {
            origin,
            thread: Some(thread),
        })
    }

    /// Media playlist URL.
    pub fn playlist_url(&self) -> String {
        format!("http://{}/live/index.m3u8", self.origin.addr)
    }

    /// Program date-time of the first frame of the first segment, Unix ms.
    pub fn t0_ms(&self) -> i64 {
        self.origin.t0
    }

    /// Media sequence number of segment `index`.
    pub fn sequence(&self, index: usize) -> u64 {
        self.origin.options.first_sequence + index as u64
    }

    /// Segment downloads completed so far, in completion order.
    pub fn served(&self) -> Vec<ServedSegment> {
        self.origin
            .served
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The current playlist text (as a client would get it).
    pub fn playlist_now(&self) -> String {
        self.origin.playlist(now_ms())
    }
}

impl Drop for FakeLiveOrigin {
    fn drop(&mut self) {
        self.origin.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Origin {
    /// Start and end (Unix ms) of segment `k`.
    fn span(&self, k: usize) -> (i64, i64) {
        (self.t0 + self.starts[k], self.t0 + self.starts[k + 1])
    }

    /// Number of complete segments at `now`.
    fn complete(&self, now: i64) -> usize {
        (0..self.segments.len())
            .take_while(|&k| self.span(k).1 <= now)
            .count()
    }

    fn url(&self, k: usize) -> String {
        format!(
            "http://{}/live/seg/{}.ts",
            self.addr,
            self.options.first_sequence + k as u64
        )
    }

    fn playlist(&self, now: i64) -> String {
        let done = self.complete(now);
        let first = done.saturating_sub(self.options.window);
        let last_listed = (done + self.options.prefetch).min(self.segments.len());
        let target = self
            .segments
            .iter()
            .map(|s| s.duration.ceil() as u64)
            .max()
            .unwrap_or(2);
        let mut out = format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:{}\n#EXT-X-TWITCH-LIVE-SEQUENCE:{}\n#EXT-X-TWITCH-ELAPSED-SECS:{:.3}\n",
            self.options.first_sequence + first as u64,
            self.options.first_sequence + done as u64,
            self.starts[done] as f64 / 1000.0
        );
        // Ad breaks touching the listed segments, announced up front.
        let mut k = 0;
        while k < self.segments.len() {
            if !self.segments[k].ad {
                k += 1;
                continue;
            }
            let start = k;
            while k < self.segments.len() && self.segments[k].ad {
                k += 1;
            }
            if start < last_listed && k > first {
                let (begin, _) = self.span(start);
                let (_, end) = self.span(k - 1);
                out.push_str(&format!(
                    "#EXT-X-DATERANGE:ID=\"stitched-ad-{start}-{}\",CLASS=\"twitch-stitched-ad\",START-DATE=\"{}\",DURATION={:.3},X-TV-TWITCH-AD-POD-LENGTH=\"1\",X-TV-TWITCH-AD-POD-POSITION=\"0\",X-TV-TWITCH-AD-ROLL-TYPE=\"MIDROLL\"\n",
                    k - start,
                    format_date_time(begin),
                    (end - begin) as f64 / 1000.0
                ));
            }
        }
        for k in first..done {
            let s = &self.segments[k];
            out.push_str(&format!(
                "#EXT-X-PROGRAM-DATE-TIME:{}\n#EXTINF:{:.3},{}\n{}\n",
                format_date_time(self.span(k).0),
                s.duration,
                if s.ad { "Amazon|fake-ad-0001" } else { "live" },
                self.url(k)
            ));
        }
        for k in done..last_listed {
            out.push_str(&format!("#EXT-X-TWITCH-PREFETCH:{}\n", self.url(k)));
        }
        if done == self.segments.len() {
            out.push_str("#EXT-X-ENDLIST\n");
        }
        out
    }

    fn serve(&self, stream: TcpStream) -> io::Result<()> {
        stream.set_nodelay(true)?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
                break;
            }
        }
        let mut w = stream;
        let now = now_ms();
        if path == "/live/index.m3u8" {
            let body = self.playlist(now);
            return respond(
                &mut w,
                200,
                "application/vnd.apple.mpegurl",
                body.as_bytes(),
            );
        }
        let seq = path
            .strip_prefix("/live/seg/")
            .and_then(|n| n.strip_suffix(".ts"))
            .and_then(|n| n.parse::<u64>().ok())
            .and_then(|s| s.checked_sub(self.options.first_sequence))
            .map(|k| k as usize);
        let Some(k) = seq.filter(|&k| k < self.segments.len()) else {
            return respond(&mut w, 404, "text/plain", b"not found");
        };
        if k >= self.complete(now) + self.options.prefetch {
            return respond(&mut w, 404, "text/plain", b"not yet");
        }
        let (start, end) = self.span(k);
        let bytes = &self.segments[k].bytes;
        if now >= end {
            respond(&mut w, 200, "video/mp2t", bytes)?;
        } else {
            // Being encoded: stream it as it "comes out of the encoder".
            if now < start {
                std::thread::sleep(Duration::from_millis((start - now) as u64));
            }
            w.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )?;
            let mut sent = 0usize;
            loop {
                let t = now_ms();
                let ready = if t >= end {
                    bytes.len()
                } else {
                    (bytes.len() as f64 * (t - start).max(0) as f64 / (end - start) as f64) as usize
                };
                if ready > sent {
                    w.write_all(format!("{:x}\r\n", ready - sent).as_bytes())?;
                    w.write_all(&bytes[sent..ready])?;
                    w.write_all(b"\r\n")?;
                    w.flush()?;
                    sent = ready;
                }
                if sent == bytes.len() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            w.write_all(b"0\r\n\r\n")?;
            w.flush()?;
        }
        self.served
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(ServedSegment {
                sequence: self.options.first_sequence + k as u64,
                bytes: bytes.len() as u64,
                ad: self.segments[k].ad,
            });
        Ok(())
    }
}

fn respond(w: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) -> io::Result<()> {
    let reason = if status == 200 { "OK" } else { "Not Found" };
    write!(
        w,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    w.write_all(body)?;
    w.flush()
}
