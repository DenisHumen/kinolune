// SPDX-License-Identifier: MPL-2.0
//! Blu-ray movie playlists (`BDMV/PLAYLIST/*.mpls`, LIB-021).
//!
//! A playlist lists the clips (`BDMV/STREAM/<clip>.m2ts`) a title plays,
//! in order, each with the in and out time of the part of the clip it
//! uses. The feature of a disc is usually its longest playlist: on a
//! seamless-branching disc the film is many clips joined by one playlist,
//! and no single `.m2ts` holds it ([`crate::structure`] picks the main
//! title with it).
//!
//! Only what the folder analysis needs is read: the play items, their clip
//! names, the first angle of multi-angle items and the in / out times.
//! Sub-paths, stream tables, marks and extension data are skipped. The
//! layout follows the public descriptions of the format (the field sizes
//! of `PlayList()` and `PlayItem()`); no code of other projects is used.
//! AVCHD `.mpl` files have the same layout.

use std::fmt;

/// Ticks per second of MPLS times (45 kHz).
pub const TICKS_PER_SECOND: u64 = 45_000;

/// Largest playlist file read; real ones are a few KiB.
pub const MAX_PLAYLIST_BYTES: usize = 512 * 1024;

/// One clip of a playlist: the part `in_time..out_time` of
/// `STREAM/<clip_id>.m2ts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayItem {
    /// Clip name, five characters (`00055`); the stream file is
    /// `STREAM/<clip_id>.m2ts` (`.MTS` on AVCHD).
    pub clip_id: String,
    /// Start in the clip, 45 kHz ticks.
    pub in_time: u32,
    /// End in the clip, 45 kHz ticks.
    pub out_time: u32,
}

impl PlayItem {
    /// Length of the item in milliseconds (0 if the times are reversed).
    #[must_use]
    pub fn duration_ms(&self) -> u64 {
        u64::from(self.out_time.saturating_sub(self.in_time)) * 1000 / TICKS_PER_SECOND
    }
}

/// A parsed playlist: its play items in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Playlist {
    pub items: Vec<PlayItem>,
}

impl Playlist {
    /// Total length of the items, in milliseconds.
    #[must_use]
    pub fn duration_ms(&self) -> u64 {
        self.items.iter().map(PlayItem::duration_ms).sum()
    }

    /// A minimal MPLS file with these items (single angle, no streams,
    /// marks or sub-paths): for tests and tools. [`parse`] reads it back.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        const PLAYLIST_AT: u32 = 58;
        let mut items = Vec::new();
        for it in &self.items {
            let mut body = Vec::new();
            let mut name = it.clip_id.clone().into_bytes();
            name.resize(5, b'0');
            body.extend_from_slice(&name[..5]);
            body.extend_from_slice(b"M2TS");
            body.extend_from_slice(&1u16.to_be_bytes()); // connection condition 1
            body.push(0); // ref_to_STC_id
            body.extend_from_slice(&it.in_time.to_be_bytes());
            body.extend_from_slice(&it.out_time.to_be_bytes());
            body.extend_from_slice(&[0; 8]); // UO mask
            body.extend_from_slice(&[0, 0, 0, 0]); // random access, still mode, still time
            body.extend_from_slice(&[0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // empty STN_table
            let len = u16::try_from(body.len()).expect("small item");
            items.extend_from_slice(&len.to_be_bytes());
            items.extend_from_slice(&body);
        }
        let mut list = Vec::new();
        list.extend_from_slice(&[0, 0]); // reserved
        let n = u16::try_from(self.items.len()).expect("fewer than 65536 items");
        list.extend_from_slice(&n.to_be_bytes());
        list.extend_from_slice(&0u16.to_be_bytes()); // sub-paths
        list.extend_from_slice(&items);
        let list_len = u32::try_from(list.len()).expect("small playlist");
        let marks_at = PLAYLIST_AT + 4 + list_len;

        let mut out = Vec::new();
        out.extend_from_slice(b"MPLS0200");
        out.extend_from_slice(&PLAYLIST_AT.to_be_bytes());
        out.extend_from_slice(&marks_at.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // no extension data
        out.resize(40, 0);
        // AppInfoPlayList: length 14, then 14 bytes.
        out.extend_from_slice(&14u32.to_be_bytes());
        out.resize(usize::try_from(PLAYLIST_AT).expect("fits"), 0);
        out.extend_from_slice(&list_len.to_be_bytes());
        out.extend_from_slice(&list);
        // PlayListMark: no marks.
        out.extend_from_slice(&2u32.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out
    }
}

/// Why a file is not a readable playlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MplsError {
    /// No `MPLS` signature (or an unknown version).
    NotMpls,
    /// A field points or reaches past the end of the file.
    Truncated,
    /// A field holds an impossible value.
    Invalid(&'static str),
}

impl fmt::Display for MplsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMpls => f.write_str("not an MPLS playlist"),
            Self::Truncated => f.write_str("truncated MPLS playlist"),
            Self::Invalid(what) => write!(f, "invalid MPLS playlist: {what}"),
        }
    }
}

impl std::error::Error for MplsError {}

/// Big-endian reads with bounds checks.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], MplsError> {
        let end = self.at.checked_add(n).ok_or(MplsError::Truncated)?;
        let out = self.data.get(self.at..end).ok_or(MplsError::Truncated)?;
        self.at = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, MplsError> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, MplsError> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, MplsError> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

fn clip_name(raw: &[u8]) -> Result<String, MplsError> {
    if raw.iter().all(u8::is_ascii_alphanumeric) {
        Ok(String::from_utf8_lossy(raw).into_owned())
    } else {
        Err(MplsError::Invalid("clip name"))
    }
}

/// Parses an MPLS (or AVCHD MPL) file.
///
/// # Errors
///
/// [`MplsError`] when the data is not a playlist or is cut short.
pub fn parse(data: &[u8]) -> Result<Playlist, MplsError> {
    if data.len() < 20 || &data[..4] != b"MPLS" {
        return Err(MplsError::NotMpls);
    }
    if !matches!(&data[4..8], b"0100" | b"0200" | b"0300") {
        return Err(MplsError::NotMpls);
    }
    let mut header = Reader { data, at: 8 };
    let playlist_at = usize::try_from(header.u32()?).map_err(|_| MplsError::Truncated)?;
    let mut r = Reader {
        data,
        at: playlist_at,
    };
    let _length = r.u32()?;
    let _reserved = r.u16()?;
    let count = r.u16()?;
    let _sub_paths = r.u16()?;
    let mut items = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let len = usize::from(r.u16()?);
        let start = r.at;
        let end = start.checked_add(len).ok_or(MplsError::Truncated)?;
        if end > data.len() {
            return Err(MplsError::Truncated);
        }
        let mut it = Reader { data, at: start };
        let name = clip_name(it.bytes(5)?)?;
        let _codec = it.bytes(4)?;
        // 11 bits reserved, is_multi_angle, 4 bits connection condition;
        // the clip of angle 1 is the one named above.
        let _flags = it.u16()?;
        let _stc = it.u8()?;
        let in_time = it.u32()?;
        let out_time = it.u32()?;
        if it.at > end {
            return Err(MplsError::Invalid("play item length"));
        }
        if out_time < in_time {
            return Err(MplsError::Invalid("out time before in time"));
        }
        items.push(PlayItem {
            clip_id: name,
            in_time,
            out_time,
        });
        r.at = end;
    }
    Ok(Playlist { items })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(clip: &str, secs: u32) -> PlayItem {
        PlayItem {
            clip_id: clip.into(),
            in_time: 27_000_000,
            out_time: 27_000_000 + secs * 45_000,
        }
    }

    #[test]
    fn round_trip_and_duration() {
        let pl = Playlist {
            items: vec![item("00055", 3600), item("00056", 1800), item("00060", 90)],
        };
        let bytes = pl.to_bytes();
        let back = parse(&bytes).unwrap();
        assert_eq!(back, pl);
        assert_eq!(back.duration_ms(), (3600 + 1800 + 90) * 1000);
    }

    #[test]
    fn multi_angle_items_use_angle_one() {
        // A hand-built item with is_multi_angle set and a second angle
        // after the fixed fields; the item length skips it.
        let mut body = Vec::new();
        body.extend_from_slice(b"00010M2TS");
        body.extend_from_slice(&0x0011u16.to_be_bytes()); // multi angle, cc 1
        body.push(0);
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&90_000u32.to_be_bytes());
        body.extend_from_slice(&[0; 12]);
        body.extend_from_slice(&[2, 0]); // two angles
        body.extend_from_slice(b"00011M2TS");
        body.push(0);
        let mut data = b"MPLS0200".to_vec();
        data.extend_from_slice(&20u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes()); // length
        data.extend_from_slice(&[0, 0, 0, 1, 0, 0]);
        data.extend_from_slice(&u16::try_from(body.len()).unwrap().to_be_bytes());
        data.extend_from_slice(&body);
        let pl = parse(&data).unwrap();
        assert_eq!(pl.items.len(), 1);
        assert_eq!(pl.items[0].clip_id, "00010");
        assert_eq!(pl.duration_ms(), 2000);
    }

    #[test]
    fn garbage_is_rejected() {
        assert_eq!(parse(b""), Err(MplsError::NotMpls));
        assert_eq!(parse(&[7u8; 300]), Err(MplsError::NotMpls));
        assert_eq!(parse(b"MPLS9999aaaaaaaaaaaaaaaa"), Err(MplsError::NotMpls));
        let mut bytes = Playlist {
            items: vec![item("00001", 10)],
        }
        .to_bytes();
        bytes.truncate(70);
        assert_eq!(parse(&bytes), Err(MplsError::Truncated));
        // Points past the end.
        let mut far = b"MPLS0200".to_vec();
        far.extend_from_slice(&u32::MAX.to_be_bytes());
        far.extend_from_slice(&[0; 8]);
        assert_eq!(parse(&far), Err(MplsError::Truncated));
        // A clip name that is not a name.
        let mut bad = Playlist {
            items: vec![item("00001", 10)],
        }
        .to_bytes();
        bad[58 + 4 + 6 + 2] = b'/';
        assert_eq!(parse(&bad), Err(MplsError::Invalid("clip name")));
    }
}
