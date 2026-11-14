// SPDX-License-Identifier: MPL-2.0
//! Which segments are on disk: the resume state of a download.

/// One bit per segment of a selection. Stored in the job checkpoint as a
/// hex string (a 48-hour VOD of 10-second segments is 4.3 KiB of hex).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SegmentBitmap {
    bits: Vec<u8>,
    len: usize,
}

impl SegmentBitmap {
    /// All `len` segments missing.
    pub fn new(len: usize) -> Self {
        Self {
            bits: vec![0; len.div_ceil(8)],
            len,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, i: usize) -> bool {
        i < self.len && self.bits[i / 8] & (1 << (i % 8)) != 0
    }

    pub fn set(&mut self, i: usize) {
        if i < self.len {
            self.bits[i / 8] |= 1 << (i % 8);
        }
    }

    pub fn clear(&mut self, i: usize) {
        if i < self.len {
            self.bits[i / 8] &= !(1 << (i % 8));
        }
    }

    /// Segments present.
    pub fn count(&self) -> usize {
        self.bits.iter().map(|b| b.count_ones() as usize).sum()
    }

    pub fn is_complete(&self) -> bool {
        self.count() == self.len
    }

    /// Indexes of the missing segments, in order.
    pub fn missing(&self) -> Vec<usize> {
        (0..self.len).filter(|&i| !self.get(i)).collect()
    }

    /// Lower-case hex, least significant bit first within each byte.
    pub fn to_hex(&self) -> String {
        self.bits.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Parses [`to_hex`](Self::to_hex) output for `len` segments; `None` if
    /// it does not fit.
    pub fn from_hex(hex: &str, len: usize) -> Option<Self> {
        if hex.len() != len.div_ceil(8) * 2 || !hex.is_ascii() {
            return None;
        }
        let bits = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
            .collect::<Option<Vec<u8>>>()?;
        let mut map = Self { bits, len };
        // Bits past `len` must be zero.
        if let Some(last) = map.bits.last_mut()
            && !len.is_multiple_of(8)
        {
            *last &= (1u8 << (len % 8)) - 1;
        }
        Some(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_count_round_trip() {
        let mut m = SegmentBitmap::new(13);
        assert_eq!(m.count(), 0);
        for i in [0, 3, 8, 12, 99] {
            m.set(i);
        }
        assert_eq!(m.count(), 4);
        m.set(5);
        m.clear(5);
        m.clear(99);
        assert!(m.get(12) && !m.get(11) && !m.get(99));
        assert_eq!(m.missing().len(), 9);
        let hex = m.to_hex();
        assert_eq!(hex.len(), 4);
        assert_eq!(SegmentBitmap::from_hex(&hex, 13), Some(m.clone()));
        assert_eq!(SegmentBitmap::from_hex(&hex, 20), None);
        assert_eq!(SegmentBitmap::from_hex("zz00", 13), None);
        for i in 0..13 {
            m.set(i);
        }
        assert!(m.is_complete());
        assert!(SegmentBitmap::new(0).is_complete());
    }
}
