// SPDX-License-Identifier: MPL-2.0
//! Audio decoding to mono PCM for audio fingerprints (PLAY-011).
//!
//! [`AudioReader`] opens a file through any [`ByteSource`], picks the main
//! audio stream and decodes a time range, handing the caller mono `f32`
//! samples at the stream's own rate (the mean of all channels). Every
//! sample format of FFmpeg is converted in Rust: no libswresample.
//!
//! Only the mirrored `AVFrame` prefix is read (data, line sizes, sample
//! count, format). The channel count and rate come from the decoder
//! through AVOptions (`ch_layout`, `ar`), which follow the stream even if
//! it changes in the middle (DVB recordings switching 2.0/5.1).

use std::ffi::{c_int, c_void};
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pa_player_io::ffmpeg::AvioContext;
use pa_vfs::ByteSource;

use crate::decode::{self, AVFrame, CodecContext, DecodeFunctions};
use crate::error::{ProbeError, ProbeErrorKind};
use crate::frames::{Cancellable, cancelled, source_error, ts_ms};
use crate::library::Ffmpeg;
use crate::probe::dict_set;
use crate::sys::{
    self, AVChannelLayout, AVChannelLayoutU, AVDictionary, AVFormatContext, AVPacket, AVRational,
    Functions,
};

/// AVIO buffer size.
const AVIO_BUFFER: usize = 64 * 1024;

/// How an [`AudioReader`] opens the file.
#[derive(Debug, Clone)]
pub struct AudioOptions {
    /// libavformat `probesize` when opening.
    pub probe_bytes: u64,
    /// Packets read per [`AudioReader::decode_mono`] call at most (a guard
    /// against files whose timestamps never reach the end of the range).
    pub max_packets: usize,
    /// Set to `true` from another thread to abort.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            probe_bytes: 4 << 20,
            max_packets: 2_000_000,
            cancel: None,
        }
    }
}

impl AudioOptions {
    fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    }
}

/// See the module docs.
pub struct AudioReader<'a> {
    f: &'a Functions,
    d: &'a DecodeFunctions,
    fmt: *mut AVFormatContext,
    dec: *mut CodecContext,
    frame: *mut AVFrame,
    pkt: *mut AVPacket,
    stream: c_int,
    time_base: AVRational,
    start: i64,
    sample_rate: u32,
    duration_ms: Option<u64>,
    opts: AudioOptions,
    mono: Vec<f32>,
    // Dropped after `Drop::drop` closed `fmt`, which reads through it.
    io: AvioContext,
}

impl std::fmt::Debug for AudioReader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioReader")
            .field("stream", &self.stream)
            .field("sample_rate", &self.sample_rate)
            .field("duration_ms", &self.duration_ms)
            .finish_non_exhaustive()
    }
}

impl Drop for AudioReader<'_> {
    fn drop(&mut self) {
        // SAFETY: each pointer is null or owned by this reader and freed
        // once; the free functions accept null.
        unsafe {
            (self.d.av_frame_free)(&raw mut self.frame);
            (self.f.av_packet_free)(&raw mut self.pkt);
            (self.d.avcodec_free_context)(&raw mut self.dec);
            if !self.fmt.is_null() {
                (self.f.avformat_close_input)(&raw mut self.fmt);
            }
        }
    }
}

impl Ffmpeg {
    /// Opens `source` for audio decoding: picks the default audio stream
    /// (else the first one with channels) and opens its decoder.
    ///
    /// # Errors
    ///
    /// [`ProbeErrorKind::LibraryUnavailable`] without the decoder
    /// functions; [`ProbeErrorKind::InvalidData`] if the file has no
    /// decodable audio; [`ProbeErrorKind::Source`] if reading fails;
    /// [`ProbeErrorKind::Cancelled`].
    pub fn audio_reader(
        &self,
        source: Arc<dyn ByteSource>,
        opts: AudioOptions,
    ) -> Result<AudioReader<'_>, ProbeError> {
        AudioReader::open(self, source, opts)
    }
}

impl<'a> AudioReader<'a> {
    fn open(
        ffmpeg: &'a Ffmpeg,
        source: Arc<dyn ByteSource>,
        opts: AudioOptions,
    ) -> Result<Self, ProbeError> {
        let d = &ffmpeg.decode_api()?.functions;
        let f = &ffmpeg.functions;
        if source.size() == 0 {
            return Err(ProbeError::new(ProbeErrorKind::InvalidData, "empty file"));
        }
        let source: Arc<dyn ByteSource> = match &opts.cancel {
            Some(cancel) => Arc::new(Cancellable {
                inner: source,
                cancel: Arc::clone(cancel),
            }),
            None => source,
        };
        // SAFETY: the AVIO functions come from the same loaded FFmpeg.
        let io = unsafe { AvioContext::new(f.avio_api(), source, AVIO_BUFFER) }
            .map_err(|e| ProbeError::new(ProbeErrorKind::LibraryUnavailable, e.to_string()))?;
        // SAFETY: plain allocations; checked below.
        let (fmt, frame, pkt) = unsafe {
            (
                (f.avformat_alloc_context)(),
                (d.av_frame_alloc)(),
                (f.av_packet_alloc)(),
            )
        };
        let mut this = Self {
            f,
            d,
            fmt,
            dec: null_mut(),
            frame,
            pkt,
            stream: -1,
            time_base: AVRational::default(),
            start: 0,
            sample_rate: 0,
            duration_ms: None,
            opts,
            mono: Vec::new(),
            io,
        };
        if this.fmt.is_null() || this.frame.is_null() || this.pkt.is_null() {
            return Err(ProbeError::new(
                ProbeErrorKind::LibraryUnavailable,
                "FFmpeg allocation failed",
            ));
        }
        this.open_input()?;
        this.open_decoder()?;
        Ok(this)
    }

    fn open_input(&mut self) -> Result<(), ProbeError> {
        let f = self.f;
        let ret = {
            let mut dict: *mut AVDictionary = null_mut();
            // SAFETY: as in `FrameGrabber::open_input`: a fresh context with
            // our live AVIOContext; FFmpeg frees `fmt` on failure.
            unsafe {
                (*self.fmt).pb = self.io.as_ptr();
                dict_set(
                    f,
                    &mut dict,
                    c"probesize",
                    &self.opts.probe_bytes.max(32 * 1024).to_string(),
                );
                dict_set(f, &mut dict, c"analyzeduration", "1000000");
                let ret = (f.avformat_open_input)(&raw mut self.fmt, null(), null(), &raw mut dict);
                (f.av_dict_free)(&raw mut dict);
                ret
            }
        };
        if ret < 0 {
            return Err(self.error(ret, "libavformat cannot open the file"));
        }
        // SAFETY: `fmt` is open; a negative result still leaves what was found.
        let _ = unsafe { (f.avformat_find_stream_info)(self.fmt, null_mut()) };
        if self.opts.is_cancelled() {
            return Err(cancelled());
        }
        // SAFETY: `fmt` is open; FFmpeg keeps `streams` valid until close.
        unsafe {
            let ctx = &*self.fmt;
            let mut first = None;
            let mut default = None;
            for i in 0..ctx.nb_streams as usize {
                let Some(st) = (*ctx.streams.add(i)).as_ref() else {
                    continue;
                };
                let Some(par) = st.codecpar.as_ref() else {
                    continue;
                };
                if par.codec_type != sys::AVMEDIA_TYPE_AUDIO || par.ch_layout.nb_channels <= 0 {
                    continue;
                }
                first.get_or_insert(i);
                if st.disposition & sys::AV_DISPOSITION_DEFAULT != 0 {
                    default.get_or_insert(i);
                }
            }
            let Some(index) = default.or(first) else {
                return Err(ProbeError::new(
                    ProbeErrorKind::InvalidData,
                    "no audio stream",
                ));
            };
            for i in 0..ctx.nb_streams as usize {
                if i != index
                    && let Some(st) = (*ctx.streams.add(i)).as_mut()
                {
                    st.discard = decode::AVDISCARD_ALL;
                }
            }
            let st = &**ctx.streams.add(index);
            self.stream = c_int::try_from(index).unwrap_or(0);
            self.time_base = st.time_base;
            self.start = if st.start_time == sys::AV_NOPTS_VALUE {
                0
            } else {
                st.start_time
            };
            self.sample_rate = u32::try_from((*st.codecpar).sample_rate).unwrap_or(0);
            self.duration_ms = if ctx.duration != sys::AV_NOPTS_VALUE && ctx.duration > 0 {
                u64::try_from(ctx.duration / 1000).ok()
            } else {
                ts_ms(st.duration, 0, st.time_base)
            };
        }
        Ok(())
    }

    fn open_decoder(&mut self) -> Result<(), ProbeError> {
        let (f, d) = (self.f, self.d);
        // SAFETY: `fmt` is open and `stream` is a valid index.
        let par = unsafe { &*(**(*self.fmt).streams.add(self.stream as usize)).codecpar };
        // SAFETY: plain lookup.
        let codec = unsafe { (d.avcodec_find_decoder)(par.codec_id) };
        if codec.is_null() {
            // SAFETY: returns a static string.
            let name = unsafe { std::ffi::CStr::from_ptr((f.avcodec_get_name)(par.codec_id)) };
            return Err(ProbeError::new(
                ProbeErrorKind::InvalidData,
                format!("no decoder for {}", name.to_string_lossy()),
            ));
        }
        // SAFETY: `codec` is a decoder of this FFmpeg; `par` is live.
        let ret = unsafe {
            self.dec = (d.avcodec_alloc_context3)(codec);
            if self.dec.is_null() {
                return Err(ProbeError::new(
                    ProbeErrorKind::LibraryUnavailable,
                    "avcodec_alloc_context3 failed",
                ));
            }
            let ret = (d.avcodec_parameters_to_context)(self.dec, par);
            if ret < 0 {
                ret
            } else {
                let mut dict: *mut AVDictionary = null_mut();
                dict_set(f, &mut dict, c"threads", "1");
                let ret = (d.avcodec_open2)(self.dec, codec, &raw mut dict);
                (f.av_dict_free)(&raw mut dict);
                ret
            }
        };
        if ret < 0 {
            return Err(self.error(ret, "cannot open the audio decoder"));
        }
        if let Some(rate) = self.decoder_rate() {
            self.sample_rate = rate;
        }
        if self.sample_rate == 0 {
            return Err(ProbeError::new(
                ProbeErrorKind::InvalidData,
                "audio without a sample rate",
            ));
        }
        Ok(())
    }

    /// Duration of the file, if the container or the stream declares one.
    #[must_use]
    pub fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// Sample rate of the samples [`decode_mono`](Self::decode_mono) hands
    /// out (the stream's own; it may change if the stream does).
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Decodes `[from_ms, to_ms)` of the audio and calls `sink` with mono
    /// samples in order, with the rate they have (normally
    /// [`sample_rate`](Self::sample_rate)) and the time of the first one.
    /// Starts at the packet at or before `from_ms` and drops what lies
    /// before it. Times count from the start of the stream; the position
    /// within the range follows the decoded sample count from the first
    /// packet's timestamp, so it is exact to a packet.
    ///
    /// # Errors
    ///
    /// [`ProbeErrorKind::Source`] if reading fails,
    /// [`ProbeErrorKind::InvalidData`] if the file cannot be sought,
    /// [`ProbeErrorKind::Cancelled`].
    pub fn decode_mono(
        &mut self,
        from_ms: u64,
        to_ms: u64,
        sink: &mut dyn FnMut(u32, u64, &[f32]),
    ) -> Result<(), ProbeError> {
        if to_ms <= from_ms {
            return Ok(());
        }
        self.seek(from_ms)?;
        // Position of the next decoded sample, in samples at `rate` from
        // `base_ms`; set from the first packet.
        let mut base_ms: Option<u64> = None;
        let mut rate = self.sample_rate;
        let mut done: u64 = 0;
        let mut eof = false;
        for _ in 0..self.opts.max_packets {
            if eof {
                break;
            }
            match self.read_packet()? {
                None => {
                    eof = true;
                    // SAFETY: drain at the end of the file.
                    unsafe { (self.d.avcodec_send_packet)(self.dec, null()) };
                }
                Some(pts_ms) => {
                    if base_ms.is_none() {
                        base_ms = Some(pts_ms.unwrap_or(from_ms));
                    }
                    // SAFETY: `dec` is open; `pkt` holds a packet of its stream.
                    let ret = unsafe {
                        let ret = (self.d.avcodec_send_packet)(self.dec, self.pkt);
                        (self.f.av_packet_unref)(self.pkt);
                        ret
                    };
                    // A damaged packet is skipped.
                    if ret < 0 && ret != decode::AVERROR_EAGAIN {
                        continue;
                    }
                }
            }
            loop {
                // SAFETY: `dec` is open and `frame` is ours.
                let ret = unsafe { (self.d.avcodec_receive_frame)(self.dec, self.frame) };
                if ret < 0 {
                    break;
                }
                let new_rate = self.decoder_rate().unwrap_or(rate);
                let channels = self.decoder_channels();
                if new_rate != rate && new_rate > 0 {
                    // Restart the clock at the new rate.
                    let base = base_ms.unwrap_or(from_ms);
                    base_ms = Some(base + samples_ms(done, rate));
                    done = 0;
                    rate = new_rate;
                    self.sample_rate = rate;
                }
                let n = self.downmix(channels);
                // SAFETY: release the frame's buffers.
                unsafe { (self.d.av_frame_unref)(self.frame) };
                if n == 0 {
                    continue;
                }
                let start = base_ms.unwrap_or(from_ms) + samples_ms(done, rate);
                done += n as u64;
                let end = base_ms.unwrap_or(from_ms) + samples_ms(done, rate);
                if end <= from_ms {
                    continue;
                }
                let skip = if start < from_ms {
                    usize::try_from((from_ms - start) * u64::from(rate) / 1000)
                        .unwrap_or(n)
                        .min(n)
                } else {
                    0
                };
                let keep_until = if end > to_ms {
                    let over = usize::try_from((end - to_ms) * u64::from(rate) / 1000).unwrap_or(n);
                    n.saturating_sub(over)
                } else {
                    n
                };
                if keep_until > skip {
                    let first_ms = start.max(from_ms);
                    sink(rate, first_ms, &self.mono[skip..keep_until]);
                }
                if end >= to_ms {
                    return Ok(());
                }
            }
            if self.opts.is_cancelled() {
                return Err(cancelled());
            }
        }
        Ok(())
    }

    /// The decoder's current sample rate.
    fn decoder_rate(&self) -> Option<u32> {
        let mut v: i64 = 0;
        // SAFETY: `dec` is an open AVCodecContext (an AVClass object) and
        // "ar" is its int option.
        let ret = unsafe {
            (self.d.av_opt_get_int)(self.dec.cast::<c_void>(), c"ar".as_ptr(), 0, &raw mut v)
        };
        (ret >= 0 && v > 0).then(|| u32::try_from(v).ok()).flatten()
    }

    /// The decoder's current channel count.
    fn decoder_channels(&self) -> usize {
        let mut layout = AVChannelLayout {
            order: 0,
            nb_channels: 0,
            u: AVChannelLayoutU { mask: 0 },
            opaque: null_mut(),
        };
        // SAFETY: `dec` is an open AVCodecContext; "ch_layout" is its
        // channel-layout option; the copy is released with uninit.
        let n = unsafe {
            let ret = (self.d.av_opt_get_chlayout)(
                self.dec.cast::<c_void>(),
                c"ch_layout".as_ptr(),
                0,
                &raw mut layout,
            );
            let n = if ret >= 0 { layout.nb_channels } else { 0 };
            (self.d.av_channel_layout_uninit)(&raw mut layout);
            n
        };
        usize::try_from(n).unwrap_or(0)
    }

    /// Mixes the decoded frame down to mono into `self.mono`; the number
    /// of samples (0 for a frame that cannot be read safely).
    fn downmix(&mut self, channels: usize) -> usize {
        // SAFETY: `frame` holds a decoded audio frame.
        let fr = unsafe { &*self.frame };
        let n = usize::try_from(fr.nb_samples).unwrap_or(0);
        self.mono.clear();
        if n == 0 || channels == 0 || channels > 64 {
            return 0;
        }
        let Some((bytes, planar)) = sample_layout(fr.format) else {
            return 0;
        };
        let line = usize::try_from(fr.linesize[0]).unwrap_or(0);
        let need = if planar {
            n * bytes
        } else {
            n * bytes * channels
        };
        if line < need || fr.extended_data.is_null() {
            return 0;
        }
        self.mono.resize(n, 0.0);
        let scale = 1.0 / channels as f32;
        if planar {
            for c in 0..channels {
                // SAFETY: an audio frame's `extended_data` has one plane per
                // channel of the decoder's layout, each of at least
                // `nb_samples * bytes` bytes (checked against `linesize[0]`,
                // which FFmpeg sets for all planes alike).
                let plane = unsafe {
                    let ptr = *fr.extended_data.add(c);
                    if ptr.is_null() {
                        continue;
                    }
                    std::slice::from_raw_parts(ptr, n * bytes)
                };
                for (i, out) in self.mono.iter_mut().enumerate() {
                    *out += sample_at(fr.format, plane, i) * scale;
                }
            }
        } else {
            // SAFETY: one interleaved plane of `need` bytes (checked above).
            let data = unsafe {
                let ptr = *fr.extended_data;
                if ptr.is_null() {
                    return 0;
                }
                std::slice::from_raw_parts(ptr, need)
            };
            for (i, out) in self.mono.iter_mut().enumerate() {
                let mut sum = 0.0;
                for c in 0..channels {
                    sum += sample_at(fr.format, data, i * channels + c);
                }
                *out = sum * scale;
            }
        }
        n
    }

    /// Seeks the audio stream to the packet at or before `at_ms`.
    fn seek(&mut self, at_ms: u64) -> Result<(), ProbeError> {
        let tb = self.time_base;
        let ts = if tb.num > 0 && tb.den > 0 {
            let t = i128::from(at_ms) * i128::from(tb.den) / (i128::from(tb.num) * 1000);
            i64::try_from(t)
                .unwrap_or(i64::MAX)
                .saturating_add(self.start)
        } else {
            0
        };
        // SAFETY: `fmt` and `dec` are open.
        let ret = unsafe {
            let mut ret =
                (self.d.av_seek_frame)(self.fmt, self.stream, ts, decode::AVSEEK_FLAG_BACKWARD);
            if ret < 0 {
                ret = (self.d.av_seek_frame)(self.fmt, self.stream, ts, 0);
            }
            if ret < 0 && at_ms == 0 {
                ret = (self.d.av_seek_frame)(self.fmt, -1, 0, decode::AVSEEK_FLAG_BYTE);
            }
            (self.d.avcodec_flush_buffers)(self.dec);
            ret
        };
        if ret < 0 {
            return Err(self.error(ret, "cannot seek"));
        }
        Ok(())
    }

    /// Reads the next packet of the audio stream into `pkt` (other streams
    /// are released): `Some(time)` with its time if it has one, `None` at
    /// the end.
    fn read_packet(&mut self) -> Result<Option<Option<u64>>, ProbeError> {
        loop {
            if self.opts.is_cancelled() {
                return Err(cancelled());
            }
            // SAFETY: `fmt` is open and `pkt` is an empty packet.
            let ret = unsafe { (self.f.av_read_frame)(self.fmt, self.pkt) };
            if ret < 0 {
                if let Some(e) = self.io.source().take_error() {
                    return Err(source_error(&e));
                }
                // The end, or a damaged file that reads as the end.
                return Ok(None);
            }
            // SAFETY: `pkt` holds the packet just read.
            let p = unsafe { &*self.pkt };
            if p.stream_index == self.stream {
                let ts = if p.pts == sys::AV_NOPTS_VALUE {
                    p.dts
                } else {
                    p.pts
                };
                return Ok(Some(ts_ms(ts, self.start, self.time_base)));
            }
            // SAFETY: as above.
            unsafe { (self.f.av_packet_unref)(self.pkt) };
        }
    }

    fn error(&self, ret: c_int, what: &str) -> ProbeError {
        if self.opts.is_cancelled() {
            return cancelled();
        }
        if let Some(e) = self.io.source().take_error() {
            return source_error(&e);
        }
        let mut buf = [0 as std::ffi::c_char; 128];
        // SAFETY: `buf` holds 128 bytes; av_strerror NUL-terminates.
        let text = unsafe {
            (self.f.av_strerror)(ret, buf.as_mut_ptr(), buf.len());
            std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_string_lossy()
                .into_owned()
        };
        ProbeError::new(ProbeErrorKind::InvalidData, format!("{what}: {text}"))
    }
}

/// `n` samples at `rate` in milliseconds.
fn samples_ms(n: u64, rate: u32) -> u64 {
    if rate == 0 {
        0
    } else {
        n * 1000 / u64::from(rate)
    }
}

/// Bytes per sample and whether the format is planar.
fn sample_layout(format: c_int) -> Option<(usize, bool)> {
    Some(match format {
        decode::AV_SAMPLE_FMT_U8 => (1, false),
        decode::AV_SAMPLE_FMT_S16 => (2, false),
        decode::AV_SAMPLE_FMT_S32 | decode::AV_SAMPLE_FMT_FLT => (4, false),
        decode::AV_SAMPLE_FMT_DBL | decode::AV_SAMPLE_FMT_S64 => (8, false),
        decode::AV_SAMPLE_FMT_U8P => (1, true),
        decode::AV_SAMPLE_FMT_S16P => (2, true),
        decode::AV_SAMPLE_FMT_S32P | decode::AV_SAMPLE_FMT_FLTP => (4, true),
        decode::AV_SAMPLE_FMT_DBLP | decode::AV_SAMPLE_FMT_S64P => (8, true),
        _ => return None,
    })
}

/// Sample `i` of `data` (native endianness) as `f32` in `[-1, 1]`.
fn sample_at(format: c_int, data: &[u8], i: usize) -> f32 {
    let at = |bytes: usize| &data[i * bytes..(i + 1) * bytes];
    match format {
        decode::AV_SAMPLE_FMT_U8 | decode::AV_SAMPLE_FMT_U8P => {
            (f32::from(data[i]) - 128.0) / 128.0
        }
        decode::AV_SAMPLE_FMT_S16 | decode::AV_SAMPLE_FMT_S16P => {
            f32::from(i16::from_ne_bytes([data[2 * i], data[2 * i + 1]])) / 32_768.0
        }
        decode::AV_SAMPLE_FMT_S32 | decode::AV_SAMPLE_FMT_S32P => {
            let b = at(4);
            i32::from_ne_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0
        }
        decode::AV_SAMPLE_FMT_FLT | decode::AV_SAMPLE_FMT_FLTP => {
            let b = at(4);
            f32::from_ne_bytes([b[0], b[1], b[2], b[3]])
        }
        decode::AV_SAMPLE_FMT_DBL | decode::AV_SAMPLE_FMT_DBLP => {
            let b = at(8);
            f64::from_ne_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32
        }
        decode::AV_SAMPLE_FMT_S64 | decode::AV_SAMPLE_FMT_S64P => {
            let b = at(8);
            (i64::from_ne_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f64
                / 9_223_372_036_854_775_808.0) as f32
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_conversion() {
        let s16 = (-16_384i16).to_ne_bytes();
        assert!((sample_at(decode::AV_SAMPLE_FMT_S16, &s16, 0) + 0.5).abs() < 1e-6);
        assert!((sample_at(decode::AV_SAMPLE_FMT_U8P, &[192], 0) - 0.5).abs() < 1e-6);
        let flt = 0.25f32.to_ne_bytes();
        assert!((sample_at(decode::AV_SAMPLE_FMT_FLTP, &flt, 0) - 0.25).abs() < 1e-6);
        let s32 = (i32::MAX / 2).to_ne_bytes();
        assert!((sample_at(decode::AV_SAMPLE_FMT_S32, &s32, 0) - 0.5).abs() < 1e-6);
        let dbl = (-0.75f64).to_ne_bytes();
        assert!((sample_at(decode::AV_SAMPLE_FMT_DBL, &dbl, 0) + 0.75).abs() < 1e-6);
        assert_eq!(sample_layout(decode::AV_SAMPLE_FMT_FLTP), Some((4, true)));
        assert_eq!(sample_layout(decode::AV_SAMPLE_FMT_S16), Some((2, false)));
        assert_eq!(sample_layout(99), None);
    }

    #[test]
    fn sample_time() {
        assert_eq!(samples_ms(48_000, 48_000), 1000);
        assert_eq!(samples_ms(1024, 44_100), 23);
        assert_eq!(samples_ms(5, 0), 0);
    }
}
