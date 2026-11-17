// SPDX-License-Identifier: MPL-2.0
//! LIB-025 on the anonymised real library of META-032
//! (`fixtures/real-library.tsv`): every file there is a movie or an
//! episode, so the content kind classifier, run folder by folder as the
//! scan runs it, must call none of them the user's own footage. A miss here
//! would silently hide a film in «Мои видео».

use std::collections::BTreeMap;

use pa_recognition::content::{ContentKind, VideoFile, classify_videos, folder_of};

const FIXTURE: &str = include_str!("fixtures/real-library.tsv");

#[test]
fn no_titled_video_is_taken_for_own_footage() {
    let paths: Vec<&str> = FIXTURE
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty() && !l.ends_with('/'))
        .filter_map(|l| l.split('\t').next())
        .collect();
    assert!(paths.len() > 1000, "{} files", paths.len());
    let mut folders: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for p in &paths {
        folders.entry(folder_of(p)).or_default().push(p);
    }
    let mut own = Vec::new();
    for files in folders.values() {
        let input: Vec<VideoFile<'_>> = files
            .iter()
            .map(|p| VideoFile {
                path: p,
                ..VideoFile::default()
            })
            .collect();
        for (v, p) in classify_videos(&input).iter().zip(files) {
            if v.kind == ContentKind::OwnVideo {
                own.push(format!("{p} {:?}", v.reasons));
            }
        }
    }
    assert!(own.is_empty(), "own footage among titled videos: {own:#?}");
}
