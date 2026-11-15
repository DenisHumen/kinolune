// SPDX-License-Identifier: MPL-2.0
//! The on-disk state of a recording in `<target>/.tmp/job-<id>/`:
//!
//! - `part-NNNN.ts`: segments appended in order, one file per
//!   `part_minutes` of media (06 §4.2: a crash or a bad file never costs the
//!   whole broadcast);
//! - `journal.jsonl`: one line per segment (sequence, duration, program
//!   date-time, ad, discontinuity, part, offset, length), gap, ad break,
//!   category and metadata change. A segment line is written after its
//!   bytes are synced, so on resume every part is cut back to the last
//!   segment the journal knows;
//! - `chat.jsonl`: chat messages as they arrived.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::source::{ChatRecord, LiveStream};

/// A recorded segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegEntry {
    pub seq: u64,
    pub dur_ms: u64,
    /// Program date-time of its first frame, Unix ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdt: Option<i64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ad: bool,
    /// A discontinuity before it (in the playlist, or after a gap).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disc: bool,
    pub part: u32,
    pub off: u64,
    pub len: u64,
    /// When it was written, Unix ms.
    pub at: i64,
}

impl SegEntry {
    pub fn end_pdt(&self) -> Option<i64> {
        self.pdt
            .map(|p| p + i64::try_from(self.dur_ms).unwrap_or(0))
    }
}

/// Segments that were not recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapEntry {
    /// First missing sequence number.
    pub from_seq: u64,
    /// One past the last missing one.
    pub to_seq: u64,
    /// Program date-time where the gap starts and ends, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_pdt: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_pdt: Option<i64>,
    /// Missing media, ms (from program date-times, else segment count ×
    /// target duration).
    pub missing_ms: u64,
    /// `window` (the playlist moved on: a network outage longer than its
    /// window), `paused` (the job was paused: storage, user), `restart`
    /// (the process stopped), `segment_failed`.
    pub reason: String,
    pub at: i64,
}

/// An ad break announced by the playlist (`#EXT-X-DATERANGE`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdEntry {
    pub id: String,
    /// Unix ms.
    pub start: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
}

/// Category or title seen in Helix (chapters when no TWI-008 timeline).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatEntry {
    /// Unix ms.
    pub at: i64,
    pub game_id: String,
    pub game_name: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum Entry {
    Seg(SegEntry),
    Gap(GapEntry),
    Ad(AdEntry),
    Cat(CatEntry),
    Meta {
        stream: LiveStream,
        variant: String,
        quality: String,
    },
}

/// One chat line on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatEntry {
    #[serde(flatten)]
    pub line: ChatRecord,
    /// Program date-time of the live edge when it arrived, Unix ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_pdt: Option<i64>,
}

fn part_name(n: u32) -> String {
    format!("part-{n:04}.ts")
}

/// The recording on disk.
pub struct Journal {
    dir: PathBuf,
    journal: File,
    chat: File,
    fsync: bool,
    pub segs: Vec<SegEntry>,
    pub gaps: Vec<GapEntry>,
    pub ads: Vec<AdEntry>,
    ad_ids: HashSet<String>,
    pub cats: Vec<CatEntry>,
    pub meta: Option<(LiveStream, String, String)>,
    pub chat_count: u64,
    /// Open part: number, file, media in it (ms).
    part: Option<(u32, File, u64)>,
    pub bytes: u64,
}

impl Journal {
    /// Opens (or resumes) the recording in `dir`. Parts are cut back to the
    /// segments the journal lists; a torn last line is dropped.
    pub fn open(dir: &Path, fsync: bool) -> std::io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join("journal.jsonl");
        let mut entries = Vec::new();
        let mut good_len = 0u64;
        if let Ok(f) = File::open(&path) {
            let mut reader = BufReader::new(f);
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader.read_line(&mut line)?;
                if n == 0 || !line.ends_with('\n') {
                    break;
                }
                match serde_json::from_str::<Entry>(line.trim_end()) {
                    Ok(e) => {
                        entries.push(e);
                        good_len += n as u64;
                    }
                    Err(_) => break,
                }
            }
        }
        let journal = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)?;
        journal.set_len(good_len)?;
        let mut journal = journal;
        journal.seek(SeekFrom::End(0))?;
        let chat = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("chat.jsonl"))?;
        let chat_count = BufReader::new(File::open(dir.join("chat.jsonl"))?)
            .lines()
            .count() as u64;
        let mut out = Self {
            dir: dir.to_owned(),
            journal,
            chat,
            fsync,
            segs: Vec::new(),
            gaps: Vec::new(),
            ads: Vec::new(),
            ad_ids: HashSet::new(),
            cats: Vec::new(),
            meta: None,
            chat_count,
            part: None,
            bytes: 0,
        };
        for e in entries {
            match e {
                Entry::Seg(s) => out.segs.push(s),
                Entry::Gap(g) => out.gaps.push(g),
                Entry::Ad(a) => {
                    out.ad_ids.insert(a.id.clone());
                    out.ads.push(a);
                }
                Entry::Cat(c) => out.cats.push(c),
                Entry::Meta {
                    stream,
                    variant,
                    quality,
                } => out.meta = Some((stream, variant, quality)),
            }
        }
        // Cut every part back to what the journal knows; drop segments
        // whose bytes are not all there.
        let mut keep = Vec::with_capacity(out.segs.len());
        let mut part_ends: std::collections::BTreeMap<u32, u64> = Default::default();
        for s in std::mem::take(&mut out.segs) {
            let len = fs::metadata(dir.join(part_name(s.part))).map_or(0, |m| m.len());
            let expected = part_ends.get(&s.part).copied().unwrap_or(0);
            if s.off == expected && s.off + s.len <= len {
                part_ends.insert(s.part, s.off + s.len);
                out.bytes += s.len;
                keep.push(s);
            }
        }
        out.segs = keep;
        for (part, end) in &part_ends {
            let f = OpenOptions::new()
                .write(true)
                .open(dir.join(part_name(*part)))?;
            if f.metadata()?.len() > *end {
                f.set_len(*end)?;
            }
        }
        // Parts the journal does not know (written, never listed).
        for entry in fs::read_dir(dir)? {
            let name = entry?.file_name();
            let Some(n) = name
                .to_str()
                .and_then(|n| n.strip_prefix("part-"))
                .and_then(|n| n.strip_suffix(".ts"))
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            if !part_ends.contains_key(&n) {
                let _ = fs::remove_file(dir.join(part_name(n)));
            }
        }
        Ok(out)
    }

    fn write(&mut self, e: &Entry) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(e).map_err(std::io::Error::other)?;
        line.push(b'\n');
        self.journal.write_all(&line)?;
        if self.fsync {
            self.journal.sync_data()?;
        }
        Ok(())
    }

    /// The newest sequence number handled: recorded, or given up as a gap.
    pub fn last_seq(&self) -> Option<u64> {
        let seg = self.segs.last().map(|s| s.seq);
        let gap = self.gaps.iter().map(|g| g.to_seq.saturating_sub(1)).max();
        seg.max(gap)
    }

    pub fn last_end_pdt(&self) -> Option<i64> {
        self.segs.last().and_then(SegEntry::end_pdt)
    }

    /// Recorded media, ms.
    pub fn media_ms(&self) -> u64 {
        self.segs.iter().map(|s| s.dur_ms).sum()
    }

    /// Part files in order.
    pub fn parts(&self) -> Vec<PathBuf> {
        let mut parts: Vec<u32> = self.segs.iter().map(|s| s.part).collect();
        parts.dedup();
        parts.into_iter().map(|n| self.dir.join(part_name(n))).collect()
    }

    /// Appends a segment's bytes to the current part (a new part once it
    /// holds `part_ms` of media) and lists it.
    #[allow(clippy::too_many_arguments)]
    pub fn append(
        &mut self,
        seq: u64,
        dur_ms: u64,
        pdt: Option<i64>,
        ad: bool,
        disc: bool,
        body: &[u8],
        part_ms: u64,
        now_ms: i64,
    ) -> std::io::Result<()> {
        let rotate = self.part.as_ref().is_none_or(|(_, _, ms)| *ms >= part_ms);
        if rotate {
            let n = self.segs.last().map_or(1, |s| s.part + 1);
            let next = match &self.part {
                // Resumed: continue the last part unless it is full.
                None if self.segs.last().is_some_and(|last| {
                    self.segs
                        .iter()
                        .filter(|s| s.part == last.part)
                        .map(|s| s.dur_ms)
                        .sum::<u64>()
                        < part_ms
                }) =>
                {
                    let last = self.segs.last().map_or(1, |s| s.part);
                    let ms = self
                        .segs
                        .iter()
                        .filter(|s| s.part == last)
                        .map(|s| s.dur_ms)
                        .sum();
                    (last, ms)
                }
                _ => (n, 0),
            };
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.join(part_name(next.0)))?;
            self.part = Some((next.0, file, next.1));
        }
        let Some((part, file, ms)) = self.part.as_mut() else {
            return Err(std::io::Error::other("no part"));
        };
        let off = file.metadata()?.len();
        file.write_all(body)?;
        if self.fsync {
            file.sync_data()?;
        }
        *ms += dur_ms;
        let entry = SegEntry {
            seq,
            dur_ms,
            pdt,
            ad,
            disc,
            part: *part,
            off,
            len: body.len() as u64,
            at: now_ms,
        };
        self.write(&Entry::Seg(entry.clone()))?;
        self.bytes += entry.len;
        self.segs.push(entry);
        Ok(())
    }

    pub fn gap(&mut self, gap: GapEntry) -> std::io::Result<()> {
        self.write(&Entry::Gap(gap.clone()))?;
        self.gaps.push(gap);
        Ok(())
    }

    /// Records an ad break once per id.
    pub fn ad(&mut self, ad: AdEntry) -> std::io::Result<()> {
        if !self.ad_ids.insert(ad.id.clone()) {
            return Ok(());
        }
        self.write(&Entry::Ad(ad.clone()))?;
        self.ads.push(ad);
        Ok(())
    }

    /// Records a category or title when it differs from the last one.
    pub fn category(&mut self, cat: CatEntry) -> std::io::Result<()> {
        if self
            .cats
            .last()
            .is_some_and(|c| c.game_id == cat.game_id && c.title == cat.title)
        {
            return Ok(());
        }
        self.write(&Entry::Cat(cat.clone()))?;
        self.cats.push(cat);
        Ok(())
    }

    pub fn set_meta(&mut self, stream: LiveStream, variant: &str, quality: &str) -> std::io::Result<()> {
        if self
            .meta
            .as_ref()
            .is_some_and(|(s, v, q)| *s == stream && v == variant && q == quality)
        {
            return Ok(());
        }
        self.write(&Entry::Meta {
            stream: stream.clone(),
            variant: variant.to_owned(),
            quality: quality.to_owned(),
        })?;
        self.meta = Some((stream, variant.to_owned(), quality.to_owned()));
        Ok(())
    }

    pub fn chat(&mut self, line: ChatRecord, edge_pdt: Option<i64>) -> std::io::Result<()> {
        let mut bytes = serde_json::to_vec(&ChatEntry { line, edge_pdt })
            .map_err(std::io::Error::other)?;
        bytes.push(b'\n');
        self.chat.write_all(&bytes)?;
        self.chat_count += 1;
        Ok(())
    }

    /// Chat lines written so far.
    pub fn chat_lines(&self) -> std::io::Result<Vec<ChatEntry>> {
        let f = File::open(self.dir.join("chat.jsonl"))?;
        let mut out = Vec::new();
        for line in BufReader::new(f).lines() {
            let line = line?;
            if let Ok(e) = serde_json::from_str(&line) {
                out.push(e);
            }
        }
        Ok(out)
    }

    /// Syncs and closes the open part.
    pub fn close_part(&mut self) -> std::io::Result<()> {
        if let Some((_, f, _)) = self.part.take() {
            f.sync_all()?;
        }
        self.chat.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resumes_and_cuts_torn_writes() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut j = Journal::open(dir.path(), true).unwrap();
            for seq in 10..16 {
                j.append(seq, 2000, Some(seq as i64 * 2000), seq == 12, false, b"abcd", 6000, 1)
                    .unwrap();
            }
            j.ad(AdEntry {
                id: "stitched-ad-1".into(),
                start: 24_000,
                duration: Some(2.0),
            })
            .unwrap();
            j.ad(AdEntry {
                id: "stitched-ad-1".into(),
                start: 24_000,
                duration: Some(2.0),
            })
            .unwrap();
            assert_eq!(j.parts().len(), 2);
        }
        // A crash after the bytes of a segment, before its journal line, and
        // a torn journal line.
        let part2 = dir.path().join(part_name(2));
        let mut f = OpenOptions::new().append(true).open(&part2).unwrap();
        f.write_all(b"garbage").unwrap();
        let mut f = OpenOptions::new()
            .append(true)
            .open(dir.path().join("journal.jsonl"))
            .unwrap();
        f.write_all(b"{\"t\":\"seg\",\"se").unwrap();
        std::fs::write(dir.path().join(part_name(7)), b"stray").unwrap();

        let mut j = Journal::open(dir.path(), true).unwrap();
        assert_eq!(j.segs.len(), 6);
        assert_eq!(j.ads.len(), 1);
        assert_eq!(j.last_seq(), Some(15));
        assert_eq!(std::fs::metadata(&part2).unwrap().len(), 12);
        assert!(!dir.path().join(part_name(7)).exists());
        assert_eq!(j.media_ms(), 12_000);
        // The next segment goes to a new part (part 2 is full).
        j.append(16, 2000, None, false, true, b"efgh", 6000, 2).unwrap();
        assert_eq!(j.segs.last().unwrap().part, 3);
        assert_eq!(j.bytes, 28);
    }
}
