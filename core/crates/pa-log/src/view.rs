// SPDX-License-Identifier: MPL-2.0
//! What the app's «Состояние» screen shows (UI-017, ADR-0125): the tail of
//! the log and the list of crash reports. Every line is redacted again on
//! the way out, so a rule added after a line was written still applies.

use std::fs;

use crate::Diagnostics;

/// Longest line handed to the screen; the rest is cut (`…`).
const MAX_LINE: usize = 2000;

/// One crash report as the screen lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashSummary {
    /// File name (`crash-<time>-<ms>-<kind>.txt`), the id for
    /// [`Diagnostics::crash_text`].
    pub name: String,
    /// `rust-panic`, `dart`, `flutter`, `android-crash`, `native-signal`…
    pub kind: String,
    /// ISO time the report was written (UTC), as in the file.
    pub time: String,
    /// First line of the message.
    pub headline: String,
}

fn cut(line: &str) -> String {
    if line.len() <= MAX_LINE {
        return line.to_string();
    }
    let mut end = MAX_LINE;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .map(str::trim)
}

fn summary(name: String, text: &str) -> CrashSummary {
    let headline = text
        .split_once("\nmessage:\n")
        .and_then(|(_, rest)| rest.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or_default();
    CrashSummary {
        kind: field(text, "kind").unwrap_or_default().to_string(),
        time: field(text, "time").unwrap_or_default().to_string(),
        headline: cut(headline.trim()),
        name,
    }
}

impl Diagnostics {
    /// The last `max_lines` lines of the log, oldest first, redacted. Empty
    /// without the opt-in or before anything was written.
    pub fn recent_log(&self, max_lines: usize) -> Vec<String> {
        if max_lines == 0 {
            return Vec::new();
        }
        // Newest file first; its last lines are the newest.
        let mut newest_first: Vec<String> = Vec::new();
        for path in self.log_files() {
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines().rev() {
                if line.trim().is_empty() {
                    continue;
                }
                newest_first.push(cut(&self.redact(line)));
                if newest_first.len() == max_lines {
                    break;
                }
            }
            if newest_first.len() == max_lines {
                break;
            }
        }
        newest_first.reverse();
        newest_first
    }

    /// Crash reports kept on the device, newest first.
    pub fn crashes(&self) -> Vec<CrashSummary> {
        let mut v: Vec<CrashSummary> = self
            .crash_reports()
            .into_iter()
            .filter_map(|p| {
                let name = p.file_name()?.to_str()?.to_string();
                let text = fs::read_to_string(&p).ok()?;
                Some(summary(name, &self.redact(&text)))
            })
            .collect();
        v.reverse();
        v
    }

    /// The text of one crash report, redacted; `None` for a name that is
    /// not one of [`crashes`](Self::crashes) (no path from outside reaches
    /// the file system).
    pub fn crash_text(&self, name: &str) -> Option<String> {
        let path = self
            .crash_reports()
            .into_iter()
            .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(name))?;
        let text = fs::read_to_string(path).ok()?;
        Some(self.redact(&text))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use crate::{Diagnostics, Redactor, Rotation};

    fn diag(dir: &Path, rotation: Rotation) -> Arc<Diagnostics> {
        Arc::new(Diagnostics::open(
            dir.join("diagnostics"),
            Redactor::with_homes(["/home/alice"]),
            rotation,
        ))
    }

    #[test]
    fn nothing_without_the_opt_in() {
        let tmp = tempfile::tempdir().unwrap();
        let d = diag(tmp.path(), Rotation::default());
        d.log("info", "app", "dropped");
        assert!(d.recent_log(10).is_empty());
        assert!(d.crashes().is_empty());
        assert!(d.crash_text("crash-x.txt").is_none());
    }

    #[test]
    fn tail_spans_rotated_files_oldest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let d = diag(
            tmp.path(),
            Rotation {
                max_bytes: 120,
                max_files: 4,
            },
        );
        d.set_enabled(true).unwrap();
        for i in 0..10 {
            d.log("info", "app", &format!("line {i}"));
        }
        assert!(d.log_files().len() > 1, "rotated");
        let tail = d.recent_log(4);
        assert_eq!(tail.len(), 4);
        for (got, want) in tail.iter().zip(["line 6", "line 7", "line 8", "line 9"]) {
            assert!(got.ends_with(want), "{got} / {want}");
        }
        assert!(d.recent_log(0).is_empty());
    }

    #[test]
    fn tail_is_redacted_again() {
        let tmp = tempfile::tempdir().unwrap();
        let d = diag(tmp.path(), Rotation::default());
        d.set_enabled(true).unwrap();
        // A line that reached the file without redaction (an older build, a
        // rule added later) is cut on the way to the screen.
        let logs = tmp.path().join("diagnostics/logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(
            logs.join("kinolune.log"),
            "t INFO net: GET smb://bob:hunter2@nas/share?api_key=k3y \
             Authorization: Bearer abcdef123456 /home/alice/films\n",
        )
        .unwrap();
        let line = d.recent_log(5).join("\n");
        for secret in ["bob", "hunter2", "k3y", "abcdef123456", "alice"] {
            assert!(!line.contains(secret), "{secret} in {line}");
        }
        assert!(line.contains("~/films"), "{line}");
    }

    #[test]
    fn long_lines_are_cut() {
        let tmp = tempfile::tempdir().unwrap();
        let d = diag(tmp.path(), Rotation::default());
        d.set_enabled(true).unwrap();
        d.log("info", "app", &"я".repeat(3000));
        let line = &d.recent_log(1)[0];
        assert!(line.len() <= super::MAX_LINE + '…'.len_utf8(), "{}", line.len());
        assert!(line.ends_with('…'));
    }

    #[test]
    fn crashes_newest_first_with_text() {
        let tmp = tempfile::tempdir().unwrap();
        let d = diag(tmp.path(), Rotation::default());
        d.set_enabled(true).unwrap();
        d.record_crash("dart", "first\nmore", "#0 main")
            .expect("saved");
        std::thread::sleep(std::time::Duration::from_millis(5));
        d.record_crash(
            "rust-panic",
            "token=s3cr3tvalue at /home/alice/x",
            "#0 core",
        )
        .expect("saved");
        let list = d.crashes();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].kind, "rust-panic");
        assert_eq!(list[1].kind, "dart");
        assert_eq!(list[1].headline, "first");
        assert!(!list[0].headline.contains("s3cr3tvalue"), "{:?}", list[0]);
        assert!(list[0].time.contains('T'), "{:?}", list[0]);
        let text = d.crash_text(&list[0].name).unwrap();
        assert!(text.contains("#0 core") && !text.contains("alice"), "{text}");
        assert!(d.crash_text("../opt-in").is_none());
        assert!(d.crash_text("crash-none.txt").is_none());
    }
}
