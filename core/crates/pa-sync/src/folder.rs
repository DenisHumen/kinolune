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
//! {"seq":1,"prev":"00…","hash":"9f…","schema_version":1,"data":{"op_id":"…","hlc":{…},"op":"progress.upsert",…}}
//! …
//! ```
//!
//! - SYNC-017 (ADR-0157 §1): the journal is the device's node log. Each op
//!   line starts with its place in the log — `seq` without holes, `prev`
//!   (hash of the line before) and `hash` ([`crate::nodelog::entry_hash`]
//!   over the op text after the prefix) — so a reader sees a damaged line
//!   and a hole. Older readers ignore the extra fields. Lines of older
//!   writers (no prefix) are numbered on load: the next `seq` after the
//!   line before, the hash of that line as `prev`; every reader gets the
//!   same numbers. [`Folder::read_logged`] hands the entries to a node log.
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
//!   cannot append (some WebDAV servers) gets the whole file again. Whole
//!   rewrites (that one and compaction) go in place, never by delete and
//!   rename, so SMB readers holding the file open keep reading it; a backup
//!   in `<device>.jsonl.part` makes them crash-safe.
//!
//! The module is sans-database: the caller keeps the [`Cursors`] and the
//! ops not written yet (pa-catalog keeps them in its library).

use std::collections::{BTreeMap, HashMap};

use pa_vfs::{ErrorKind, Vfs, VfsError, VfsPath, VfsWrite};
use serde::{Deserialize, Serialize};

use crate::hlc::{MAX_DRIFT_MS, is_valid_device_id};
use crate::nodelog::{Entry, GENESIS, HASH_LEN, is_hash};
use crate::op::{Op, OpBody, OpId, is_secret_setting};
use crate::outbox;
use crate::proto::{ENC_PLAIN, Payload};

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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    /// Generation of the journal ([`Folder::compact`] starts a new one).
    pub generation: u64,
    /// Byte offset of the first line not read yet.
    pub offset: u64,
    /// `seq` of the last log entry read (SYNC-017).
    #[serde(default)]
    pub seq: u64,
    /// Hash of that entry; empty before the first one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hash: String,
}

impl Cursor {
    fn chain(&self) -> Chain {
        Chain {
            seq: self.seq,
            hash: if self.hash.is_empty() {
                GENESIS.to_owned()
            } else {
                self.hash.clone()
            },
        }
    }
}

/// The last entry of a journal seen so far: where the next one continues.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Chain {
    seq: u64,
    hash: String,
}

impl Chain {
    fn start() -> Self {
        Self {
            seq: 0,
            hash: GENESIS.to_owned(),
        }
    }
}

/// What a journal line is.
enum Line {
    Blank,
    /// Not an op line (cut, garbage, or JSON without an op id).
    Broken,
    /// The text does not match its hash.
    Corrupt,
    Entry(Entry),
}

const PREFIX_SEQ: &str = "{\"seq\":";

/// Splits a line into its log place `(seq, prev, hash)` and the op text.
fn split_line(text: &str) -> (Option<(u64, &str, &str)>, std::borrow::Cow<'_, str>) {
    let parsed = (|| {
        let rest = text.strip_prefix(PREFIX_SEQ)?;
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || digits > 20 {
            return None;
        }
        let seq: u64 = rest[..digits].parse().ok()?;
        let rest = rest[digits..].strip_prefix(",\"prev\":\"")?;
        let prev = rest.get(..HASH_LEN)?;
        let rest = rest.get(HASH_LEN..)?.strip_prefix("\",\"hash\":\"")?;
        let hash = rest.get(..HASH_LEN)?;
        let rest = rest.get(HASH_LEN..)?.strip_prefix("\",")?;
        (is_hash(prev) && is_hash(hash)).then_some((seq, prev, hash, rest))
    })();
    match parsed {
        Some((seq, prev, hash, rest)) => (Some((seq, prev, hash)), format!("{{{rest}").into()),
        None => (None, text.into()),
    }
}

/// An op line with its log place in front.
fn log_line(entry: &Entry, hash: &str) -> String {
    let body = entry.payload.body.strip_prefix('{').unwrap_or(&entry.payload.body);
    format!(
        "{{\"seq\":{},\"prev\":\"{}\",\"hash\":\"{hash}\",{body}",
        entry.seq, entry.prev
    )
}

fn peek_op_id(body: &str) -> Option<OpId> {
    #[derive(Deserialize)]
    struct Data {
        op_id: OpId,
    }
    #[derive(Deserialize)]
    struct Peek {
        data: Data,
    }
    serde_json::from_str::<Peek>(body).ok().map(|p| p.data.op_id)
}

/// Reads one line of `device`'s journal after `last`; a line of an older
/// writer gets the next `seq`. Advances `last` past every entry.
fn classify(device: &str, line: &[u8], last: &mut Chain) -> Line {
    let Ok(text) = std::str::from_utf8(line) else {
        return Line::Broken;
    };
    let text = text.trim();
    if text.is_empty() {
        return Line::Blank;
    }
    let (place, body) = split_line(text);
    let Some(op_id) = peek_op_id(&body) else {
        return Line::Broken;
    };
    let payload = Payload {
        enc: ENC_PLAIN.to_owned(),
        body: body.into_owned(),
    };
    let entry = match place {
        Some((seq, prev, hash)) => {
            let entry = Entry {
                origin: device.to_owned(),
                seq,
                prev: prev.to_owned(),
                op_id,
                payload,
            };
            if entry.hash() != hash {
                return Line::Corrupt;
            }
            entry
        }
        None => Entry {
            origin: device.to_owned(),
            seq: last.seq + 1,
            prev: last.hash.clone(),
            op_id,
            payload,
        },
    };
    *last = Chain {
        seq: entry.seq,
        hash: entry.hash(),
    };
    Line::Entry(entry)
}

/// Cursors by device id.
pub type Cursors = BTreeMap<String, Cursor>;

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    kinolune_state: u32,
    device: String,
    #[serde(rename = "gen")]
    generation: u64,
    /// After a compaction: entries up to this `seq` are a snapshot, with
    /// holes where older writes of a register were dropped (SYNC-017).
    #[serde(default, skip_serializing_if = "is_zero")]
    base: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
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
    /// Op lines that do not match their hash (SYNC-017); also counted in
    /// `skipped`. Not applied: the op comes again from another member.
    pub corrupt: usize,
    /// Places where a journal's log has a hole or does not chain (SYNC-017).
    pub gaps: usize,
    /// Journals that could not be read this round (being rewritten, locked);
    /// read on next round.
    pub unreadable: usize,
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
#[derive(Debug)]
pub struct Folder<'a> {
    vfs: &'a dyn Vfs,
    /// Whether the backend appended (`Some(true)`) or refused to
    /// (`Some(false)`) in this round's [`write`](Folder::write).
    appends: std::cell::Cell<Option<bool>>,
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

/// The whole source is unreachable or refuses us, not just one file.
fn is_source_down(e: &VfsError) -> bool {
    matches!(
        e.kind(),
        ErrorKind::Unavailable | ErrorKind::TimedOut | ErrorKind::AuthenticationFailed
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

fn header_line(device: &str, generation: u64, base: u64) -> Result<Vec<u8>, FolderError> {
    let mut line = serde_json::to_vec(&Header {
        kinolune_state: FORMAT,
        device: device.to_owned(),
        generation,
        base,
    })
    .map_err(crate::Error::from)?;
    line.push(b'\n');
    Ok(line)
}

/// Last line of a `.part` backup: the length of the content before it.
fn trailer_line(len: usize) -> Vec<u8> {
    format!("{{\"kinolune_state_end\":{len}}}\n").into_bytes()
}

/// A journal in the folder, as [`Folder::journals`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalInfo {
    /// Device the journal belongs to (from its file name).
    pub device: String,
    /// Size in bytes.
    pub size: u64,
    /// Last change (Unix ms), if the backend reports it.
    pub modified_ms: Option<i64>,
}

/// The content of a complete `.part` backup, or `None` if it was cut.
fn complete_backup(backup: &[u8]) -> Option<&[u8]> {
    let body = backup.strip_suffix(b"\n")?;
    let start = body.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    #[derive(Deserialize)]
    struct End {
        kinolune_state_end: usize,
    }
    let end: End = serde_json::from_slice(&body[start..]).ok()?;
    (end.kinolune_state_end == start).then_some(&backup[..start])
}

fn remove_quietly(w: &dyn VfsWrite, path: &VfsPath) -> Result<(), VfsError> {
    match w.remove_file(path) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
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
        Self {
            vfs,
            appends: std::cell::Cell::new(None),
        }
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

    /// Rewrites `path` with `bytes`. Where the backend appends (SMB, SFTP,
    /// local, Apache) it goes in place, so the file stays the same file:
    /// SMB readers keep their open handles to it (they share reads and
    /// writes but not deletes, so a remove and rename would fail or leave
    /// them reading the old file). Where it does not (WebDAV servers such
    /// as rclone, which also cache the old size of a file overwritten in
    /// place) the journal is removed first and written anew. A reader that
    /// comes in the middle sees no file, a short file or the new header,
    /// and reads again next round.
    ///
    /// The new content goes to `<device>.jsonl.part` first, with a trailer
    /// that proves it complete, so a crash in the middle of the rewrite
    /// loses nothing: the next [`write`](Self::write) finishes it
    /// ([`Self::recover`]).
    fn replace(
        &self,
        w: &dyn VfsWrite,
        device: &str,
        path: &VfsPath,
        bytes: &[u8],
    ) -> Result<(), FolderError> {
        let part = Self::part(device)?;
        let mut backup = bytes.to_vec();
        backup.extend_from_slice(&trailer_line(bytes.len()));
        write_all(w, &part, 0, &backup)?;
        if self.appends.get() == Some(false) {
            remove_quietly(w, path)?;
        }
        write_all(w, path, 0, bytes)?;
        // A server that kept the old size of the file it overwrote would
        // serve the new content cut or padded: write it anew instead.
        if stat_opt(self.vfs, path)?.is_none_or(|m| m.size != bytes.len() as u64) {
            remove_quietly(w, path)?;
            write_all(w, path, 0, bytes)?;
        }
        remove_quietly(w, &part)?;
        Ok(())
    }

    /// Finishes a [`replace`](Self::replace) a crash stopped: a complete
    /// `.part` is written over the journal; an incomplete one (the journal
    /// was not touched yet) is dropped.
    fn recover(&self, w: &dyn VfsWrite, device: &str, path: &VfsPath) -> Result<(), FolderError> {
        let part = Self::part(device)?;
        if stat_opt(self.vfs, &part)?.is_none() {
            return Ok(());
        }
        let backup = read_all(self.vfs, &part, MAX_COMPACT_READ.saturating_add(64))?;
        if let Some(bytes) = complete_backup(&backup)
            && parse_header(bytes).is_some_and(|(h, _)| h.device == device)
        {
            write_all(w, path, 0, bytes)?;
        }
        remove_quietly(w, &part)?;
        Ok(())
    }

    /// The last log entry of `device`'s journal of `size` bytes: from its
    /// tail when that line has a log place, else from the whole journal
    /// (an older writer's journal, numbered on load).
    fn last_entry(&self, device: &str, path: &VfsPath, size: u64) -> Result<Chain, VfsError> {
        const TAIL: u64 = 64 << 10;
        if size > TAIL {
            let src = self.vfs.open(path)?;
            let mut buf = vec![0u8; TAIL as usize];
            src.read_exact_at(size - TAIL, &mut buf)?;
            // The first piece may be cut: only lines after a `\n` count.
            if let Some(nl) = buf.iter().position(|b| *b == b'\n') {
                for line in buf[nl + 1..].split(|b| *b == b'\n').rev() {
                    let mut probe = Chain::start();
                    match classify(device, line, &mut probe) {
                        Line::Blank | Line::Broken => continue,
                        Line::Entry(_) if split_line_has_place(line) => return Ok(probe),
                        _ => break,
                    }
                }
            }
        }
        let bytes = read_all(self.vfs, path, MAX_COMPACT_READ)?;
        let mut last = Chain::start();
        if let Some((_, body)) = parse_header(&bytes) {
            for line in bytes[body..].split(|b| *b == b'\n') {
                classify(device, line, &mut last);
            }
        }
        Ok(last)
    }

    /// Appends `ops` (those [`for_folder`] lets through) to the journal of
    /// `device`, creating the folder and the journal if needed; an empty
    /// `ops` still creates the journal, so the others see this device.
    /// `now_ms` names the generation of a new journal. On a read-only
    /// source nothing is written and [`Written::read_only`] is set.
    pub fn write(&self, device: &str, ops: &[Op], now_ms: u64) -> Result<Written, FolderError> {
        let path = Self::journal(device)?;
        let mut bodies = Vec::new();
        for op in ops.iter().filter_map(for_folder) {
            bodies.push((op.op_id, outbox::encode(&op)?));
        }
        let count = bodies.len();
        // The op lines, each in the log after `last` (SYNC-017).
        let lines = |mut last: Chain| -> Vec<u8> {
            let mut out = Vec::new();
            for (op_id, body) in &bodies {
                let entry = Entry {
                    origin: device.to_owned(),
                    seq: last.seq + 1,
                    prev: last.hash.clone(),
                    op_id: *op_id,
                    payload: Payload {
                        enc: ENC_PLAIN.to_owned(),
                        body: body.clone(),
                    },
                };
                let hash = entry.hash();
                out.extend_from_slice(log_line(&entry, &hash).as_bytes());
                out.push(b'\n');
                last = Chain {
                    seq: entry.seq,
                    hash,
                };
            }
            out
        };
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
        match self.recover(w, device, &path) {
            Ok(()) => {}
            Err(FolderError::Vfs(e)) if is_read_only(&e) => return Ok(read_only),
            Err(e) => return Err(e),
        }
        let Some(meta) = stat_opt(self.vfs, &path)? else {
            let mut bytes = header_line(device, now_ms.max(1), 0)?;
            bytes.extend_from_slice(&lines(Chain::start()));
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
        if bodies.is_empty() {
            return Ok(Written {
                ops: 0,
                size: meta.size,
                read_only: false,
            });
        }
        let last = match self.last_entry(device, &path, meta.size) {
            Ok(last) => last,
            Err(e) if is_source_down(&e) => return Err(e.into()),
            // The journal cannot be read back: write on as a new log after
            // what is there (readers see a hole, not a fork).
            Err(_) => Chain::start(),
        };
        let lines = lines(last);
        // A line cut by a crash stays broken; ours starts on a new line.
        let mut data = Vec::with_capacity(lines.len() + 1);
        if meta.size > 0 {
            let mut last = [0u8; 1];
            let read = self
                .vfs
                .open(&path)
                .and_then(|src| src.read_exact_at(meta.size - 1, &mut last));
            match read {
                Ok(()) if last[0] == b'\n' => {}
                // Cut, or the server did not give the byte (rclone may fail
                // a one-byte range of a file it just rewrote): an empty
                // line costs nothing, readers skip it.
                Ok(()) => data.push(b'\n'),
                Err(e) if is_source_down(&e) => return Err(e.into()),
                Err(_) => data.push(b'\n'),
            }
        }
        data.extend_from_slice(&lines);
        let appended = write_all(w, &path, meta.size, &data);
        self.appends.set(Some(
            !matches!(&appended, Err(e) if e.kind() == ErrorKind::Unsupported),
        ));
        match appended {
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

    /// The journals in the folder, by device id; empty when the folder is
    /// not there yet. Reads nothing but the listing (SYNC-015: which
    /// folder to join).
    pub fn journals(&self) -> Result<Vec<JournalInfo>, FolderError> {
        let entries = match self.vfs.list(&dir()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut out: Vec<JournalInfo> = entries
            .into_iter()
            .filter(|e| e.metadata.is_file())
            .filter_map(|e| {
                Some(JournalInfo {
                    device: journal_device(&e.name)?.to_owned(),
                    size: e.metadata.size,
                    modified_ms: e.metadata.modified_ms,
                })
            })
            .collect();
        out.sort_by(|a, b| a.device.cmp(&b.device));
        Ok(out)
    }

    /// The ops of `device`'s own journal that this build reads, oldest
    /// first; empty when there is no journal. Used to carry them to another
    /// folder (SYNC-015): ops are idempotent, so the same op twice is
    /// harmless.
    pub fn own_ops(&self, device: &str) -> Result<Vec<Op>, FolderError> {
        let path = Self::journal(device)?;
        let bytes = match read_all(self.vfs, &path, MAX_COMPACT_READ) {
            Ok(b) => b,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let Some((header, body)) = parse_header(&bytes) else {
            return Ok(Vec::new());
        };
        if header.device != device {
            return Ok(Vec::new());
        }
        let mut ops: Vec<Op> = bytes[body..]
            .split(|b| *b == b'\n')
            .filter_map(|line| std::str::from_utf8(line).ok())
            .filter_map(|s| outbox::decode(s.trim()).ok())
            .filter(|op| op.hlc.device == device)
            .collect();
        ops.sort_by(|a, b| (&a.hlc, a.op_id).cmp(&(&b.hlc, b.op_id)));
        Ok(ops)
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
        self.read_logged(me, cursors, now_ms, apply, &mut |_, _| {})
    }

    /// [`read`](Self::read) that also hands every intact log entry to
    /// `entry` with its hash (SYNC-017), for a [`crate::nodelog::NodeLog`]:
    /// the `bool` is true for entries of a compaction snapshot, which have
    /// holes by design. Damaged lines are counted and skipped; the node log
    /// asks other members for them.
    pub fn read_logged(
        &self,
        me: &str,
        cursors: &mut Cursors,
        now_ms: u64,
        apply: &mut dyn FnMut(&Op) -> Result<bool, String>,
        entry: &mut dyn FnMut(Entry, bool),
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
        for (device, file) in journals {
            let cursor = cursors.entry(device.clone()).or_default();
            report.devices.push(device.clone());
            let mut sink = Sink {
                now_ms,
                apply,
                entry,
                report: &mut report,
            };
            // Always opened: the header's generation is the only reliable
            // sign of a compaction (a snapshot may keep the size, and many
            // servers give the time in whole seconds).
            match self.read_journal(&device, &file.path, cursor, &mut sink) {
                Ok(()) => {}
                // The source as a whole is away or refuses us.
                Err(e) if is_source_down(&e) => return Err(e.into()),
                // This one file is being rewritten (a server cut the body
                // short), gone for a moment, locked: the next round reads
                // on from the same cursor.
                Err(_) => report.unreadable += 1,
            }
        }
        Ok(report)
    }

    fn read_journal(
        &self,
        device: &str,
        path: &VfsPath,
        cursor: &mut Cursor,
        sink: &mut Sink<'_>,
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
            sink.report.newer += 1;
            return Ok(());
        }
        if header.kinolune_state != FORMAT || header.device != device {
            sink.report.skipped += 1;
            return Ok(());
        }
        let body = body as u64;
        if cursor.generation != header.generation || cursor.offset < body || cursor.offset > size {
            *cursor = Cursor {
                generation: header.generation,
                offset: body,
                ..Cursor::default()
            };
        }
        let mut chain = cursor.chain();
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
                    sink.take(line, device, header.base, &mut chain);
                }
                consumed += nl + 1;
            }
            buf.drain(..consumed);
            cursor.offset += consumed as u64;
            (cursor.seq, cursor.hash) = (chain.seq, chain.hash.clone());
            if buf.len() > MAX_LINE {
                if !skipping {
                    sink.report.skipped += 1;
                }
                skipping = true;
                cursor.offset += buf.len() as u64;
                buf.clear();
            }
        }
        Ok(())
    }

    /// Rewrites the journal of `device` as a snapshot: the newest op of
    /// every register ([`OpBody::register`]), in log order, under a new
    /// generation (`now_ms`, or the next one). Lines keep their log place
    /// (lines of an older writer get theirs), so the log goes on after the
    /// snapshot; the header's `base` marks the holes left before it. Lines
    /// this build cannot read are kept as they are. Returns the new size,
    /// or `None` when nothing was done: no journal, a read-only source, or
    /// no ops were appended by [`write`](Self::write) on this `Folder` yet —
    /// the rewrite strategy depends on whether the backend appends (see
    /// `replace`), so call it after a write that had ops.
    pub fn compact(&self, device: &str, now_ms: u64) -> Result<Option<u64>, FolderError> {
        let path = Self::journal(device)?;
        let Some(w) = self.vfs.as_write() else {
            return Ok(None);
        };
        if self.appends.get().is_none() {
            return Ok(None);
        }
        let bytes = match read_all(self.vfs, &path, MAX_COMPACT_READ) {
            Ok(b) => b,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let Some((header, body)) = parse_header(&bytes) else {
            return Ok(None);
        };
        let mut newest: HashMap<String, (Op, Entry)> = HashMap::new();
        // Entries of a newer build's ops: kept for that build's readers.
        let mut unknown: Vec<Entry> = Vec::new();
        // JSON lines without an op id: kept as they are.
        let mut foreign: Vec<&[u8]> = Vec::new();
        let mut chain = Chain::start();
        for line in bytes[body..].split(|b| *b == b'\n') {
            match classify(device, line, &mut chain) {
                Line::Entry(e) => match outbox::decode(&e.payload.body) {
                    Ok(op) => {
                        let register = op.body.register();
                        match newest.get(&register) {
                            Some((kept, _)) if (&kept.hlc, kept.op_id) >= (&op.hlc, op.op_id) => {}
                            _ => {
                                newest.insert(register, (op, e));
                            }
                        }
                    }
                    Err(_) => unknown.push(e),
                },
                // Broken (a cut line) or unknown: unknown JSON is kept,
                // broken JSON and damaged lines are dropped.
                Line::Broken if serde_json::from_slice::<serde_json::Value>(line).is_ok() => {
                    foreign.push(line);
                }
                Line::Blank | Line::Broken | Line::Corrupt => {}
            }
        }
        let mut kept: Vec<Entry> = newest.into_values().map(|(_, e)| e).collect();
        kept.extend(unknown);
        kept.sort_by_key(|e| e.seq);
        let generation = now_ms.max(header.generation.saturating_add(1));
        let mut out = header_line(device, generation, chain.seq)?;
        for line in foreign {
            out.extend_from_slice(line);
            out.push(b'\n');
        }
        for e in &kept {
            out.extend_from_slice(log_line(e, &e.hash()).as_bytes());
            out.push(b'\n');
        }
        match self.replace(w, device, &path, &out) {
            Ok(()) => Ok(Some(out.len() as u64)),
            Err(FolderError::Vfs(e)) if is_read_only(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Where the lines of a read go.
struct Sink<'r> {
    now_ms: u64,
    apply: &'r mut dyn FnMut(&Op) -> Result<bool, String>,
    entry: &'r mut dyn FnMut(Entry, bool),
    report: &'r mut ReadReport,
}

impl Sink<'_> {
    fn take(&mut self, line: &[u8], device: &str, base: u64, chain: &mut Chain) {
        let before = chain.clone();
        let entry = match classify(device, line, chain) {
            Line::Blank => return,
            Line::Broken => {
                self.report.skipped += 1;
                return;
            }
            Line::Corrupt => {
                self.report.corrupt += 1;
                self.report.skipped += 1;
                return;
            }
            Line::Entry(e) => e,
        };
        let snapshot = entry.seq <= base;
        if !snapshot && (entry.seq != before.seq + 1 || entry.prev != before.hash) {
            self.report.gaps += 1;
        }
        let Ok(op) = outbox::decode(&entry.payload.body) else {
            self.report.skipped += 1;
            (self.entry)(entry, snapshot);
            return;
        };
        if op.hlc.device != device
            || op.hlc.wall_ms > self.now_ms.saturating_add(MAX_DRIFT_MS)
        {
            self.report.skipped += 1;
            return;
        }
        (self.entry)(entry, snapshot);
        self.report.ops += 1;
        match (self.apply)(&op) {
            Ok(true) => self.report.changed += 1,
            Ok(false) => {}
            Err(_) => self.report.skipped += 1,
        }
    }
}

fn split_line_has_place(line: &[u8]) -> bool {
    std::str::from_utf8(line).is_ok_and(|t| split_line(t.trim()).0.is_some())
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
        assert!(!should_compact(
            COMPACT_MIN_BYTES * 3,
            COMPACT_MIN_BYTES * 2
        ));
        assert!(should_compact(COMPACT_MIN_BYTES * 4, COMPACT_MIN_BYTES * 2));
    }

    #[test]
    fn backups_are_complete_only_with_their_trailer() {
        let body = b"{\"kinolune_state\":1}\n{\"a\":1}\n".to_vec();
        let mut backup = body.clone();
        backup.extend_from_slice(&trailer_line(body.len()));
        assert_eq!(complete_backup(&backup), Some(&body[..]));
        // Cut anywhere: not complete.
        for cut in 0..backup.len() {
            assert_eq!(complete_backup(&backup[..cut]), None, "cut at {cut}");
        }
        let mut wrong = body.clone();
        wrong.extend_from_slice(&trailer_line(body.len() + 1));
        assert_eq!(complete_backup(&wrong), None);
    }

    #[test]
    fn headers_round_trip() {
        let line = header_line("tv", 42, 0).unwrap();
        let (h, body) = parse_header(&line).unwrap();
        assert_eq!(
            (h.device.as_str(), h.generation, body),
            ("tv", 42, line.len())
        );
        assert!(parse_header(b"{\"kinolune_state\":1").is_none());
        assert!(!String::from_utf8(line).unwrap().contains("base"));
    }

    fn op_text(n: u128) -> (OpId, String) {
        let op = Op {
            op_id: OpId(uuid::Uuid::from_u128(n)),
            hlc: crate::Hlc {
                wall_ms: 1_763_000_000_000,
                counter: 0,
                device: "tv".into(),
            },
            aliases: Vec::new(),
            body: OpBody::Watched {
                profile: "default".into(),
                key: "tmdb:movie:603".parse().unwrap(),
            },
        };
        (op.op_id, outbox::encode(&op).unwrap())
    }

    /// SYNC-017: an op line carries its log place; older readers still
    /// decode the op; a changed byte is seen; a line of an older writer is
    /// numbered after the line before it.
    #[test]
    fn log_lines_round_trip_and_older_lines_are_numbered() {
        let (op_id, body) = op_text(1);
        let entry = Entry {
            origin: "tv".into(),
            seq: 1,
            prev: GENESIS.into(),
            op_id,
            payload: Payload {
                enc: ENC_PLAIN.into(),
                body: body.clone(),
            },
        };
        let line = log_line(&entry, &entry.hash());
        assert!(outbox::decode(&line).is_ok(), "an older reader reads it");
        let mut chain = Chain::start();
        let Line::Entry(back) = classify("tv", line.as_bytes(), &mut chain) else {
            panic!("not an entry");
        };
        assert_eq!(back, entry);
        assert_eq!(chain.seq, 1);
        let damaged = line.replace("default", "defaulT");
        assert!(matches!(
            classify("tv", damaged.as_bytes(), &mut Chain::start()),
            Line::Corrupt
        ));
        // An older writer's line after it: seq 2, chained to line 1.
        let (_, old) = op_text(2);
        let Line::Entry(two) = classify("tv", old.as_bytes(), &mut chain) else {
            panic!("not an entry");
        };
        assert_eq!((two.seq, two.prev.as_str()), (2, entry.hash().as_str()));
        assert!(matches!(
            classify("tv", b"{\"a\":1}", &mut chain),
            Line::Broken
        ));
        assert!(matches!(classify("tv", b"  ", &mut chain), Line::Blank));
        assert_eq!(chain.seq, 2);
    }
}
