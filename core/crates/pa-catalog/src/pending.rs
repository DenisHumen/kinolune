// SPDX-License-Identifier: MPL-2.0
//! Recognition work the library still owes (LIB-024, ADR-NNNN).
//!
//! Every step of recognition keeps its state in the catalog, so nothing is
//! lost when a run stops — cancelled, killed with the app, or cut off by
//! the network. What is left is found again here:
//!
//! - **Unrecognised files**: videos without a live match whose name the
//!   current parser has not read yet (the scan stopped between listing and
//!   recognition). Only a scan of their source finishes them (fingerprints
//!   and moves come first), so their sources are scanned again
//!   ([`Catalog::sources_to_finish`]).
//! - **Unfinished scans**: a source whose last scan started and never
//!   ended (`sources.last_scan_finished_at_ms` is null): the app was killed
//!   while it ran.
//! - **Works waiting for metadata**: what the next metadata run takes —
//!   works without a `work_metadata` row, with `anime_pending`, in another
//!   language, stale, or failed long enough ago (the same query as the run).
//! - **Failed works** not due yet (network errors, retried after an hour or
//!   at once when the network comes back).
//! - **Deferred probes**: files whose headers are not read yet
//!   (`probed_at_ms` is null; later episodes, files the share refused).
//!
//! [`Catalog::pending_work`] counts all of it for the app's status line and
//! for deciding whether a catch-up run is needed; it reads the database
//! only.

use pa_library::rusqlite::{Connection, params};
use pa_recognition::parse::PARSER_VERSION;
use pa_scanner::MediaKind;

use crate::metadata::{MetadataOptions, pending};
use crate::{Catalog, Error};

/// What recognition still has to do; see the [module docs](self).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingWork {
    /// Videos the parser has not read yet (a scan stopped before
    /// recognition).
    pub unrecognized_files: u64,
    /// Works the next metadata run takes.
    pub works_waiting: u64,
    /// Of them: anime whose AniList / Shikimori stage has not run.
    pub anime_pending: u64,
    /// Works whose last fetch failed and that are not due yet.
    pub works_failed: u64,
    /// Files whose headers are not read yet.
    pub deferred_probes: u64,
    /// Sources whose last scan never finished.
    pub unfinished_scans: Vec<i64>,
}

impl PendingWork {
    /// Recognition still has something to do that a run can do now
    /// (failed works wait for their retry time).
    #[must_use]
    pub fn has_work(&self) -> bool {
        self.unrecognized_files > 0
            || self.works_waiting > 0
            || self.deferred_probes > 0
            || !self.unfinished_scans.is_empty()
    }
}

fn count(conn: &Connection, sql: &str) -> Result<u64, Error> {
    let n: i64 = conn.query_row(sql, [], |r| r.get(0))?;
    Ok(u64::try_from(n).unwrap_or(0))
}

/// `(source, path)` of the videos without a live match that the current
/// parser has not read.
fn unrecognized(conn: &Connection) -> Result<Vec<(i64, String)>, Error> {
    let mut stmt = conn.prepare(
        "SELECT f.source_id, f.path FROM files f
         WHERE f.missing_since_ms IS NULL
           AND NOT EXISTS (SELECT 1 FROM matches m WHERE m.file_id = f.id AND m.state <> 'rejected')
           AND NOT EXISTS (SELECT 1 FROM group_assets ga
                           WHERE ga.file_id = f.id AND ga.role = 'ignore')
           AND NOT EXISTS (SELECT 1 FROM parse_results pr
                           WHERE pr.file_id = f.id AND pr.parser_version = ?1)",
    )?;
    let rows = stmt.query_map(params![PARSER_VERSION], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (source, path): (i64, String) = row?;
        if pa_scanner::media_kind(&path) == Some(MediaKind::Video) {
            out.push((source, path));
        }
    }
    Ok(out)
}

/// Sources whose last scan started and never ended.
fn unfinished(conn: &Connection) -> Result<Vec<i64>, Error> {
    let mut stmt = conn.prepare(
        "SELECT id FROM sources
         WHERE last_scan_started_at_ms IS NOT NULL AND last_scan_finished_at_ms IS NULL
         ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

impl Catalog {
    /// What recognition still owes ([module docs](crate::pending)), for
    /// metadata in `language`. Without TMDB (`tmdb` false: no key) only
    /// anime works are counted as waiting, as the run asks only for them.
    pub fn pending_work(&self, language: &str, tmdb: bool) -> Result<PendingWork, Error> {
        let conn = self.lock();
        let options = MetadataOptions::default();
        let now = crate::now_ms();
        let ids = pending(&conn, language, &options, now)?;
        let (mut works_waiting, mut anime_pending) = (0u64, 0u64);
        {
            let mut kind = conn.prepare(
                "SELECT w.kind, ifnull(wm.anime_pending, 0) FROM works w
                 LEFT JOIN work_metadata wm ON wm.work_id = w.id WHERE w.id = ?1",
            )?;
            for id in &ids {
                let (k, anime): (String, bool) =
                    kind.query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))?;
                if tmdb || k == "anime" {
                    works_waiting += 1;
                    anime_pending += u64::from(anime);
                }
            }
        }
        let works_failed: i64 = conn.query_row(
            "SELECT count(*) FROM work_metadata WHERE outcome = 'failed' AND fetched_at_ms >= ?1",
            [now - options.retry_failed_after_ms],
            |r| r.get(0),
        )?;
        Ok(PendingWork {
            unrecognized_files: unrecognized(&conn)?.len() as u64,
            works_waiting,
            anime_pending,
            works_failed: u64::try_from(works_failed).unwrap_or(0),
            deferred_probes: count(
                &conn,
                "SELECT count(*) FROM files
                 WHERE probed_at_ms IS NULL AND missing_since_ms IS NULL",
            )?,
            unfinished_scans: unfinished(&conn)?,
        })
    }

    /// Sources a scan has to finish: the last scan never ended, or videos
    /// wait for recognition. The watcher scans them on start and on a
    /// check even when nothing changed (LIB-024).
    pub fn sources_to_finish(&self) -> Result<Vec<i64>, Error> {
        let conn = self.lock();
        let mut ids = unfinished(&conn)?;
        ids.extend(unrecognized(&conn)?.into_iter().map(|(s, _)| s));
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }
}
