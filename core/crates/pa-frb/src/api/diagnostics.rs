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
    /// Stable code (CORE-009): `io` (logs or the report could not be
    /// written) or `not_initialized` (diagnostics are not open).
    pub code: String,
    /// Detail for logs (redacted).
    pub message: String,
}

fn err(e: impl std::fmt::Display) -> DiagnosticsError {
    let text = e.to_string();
    DiagnosticsError {
        code: "io".into(),
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
        code: "not_initialized".into(),
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
/// (when kept), `versions`, `config` without secrets, and `attachments`
/// (file name → text, e.g. the player's stats report, PLAY-010) in
/// `extra/`.
pub fn diagnostics_export_report(
    out_path: String,
    versions: Vec<DiagnosticsEntry>,
    config: Vec<DiagnosticsEntry>,
    attachments: Vec<DiagnosticsEntry>,
) -> Result<DiagnosticsReport, DiagnosticsError> {
    let d = installed()?;
    let info = ReportInfo {
        versions: rows(versions),
        config: rows(config),
        attachments: rows(attachments),
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

/// One crash report kept on the device, for the «Состояние» screen
/// (UI-017).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsCrash {
    /// File name; the id for [`diagnostics_crash_text`].
    pub name: String,
    /// `rust-panic`, `dart`, `flutter`, `android-crash`, …
    pub kind: String,
    /// ISO time (UTC).
    pub time: String,
    /// First line of the message (redacted).
    pub headline: String,
}

/// The last `max_lines` lines of the log, oldest first, redacted again on
/// the way out (UI-017). Empty without the opt-in or before
/// [`diagnostics_open`].
pub fn diagnostics_recent_log(max_lines: u32) -> Vec<String> {
    pa_log::global()
        .map(|d| d.recent_log(max_lines as usize))
        .unwrap_or_default()
}

/// Crash reports kept on the device, newest first (UI-017).
#[frb(sync)]
pub fn diagnostics_crashes() -> Vec<DiagnosticsCrash> {
    pa_log::global()
        .map(|d| {
            d.crashes()
                .into_iter()
                .map(|c| DiagnosticsCrash {
                    name: c.name,
                    kind: c.kind,
                    time: c.time,
                    headline: c.headline,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The text of one crash report (redacted): what the user copies or
/// attaches by hand. Error `not_found` for a name not in
/// [`diagnostics_crashes`].
#[frb(sync)]
pub fn diagnostics_crash_text(name: String) -> Result<String, DiagnosticsError> {
    installed()?
        .crash_text(&name)
        .ok_or_else(|| DiagnosticsError {
            code: "not_found".into(),
            message: "no such crash report".into(),
        })
}

/// Versions of the engines inside the core (UI-017): the bundled SQLite
/// and the target the core was built for. libmpv and FFmpeg are read by
/// the player.
#[frb(sync)]
pub fn diagnostics_engine_versions() -> Vec<DiagnosticsEntry> {
    vec![
        DiagnosticsEntry {
            key: "sqlite".into(),
            value: pa_library::rusqlite::version().to_string(),
        },
        DiagnosticsEntry {
            key: "core_target".into(),
            value: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_versions_name_sqlite() {
        let v = diagnostics_engine_versions();
        let sqlite = v.iter().find(|e| e.key == "sqlite").unwrap();
        assert!(sqlite.value.starts_with('3'), "{sqlite:?}");
    }

    #[test]
    fn redacts_without_open() {
        assert_eq!(
            diagnostics_redact("x?api_key=abc".into()),
            "x?api_key=<redacted>"
        );
    }
}
