// SPDX-License-Identifier: MPL-2.0
//! Keyframe extraction for trickplay previews (PLAY-009).
//!
//! [`FrameGrabber`] opens a file through any [`ByteSource`] (local, SMB,
//! WebDAV), seeks to a time and decodes the keyframe at or before it,
//! scaled down to an RGB thumbnail by libswscale. Only the keyframe's
//! packet is read and decoded: no GOP is walked, so one grab costs one
//! seek and one intra frame, which keeps network sources fast.
//!
//! One grabber is single-threaded; parallel extraction opens one grabber per
//! worker over the same (`Sync`) source.

use std::ffi::c_int;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use pa_player_io::ffmpeg::AvioContext;
use pa_vfs::{ByteSource, ErrorKind, Result as VfsResult, VfsError};

use crate::decode::{self, AVFrame, CodecContext, DecodeFunctions, SwsContext};
use crate::error::{ProbeError, ProbeErrorKind};
use crate::library::Ffmpeg;
use crate::probe::dict_set;
use crate::sys::{self, AVDictionary, AVFormatContext, AVPacket, AVRational, Functions};

/// Message of the source error after cancellation.
const CANCELLED: &str = "frame extraction cancelled";

/// AVIO buffer size.
const AVIO_BUFFER: usize = 64 * 1024;

/// How a [`FrameGrabber`] opens and decodes.
#[derive(Debug, Clone)]
pub struct GrabOptions {
    /// Width of the thumbnails in pixels; the height follows the display
    /// aspect ratio. Never more than the video's own width.
    pub width: u32,
    /// Decoder threads (`0` lets FFmpeg choose). Parallel grabbers each
    /// want one.
    pub threads: u32,
    /// Packets read per grab before giving up on finding a keyframe.
    pub max_packets: usize,
    /// libavformat `probesize` when opening.
    pub probe_bytes: u64,
    /// Set to `true` from another thread to abort.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for GrabOptions {
    fn default() -> Self {
        Self {
            width: 320,
            threads: 1,
            max_packets: 4096,
            probe_bytes: 4 << 20,
            cancel: None,
        }
    }
}

impl GrabOptions {
    fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    }
}

/// A decoded, scaled frame: packed RGB, 3 bytes per pixel, rows of
/// `width * 3` bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct VideoFrame {
    /// Presentation time of the keyframe, from the start of the file.
    pub position_ms: u64,
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoFrame")
            .field("position_ms", &self.position_ms)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Result of [`FrameGrabber::grab`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grab {
    /// The keyframe at or before the requested time.
    Frame(VideoFrame),
    /// The keyframe there is at this position, which the caller said it
    /// already has (long GOPs give several times the same keyframe).
    Same(u64),
    /// No keyframe there (past the end, or none within the packet budget).
    End,
}

/// A source whose reads fail once a flag is set.
#[derive(Debug)]
struct Cancellable {
    inner: Arc<dyn ByteSource>,
    cancel: Arc<AtomicBool>,
}

impl ByteSource for Cancellable {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(VfsError::new(ErrorKind::Other, CANCELLED));
        }
        self.inner.read_at(offset, buf)
    }
}

/// See the module docs.
pub struct FrameGrabber<'a> {
    f: &'a Functions,
    d: &'a DecodeFunctions,
    fmt: *mut AVFormatContext,
    dec: *mut CodecContext,
    frame: *mut AVFrame,
    pkt: *mut AVPacket,
    sws: *mut SwsContext,
    sws_input: (c_int, c_int, c_int),
    stream: c_int,
    time_base: AVRational,
    start: i64,
    source_size: (u32, u32),
    out_size: (u32, u32),
    colorspace: c_int,
    full_range: bool,
    pq: bool,
    duration_ms: Option<u64>,
    opts: GrabOptions,
    // Dropped after `Drop::drop` closed `fmt`, which reads through it.
    io: AvioContext,
}

impl std::fmt::Debug for FrameGrabber<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameGrabber")
            .field("stream", &self.stream)
            .field("source_size", &self.source_size)
            .field("out_size", &self.out_size)
            .field("duration_ms", &self.duration_ms)
            .finish_non_exhaustive()
    }
}

impl Drop for FrameGrabber<'_> {
    fn drop(&mut self) {
        // SAFETY: each pointer is null or owned by this grabber and freed
        // once; the free functions accept null.
        unsafe {
            if !self.sws.is_null() {
                (self.d.sws_freeContext)(self.sws);
            }
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
    /// Opens `source` for keyframe extraction: picks the main video stream
    /// (the largest that is not cover art) and opens its decoder.
    ///
    /// # Errors
    ///
    /// [`ProbeErrorKind::LibraryUnavailable`] without libswscale or the
    /// decoder functions; [`ProbeErrorKind::InvalidData`] if the file has no
    /// decodable video; [`ProbeErrorKind::Source`] if reading fails;
    /// [`ProbeErrorKind::Cancelled`].
    pub fn frame_grabber(
        &self,
        source: Arc<dyn ByteSource>,
        opts: GrabOptions,
    ) -> Result<FrameGrabber<'_>, ProbeError> {
        FrameGrabber::open(self, source, opts)
    }
}

impl<'a> FrameGrabber<'a> {
    fn open(
        ffmpeg: &'a Ffmpeg,
        source: Arc<dyn ByteSource>,
        opts: GrabOptions,
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
            sws: null_mut(),
            sws_input: (0, 0, -1),
            stream: -1,
            time_base: AVRational::default(),
            start: 0,
            source_size: (0, 0),
            out_size: (0, 0),
            colorspace: 0,
            full_range: false,
            pq: false,
            duration_ms: None,
            opts,
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
            // SAFETY: `fmt` is a fresh context; `pb` is our live AVIOContext
            // (custom IO: FFmpeg does not free it). On failure FFmpeg frees
            // `fmt` and nulls it.
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
            let mut best: Option<(usize, i64)> = None;
            for i in 0..ctx.nb_streams as usize {
                let Some(st) = (*ctx.streams.add(i)).as_ref() else {
                    continue;
                };
                let Some(par) = st.codecpar.as_ref() else {
                    continue;
                };
                if par.codec_type != sys::AVMEDIA_TYPE_VIDEO
                    || st.disposition & sys::AV_DISPOSITION_ATTACHED_PIC != 0
                    || par.width <= 0
                    || par.height <= 0
                {
                    continue;
                }
                let area = i64::from(par.width) * i64::from(par.height);
                if best.is_none_or(|(_, a)| area > a) {
                    best = Some((i, area));
                }
            }
            let Some((index, _)) = best else {
                return Err(ProbeError::new(
                    ProbeErrorKind::InvalidData,
                    "no video stream",
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
        // SAFETY: plain lookups and allocation.
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
                dict_set(f, &mut dict, c"threads", &self.opts.threads.to_string());
                let ret = (d.avcodec_open2)(self.dec, codec, &raw mut dict);
                (f.av_dict_free)(&raw mut dict);
                ret
            }
        };
        if ret < 0 {
            return Err(self.error(ret, "cannot open the video decoder"));
        }
        let (w, h) = (par.width, par.height);
        self.source_size = (
            u32::try_from(w).unwrap_or(0),
            u32::try_from(h).unwrap_or(0),
        );
        let sar = par.sample_aspect_ratio.to_f64().unwrap_or(1.0);
        self.out_size = out_size(self.source_size, sar, self.opts.width);
        self.colorspace = par.color_space;
        self.full_range = par.color_range == decode::AVCOL_RANGE_JPEG;
        self.pq = par.color_trc == decode::AVCOL_TRC_SMPTE2084;
        Ok(())
    }

    /// Duration of the file, if the container or the stream declares one.
    #[must_use]
    pub fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// Coded size of the video stream.
    #[must_use]
    pub fn source_size(&self) -> (u32, u32) {
        self.source_size
    }

    /// Size of the thumbnails [`grab`](Self::grab) returns.
    #[must_use]
    pub fn thumbnail_size(&self) -> (u32, u32) {
        self.out_size
    }

    /// Decodes the keyframe at or before `at_ms`. If `seen` returns `true`
    /// for that keyframe's position, it is not decoded ([`Grab::Same`]).
    ///
    /// # Errors
    ///
    /// [`ProbeErrorKind::Source`] if reading fails,
    /// [`ProbeErrorKind::InvalidData`] if the file cannot be sought or
    /// decoded there, [`ProbeErrorKind::Cancelled`].
    pub fn grab(&mut self, at_ms: u64, seen: impl Fn(u64) -> bool) -> Result<Grab, ProbeError> {
        if self.opts.is_cancelled() {
            return Err(cancelled());
        }
        self.seek(at_ms)?;
        // Fast path: the keyframe alone, then drain the decoder.
        let Some(key_ms) = self.next_key_packet()? else {
            return Ok(Grab::End);
        };
        if seen(key_ms) {
            // SAFETY: `pkt` holds the packet read above.
            unsafe { (self.f.av_packet_unref)(self.pkt) };
            return Ok(Grab::Same(key_ms));
        }
        // SAFETY: `dec` is open; `pkt` holds a packet of its stream; a null
        // packet enters draining mode.
        unsafe {
            let ret = (self.d.avcodec_send_packet)(self.dec, self.pkt);
            (self.f.av_packet_unref)(self.pkt);
            if ret >= 0 {
                (self.d.avcodec_send_packet)(self.dec, null());
                if let Some(frame) = self.receive(key_ms)? {
                    return Ok(Grab::Frame(frame));
                }
            }
        }
        // Some decoders want the following packets before they output the
        // keyframe (H.264 recovery points without IDR): decode on from it.
        self.seek(at_ms)?;
        self.decode_from_key(key_ms)
    }

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
            let mut ret = (self.d.av_seek_frame)(self.fmt, self.stream, ts, decode::AVSEEK_FLAG_BACKWARD);
            if ret < 0 {
                ret = (self.d.av_seek_frame)(self.fmt, self.stream, ts, 0);
            }
            (self.d.avcodec_flush_buffers)(self.dec);
            ret
        };
        if ret < 0 {
            return Err(self.error(ret, "cannot seek"));
        }
        Ok(())
    }

    /// Reads up to the next keyframe packet of the stream (left in `pkt`);
    /// its position, or `None` at the end or past the packet budget.
    fn next_key_packet(&mut self) -> Result<Option<u64>, ProbeError> {
        for _ in 0..self.opts.max_packets {
            match self.read_packet()? {
                None => return Ok(None),
                Some(true) => {
                    // SAFETY: `pkt` holds the packet just read.
                    let p = unsafe { &*self.pkt };
                    if p.flags & decode::AV_PKT_FLAG_KEY != 0 {
                        let ts = if p.pts == sys::AV_NOPTS_VALUE { p.dts } else { p.pts };
                        return Ok(Some(ts_ms(ts, self.start, self.time_base).unwrap_or(0)));
                    }
                    // SAFETY: as above.
                    unsafe { (self.f.av_packet_unref)(self.pkt) };
                }
                Some(false) => {}
            }
        }
        Ok(None)
    }

    /// Reads one packet into `pkt`: `Some(true)` for our stream (kept),
    /// `Some(false)` for another one (already released), `None` at the end.
    fn read_packet(&mut self) -> Result<Option<bool>, ProbeError> {
        if self.opts.is_cancelled() {
            return Err(cancelled());
        }
        // SAFETY: `fmt` is open and `pkt` is an empty packet.
        let ret = unsafe { (self.f.av_read_frame)(self.fmt, self.pkt) };
        if ret == decode::AVERROR_EOF {
            return Ok(None);
        }
        if ret < 0 {
            if let Some(e) = self.io.source().take_error() {
                return Err(source_error(&e));
            }
            // A damaged packet in the middle of a file reads as the end.
            return Ok(None);
        }
        // SAFETY: `pkt` holds the packet just read.
        if unsafe { (*self.pkt).stream_index } == self.stream {
            Ok(Some(true))
        } else {
            // SAFETY: as above.
            unsafe { (self.f.av_packet_unref)(self.pkt) };
            Ok(Some(false))
        }
    }

    fn decode_from_key(&mut self, key_ms: u64) -> Result<Grab, ProbeError> {
        if self.next_key_packet()?.is_none() {
            return Ok(Grab::End);
        }
        let mut have_packet = true;
        for _ in 0..self.opts.max_packets {
            if have_packet {
                // SAFETY: `dec` is open; `pkt` holds a packet of its stream.
                let ret = unsafe {
                    let ret = (self.d.avcodec_send_packet)(self.dec, self.pkt);
                    (self.f.av_packet_unref)(self.pkt);
                    ret
                };
                if ret < 0 && ret != decode::AVERROR_EAGAIN {
                    return Err(self.error(ret, "cannot decode"));
                }
            }
            if let Some(frame) = self.receive(key_ms)? {
                return Ok(Grab::Frame(frame));
            }
            match self.read_packet()? {
                Some(ours) => have_packet = ours,
                None => {
                    // SAFETY: drain at the end of the file.
                    unsafe { (self.d.avcodec_send_packet)(self.dec, null()) };
                    return Ok(self.receive(key_ms)?.map_or(Grab::End, Grab::Frame));
                }
            }
        }
        Ok(Grab::End)
    }

    /// One frame from the decoder, converted; `None` if it has none yet.
    fn receive(&mut self, key_ms: u64) -> Result<Option<VideoFrame>, ProbeError> {
        // SAFETY: `dec` is open and `frame` is ours.
        let ret = unsafe { (self.d.avcodec_receive_frame)(self.dec, self.frame) };
        if ret < 0 {
            return Ok(None);
        }
        let out = self.convert(key_ms);
        // SAFETY: release the frame's buffers; reset the decoder after a drain.
        unsafe {
            (self.d.av_frame_unref)(self.frame);
            (self.d.avcodec_flush_buffers)(self.dec);
        }
        out.map(Some)
    }

    fn convert(&mut self, position_ms: u64) -> Result<VideoFrame, ProbeError> {
        let d = self.d;
        // SAFETY: `frame` holds a decoded video frame.
        let fr = unsafe { &*self.frame };
        if fr.width <= 0 || fr.height <= 0 {
            return Err(ProbeError::new(ProbeErrorKind::InvalidData, "empty frame"));
        }
        let input = (fr.width, fr.height, fr.format);
        let (ow, oh) = self.out_size;
        let (ow_c, oh_c) = (
            c_int::try_from(ow).unwrap_or(2),
            c_int::try_from(oh).unwrap_or(2),
        );
        if self.sws.is_null() || self.sws_input != input {
            // SAFETY: plain context management with valid sizes and formats.
            unsafe {
                if !self.sws.is_null() {
                    (d.sws_freeContext)(self.sws);
                }
                self.sws = (d.sws_getContext)(
                    fr.width,
                    fr.height,
                    fr.format,
                    ow_c,
                    oh_c,
                    decode::AV_PIX_FMT_RGB24,
                    decode::SWS_AREA,
                    null_mut(),
                    null_mut(),
                    null(),
                );
                if self.sws.is_null() {
                    return Err(ProbeError::new(
                        ProbeErrorKind::InvalidData,
                        format!("cannot convert pixel format {}", fr.format),
                    ));
                }
                let matrix = match self.colorspace {
                    decode::AVCOL_SPC_BT709 => decode::SWS_CS_ITU709,
                    decode::AVCOL_SPC_BT2020_NCL | decode::AVCOL_SPC_BT2020_CL => {
                        decode::SWS_CS_BT2020
                    }
                    // Unspecified: HD and larger is BT.709 in practice.
                    _ if fr.height >= 720 => decode::SWS_CS_ITU709,
                    _ => decode::SWS_CS_DEFAULT,
                };
                // Fails (harmlessly) for RGB input.
                (d.sws_setColorspaceDetails)(
                    self.sws,
                    (d.sws_getCoefficients)(matrix),
                    c_int::from(self.full_range),
                    (d.sws_getCoefficients)(decode::SWS_CS_DEFAULT),
                    1,
                    0,
                    1 << 16,
                    1 << 16,
                );
            }
            self.sws_input = input;
        }
        let row = ow as usize * 3;
        let stride = (row + 63) & !63;
        let mut buf = vec![0u8; stride * oh as usize + 64];
        let dst = [buf.as_mut_ptr(), null_mut(), null_mut(), null_mut()];
        let dst_stride = [c_int::try_from(stride).unwrap_or(c_int::MAX), 0, 0, 0];
        // SAFETY: the source planes belong to `frame`; `buf` holds `oh`
        // rows of `stride` bytes plus slack.
        let ret = unsafe {
            (d.sws_scale)(
                self.sws,
                fr.data.as_ptr().cast(),
                fr.linesize.as_ptr(),
                0,
                fr.height,
                dst.as_ptr(),
                dst_stride.as_ptr(),
            )
        };
        if ret <= 0 {
            return Err(ProbeError::new(ProbeErrorKind::InvalidData, "sws_scale failed"));
        }
        let mut rgb = Vec::with_capacity(row * oh as usize);
        for y in 0..oh as usize {
            rgb.extend_from_slice(&buf[y * stride..y * stride + row]);
        }
        if self.pq {
            let lut = pq_lut();
            for v in &mut rgb {
                *v = lut[usize::from(*v)];
            }
        }
        Ok(VideoFrame {
            position_ms,
            width: ow,
            height: oh,
            rgb,
        })
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
            std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
        };
        ProbeError::new(ProbeErrorKind::InvalidData, format!("{what}: {text}"))
    }
}

fn cancelled() -> ProbeError {
    ProbeError::new(ProbeErrorKind::Cancelled, CANCELLED)
}

fn source_error(e: &VfsError) -> ProbeError {
    if e.to_string().contains(CANCELLED) {
        cancelled()
    } else {
        ProbeError::from_vfs(e)
    }
}

/// `ts - start` in `tb` units as milliseconds.
fn ts_ms(ts: i64, start: i64, tb: AVRational) -> Option<u64> {
    if ts == sys::AV_NOPTS_VALUE || tb.num <= 0 || tb.den <= 0 {
        return None;
    }
    let rel = i128::from(ts) - i128::from(start);
    let ms = rel.max(0) * i128::from(tb.num) * 1000 / i128::from(tb.den);
    u64::try_from(ms).ok()
}

/// Thumbnail size for a `source` of sample aspect ratio `sar` at `width`:
/// even sides, at least 2, no upscaling.
fn out_size(source: (u32, u32), sar: f64, width: u32) -> (u32, u32) {
    let (sw, sh) = source;
    if sw == 0 || sh == 0 {
        return (2, 2);
    }
    let display = f64::from(sw) * if sar > 0.0 { sar } else { 1.0 } / f64::from(sh);
    let w = width.min(sw).max(2) & !1;
    let h = ((f64::from(w) / display).round() as u32).max(2);
    (w, (h + 1) & !1)
}

/// 8-bit PQ (SMPTE ST 2084) to SDR gamma 2.2, Reinhard-extended with
/// reference white at 203 cd/m² and 1000 cd/m² mapped to white. The
/// primaries stay BT.2020 (slightly muted colours): enough for previews.
fn pq_lut() -> &'static [u8; 256] {
    static LUT: OnceLock<[u8; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        const M1: f64 = 2610.0 / 16384.0;
        const M2: f64 = 2523.0 / 4096.0 * 128.0;
        const C1: f64 = 3424.0 / 4096.0;
        const C2: f64 = 2413.0 / 4096.0 * 32.0;
        const C3: f64 = 2392.0 / 4096.0 * 32.0;
        let white = 1000.0 / 203.0;
        let mut lut = [0u8; 256];
        for (i, out) in lut.iter_mut().enumerate() {
            let e = i as f64 / 255.0;
            let p = e.powf(1.0 / M2);
            let nits = ((p - C1).max(0.0) / (C2 - C3 * p)).powf(1.0 / M1) * 10_000.0;
            let x = nits / 203.0;
            let y = (x * (1.0 + x / (white * white)) / (1.0 + x)).clamp(0.0, 1.0);
            *out = (y.powf(1.0 / 2.2) * 255.0).round() as u8;
        }
        lut
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_sizes() {
        assert_eq!(out_size((1920, 1080), 1.0, 320), (320, 180));
        assert_eq!(out_size((3840, 1600), 1.0, 320), (320, 134));
        // Anamorphic DVD: 720x576 at 16:9.
        assert_eq!(out_size((720, 576), 64.0 / 45.0, 320), (320, 180));
        // No upscaling.
        assert_eq!(out_size((160, 120), 1.0, 320), (160, 120));
        assert_eq!(out_size((0, 0), 1.0, 320), (2, 2));
    }

    #[test]
    fn timestamps() {
        let tb = AVRational { num: 1, den: 1000 };
        assert_eq!(ts_ms(5000, 1000, tb), Some(4000));
        assert_eq!(ts_ms(500, 1000, tb), Some(0));
        assert_eq!(ts_ms(sys::AV_NOPTS_VALUE, 0, tb), None);
        let tb = AVRational { num: 1, den: 90_000 };
        assert_eq!(ts_ms(90_000 * 3, 0, tb), Some(3000));
    }

    #[test]
    fn pq_tone_map_is_monotonic() {
        let lut = pq_lut();
        assert_eq!(lut[0], 0);
        assert!(lut.windows(2).all(|w| w[0] <= w[1]));
        // 203 cd/m² (PQ ≈ 0.58) lands in the upper midtones, 1000 cd/m²
        // (PQ ≈ 0.75) at or near white.
        assert!((150..=230).contains(&lut[148]));
        assert!(lut[192] >= 250);
    }
}
