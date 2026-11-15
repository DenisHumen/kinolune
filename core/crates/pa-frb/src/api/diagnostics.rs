// SPDX-License-Identifier: MPL-2.0
//! Diagnostics for Dart (`pa-log`, CORE-008, ADR-NNNN): the opt-in, app
//! log lines and crashes, the diagnostic report zip and the prefilled
//! GitHub issue link. Nothing is written before the user opts in, and
//! nothing is ever sent: the user attaches the zip to an issue themselves.

use std::path::Path;

use flutter_rust_bridge::frb;
use pa_log::{Diagnostics, Redactor, ReportInfo, Rotation};

/// Error of a diagnostics call; Dart receives it as an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsError {
    /// Human-readable message (redacted).
    pub message: String,
}

fn err(e: impl std::fmt::Display) -> DiagnosticsError {
    let text = e.to_string();
    DiagnosticsError {
        message: match pa_log::global() {
            Some(d) => d.redact(&text),
            None => Redactor::default().redact(&text).into_owned(),
        },
    }
}

/// What the settings screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticsStatus {
    /// The user opted in: logs and crash reports are kept on the device.
    pub enabled: bool,
    pub log_files: u32,
    pub log_bytes: u64,
    pub crash_reports: u32,
}

impl From<pa_log::Status> for DiagnosticsStatus {
    fn from(s: pa_log::Status) -> Self {
        Self {
            enabled: s.enabled,
            log_files: s.log_files,
            log_bytes: s.log_bytes,
            crash_reports: s.crash_reports,
        }
    }
}

/// A `key: value` row of the report (versions, settings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsEntry {
    pub key: String,
    pub value: String,
}

/// The written report zip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsReport {
    pub path: String,
    /// Entries in the zip.
    pub files: u32,
    pub bytes: u64,
}

fn rows(v: Vec<DiagnosticsEntry>) -> Vec<(String, String)> {
    v.into_iter().map(|e| (e.key, e.value)).collect()
}

fn installed() -> Result<std::sync::Arc<Diagnostics>, DiagnosticsError> {
    pa_log::global().ok_or_else(|| DiagnosticsError {
        message: "diagnostics are not open".into(),
    })
}

/// Opens the diagnostics in `dir` (`<app data>/diagnostics`) and installs
/// them for the process: the core's `tracing` log, the panic hook and the
/// native crash marker. `home_dirs` are the user's folders as the app sees
/// them (cut from every line as `~`, on top of `$HOME`). Creates nothing
/// until the user opts in. A second call (a Dart hot restart) returns the
/// diagnostics already open.
#[frb(sync)]
pub fn diagnostics_open(dir: String, home_dirs: Vec<String>) -> DiagnosticsStatus {
    let d = pa_log::install(Diagnostics::open(
        dir,
        Redactor::new(home_dirs),
        Rotation::default(),
    ));
    d.status().into()
}

/// Opt-in and what is kept; `None` before [`diagnostics_open`].
#[frb(sync)]
pub fn diagnostics_status() -> Option<DiagnosticsStatus> {
    pa_log::global().map(|d| d.status().into())
}

/// Turns diagnostics on or off. Off deletes the logs and crash reports.
#[frb(sync)]
pub fn diagnostics_set_enabled(enabled: bool) -> Result<DiagnosticsStatus, DiagnosticsError> {
    let d = installed()?;
    d.set_enabled(enabled).map_err(err)?;
    Ok(d.status().into())
}

/// Deletes the logs and crash reports; the opt-in stays.
#[frb(sync)]
pub fn diagnostics_clear() -> Result<DiagnosticsStatus, DiagnosticsError> {
    let d = installed()?;
    d.clear().map_err(err)?;
    Ok(d.status().into())
}

/// An app log line (`level`: `debug`, `info`, `warn`, `error`). Dropped
/// without the opt-in or before [`diagnostics_open`].
#[frb(sync)]
pub fn diagnostics_log(level: String, target: String, message: String) {
    if let Some(d) = pa_log::global() {
        d.log(&level, &target, &message);
    }
}

/// A crash the app saw (`kind`: `dart`, `flutter`, `android-native`, …).
/// True when a report was saved (only with the opt-in).
#[frb(sync)]
pub fn diagnostics_record_crash(kind: String, message: String, stack: String) -> bool {
    pa_log::global()
        .and_then(|d| d.record_crash(&kind, &message, &stack))
        .is_some()
}

/// Writes the diagnostic report zip to `out_path`: logs and crash reports
/// (when kept), `versions`, and `config` without secrets.
pub fn diagnostics_export_report(
    out_path: String,
    versions: Vec<DiagnosticsEntry>,
    config: Vec<DiagnosticsEntry>,
) -> Result<DiagnosticsReport, DiagnosticsError> {
    let d = installed()?;
    let info = ReportInfo {
        versions: rows(versions),
        config: rows(config),
    };
    let s = d.export_report(Path::new(&out_path), &info).map_err(err)?;
    Ok(DiagnosticsReport {
        path: s.path.to_string_lossy().into_owned(),
        files: s.files,
        bytes: s.bytes,
    })
}

/// A "new issue" link to `repo_url` with the bug form prefilled (versions,
/// the report file name, the newest crash). The app opens it in the
/// browser when the user asks; nothing is sent from the core.
#[frb(sync)]
pub fn diagnostics_issue_url(
    repo_url: String,
    versions: Vec<DiagnosticsEntry>,
    report_name: String,
) -> Result<String, DiagnosticsError> {
    let d = installed()?;
    Ok(d.issue_url(&repo_url, &rows(versions), &report_name))
}

/// `text` with user paths and secrets cut, as the logs keep it.
#[frb(sync)]
pub fn diagnostics_redact(text: String) -> String {
    match pa_log::global() {
        Some(d) => d.redact(&text),
        None => Redactor::default().redact(&text).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_without_open() {
        assert_eq!(
            diagnostics_redact("x?api_key=abc".into()),
            "x?api_key=<redacted>"
        );
    }
}
