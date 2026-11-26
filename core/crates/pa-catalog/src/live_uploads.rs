// SPDX-License-Identifier: MPL-2.0
//! Library uploads visible on every device (LIB-028, ADR-NNNN).
//!
//! The durable part of the grey card: the last step of every upload
//! (started, recognized, done, cancelled) in `live_uploads`, synced as an
//! `upload.event` op (last writer wins by HLC). Progress itself never
//! comes here: it moves on the ephemeral `up` frames of `pa_sync::upload`.
//!
//! [`guess_upload`] is the recognition before copying: the same planner
//! the scan runs on the file once it is on the source
//! ([`crate::recognize::plan_files`]), on the path the file is going to,
//! so the card shows what the library will show.

use pa_library::rusqlite::{OptionalExtension, params};
use pa_sync::{Hlc, OpBody, OpId, Stamp, UploadPhase};
use serde::{Deserialize, Serialize};

use crate::{Catalog, Error, now_ms};

/// A step of an upload as the library keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveUpload {
    pub upload_id: String,
    pub phase: UploadPhase,
    /// The apps' JSON (file name, size, kind, guess, source, folder,
    /// device name).
    pub info: String,
    /// The device that uploads.
    pub device_id: String,
    /// Local time of the last change.
    pub updated_ms: i64,
}

/// A finished or cancelled upload is listed this long (the card turns
/// into the item or goes away), then forgotten.
pub const FINAL_KEEP_MS: i64 = 24 * 3600 * 1000;

/// An upload with no step for this long is abandoned (its device is gone
/// for good) and forgotten.
pub const ABANDONED_MS: i64 = 30 * 24 * 3600 * 1000;

/// What the library will make of a file before it is copied.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UploadGuess {
    /// `movie`, `show`, `anime`, or empty when the name says nothing.
    pub kind: String,
    pub title: String,
    pub year: Option<i64>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    /// `0..=100`: how sure the parser is.
    pub confidence: u8,
}

/// Recognition before copying (LIB-028): `dest` is the file's path
/// relative to the source root, where it is going (`Movies/Dune Part Two
/// 2024 2160p.mkv`, `TV Shows/Severance/Severance.S02E03.mkv`); a disc
/// folder is guessed by its folder name (`Movies/Inception (2010)/`).
/// No network: the provider search for a poster is the app's next step.
pub fn guess_upload(dest: &str) -> UploadGuess {
    let path = match dest.strip_suffix('/') {
        Some(folder) => format!("{folder}.mkv"),
        None => dest.to_owned(),
    };
    let Some(Some(plan)) = crate::recognize::plan_files(&[path.as_str()], &[]).pop() else {
        return UploadGuess::default();
    };
    let first = plan.episodes.first();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let confidence = (plan.confidence.clamp(0.0, 1.0) * 100.0).round() as u8;
    UploadGuess {
        kind: plan.work.kind.to_owned(),
        title: plan.work.title,
        year: plan.work.year,
        season: first.and_then(|e| e.season),
        episode: first.and_then(|e| e.episode.or(e.absolute)),
        confidence,
    }
}

#[derive(Serialize, Deserialize)]
struct StampJson {
    hlc: Hlc,
    op_id: OpId,
}

fn stamp_json(stamp: &Stamp) -> String {
    serde_json::to_string(&StampJson {
        hlc: stamp.0.clone(),
        op_id: stamp.1,
    })
    .unwrap_or_default()
}

fn parse_stamp(text: Option<String>) -> Option<Stamp> {
    let s: StampJson = serde_json::from_str(&text?).ok()?;
    Some((s.hlc, s.op_id))
}

fn phase_text(phase: UploadPhase) -> String {
    serde_json::to_value(phase)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "other".to_owned())
}

fn phase_of(text: &str) -> UploadPhase {
    serde_json::from_value(serde_json::Value::String(text.to_owned())).unwrap_or(UploadPhase::Other)
}

impl Catalog {
    /// Records a step of an upload of this device (LIB-028): written here
    /// and, while sync is on, queued as an `upload.event` op for the
    /// family. `info` is a JSON object without credentials.
    pub fn record_upload_event(
        &self,
        upload_id: &str,
        phase: UploadPhase,
        info: &str,
    ) -> Result<(), Error> {
        let body = OpBody::UploadEvent {
            upload: upload_id.to_owned(),
            phase,
            info: info.to_owned(),
        };
        body.validate()
            .map_err(|e| Error::Invalid(format!("upload event: {e}")))?;
        let mut conn = self.lock();
        let mut sync = self.sync_lock();
        let now = now_ms();
        let tx = conn.transaction()?;
        let (device, stamp) = match sync.as_mut() {
            Some(replica) => {
                let op = replica.record(&tx, body, now)?;
                (op.hlc.device.clone(), Some(stamp_json(&(op.hlc, op.op_id))))
            }
            None => ("local".to_owned(), None),
        };
        tx.execute(
            "INSERT INTO live_uploads (upload_id, phase, info, device_id, stamp, updated_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (upload_id) DO UPDATE SET phase = excluded.phase,
                 info = excluded.info, device_id = excluded.device_id,
                 stamp = excluded.stamp, updated_ms = excluded.updated_ms",
            params![upload_id, phase_text(phase), info, device, stamp, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Writes an `upload.event` op of another device: the newer step wins.
    pub(crate) fn apply_remote_upload(&self, op: &pa_sync::Op) -> Result<bool, Error> {
        let OpBody::UploadEvent {
            upload,
            phase,
            info,
        } = &op.body
        else {
            return Ok(false);
        };
        let conn = self.lock();
        let mut sync = self.sync_lock();
        let Some(replica) = sync.as_mut() else {
            return Ok(false);
        };
        let now = now_ms();
        replica.receive(op, now)?;
        drop(sync);
        let stamp: Stamp = (op.hlc.clone(), op.op_id);
        let known: Option<Option<String>> = conn
            .query_row(
                "SELECT stamp FROM live_uploads WHERE upload_id = ?1",
                [upload],
                |r| r.get(0),
            )
            .optional()?;
        if known.and_then(parse_stamp).is_some_and(|k| k >= stamp) {
            return Ok(false);
        }
        conn.execute(
            "INSERT INTO live_uploads (upload_id, phase, info, device_id, stamp, updated_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (upload_id) DO UPDATE SET phase = excluded.phase,
                 info = excluded.info, device_id = excluded.device_id,
                 stamp = excluded.stamp, updated_ms = excluded.updated_ms",
            params![
                upload,
                phase_text(*phase),
                info,
                op.hlc.device,
                stamp_json(&stamp),
                now
            ],
        )?;
        Ok(true)
    }

    /// The uploads a card is shown for: every one still on its way, and
    /// those that ended in the last [`FINAL_KEEP_MS`] (the app hides a
    /// done one once its item is in the library). Old rows are cleaned up
    /// on the way.
    pub fn live_uploads(&self) -> Result<Vec<LiveUpload>, Error> {
        let conn = self.lock();
        let now = now_ms();
        conn.execute(
            "DELETE FROM live_uploads
             WHERE (phase IN ('done', 'cancelled') AND updated_ms < ?1) OR updated_ms < ?2",
            params![now - FINAL_KEEP_MS, now - ABANDONED_MS],
        )?;
        let mut stmt = conn.prepare(
            "SELECT upload_id, phase, info, device_id, updated_ms FROM live_uploads
             ORDER BY updated_ms DESC, upload_id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(LiveUpload {
                    upload_id: r.get(0)?,
                    phase: phase_of(&r.get::<_, String>(1)?),
                    info: r.get(2)?,
                    device_id: r.get(3)?,
                    updated_ms: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter(|u| u.phase != UploadPhase::Other)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guess_is_what_the_scan_will_plan() {
        let g = guess_upload("Movies/Dune.Part.Two.2024.2160p.WEB-DL.mkv");
        assert_eq!(
            (g.kind.as_str(), g.title.as_str()),
            ("movie", "Dune Part Two")
        );
        assert_eq!(g.year, Some(2024));
        let g = guess_upload("TV Shows/Severance/Severance.S02E03.1080p.mkv");
        assert_eq!(g.kind, "show");
        assert_eq!((g.season, g.episode), (Some(2), Some(3)));
        let g = guess_upload("Movies/Inception (2010)/");
        assert_eq!((g.title.as_str(), g.year), ("Inception", Some(2010)));
    }

    #[test]
    fn phases_round_trip_as_text() {
        for p in [
            UploadPhase::Started,
            UploadPhase::Recognized,
            UploadPhase::Done,
            UploadPhase::Cancelled,
        ] {
            assert_eq!(phase_of(&phase_text(p)), p);
        }
        assert_eq!(phase_of("v9"), UploadPhase::Other);
    }
}
