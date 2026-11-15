// SPDX-License-Identifier: MPL-2.0
//! The diagnostic report zip and the prefilled GitHub issue link.

use std::fs;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};

use crate::redact::{REDACTED, Redactor};
use crate::zip::ZipWriter;
use crate::{Diagnostics, time};

/// What the app adds to the report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportInfo {
    /// App, core and engine versions, flavor, OS (`key`, `value`).
    pub versions: Vec<(String, String)>,
    /// Non-secret settings. Keys that look like secrets lose their value,
    /// the other values are redacted.
    pub config: Vec<(String, String)>,
}

/// The written report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportSummary {
    pub path: PathBuf,
    /// Entries in the zip.
    pub files: u32,
    pub bytes: u64,
}

const README: &str = "Kinolune diagnostic report\n\
==========================\n\
\n\
This archive was made on your device when you chose \"Export diagnostic report\".\n\
Nothing was sent anywhere. To report a problem, attach it to a GitHub issue\n\
yourself. Home folders, passwords, tokens and keys were cut out\n\
(<redacted>, <user>, ~); please look through it before you share it.\n\
\n\
versions.txt   app, core and system versions\n\
config.txt     settings, without secrets\n\
logs/          app and core logs (only when diagnostics are on)\n\
crashes/       crash reports (only when diagnostics are on)\n";

fn kv_text(r: &Redactor, rows: &[(String, String)], secret_keys: bool) -> String {
    let mut s = String::new();
    for (k, v) in rows {
        let v = if secret_keys && Redactor::is_secret_key(k) {
            REDACTED.to_string()
        } else {
            r.redact(v).into_owned()
        };
        // One line per value: a newline in a value must not forge a row.
        let v = v.replace(['\n', '\r'], " ");
        s.push_str(&format!("{}: {v}\n", r.redact(k)));
    }
    s
}

fn redact_file(r: &Redactor, p: &Path) -> io::Result<Vec<u8>> {
    let raw = fs::read(p)?;
    let text = String::from_utf8_lossy(&raw);
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        out.push_str(&r.redact(line));
        out.push('\n');
    }
    Ok(out.into_bytes())
}

pub(crate) fn build(d: &Diagnostics, out: &Path, info: &ReportInfo) -> io::Result<ReportSummary> {
    let now = time::now_secs();
    let r = d.redactor.read().unwrap_or_else(|e| e.into_inner());
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = out.with_extension("zip.part");
    let file = fs::File::create(&tmp)?;
    let mut z = ZipWriter::new(BufWriter::new(file), now);
    let mut files = 0u32;
    z.add("README.txt", README.as_bytes())?;
    files += 1;
    let mut versions = info.versions.clone();
    versions.push(("report_time".into(), time::iso(now)));
    versions.push(("os".into(), std::env::consts::OS.into()));
    versions.push(("arch".into(), std::env::consts::ARCH.into()));
    versions.push(("pa_log".into(), env!("CARGO_PKG_VERSION").into()));
    versions.push(("diagnostics_opt_in".into(), d.enabled().to_string()));
    z.add("versions.txt", kv_text(&r, &versions, false).as_bytes())?;
    z.add("config.txt", kv_text(&r, &info.config, true).as_bytes())?;
    files += 2;
    // Logs and crashes exist only with the opt-in; written ones were
    // redacted already, the second pass catches rules added since.
    drop(r);
    let logs = d.log_files();
    let crashes = d.crash_reports();
    let r = d.redactor.read().unwrap_or_else(|e| e.into_inner());
    for (prefix, paths) in [("logs", &logs), ("crashes", &crashes)] {
        for p in paths {
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Ok(data) = redact_file(&r, p) else {
                continue;
            };
            z.add(&format!("{prefix}/{name}"), &data)?;
            files += 1;
        }
    }
    let w = z.finish()?;
    w.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    fs::rename(&tmp, out)?;
    Ok(ReportSummary {
        path: out.to_path_buf(),
        files,
        bytes: fs::metadata(out)?.len(),
    })
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Cuts `s` to at most `max` bytes on a char boundary.
fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub(crate) fn issue_url(
    d: &Diagnostics,
    repo_url: &str,
    versions: &[(String, String)],
    report_name: &str,
) -> String {
    let platform = versions
        .iter()
        .map(|(k, v)| format!("{k} {v}"))
        .chain([format!(
            "{} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )])
        .collect::<Vec<_>>()
        .join(", ");
    let mut logs = format!("Diagnostic report: {report_name} (attach the file to this issue)");
    let newest_crash = d
        .crash_reports()
        .last()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|t| {
            t.split("message:\n")
                .nth(1)
                .and_then(|m| m.lines().next().map(str::to_string))
        });
    if let Some(c) = newest_crash {
        logs.push_str("\nLast crash: ");
        logs.push_str(clip(&c, 300));
    }
    let platform = d.redact(clip(&platform, 300));
    let logs = d.redact(&logs);
    format!(
        "{}/issues/new?template=bug_report.yml&labels=bug,triage&platform={}&logs={}",
        repo_url.trim_end_matches('/'),
        percent_encode(&platform),
        percent_encode(&logs)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_clips() {
        assert_eq!(percent_encode("a b/ü"), "a%20b%2F%C3%BC");
        assert_eq!(clip("привет", 3), "п");
    }

    #[test]
    fn config_drops_secret_values() {
        let r = Redactor::default();
        let t = kv_text(
            &r,
            &[
                ("tmdb_api_key".into(), "plain".into()),
                ("ui_locale".into(), "ru\nforged: x".into()),
            ],
            true,
        );
        assert_eq!(t, "tmdb_api_key: <redacted>\nui_locale: ru forged: x\n");
    }
}
