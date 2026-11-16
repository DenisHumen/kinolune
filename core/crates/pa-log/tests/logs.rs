// SPDX-License-Identifier: MPL-2.0
//! Opted-in logs land in a file, from `tracing` and from the app, with a
//! data directory written the way the Windows app passes it (its own test
//! binary: `install` works once per process).

use std::path::{Path, PathBuf};

use pa_log::{Diagnostics, Redactor, Rotation};

fn log_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir.join("logs")) else {
        return Vec::new();
    };
    rd.filter_map(|e| e.ok().map(|e| e.path())).collect()
}

#[test]
fn opted_in_lines_are_written_to_the_log_file() {
    let tmp = tempfile::tempdir().unwrap();
    // The app joins with '/' even on Windows: `<data>/diagnostics`.
    let dir = PathBuf::from(format!("{}/diagnostics", tmp.path().display()));
    let d = pa_log::install(Diagnostics::open(
        &dir,
        Redactor::with_homes(Vec::<String>::new()),
        Rotation::default(),
    ));
    tracing::info!(target: "pa_test", "before opt-in");
    d.log("info", "app", "before opt-in");
    assert!(log_files(&dir).is_empty());

    d.set_enabled(true).unwrap();
    tracing::info!(target: "pa_test", "core line after opt-in");
    d.log("info", "app", "app line after opt-in");

    let files = log_files(&dir);
    assert_eq!(files.len(), 1, "{files:?}");
    let text = std::fs::read_to_string(&files[0]).unwrap();
    assert!(text.contains("app line after opt-in"), "{text}");
    assert!(text.contains("core line after opt-in"), "{text}");
    assert!(!text.contains("before opt-in"), "{text}");
    assert!(d.status().log_bytes > 0);
}
