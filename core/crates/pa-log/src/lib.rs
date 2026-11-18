// SPDX-License-Identifier: MPL-2.0
//! Local diagnostics of the core (CORE-008, ADR-NNNN): opt-in logs with
//! rotation, crash capture and the diagnostic report.
//!
//! - **Opt-in.** Nothing is written until the user turns diagnostics on
//!   ([`Diagnostics::set_enabled`]); the choice is the marker file
//!   `<dir>/opt-in`. Turning it off deletes the logs and crash reports.
//! - **Redaction.** Every line is passed through [`Redactor`] before it is
//!   written, and again when the report is built: home directories, URL
//!   tokens, the pa-proxy token, OAuth/Twitch tokens and passwords never
//!   reach the disk.
//! - **Logs.** `tracing` events of the core (a global subscriber, see
//!   [`install`]) and lines from the app ([`Diagnostics::log`]) go to
//!   `<dir>/logs/kinolune.log`, rotated by size ([`Rotation`]).
//! - **Crashes.** Rust panics (a panic hook), errors the app reports
//!   ([`Diagnostics::record_crash`]: Dart, Android exit reasons) and, on
//!   Linux and macOS, fatal signals of native code (a marker written by the
//!   signal handler, turned into a report on the next start) become text
//!   files in `<dir>/crashes`.
//! - **Report.** [`Diagnostics::export_report`] writes one zip with the
//!   logs, crash reports, versions and the configuration without secrets.
//!   Sending it is manual: [`Diagnostics::issue_url`] makes a prefilled
//!   GitHub issue link, the user attaches the file. There is no upload and
//!   no server (ADR-0004).
//! - **Status screen.** [`Diagnostics::recent_log`] and
//!   [`Diagnostics::crashes`] feed the app's «Состояние» screen (UI-017,
//!   ADR-0125), redacted again on the way out.

mod crash;
pub mod redact;
mod report;
mod rotate;
mod time;
mod view;
mod zip;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use tracing_subscriber::fmt::MakeWriter;

pub use redact::Redactor;
pub use report::{ReportInfo, ReportSummary};
pub use rotate::Rotation;
pub use view::CrashSummary;
pub use zip::read_stored;

use rotate::RotatingFile;

/// File whose presence records the opt-in.
const OPT_IN: &str = "opt-in";
const LOGS: &str = "logs";
const CRASHES: &str = "crashes";
/// Crash reports kept; older ones are deleted.
const MAX_CRASHES: usize = 20;

/// What the settings screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Status {
    pub enabled: bool,
    pub log_files: u32,
    pub log_bytes: u64,
    pub crash_reports: u32,
}

/// The diagnostics of one app data directory.
#[derive(Debug)]
pub struct Diagnostics {
    dir: PathBuf,
    rotation: Rotation,
    redactor: RwLock<Redactor>,
    enabled: AtomicBool,
    log: Mutex<Option<RotatingFile>>,
}

impl Diagnostics {
    /// Diagnostics kept in `dir` (e.g. `<app data>/diagnostics`). Creates
    /// nothing: the directory appears when the user opts in. If the user
    /// opted in before, a native crash of the previous run becomes a crash
    /// report now.
    pub fn open(dir: impl Into<PathBuf>, redactor: Redactor, rotation: Rotation) -> Self {
        let dir = dir.into();
        let enabled = dir.join(OPT_IN).is_file();
        let d = Self {
            dir,
            rotation,
            redactor: RwLock::new(redactor),
            enabled: AtomicBool::new(enabled),
            log: Mutex::new(None),
        };
        if enabled {
            crash::collect_native(&d);
        }
        d
    }

    /// The directory of the logs and crash reports.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn logs_dir(&self) -> PathBuf {
        self.dir.join(LOGS)
    }

    pub(crate) fn crashes_dir(&self) -> PathBuf {
        self.dir.join(CRASHES)
    }

    /// Whether the user opted in.
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Opts in or out. Opting out deletes the logs and crash reports.
    pub fn set_enabled(&self, on: bool) -> io::Result<()> {
        if on {
            fs::create_dir_all(&self.dir)?;
            fs::write(
                self.dir.join(OPT_IN),
                "Diagnostics are on: logs and crash reports stay on this device \
                 until you export them.\n",
            )?;
            self.enabled.store(true, Ordering::Release);
            crash::arm_native(&self.crashes_dir());
        } else {
            self.enabled.store(false, Ordering::Release);
            crash::disarm_native();
            *self.log.lock().unwrap_or_else(|e| e.into_inner()) = None;
            self.clear()?;
            remove_file_if_exists(&self.dir.join(OPT_IN))?;
            // Empty now: leave no trace.
            let _ = fs::remove_dir(&self.dir);
        }
        Ok(())
    }

    /// Deletes the logs and crash reports; the opt-in stays.
    pub fn clear(&self) -> io::Result<()> {
        *self.log.lock().unwrap_or_else(|e| e.into_inner()) = None;
        for d in [self.logs_dir(), self.crashes_dir()] {
            match fs::remove_dir_all(&d) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }

    /// Cuts `secret` from everything written from now on (and from the
    /// report).
    pub fn add_secret(&self, secret: impl Into<String>) {
        self.redactor
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .add_secret(secret);
    }

    /// `text` as it would be written.
    pub fn redact(&self, text: &str) -> String {
        self.redactor
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .redact(text)
            .into_owned()
    }

    /// A line from the app: `<time> <LEVEL> <target>: <message>`. Dropped
    /// without the opt-in.
    pub fn log(&self, level: &str, target: &str, message: &str) {
        if !self.enabled() {
            return;
        }
        let line = format!(
            "{} {:>5} {}: {}",
            time::iso(time::now_secs()),
            level.to_ascii_uppercase(),
            target,
            message
        );
        self.write_line(&line, true);
    }

    /// Redacts and appends one line (several for a multi-line event).
    /// `block = false` (the panic hook) gives up instead of waiting for the
    /// file lock.
    fn write_line(&self, line: &str, block: bool) {
        if !self.enabled() {
            return;
        }
        let clean = self.redact(line);
        let mut guard = if block {
            self.log.lock().unwrap_or_else(|e| e.into_inner())
        } else {
            match self.log.try_lock() {
                Ok(g) => g,
                Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => return,
            }
        };
        if guard.is_none() {
            match RotatingFile::open(&self.logs_dir(), self.rotation) {
                Ok(f) => *guard = Some(f),
                Err(_) => return,
            }
        }
        if let Some(f) = guard.as_mut() {
            // A full disk must not take the app down with it.
            let _ = f.write_line(&clean);
        }
    }

    /// Saves a crash report (`kind`: `rust-panic`, `dart`, `flutter`,
    /// `android-native`, …). Nothing without the opt-in; returns the file.
    pub fn record_crash(&self, kind: &str, message: &str, stack: &str) -> Option<PathBuf> {
        if !self.enabled() {
            return None;
        }
        let now = time::now_secs();
        let kind_slug: String = kind
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .take(32)
            .collect();
        let text = format!(
            "Kinolune crash report\nkind: {kind}\ntime: {}\nos: {} {}\n\nmessage:\n{message}\n\nstack:\n{stack}\n",
            time::iso(now),
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
        let clean = self.redact(&text);
        let dir = self.crashes_dir();
        fs::create_dir_all(&dir).ok()?;
        let path = dir.join(format!(
            "crash-{}-{:03}-{kind_slug}.txt",
            time::compact(now),
            time::now_millis() % 1000
        ));
        fs::write(&path, clean).ok()?;
        let first = message.lines().next().unwrap_or_default();
        self.write_line(
            &format!(
                "{} ERROR crash: {kind}: {first}",
                time::iso(time::now_secs())
            ),
            false,
        );
        self.prune_crashes();
        Some(path)
    }

    fn prune_crashes(&self) {
        let mut files = self.crash_reports();
        if files.len() > MAX_CRASHES {
            files.sort();
            for f in &files[..files.len() - MAX_CRASHES] {
                let _ = fs::remove_file(f);
            }
        }
    }

    /// Crash report files, oldest first.
    pub fn crash_reports(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(self.crashes_dir())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|e| e == "txt")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("crash-"))
            })
            .collect();
        v.sort();
        v
    }

    /// Log files, newest first.
    pub fn log_files(&self) -> Vec<PathBuf> {
        if let Some(f) = self.log.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = f.flush();
        }
        rotate::log_files(&self.logs_dir(), self.rotation)
    }

    /// Opt-in and what is kept.
    pub fn status(&self) -> Status {
        let logs = self.log_files();
        Status {
            enabled: self.enabled(),
            log_files: logs.len() as u32,
            log_bytes: logs
                .iter()
                .filter_map(|p| fs::metadata(p).ok())
                .map(|m| m.len())
                .sum(),
            crash_reports: self.crash_reports().len() as u32,
        }
    }

    /// Writes the diagnostic report zip to `out` (atomically). Works
    /// without the opt-in too: then it holds versions and configuration
    /// only.
    pub fn export_report(&self, out: &Path, info: &ReportInfo) -> io::Result<ReportSummary> {
        report::build(self, out, info)
    }

    /// A GitHub "new issue" link with the bug form prefilled: platform and
    /// versions, the report file name and the headline of the newest crash.
    /// Opening it is up to the user; nothing is sent from here.
    pub fn issue_url(
        &self,
        repo_url: &str,
        versions: &[(String, String)],
        report_name: &str,
    ) -> String {
        report::issue_url(self, repo_url, versions, report_name)
    }

    /// A `tracing` subscriber that writes into these diagnostics (INFO and
    /// above). [`install`] makes it the global one.
    pub fn subscriber(self: &Arc<Self>) -> impl tracing::Subscriber + Send + Sync + 'static {
        tracing_subscriber::fmt()
            .with_writer(EventWriter(Arc::clone(self)))
            .with_max_level(tracing::Level::INFO)
            .with_target(true)
            .finish()
    }
}

fn remove_file_if_exists(p: &Path) -> io::Result<()> {
    match fs::remove_file(p) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// `MakeWriter` of the tracing subscriber: one buffer per event, written
/// as a line when the event is done.
struct EventWriter(Arc<Diagnostics>);

impl<'a> MakeWriter<'a> for EventWriter {
    type Writer = EventBuf;

    fn make_writer(&'a self) -> EventBuf {
        EventBuf {
            diag: self.0.enabled().then(|| Arc::clone(&self.0)),
            buf: Vec::new(),
        }
    }
}

struct EventBuf {
    diag: Option<Arc<Diagnostics>>,
    buf: Vec<u8>,
}

impl Write for EventBuf {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if self.diag.is_some() {
            self.buf.extend_from_slice(b);
        }
        Ok(b.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for EventBuf {
    fn drop(&mut self) {
        if let Some(d) = &self.diag {
            let text = String::from_utf8_lossy(&self.buf);
            let text = text.trim_end();
            if !text.is_empty() {
                d.write_line(text, true);
            }
        }
    }
}

static GLOBAL: OnceLock<Arc<Diagnostics>> = OnceLock::new();

/// Makes `diag` the diagnostics of the process: the global `tracing`
/// subscriber (unless one is set already), the panic hook and, on Linux
/// and macOS, the fatal-signal handlers. Only the first call installs; later
/// calls return the diagnostics already installed.
pub fn install(diag: Diagnostics) -> Arc<Diagnostics> {
    let mut fresh = false;
    let d = GLOBAL.get_or_init(|| {
        fresh = true;
        Arc::new(diag)
    });
    if fresh {
        let _ = tracing::subscriber::set_global_default(d.subscriber());
        crash::install_panic_hook();
        crash::install_native();
        if d.enabled() {
            crash::arm_native(&d.crashes_dir());
        }
    }
    Arc::clone(d)
}

/// The installed diagnostics, if any.
pub fn global() -> Option<Arc<Diagnostics>> {
    GLOBAL.get().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diag(dir: &Path) -> Arc<Diagnostics> {
        Arc::new(Diagnostics::open(
            dir.join("diagnostics"),
            Redactor::with_homes(["/home/alice"]),
            Rotation::default(),
        ))
    }

    #[test]
    fn opt_in_survives_reopen_and_opt_out_deletes() {
        let tmp = tempfile::tempdir().unwrap();
        let d = diag(tmp.path());
        assert!(!d.enabled());
        d.set_enabled(true).unwrap();
        d.log("info", "app", "hello from /home/alice/x");
        assert!(d.record_crash("dart", "boom", "#0 main").is_some());
        let again = diag(tmp.path());
        assert!(again.enabled());
        let s = again.status();
        assert_eq!((s.log_files, s.crash_reports), (1, 1));
        let log = fs::read_to_string(&again.log_files()[0]).unwrap();
        assert!(log.contains("hello from ~/x"), "{log}");
        again.set_enabled(false).unwrap();
        assert!(!tmp.path().join("diagnostics").exists());
    }

    #[test]
    fn tracing_events_reach_the_log() {
        let tmp = tempfile::tempdir().unwrap();
        let d = diag(tmp.path());
        d.set_enabled(true).unwrap();
        tracing::subscriber::with_default(d.subscriber(), || {
            tracing::info!(target: "pa_test", url = "https://x/?access_token=abc123", "fetched");
            tracing::debug!("below the level");
        });
        let log = fs::read_to_string(&d.log_files()[0]).unwrap();
        assert!(log.contains("pa_test") && log.contains("fetched"), "{log}");
        assert!(
            !log.contains("abc123") && !log.contains("below the level"),
            "{log}"
        );
    }
}
