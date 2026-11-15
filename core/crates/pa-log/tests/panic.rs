// SPDX-License-Identifier: MPL-2.0
//! The panic hook of the installed diagnostics (its own test binary:
//! `install` works once per process).

use pa_log::{Diagnostics, Redactor, Rotation};

#[test]
fn panics_become_crash_reports_only_with_opt_in() {
    let tmp = tempfile::tempdir().unwrap();
    let d = pa_log::install(Diagnostics::open(
        tmp.path().join("diagnostics"),
        Redactor::with_homes(["/home/alice"]),
        Rotation::default(),
    ));
    let boom = || {
        let _ = std::panic::catch_unwind(|| {
            panic!("cannot open /home/alice/a.mkv?access_token=s3cretvalue");
        });
    };
    boom();
    assert!(!tmp.path().join("diagnostics").exists());

    d.set_enabled(true).unwrap();
    tracing::info!(target: "pa_test", "global subscriber line");
    boom();
    let reports = d.crash_reports();
    assert_eq!(reports.len(), 1);
    let text = std::fs::read_to_string(&reports[0]).unwrap();
    assert!(text.contains("kind: rust-panic"), "{text}");
    assert!(
        text.contains("cannot open ~/a.mkv?access_token=<redacted>"),
        "{text}"
    );
    assert!(
        !text.contains("s3cretvalue") && !text.contains("alice"),
        "{text}"
    );
    let log = std::fs::read_to_string(&d.log_files()[0]).unwrap();
    assert!(log.contains("global subscriber line"), "{log}");
    assert!(log.contains("crash: rust-panic"), "{log}");
}
