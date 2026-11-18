// SPDX-License-Identifier: MPL-2.0
//! Sync through a shared folder (SYNC-007, ADR-0129).
//!
//! Every device keeps one append-only journal in a folder of a source the
//! user marked (SMB, WebDAV, SFTP, a local folder another tool syncs):
//! `/.kinolune/state/<device_id>.jsonl`. A device writes only its own file,
//! so no locks are needed; it reads the files of the others from where it
//! stopped last time and applies their ops. Ops are last-writer-wins
//! registers ordered by HLC, so any order of reads converges.
//!
//! File format, one JSON value per line, `\n` after each:
//!
//! ```text
//! {"kinolune_state":1,"device":"k0193…","gen":1763000000000}
//! {"schema_version":1,"data":{"op_id":"…","hlc":{…},"op":"progress.upsert",…}}
//! …
//! ```
//!
//! - The header names the device and the *generation*. Compaction
//!   ([`Folder::compact`]) rewrites the journal as a snapshot (the newest op
//!   of every register) under a new generation; readers that see another
//!   generation read the file again from the start. Ops are idempotent, so
//!   reading something twice is harmless.
//! - Op lines are [`outbox::encode`] payloads, the same versioned JSON as
//!   on the LAN. Secrets never go there ([`for_folder`]).
//! - A reader takes complete lines only: a line still being appended (no
//!   `\n` yet) waits for the next round. Lines that do not parse, ops of an
//!   unknown kind, ops of another device and ops with a clock far ahead
//!   are skipped and counted; they never stop the journal.
//! - Writes append (`write_file` at the current size). A backend that
//!   cannot append (some WebDAV servers) gets the whole file again through
//!   `<device>.jsonl.part` and a rename.
//!
//! The module is sans-database: the caller keeps the [`Cursors`] and the
//! ops not written yet (pa-catalog keeps them in its library).

use std::collections::{BTreeMap, HashMap};

use pa_vfs::{ErrorKind, Vfs, VfsError, VfsPath, VfsWrite};
use serde::{Deserialize, Serialize};

use crate::hlc::{MAX_DRIFT_MS, is_valid_device_id};
use crate::op::{Op, OpBody, is_secret_setting};
use crate::outbox;

/// Folder of the journals, from the source root.
pub const DIR: &str = "/.kinolune/state";
/// Format of the journal header this build writes and reads.
pub const FORMAT: u32 = 1;
/// Longest accepted line; a longer one is skipped as broken.
pub const MAX_LINE: usize = 256 * 1024;
/// Most bytes read from one journal in one round; the rest waits.
pub const MAX_READ_PER_ROUND: u64 = 64 << 20;
/// Largest own journal [`Folder::compact`] reads back.
pub const MAX_COMPACT_READ: u64 = 128 << 20;
/// An own journal is compacted once it is this big and twice the size of
/// its last snapshot ([`should_compact`]).
pub const COMPACT_MIN_BYTES: u64 = 1 << 20;

const EXT: &str = ".jsonl";
const PART_EXT: &str = ".jsonl.part";
const CHUNK: usize = 1 << 20;
const HEADER_MAX: usize = 4096;

/// Error of the folder transport.
#[derive(Debug, thiserror::Error)]
pub enum FolderError {
    /// The source said no or is unreachable.
    #[error(transparent)]
    Vfs(#[from] VfsError),
    /// An op could not be encoded, or a device id is not usable.
    #[error(transparent)]
    Sync(#[from] crate::Error),
}

/// Where a reader stopped in another device's journal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    /// Generation of the journal ([`Folder::compact`] starts a new one).
    pub generation: u64,
    /// Byte offset of the first line not read yet.
    pub offset: u64,
    /// Size and modification time the listing showed when the journal was
    /// last read up to its end: an unchanged file is not opened again.
    #[serde(default)]
    pub seen_size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_modified_ms: Option<i64>,
}

/// Cursors by device id.
pub type Cursors = BTreeMap<String, Cursor>;

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    kinolune_state: u32,
    device: String,
    #[serde(rename = "gen")]
    generation: u64,
}

/// What a write did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Written {
    /// Ops written (after [`for_folder`] dropped what must not go there).
    pub ops: usize,
    /// Size of the own journal after the write.
    pub size: u64,
    /// The source cannot be written: nothing was written and the ops wait.
    pub read_only: bool,
}

/// What a read did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadReport {
    /// Other devices with a journal in the folder.
    pub devices: Vec<String>,
    /// Ops read.
    pub ops: usize,
    /// Ops that changed something here.
    pub changed: usize,
    /// Lines skipped: broken, unknown, of another device, a clock far ahead,
    /// or refused by the library.
    pub skipped: usize,
    /// Journals of a newer format this build cannot read.
    pub newer: usize,
}

/// The op as it may go into a shared folder, or `None` if it must not go at
/// all. Strips the sealed source password and the sealed PIN hash (the
/// receiver keeps its own, as without a family key) and drops secret
/// settings. A folder is readable by everyone who can open the share.
pub fn for_folder(op: &Op) -> Option<Op> {
    let mut op = op.clone();
    match &mut op.body {
        OpBody::SourceUpsert { secret, .. } => *secret = None,
        OpBody::ProfileUpsert { pin, .. } => *pin = None,
        OpBody::Setting { key, .. } if is_secret_setting(key) => return None,
        _ => {}
    }
    Some(op)
}

/// Whether an own journal of `size` bytes whose last snapshot had
/// `snapshot_size` bytes should be compacted now.
pub fn should_compact(size: u64, snapshot_size: u64) -> bool {
    size >= COMPACT_MIN_BYTES && size >= snapshot_size.saturating_mul(2)
}

/// The journals folder of one source.
#[derive(Debug, Clone, Copy)]
pub struct Folder<'a> {
    vfs: &'a dyn Vfs,
}

fn dir() -> VfsPath {
    VfsPath::parse(DIR).expect("DIR is a valid path")
}

fn stat_opt(vfs: &dyn Vfs, path: &VfsPath) -> Result<Option<pa_vfs::Metadata>, VfsError> {
    match vfs.stat(path) {
        Ok(m) => Ok(Some(m)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The source refuses writes for good (not a passing failure).
fn is_read_only(e: &VfsError) -> bool {
    matches!(
        e.kind(),
        ErrorKind::PermissionDenied | ErrorKind::Unsupported
    )
}

fn write_all(w: &dyn VfsWrite, path: &VfsPath, offset: u64, bytes: &[u8]) -> Result<(), VfsError> {
    let mut r = bytes;
    let len = bytes.len() as u64;
    let n = w.write_file(path, offset, &mut r, len)?;
    if n != len {
        return Err(VfsError::new(
            ErrorKind::Other,
            format!("{path}: short write ({n} of {len} bytes)"),
        ));
    }
    Ok(())
}

fn read_all(vfs: &dyn Vfs, path: &VfsPath, max: u64) -> Result<Vec<u8>, VfsError> {
    let src = vfs.open(path)?;
    let size = src.size();
    if size > max {
        return Err(VfsError::new(
            ErrorKind::Other,
            format!("{path}: {size} bytes is too big"),
        ));
    }
    let mut buf = vec![0u8; usize::try_from(size).unwrap_or(usize::MAX)];
    src.read_exact_at(0, &mut buf)?;
    Ok(buf)
}

fn header_line(device: &str, generation: u64) -> Result<Vec<u8>, FolderError> {
    let mut line = serde_json::to_vec(&Header {
        kinolune_state: FORMAT,
        device: device.to_owned(),
        generation,
    })
    .map_err(crate::Error::from)?;
    line.push(b'\n');
    Ok(line)
}

/// Splits off the header line of a journal: `(header, body offset)`, or
/// `None` while the header is incomplete or broken.
fn parse_header(bytes: &[u8]) -> Option<(Header, usize)> {
    let nl = bytes.iter().position(|b| *b == b'\n')?;
    let header: Header = serde_json::from_slice(&bytes[..nl]).ok()?;
    Some((header, nl + 1))
}

/// Device id of a journal file name, if it is one.
fn journal_device(name: &str) -> Option<&str> {
    let device = name.strip_suffix(EXT)?;
    is_valid_device_id(device).then_some(device)
}

impl<'a> Folder<'a> {
    /// The folder on `vfs`.
    pub fn new(vfs: &'a dyn Vfs) -> Self {
        Self { vfs }
    }

    /// Path of the journal of `device`.
    pub fn journal(device: &str) -> Result<VfsPath, FolderError> {
        if !is_valid_device_id(device) {
            return Err(crate::Error::Invalid("device id").into());
        }
        Ok(dir().join(&format!("{device}{EXT}"))?)
    }

    fn part(device: &str) -> Result<VfsPath, FolderError> {
        Ok(dir().join(&format!("{device}{PART_EXT}"))?)
    }

    /// `mkdir -p /.kinolune/state`.
    fn ensure_dir(&self, w: &dyn VfsWrite) -> Result<(), VfsError> {
        let mut at = VfsPath::root();
        for c in dir().components() {
            at = at.join(c)?;
            match stat_opt(self.vfs, &at)? {
                Some(m) if m.is_dir() => continue,
                Some(_) => {
                    return Err(VfsError::new(
                        ErrorKind::NotADirectory,
                        format!("{at}: a file is in the way"),
                    ));
                }
                None => {}
            }
            if let Err(e) = w.create_dir(&at) {
                // Another device made it meanwhile.
                if !stat_opt(self.vfs, &at)?.is_some_and(|m| m.is_dir()) {
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Replaces `path` with `bytes` through the `.part` file: readers see
    /// the old file, no file for a moment, or the new one.
    fn replace(
        &self,
        w: &dyn VfsWrite,
        device: &str,
        path: &VfsPath,
        bytes: &[u8],
    ) -> Result<(), FolderError> {
        let part = Self::part(device)?;
        write_all(w, &part, 0, bytes)?;
        match w.remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        w.rename(&part, path)?;
        Ok(())
    }

    /// Appends `ops` (those [`for_folder`] lets through) to the journal of
    /// `device`, creating the folder and the journal if needed; an empty
    /// `ops` still creates the journal, so the others see this device.
    /// `now_ms` names the generation of a new journal. On a read-only
    /// source nothing is written and [`Written::read_only`] is set.
    pub fn write(&self, device: &str, ops: &[Op], now_ms: u64) -> Result<Written, FolderError> {
        let path = Self::journal(device)?;
        let mut lines = Vec::new();
        let mut count = 0;
        for op in ops.iter().filter_map(for_folder) {
            lines.extend_from_slice(outbox::encode(&op)?.as_bytes());
            lines.push(b'\n');
            count += 1;
        }
        let read_only = Written {
            read_only: true,
            ..Written::default()
        };
        let Some(w) = self.vfs.as_write() else {
            return Ok(read_only);
        };
        match self.ensure_dir(w) {
            Ok(()) => {}
            Err(e) if is_read_only(&e) => return Ok(read_only),
            Err(e) => return Err(e.into()),
        }
        let mut current = stat_opt(self.vfs, &path)?;
        if current.is_none() {
            // A compaction stopped between removing the journal and
            // renaming its snapshot: the snapshot is complete, use it.
            let part = Self::part(device)?;
            if let Some(m) = stat_opt(self.vfs, &part)?
                && m.size > 0
                && let Ok(bytes) = read_all(self.vfs, &part, MAX_COMPACT_READ)
                && parse_header(&bytes).is_some_and(|(h, _)| h.device == device)
                && bytes.ends_with(b"\n")
            {
                w.rename(&part, &path)?;
                current = stat_opt(self.vfs, &path)?;
            }
        }
        let Some(meta) = current else {
            let mut bytes = header_line(device, now_ms.max(1))?;
            bytes.extend_from_slice(&lines);
            return match write_all(w, &path, 0, &bytes) {
                Ok(()) => Ok(Written {
                    ops: count,
                    size: bytes.len() as u64,
                    read_only: false,
                }),
                Err(e) if is_read_only(&e) => Ok(read_only),
                Err(e) => Err(e.into()),
            };
        };
        if lines.is_empty() {
            return Ok(Written {
                ops: 0,
                size: meta.size,
                read_only: false,
            });
        }
        // A line cut by a crash stays broken; ours starts on a new line.
        let mut data = Vec::with_capacity(lines.len() + 1);
        if meta.size > 0 {
            let src = self.vfs.open(&path)?;
            let mut last = [0u8; 1];
            src.read_exact_at(meta.size - 1, &mut last)?;
            if last[0] != b'\n' {
                data.push(b'\n');
            }
        }
        data.extend_from_slice(&lines);
        match write_all(w, &path, meta.size, &data) {
            Ok(()) => Ok(Written {
                ops: count,
                size: meta.size + data.len() as u64,
                read_only: false,
            }),
            Err(e) if e.kind() == ErrorKind::Unsupported => {
                // No append here: the whole file again.
                let mut bytes = read_all(self.vfs, &path, MAX_COMPACT_READ)?;
                bytes.extend_from_slice(&data);
                match self.replace(w, device, &path, &bytes) {
                    Ok(()) => Ok(Written {
                        ops: count,
                        size: bytes.len() as u64,
                        read_only: false,
                    }),
                    Err(FolderError::Vfs(e)) if is_read_only(&e) => Ok(read_only),
                    Err(e) => Err(e),
                }
            }
            Err(e) if e.kind() == ErrorKind::PermissionDenied => Ok(read_only),
            Err(e) => Err(e.into()),
        }
    }

    /// Reads the journals of every device but `me` from their cursors and
    /// hands each op to `apply` (which returns whether it changed
    /// anything; an error skips the op). Cursors of journals that are gone
    /// are kept: a journal may be away for a moment during a compaction.
    /// `now_ms` is the physical clock: ops more than
    /// [`MAX_DRIFT_MS`] ahead of it are skipped.
    pub fn read(
        &self,
        me: &str,
        cursors: &mut Cursors,
        now_ms: u64,
        apply: &mut dyn FnMut(&Op) -> Result<bool, String>,
    ) -> Result<ReadReport, FolderError> {
        let mut report = ReadReport::default();
        let entries = match self.vfs.list(&dir()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(report),
            Err(e) => return Err(e.into()),
        };
        let mut journals: Vec<_> = entries
            .into_iter()
            .filter(|e| e.metadata.is_file())
            .filter_map(|e| {
                let device = journal_device(&e.name)?.to_owned();
                (device != me).then_some((device, e))
            })
            .collect();
        journals.sort_by(|a, b| a.0.cmp(&b.0));
        for (device, entry) in journals {
            let cursor = cursors.entry(device.clone()).or_default();
            report.devices.push(device.clone());
            if cursor.offset > 0
                && cursor.seen_size == entry.metadata.size
                && cursor.seen_modified_ms == entry.metadata.modified_ms
            {
                continue;
            }
            match self.read_journal(&device, &entry.path, cursor, now_ms, apply, &mut report) {
                Ok(()) => {}
                // Gone (a compaction is renaming) or being replaced: next
                // round.
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            if cursor.offset >= entry.metadata.size {
                cursor.seen_size = entry.metadata.size;
                cursor.seen_modified_ms = entry.metadata.modified_ms;
            } else {
                cursor.seen_size = u64::MAX;
            }
        }
        Ok(report)
    }

    fn read_journal(
        &self,
        device: &str,
        path: &VfsPath,
        cursor: &mut Cursor,
        now_ms: u64,
        apply: &mut dyn FnMut(&Op) -> Result<bool, String>,
        report: &mut ReadReport,
    ) -> Result<(), VfsError> {
        let src = self.vfs.open(path)?;
        let size = src.size();
        let mut head = vec![0u8; usize::try_from(size).unwrap_or(HEADER_MAX).min(HEADER_MAX)];
        src.read_exact_at(0, &mut head)?;
        let Some((header, body)) = parse_header(&head) else {
            // Being written, or not a journal: look again next round.
            return Ok(());
        };
        if header.kinolune_state > FORMAT {
            report.newer += 1;
            return Ok(());
        }
        if header.kinolune_state != FORMAT || header.device != device {
            report.skipped += 1;
            return Ok(());
        }
        let body = body as u64;
        if cursor.generation != header.generation || cursor.offset < body || cursor.offset > size
        {
            *cursor = Cursor {
                generation: header.generation,
                offset: body,
                ..Cursor::default()
            };
        }
        let end = size.min(cursor.offset.saturating_add(MAX_READ_PER_ROUND));
        let mut buf: Vec<u8> = Vec::new();
        // Bytes of `buf` start at `cursor.offset`; while skipping an
        // overlong line, everything up to its `\n` is dropped.
        let mut skipping = false;
        let mut at = cursor.offset;
        while at < end {
            let want = usize::try_from(end - at).unwrap_or(CHUNK).min(CHUNK);
            let start = buf.len();
            buf.resize(start + want, 0);
            let n = src.read_at(at, &mut buf[start..])?;
            buf.truncate(start + n);
            if n == 0 {
                break;
            }
            at += n as u64;
            let mut consumed = 0;
            while let Some(nl) = buf[consumed..].iter().position(|b| *b == b'\n') {
                let line = &buf[consumed..consumed + nl];
                if skipping {
                    skipping = false;
                } else {
                    take_line(line, device, now_ms, apply, report);
                }
                consumed += nl + 1;
            }
            buf.drain(..consumed);
            cursor.offset += consumed as u64;
            if buf.len() > MAX_LINE {
                if !skipping {
                    report.skipped += 1;
                }
                skipping = true;
                cursor.offset += buf.len() as u64;
                buf.clear();
            }
        }
        Ok(())
    }

    /// Rewrites the journal of `device` as a snapshot: the newest op of
    /// every register ([`OpBody::register`]), oldest first, under a new
    /// generation (`now_ms`, or the next one). Lines this build cannot read
    /// are kept as they are. Returns the new size, or `None` when there is
    /// no journal or the source is read-only.
    pub fn compact(&self, device: &str, now_ms: u64) -> Result<Option<u64>, FolderError> {
        let path = Self::journal(device)?;
        let Some(w) = self.vfs.as_write() else {
            return Ok(None);
        };
        let bytes = match read_all(self.vfs, &path, MAX_COMPACT_READ) {
            Ok(b) => b,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let Some((header, body)) = parse_header(&bytes) else {
            return Ok(None);
        };
        let mut newest: HashMap<String, Op> = HashMap::new();
        let mut unknown: Vec<&[u8]> = Vec::new();
        for line in bytes[body..].split(|b| *b == b'\n') {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let decoded = std::str::from_utf8(line)
                .ok()
                .map(|s| outbox::decode(s.trim()));
            match decoded {
                Some(Ok(op)) => {
                    let register = op.body.register();
                    match newest.get(&register) {
                        Some(kept) if (&kept.hlc, kept.op_id) >= (&op.hlc, op.op_id) => {}
                        _ => {
                            newest.insert(register, op);
                        }
                    }
                }
                // A newer build's op: keep it for that build's readers.
                Some(Err(crate::Error::Invalid("schema version"))) => unknown.push(line),
                // Broken (a cut line) or unknown kind: unknown kinds are
                // kept, broken JSON is dropped.
                Some(Err(_)) if serde_json::from_slice::<serde_json::Value>(line).is_ok() => {
                    unknown.push(line);
                }
                _ => {}
            }
        }
        let mut ops: Vec<Op> = newest.into_values().collect();
        ops.sort_by(|a, b| (&a.hlc, a.op_id).cmp(&(&b.hlc, b.op_id)));
        let generation = now_ms.max(header.generation.saturating_add(1));
        let mut out = header_line(device, generation)?;
        for line in unknown {
            out.extend_from_slice(line);
            out.push(b'\n');
        }
        for op in &ops {
            out.extend_from_slice(outbox::encode(op)?.as_bytes());
            out.push(b'\n');
        }
        match self.replace(w, device, &path, &out) {
            Ok(()) => Ok(Some(out.len() as u64)),
            Err(FolderError::Vfs(e)) if is_read_only(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

fn take_line(
    line: &[u8],
    device: &str,
    now_ms: u64,
    apply: &mut dyn FnMut(&Op) -> Result<bool, String>,
    report: &mut ReadReport,
) {
    let Ok(text) = std::str::from_utf8(line) else {
        report.skipped += 1;
        return;
    };
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let Ok(op) = outbox::decode(text) else {
        report.skipped += 1;
        return;
    };
    if op.hlc.device != device || op.hlc.wall_ms > now_ms.saturating_add(MAX_DRIFT_MS) {
        report.skipped += 1;
        return;
    }
    report.ops += 1;
    match apply(&op) {
        Ok(true) => report.changed += 1,
        Ok(false) => {}
        Err(_) => report.skipped += 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_names() {
        assert_eq!(journal_device("k01ab.jsonl"), Some("k01ab"));
        assert_eq!(journal_device("k01ab.jsonl.part"), None);
        assert_eq!(journal_device("a b.jsonl"), None);
        assert_eq!(journal_device(".jsonl"), None);
        assert_eq!(
            Folder::journal("tv").unwrap().as_str(),
            "/.kinolune/state/tv.jsonl"
        );
        assert!(Folder::journal("../x").is_err());
    }

    #[test]
    fn compaction_threshold() {
        assert!(!should_compact(1000, 0));
        assert!(should_compact(COMPACT_MIN_BYTES, 0));
        assert!(!should_compact(COMPACT_MIN_BYTES * 3, COMPACT_MIN_BYTES * 2));
        assert!(should_compact(COMPACT_MIN_BYTES * 4, COMPACT_MIN_BYTES * 2));
    }

    #[test]
    fn headers_round_trip() {
        let line = header_line("tv", 42).unwrap();
        let (h, body) = parse_header(&line).unwrap();
        assert_eq!((h.device.as_str(), h.generation, body), ("tv", 42, line.len()));
        assert!(parse_header(b"{\"kinolune_state\":1").is_none());
    }
}
