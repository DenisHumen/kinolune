// SPDX-License-Identifier: MPL-2.0
//! Detects intros and credits in a folder of episodes and, with a truth
//! file, reports precision and recall (PLAY-011).
//!
//! ```text
//! core/crates/pa-skip/tools/make-test-season.py --out /tmp/season
//! PA_FFMPEG_LIB_DIR=… cargo run --release -p pa-skip --example skip_season -- \
//!     "/tmp/season/Show/Season 01" /tmp/season/truth.tsv
//! ```
//!
//! `truth.tsv`: file name, intro start and end, credits start and end in
//! milliseconds (`-` for none). A detection is correct when both its start
//! and end are within the tolerance (`PA_SKIP_TOLERANCE_MS`, default 2000).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use pa_library::skip::SkipKind;
use pa_probe::Ffmpeg;
use pa_skip::{AnalyzeOptions, FileAnalysis, Found, analyze_file, season_segments};
use pa_vfs::{ByteSource, LocalFs, Vfs, VfsPath};

type Truth = HashMap<String, [Option<(u64, u64)>; 2]>;

fn read_truth(path: &str) -> Truth {
    let text = std::fs::read_to_string(path).expect("truth file");
    let mut out = Truth::new();
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        let f: Vec<&str> = line.split('\t').collect();
        let pair = |a: &str, b: &str| a.parse().ok().zip(b.parse().ok());
        out.insert(f[0].to_owned(), [pair(f[1], f[2]), pair(f[3], f[4])]);
    }
    out
}

fn fmt(ms: u64) -> String {
    format!("{}:{:02}.{:03}", ms / 60_000, ms / 1000 % 60, ms % 1000)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("usage: skip_season <folder> [truth.tsv]");
    let truth = args.next().map(|p| read_truth(&p));
    let tolerance: u64 = std::env::var("PA_SKIP_TOLERANCE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let ffmpeg = Ffmpeg::shared().expect("FFmpeg (PA_FFMPEG_LIB_DIR)");
    let vfs = LocalFs::new(&dir).expect("folder");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("folder")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| {
            Path::new(n)
                .extension()
                .is_some_and(|e| matches!(e.to_str(), Some("mkv" | "mp4" | "ts" | "avi")))
        })
        .collect();
    names.sort();
    let opts = AnalyzeOptions::default();
    let started = Instant::now();
    let mut analyses: Vec<FileAnalysis> = Vec::new();
    for name in &names {
        let t = Instant::now();
        let source: Arc<dyn ByteSource> = Arc::from(
            vfs.open(&VfsPath::parse(&format!("/{name}")).expect("path"))
                .expect("open"),
        );
        let a = analyze_file(ffmpeg, &source, &opts).expect("analysis");
        eprintln!(
            "{name}: {} frames intro, {} frames outro, video credits {:?}, {:.1} s",
            a.intro.as_ref().map_or(0, |f| f.hashes.len()),
            a.outro.as_ref().map_or(0, |f| f.hashes.len()),
            a.video_credits,
            t.elapsed().as_secs_f64()
        );
        analyses.push(a);
    }
    let t = Instant::now();
    let refs: Vec<&FileAnalysis> = analyses.iter().collect();
    let found = season_segments(&refs);
    eprintln!(
        "matching: {:.2} s; total {:.1} s",
        t.elapsed().as_secs_f64(),
        started.elapsed().as_secs_f64()
    );
    // [kind][tp, detections, truths]
    let mut score = [[0usize; 3]; 2];
    for (name, segs) in names.iter().zip(&found) {
        let pick = |kind: SkipKind| -> Option<&Found> {
            segs.iter()
                .filter(|s| s.kind == kind)
                .min_by_key(|s| s.origin)
        };
        let mut line = format!("{name}:");
        for (k, kind) in [SkipKind::Intro, SkipKind::Credits].into_iter().enumerate() {
            let got = pick(kind);
            let want = truth.as_ref().and_then(|t| t.get(name)).and_then(|t| t[k]);
            line += &match got {
                Some(s) => format!(
                    "  {} {}–{} ({:?}, {:.2})",
                    kind.as_str(),
                    fmt(s.start_ms),
                    fmt(s.end_ms),
                    s.origin,
                    s.confidence
                ),
                None => format!("  {} none", kind.as_str()),
            };
            if let Some(t) = truth.as_ref().and_then(|t| t.get(name)) {
                let want_k = t[k];
                score[k][2] += usize::from(want_k.is_some());
                if let Some(s) = got {
                    score[k][1] += 1;
                    if let Some((ws, we)) = want {
                        let ok = s.start_ms.abs_diff(ws) <= tolerance && s.end_ms.abs_diff(we) <= tolerance;
                        line += &format!(
                            " [Δ {:+} / {:+} ms{}]",
                            s.start_ms as i64 - ws as i64,
                            s.end_ms as i64 - we as i64,
                            if ok { "" } else { " MISS" }
                        );
                        score[k][0] += usize::from(ok);
                    } else {
                        line += " [FALSE]";
                    }
                }
            }
        }
        println!("{line}");
    }
    if truth.is_some() {
        for (k, kind) in ["intro", "credits"].iter().enumerate() {
            let [tp, det, all] = score[k];
            let p = if det == 0 { 1.0 } else { tp as f64 / det as f64 };
            let r = if all == 0 { 1.0 } else { tp as f64 / all as f64 };
            println!(
                "{kind}: precision {:.1} % ({tp}/{det}), recall {:.1} % ({tp}/{all}) at ±{tolerance} ms",
                p * 100.0,
                r * 100.0
            );
        }
    }
}
