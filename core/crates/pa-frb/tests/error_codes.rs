// SPDX-License-Identifier: MPL-2.0
//! CORE-009: every public error of the Dart facade carries a stable code.
//!
//! Reads the sources of `src/api`: each `pub struct …Error` must have a
//! `pub code: String` field, and every code literal there must have the
//! shape of a code (`pa_runtime::is_valid_code`).

use std::fs;
use std::path::Path;

fn api_sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&p).unwrap())
        })
        .collect();
    files.sort();
    files
}

/// Bodies of `pub struct <Name>Error { … }` as (name, body).
fn error_structs(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(at) = rest.find("pub struct ") {
        rest = &rest[at + "pub struct ".len()..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.ends_with("Error") {
            continue;
        }
        let open = rest.find('{').unwrap();
        let close = rest[open..].find("\n}").unwrap() + open;
        out.push((name, rest[open..close].to_owned()));
    }
    out
}

/// Every string after one of `prefixes` up to the closing quote.
fn literals_after<'a>(src: &'a str, prefix: &str) -> Vec<&'a str> {
    src.match_indices(prefix)
        .filter_map(|(at, _)| {
            let start = at + prefix.len();
            src[start..].find('"').map(|end| &src[start..start + end])
        })
        .collect()
}

#[test]
fn every_public_error_has_a_code_field() {
    let mut seen = 0;
    let mut missing = Vec::new();
    for (file, src) in api_sources() {
        for (name, body) in error_structs(&src) {
            seen += 1;
            if !body.contains("pub code: String,") {
                missing.push(format!("{file}: {name}"));
            }
        }
    }
    assert!(seen >= 15, "found only {seen} error structs");
    assert!(missing.is_empty(), "errors without `code`: {missing:?}");
}

#[test]
fn code_literals_are_valid_codes() {
    let prefixes = [
        "code: \"",
        "LibraryError::new(\"",
        "err_as(\"",
        "error(\"",
        "u.fail(\"",
    ];
    let mut bad = Vec::new();
    let mut checked = 0;
    for (file, src) in api_sources() {
        for prefix in prefixes {
            for code in literals_after(&src, prefix) {
                checked += 1;
                if !pa_runtime::is_valid_code(code) {
                    bad.push(format!("{file}: {code:?}"));
                }
            }
        }
    }
    assert!(checked > 30, "checked only {checked} codes");
    assert!(bad.is_empty(), "malformed codes: {bad:?}");
}
