// SPDX-License-Identifier: MPL-2.0
//! Container probe of new and changed files in the scan pipeline (LIB-006).
//!
//! Every file the scan left unprobed is read through the source's [`Vfs`]
//! (local, SMB, WebDAV alike) and gets `duration_ms`, `container`,
//! `video_json`, `audio_langs`, `sub_langs` and `audio_formats`
//! ([`pa_probe::store`]). Recognition uses the duration, the library
//! queries (LIB-009) the video and audio columns.
//!
//! LIB-023 (large network libraries):
//! - files are probed [`WORKERS`] at a time, each on its own connection of
//!   the source's pool, and written by this thread;
//! - a probe reads only the head and the tail window of `pa-probe`: the
//!   source is opened for random access, so a network backend does not
//!   stream read-ahead blocks of a 60 GB remux behind libavformat;
//! - before recognition only what recognition needs is probed
//!   ([`ProbeScope::First`]): every file except the later episodes of a
//!   series folder, whose first episode stands for them. The rest is probed
//!   after the metadata run ([`Catalog::probe_deferred`]), so titles and
//!   posters of a library of thousands of episodes do not wait for
//!   thousands of probes;
//! - every file is tried **once per run**. A file that fails with an error
//!   that may pass (timeout, dropped connection) stays pending for the next
//!   scan; before LIB-023 such files were picked again by every batch, so a
//!   few unreachable files slowed the probe of the rest to a crawl and 32 of
//!   them stopped it.
//!
//! FFmpeg is loaded at run time; without it the step is skipped and the
//! files stay pending for a later scan.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

use pa_library::rusqlite::Connection;
use pa_probe::store::{mark_unreadable, store};
use pa_probe::{Ffmpeg, MediaInfo, ProbeError, ProbeErrorKind, ProbeOptions};
use pa_scanner::CancelToken;
use pa_vfs::{Vfs, VfsPath};

use crate::scan::{ScanPhase, ScanStep, ScanUpdate};
use crate::{Catalog, Error, sources};

/// Which pending files a probe run takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbeScope {
    /// Before recognition: all but the later episodes of series folders.
    First,
    /// Everything pending.
    All,
}

/// Later episodes of the series folders of `source_id`: probed after the
/// metadata run.
fn deferred(conn: &Connection, source_id: i64) -> Result<HashSet<String>, Error> {
    let mut out = HashSet::new();
    for g in pa_library::groups::source_groups(conn, source_id)? {
        if g.group.kind != pa_library::groups::GroupKind::SeriesFolder {
            continue;
        }
        let mut videos: Vec<&str> = g.group.playable().map(|e| e.path.as_str()).collect();
        videos.sort_unstable();
        out.extend(videos.into_iter().skip(1).map(str::to_owned));
    }
    Ok(out)
}

/// Files probed at once. A network source pools a handful of connections
/// (SMB: 6); four probes keep them busy without starving playback.
pub(crate) const WORKERS: usize = 4;

/// Files written per transaction.
const WRITE_BATCH: usize = 16;

/// What happened to one file.
enum Probed {
    Info(Box<MediaInfo>),
    /// libavformat cannot read it, or the source refuses it for good:
    /// marked probed without data until the file changes.
    Unreadable,
    /// May pass (timeout, connection dropped): left for the next scan.
    Later,
    /// FFmpeg failed as a whole or the run was cancelled.
    Stop(ProbeError),
}

fn probe_one(vfs: &dyn Vfs, ffmpeg: &Ffmpeg, options: &ProbeOptions, path: &str) -> Probed {
    let src = match VfsPath::parse(path).and_then(|p| vfs.open(&p)) {
        Ok(src) => src,
        Err(e) => return source_failure(&e),
    };
    // Head and tail only: no read-ahead behind libavformat.
    src.hint_random_access();
    match ffmpeg.probe(Arc::from(src), options) {
        Ok(info) => Probed::Info(Box::new(info)),
        Err(e) if e.kind() == ProbeErrorKind::Source => {
            if e.is_retryable() {
                Probed::Later
            } else {
                Probed::Unreadable
            }
        }
        Err(e) if e.is_retryable() => Probed::Stop(e),
        Err(_) => Probed::Unreadable,
    }
}

/// A file the source would not open: later if that may pass.
fn source_failure(e: &pa_vfs::VfsError) -> Probed {
    if e.kind().is_transient() {
        Probed::Later
    } else {
        Probed::Unreadable
    }
}

/// Probes the pending files of `source_id`. Returns a message for the user
/// when probing stopped early (FFmpeg failed mid-way) or files are left for
/// the next scan; `None` when done or FFmpeg is not installed.
pub(crate) fn run(
    conn: &Connection,
    source_id: i64,
    vfs: &dyn Vfs,
    scope: ProbeScope,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<Option<String>, Error> {
    let mut files = pa_probe::store::pending_files(conn, source_id, usize::MAX)?;
    if scope == ProbeScope::First && !files.is_empty() {
        let later = deferred(conn, source_id)?;
        files.retain(|(_, path)| !later.contains(path));
    }
    if files.is_empty() {
        return Ok(None);
    }
    let Ok(ffmpeg) = Ffmpeg::shared() else {
        // No FFmpeg here (tests, a build without it): durations stay empty.
        return Ok(None);
    };
    let stop = Arc::new(AtomicBool::new(false));
    let options = ProbeOptions {
        cancel: Some(Arc::clone(&stop)),
        ..ProbeOptions::default()
    };
    let probe = |path: &str| probe_one(vfs, ffmpeg, &options, path);
    run_with(conn, &probe, &stop, &files, cancel, on_progress)
}

fn run_with(
    conn: &Connection,
    probe: &(dyn Fn(&str) -> Probed + Sync),
    stop: &AtomicBool,
    files: &[(i64, String)],
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<Option<String>, Error> {
    let total = files.len();
    let next = AtomicUsize::new(0);
    let (tx, rx) = mpsc::channel::<(usize, Probed)>();
    on_progress(0, total);
    std::thread::scope(|scope| -> Result<Option<String>, Error> {
        for _ in 0..WORKERS.clamp(1, total) {
            let tx = tx.clone();
            let next = &next;
            scope.spawn(move || {
                while !stop.load(Ordering::Relaxed) && !cancel.is_cancelled() {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((_, path)) = files.get(i) else { break };
                    if tx.send((i, probe(path))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        let mut done = 0;
        let mut later = 0;
        let mut message = None;
        let mut pending: Vec<(i64, Probed)> = Vec::with_capacity(WRITE_BATCH);
        let write = |pending: &mut Vec<(i64, Probed)>| -> Result<(), Error> {
            if pending.is_empty() {
                return Ok(());
            }
            let now = crate::now_ms();
            conn.execute_batch("BEGIN")?;
            let result = pending.drain(..).try_for_each(|(id, p)| match p {
                Probed::Info(info) => store(conn, id, &info, now),
                Probed::Unreadable => mark_unreadable(conn, id, now),
                Probed::Later | Probed::Stop(_) => Ok(()),
            });
            match result {
                Ok(()) => conn.execute_batch("COMMIT")?,
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(e.into());
                }
            }
            Ok(())
        };
        let result = (|| -> Result<(), Error> {
            for (i, probed) in &rx {
                if cancel.is_cancelled() {
                    stop.store(true, Ordering::Relaxed);
                }
                match probed {
                    Probed::Stop(e) => {
                        if !stop.swap(true, Ordering::Relaxed) && !cancel.is_cancelled() {
                            message = Some(format!("probe: {e}"));
                        }
                        continue;
                    }
                    Probed::Later => later += 1,
                    _ => {}
                }
                pending.push((files[i].0, probed));
                done += 1;
                if pending.len() >= WRITE_BATCH {
                    write(&mut pending)?;
                }
                on_progress(done, total);
            }
            write(&mut pending)
        })();
        if result.is_err() {
            stop.store(true, Ordering::Relaxed);
        }
        result?;
        if message.is_none() && later > 0 {
            message = Some(format!(
                "probe: {later} files could not be read now, they are tried again on the next scan"
            ));
        }
        Ok(message)
    })
}

/// Number of files of `source_id` waiting for a probe.
fn pending_count(conn: &Connection, source_id: i64) -> Result<i64, Error> {
    Ok(conn.query_row(
        "SELECT count(*) FROM files
         WHERE source_id = ?1 AND probed_at_ms IS NULL AND missing_since_ms IS NULL",
        [source_id],
        |r| r.get(0),
    )?)
}

impl Catalog {
    /// Probes what the scans left for later ([`ProbeScope::First`]): the
    /// later episodes of series folders, and files that could not be read
    /// last time. Run it after the metadata run; serialized with scans.
    /// Updates are [`ScanStep::Probing`]; a source that cannot be reached
    /// is reported and skipped.
    pub fn probe_deferred(
        &self,
        cancel: &CancelToken,
        on_update: &mut dyn FnMut(ScanUpdate),
    ) -> Result<(), Error> {
        let _serial = self.scan_lock.lock().unwrap_or_else(|e| e.into_inner());
        let conn = pa_library::open(&self.path)?;
        for src in sources::list(&conn)? {
            if cancel.is_cancelled() {
                break;
            }
            // Nothing to do: do not wake a network share up.
            if pending_count(&conn, src.id)? == 0 {
                continue;
            }
            let update = |processed: usize, total: usize| {
                let mut u = ScanUpdate::new(ScanPhase::Analyzing, Some(src.id), 0);
                u.step = ScanStep::Probing;
                u.processed = processed as u64;
                u.total = Some(total as u64);
                u
            };
            let vfs = match self.connector().connect(&src) {
                Ok(v) => v,
                Err(e) => {
                    let mut u = update(0, 0);
                    u.error = Some(format!("{}: {e}", src.name));
                    on_update(u);
                    continue;
                }
            };
            let message = run(
                &conn,
                src.id,
                vfs.as_ref(),
                ProbeScope::All,
                cancel,
                &mut |d, t| {
                    on_update(update(d, t));
                },
            )?;
            if let Some(m) = message {
                let mut u = update(0, 0);
                u.error = Some(format!("{}: {m}", src.name));
                on_update(u);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pa_library::rusqlite::params;
    use pa_vfs::{ErrorKind, VfsError};
    use std::sync::Mutex;
    use std::time::Duration;

    fn library(n_slow: usize, n_ok: usize) -> (Connection, Vec<(i64, String)>) {
        let conn = pa_library::open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO sources (id, kind, name, uri, created_at_ms)
             VALUES (1, 'smb', 'NAS', 'smb://nas/media', 0)",
            [],
        )
        .unwrap();
        for i in 0..n_slow + n_ok {
            let name = if i < n_slow {
                format!("/slow/{i:03}.mkv")
            } else {
                format!("/ok/{i:03}.mkv")
            };
            conn.execute(
                "INSERT INTO files (source_id, path, size_bytes, added_at_ms) VALUES (1, ?1, 1000, 0)",
                params![name],
            )
            .unwrap();
        }
        let files = pa_probe::store::pending_files(&conn, 1, usize::MAX).unwrap();
        (conn, files)
    }

    /// LIB-023 regression: files that time out used to be picked again by
    /// every batch of 32 (oldest first), so each batch re-read them before
    /// any new file and 32 of them stopped the probe for good. Now each
    /// file is opened once per run and the rest of the library is probed.
    #[test]
    fn unreachable_files_do_not_hold_up_the_rest() {
        let (conn, files) = library(40, 60);
        let opens = Mutex::new(Vec::new());
        let probe = |path: &str| {
            opens.lock().unwrap().push(path.to_owned());
            std::thread::sleep(Duration::from_millis(2));
            let e = if path.contains("slow") {
                VfsError::new(ErrorKind::TimedOut, "share did not answer")
            } else {
                VfsError::new(ErrorKind::PermissionDenied, "no access")
            };
            source_failure(&e)
        };
        let mut last = (0, 0);
        let message = run_with(
            &conn,
            &probe,
            &AtomicBool::new(false),
            &files,
            &CancelToken::new(),
            &mut |d, t| last = (d, t),
        )
        .unwrap();
        assert_eq!(opens.lock().unwrap().len(), 100, "every file opened once");
        assert_eq!(last, (100, 100), "progress reaches the end");
        // The 60 refused files are settled; the 40 slow ones wait for the
        // next scan.
        let left = pa_probe::store::pending_files(&conn, 1, usize::MAX).unwrap();
        assert_eq!(left.len(), 40);
        assert!(left.iter().all(|(_, p)| p.contains("slow")));
        assert!(message.unwrap().contains("40 files"));
    }

    #[test]
    fn cancel_stops_the_workers() {
        let (conn, files) = library(0, 50);
        let cancel = CancelToken::new();
        let stop = AtomicBool::new(false);
        let probe = |_: &str| {
            cancel.cancel();
            Probed::Unreadable
        };
        run_with(&conn, &probe, &stop, &files, &cancel, &mut |_, _| {}).unwrap();
        let left = pa_probe::store::pending_files(&conn, 1, usize::MAX).unwrap();
        assert!(left.len() > 30, "stopped early, {} left", left.len());
    }
}
