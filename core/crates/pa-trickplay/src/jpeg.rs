// SPDX-License-Identifier: MPL-2.0
//! JPEG encoding of thumbnails (`jpeg-encoder`, pure Rust) and reading
//! the size of a JPEG without decoding it.

use pa_probe::VideoFrame;

/// Encodes an RGB thumbnail at `quality` (1–100).
///
/// # Errors
///
/// A message if the frame is larger than JPEG allows or its buffer does
/// not match its size.
pub fn encode(frame: &VideoFrame, quality: u8) -> Result<Vec<u8>, String> {
    let w = u16::try_from(frame.width).map_err(|_| "frame too wide".to_owned())?;
    let h = u16::try_from(frame.height).map_err(|_| "frame too tall".to_owned())?;
    let mut out = Vec::with_capacity(frame.rgb.len() / 8);
    jpeg_encoder::Encoder::new(&mut out, quality.clamp(1, 100))
        .encode(&frame.rgb, w, h, jpeg_encoder::ColorType::Rgb)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Width and height from the frame header (SOFn) of a JPEG.
#[must_use]
pub fn dimensions(jpeg: &[u8]) -> Option<(u32, u32)> {
    if jpeg.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut i = 2;
    loop {
        // Markers may be preceded by fill bytes.
        while *jpeg.get(i)? == 0xFF && *jpeg.get(i + 1)? == 0xFF {
            i += 1;
        }
        if *jpeg.get(i)? != 0xFF {
            return None;
        }
        let marker = *jpeg.get(i + 1)?;
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        let len = usize::from(u16::from_be_bytes([*jpeg.get(i + 2)?, *jpeg.get(i + 3)?]));
        let is_sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof {
            let h = u16::from_be_bytes([*jpeg.get(i + 5)?, *jpeg.get(i + 6)?]);
            let w = u16::from_be_bytes([*jpeg.get(i + 7)?, *jpeg.get(i + 8)?]);
            return Some((u32::from(w), u32::from(h)));
        }
        if marker == 0xD9 || marker == 0xDA || len < 2 {
            return None;
        }
        i += 2 + len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_reads_size() {
        let frame = VideoFrame {
            position_ms: 0,
            width: 32,
            height: 18,
            rgb: (0..32 * 18 * 3).map(|i| (i % 251) as u8).collect(),
        };
        let jpeg = encode(&frame, 75).unwrap();
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        assert_eq!(dimensions(&jpeg), Some((32, 18)));
    }

    #[test]
    fn rejects_non_jpeg() {
        assert_eq!(dimensions(b"\x89PNG"), None);
        assert_eq!(dimensions(&[0xFF, 0xD8, 0xFF]), None);
        assert_eq!(dimensions(&[]), None);
    }
}
