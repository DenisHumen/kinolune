// SPDX-License-Identifier: MPL-2.0
//! Panic isolation in the scan pipeline.
//!
//! A scan reads whatever the user's folders hold: names in any script,
//! broken containers, odd server answers. A bug triggered by one such file
//! must cost that file, never the scan: before this guard a panic on one
//! name (a `str` slice inside a Cyrillic letter, `pa-vfs-http::is_strm`)
//! unwound through the probe workers and the whole pipeline, so no file of
//! the source was ever recognized and every rescan failed the same way.
//!
//! - [`per_file`] wraps the work on one file (probe, fingerprint): a panic
//!   is logged and the file is treated as unreadable;
//! - [`per_source`] wraps the analysis of one source in
//!   [`Catalog::scan`](crate::Catalog::scan): a panic is reported as an
//!   error of that source and the scan goes on with the next one.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Text of a panic payload.
pub(crate) fn message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_owned()
    }
}

/// Runs `f` on the file `path`; `None` (logged) if it panicked.
pub(crate) fn per_file<T>(step: &'static str, path: &str, f: impl FnOnce() -> T) -> Option<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(v) => Some(v),
        Err(p) => {
            tracing::error!(
                target: "pa_catalog::scan",
                step,
                path,
                "internal error on one file, the file is skipped: {}",
                message(p.as_ref())
            );
            None
        }
    }
}

/// Runs the analysis of one source; `Err` with the panic text if it
/// panicked.
pub(crate) fn per_source<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| message(p.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_costs_one_file() {
        assert_eq!(per_file("probe", "/a.mkv", || 7), Some(7));
        let s = String::from("Фильм.mkv");
        let r = per_file("probe", "/Фильм.mkv", || {
            // The original bug: a slice inside `м`.
            let tail = &s[s.len() - 5..];
            tail.len()
        });
        assert_eq!(r, None);
    }

    #[test]
    fn a_panic_of_a_source_is_its_error() {
        assert_eq!(per_source(|| 1), Ok(1));
        let e = per_source(|| -> i32 { panic!("boom {}", 1) }).unwrap_err();
        assert_eq!(e, "boom 1");
        let e = per_source(|| -> i32 { panic!("static") }).unwrap_err();
        assert_eq!(e, "static");
    }
}
