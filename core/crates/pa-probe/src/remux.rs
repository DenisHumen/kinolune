// SPDX-License-Identifier: MPL-2.0
//! Stream-copy remux of downloaded HLS segments into one MKV or MP4
//! (TWI-017), over the same runtime-loaded LGPL FFmpeg as the prober.
//!
//! The segments are read as one byte stream through our own
//! `AVIOContext` ([`ConcatFiles`], one file open at a time, so a 48-hour
//! VOD of 17 000 segments does not hit the open-file limit): MPEG-TS
//! segments, and fMP4 fragments after their init segment, are valid when
//! concatenated, so no copy of the whole download is made before the
//! remux. Only video and audio streams are
//! copied (Twitch's timed ID3 data stream has no place in MKV or MP4);
//! nothing is decoded or encoded. Timestamps start at zero.
//!
//! Global tags and chapters come from an `ffmetadata` file, parsed by
//! FFmpeg's own `ffmetadata` demuxer. Attachments become Matroska
//! attachments. MP4 output is written with
//! `movflags=+faststart+use_metadata_tags`, which keeps custom tags
//! (`mdta` keys) but has no iTunes `covr` atom, so attachments are not
//! written into MP4. ADTS AAC is converted by the muxers' automatic
//! bitstream filters.
//!
//! The remux functions are resolved on first use, like the decoder ones
//! ([`crate::FrameGrabber`]), so probing does not depend on them.

#![allow(non_snake_case)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex};

use pa_player_io::ffmpeg::AvioContext;
use pa_vfs::{ByteSource, ErrorKind, VfsError};

use crate::error::{ProbeError, ProbeErrorKind};
use crate::library::Ffmpeg;
use crate::sys::{
    AV_DISPOSITION_ATTACHED_PIC, AV_NOPTS_VALUE, AVChapter, AVCodecParameters,
    AVDictionary, AVFormatContext, AVInputFormat, AVMEDIA_TYPE_ATTACHMENT, AVMEDIA_TYPE_AUDIO,
    AVMEDIA_TYPE_VIDEO, AVPacket, AVRational, AVStream,
};

/// `AVIO_FLAG_WRITE`.
const AVIO_FLAG_WRITE: c_int = 2;
/// `AV_INPUT_BUFFER_PADDING_SIZE` (64 since FFmpeg 4.0).
const PADDING: usize = 64;
/// `AV_TIME_BASE_Q`.
const MICROS: AVRational = AVRational {
    num: 1,
    den: 1_000_000,
};
/// `AVERROR_EOF`.
const AVERROR_EOF: c_int = -0x2046_4F45;

type Dict = *mut AVDictionary;

/// Output container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemuxFormat {
    /// Matroska (`.mkv`): chapters, tags and attachments; the default.
    Matroska,
    /// MP4 (`.mp4`) with `+faststart`, for AVPlayer and old TVs.
    Mp4,
}

impl RemuxFormat {
    /// libavformat muxer name.
    #[must_use]
    pub fn muxer(self) -> &'static str {
        match self {
            RemuxFormat::Matroska => "matroska",
            RemuxFormat::Mp4 => "mp4",
        }
    }

    /// File extension without the dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            RemuxFormat::Matroska => "mkv",
            RemuxFormat::Mp4 => "mp4",
        }
    }
}

/// A file embedded into the output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemuxAttachment {
    /// File name inside the container (`cover.jpg`).
    pub file_name: String,
    /// MIME type (`image/jpeg`).
    pub mime_type: String,
    pub data: Vec<u8>,
}

/// What to remux.
pub struct RemuxRequest<'a> {
    /// Input files, read as one byte stream in this order (an fMP4 init
    /// segment first, if any).
    pub inputs: &'a [PathBuf],
    /// Output file; replaced if it exists.
    pub output: &'a Path,
    pub format: RemuxFormat,
    /// `ffmetadata` file with global tags and chapters, if any.
    pub metadata: Option<&'a Path>,
    /// Matroska attachments; ignored for MP4.
    pub attachments: &'a [RemuxAttachment],
    /// Drop packets at or after this time (ms from the start of the
    /// output): exact end of a partial download.
    pub end_ms: Option<u64>,
    /// Polled between packets; `true` aborts with
    /// [`ProbeErrorKind::Cancelled`].
    pub cancel: Option<&'a dyn Fn() -> bool>,
    /// Called now and then with the media time written so far (ms).
    pub progress: Option<&'a dyn Fn(u64)>,
}

/// What a remux produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RemuxReport {
    /// End of the last written packet, ms from the start.
    pub duration_ms: u64,
    pub packets: u64,
    /// Video and audio streams copied.
    pub streams: u32,
    pub chapters: u32,
    pub attachments: u32,
}

/// Functions of the remux, resolved on first use.
#[derive(Clone, Copy)]
pub(crate) struct RemuxApi {
    av_mallocz: unsafe extern "C" fn(usize) -> *mut c_void,
    av_dict_copy: unsafe extern "C" fn(*mut Dict, *const AVDictionary, c_int) -> c_int,
    av_dynarray_add: unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_void),
    av_rescale_q: unsafe extern "C" fn(i64, AVRational, AVRational) -> i64,
    avcodec_parameters_copy:
        unsafe extern "C" fn(*mut AVCodecParameters, *const AVCodecParameters) -> c_int,
    av_packet_rescale_ts: unsafe extern "C" fn(*mut AVPacket, AVRational, AVRational),
    av_find_input_format: unsafe extern "C" fn(*const c_char) -> *const AVInputFormat,
    avformat_alloc_output_context2: unsafe extern "C" fn(
        *mut *mut AVFormatContext,
        *const c_void,
        *const c_char,
        *const c_char,
    ) -> c_int,
    avformat_new_stream: unsafe extern "C" fn(*mut AVFormatContext, *const c_void) -> *mut AVStream,
    avformat_free_context: unsafe extern "C" fn(*mut AVFormatContext),
    avformat_write_header: unsafe extern "C" fn(*mut AVFormatContext, *mut Dict) -> c_int,
    av_interleaved_write_frame: unsafe extern "C" fn(*mut AVFormatContext, *mut AVPacket) -> c_int,
    av_write_trailer: unsafe extern "C" fn(*mut AVFormatContext) -> c_int,
    avio_open: unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int) -> c_int,
    avio_closep: unsafe extern "C" fn(*mut *mut c_void) -> c_int,
}

impl RemuxApi {
    /// Resolves every function.
    ///
    /// # Safety
    ///
    /// The libraries are FFmpeg's libavutil, libavcodec and libavformat of
    /// one release line.
    pub(crate) unsafe fn load(
        avutil: &libloading::Library,
        avcodec: &libloading::Library,
        avformat: &libloading::Library,
    ) -> Result<Self, String> {
        macro_rules! sym {
            // SAFETY: the caller guarantees the library is the FFmpeg one,
            // whose function has the C type of the field it is assigned to.
            ($lib:expr, $name:literal) => {
                unsafe { get($lib, concat!($name, "\0")) }?
            };
        }
        Ok(Self {
            av_mallocz: sym!(avutil, "av_mallocz"),
            av_dict_copy: sym!(avutil, "av_dict_copy"),
            av_dynarray_add: sym!(avutil, "av_dynarray_add"),
            av_rescale_q: sym!(avutil, "av_rescale_q"),
            avcodec_parameters_copy: sym!(avcodec, "avcodec_parameters_copy"),
            av_packet_rescale_ts: sym!(avcodec, "av_packet_rescale_ts"),
            av_find_input_format: sym!(avformat, "av_find_input_format"),
            avformat_alloc_output_context2: sym!(avformat, "avformat_alloc_output_context2"),
            avformat_new_stream: sym!(avformat, "avformat_new_stream"),
            avformat_free_context: sym!(avformat, "avformat_free_context"),
            avformat_write_header: sym!(avformat, "avformat_write_header"),
            av_interleaved_write_frame: sym!(avformat, "av_interleaved_write_frame"),
            av_write_trailer: sym!(avformat, "av_write_trailer"),
            avio_open: sym!(avformat, "avio_open"),
            avio_closep: sym!(avformat, "avio_closep"),
        })
    }
}

/// Resolves one symbol as a value of type `T` (a function pointer).
///
/// # Safety
///
/// The symbol must have type `T`.
unsafe fn get<T: Copy>(lib: &libloading::Library, name: &str) -> Result<T, String> {
    // SAFETY: forwarded to the caller.
    match unsafe { lib.get::<T>(name.as_bytes()) } {
        Ok(symbol) => Ok(*symbol),
        Err(e) => Err(format!("{}: {e}", name.trim_end_matches('\0'))),
    }
}

fn cstring(s: &str) -> Result<CString, ProbeError> {
    CString::new(s).map_err(|_| ProbeError::new(ProbeErrorKind::InvalidData, "NUL in a string"))
}

fn path_url(path: &Path) -> Result<CString, ProbeError> {
    // `file:` keeps a path with a colon from being taken for a protocol.
    cstring(&format!("file:{}", path.display()))
}

/// Files read as one stream, at most one of them open at a time.
#[derive(Debug)]
pub struct ConcatFiles {
    files: Vec<PathBuf>,
    /// Start offset of each file, plus the total size at the end.
    starts: Vec<u64>,
    open: Mutex<Option<(usize, File)>>,
}

impl ConcatFiles {
    /// The files in order; their sizes are read now.
    ///
    /// # Errors
    ///
    /// The I/O error of a missing or unreadable file.
    pub fn new(files: &[PathBuf]) -> std::io::Result<Self> {
        let mut starts = Vec::with_capacity(files.len() + 1);
        let mut at = 0u64;
        for f in files {
            starts.push(at);
            at += std::fs::metadata(f)?.len();
        }
        starts.push(at);
        Ok(Self {
            files: files.to_vec(),
            starts,
            open: Mutex::new(None),
        })
    }
}

impl ByteSource for ConcatFiles {
    fn size(&self) -> u64 {
        self.starts.last().copied().unwrap_or(0)
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> pa_vfs::Result<usize> {
        if offset >= self.size() || buf.is_empty() || self.files.is_empty() {
            return Ok(0);
        }
        // Last file starting at or before `offset` (skips empty files).
        let i = self.starts.partition_point(|&s| s <= offset) - 1;
        let i = i.min(self.files.len() - 1);
        let within = offset - self.starts[i];
        let len = (self.starts[i + 1] - offset).min(buf.len() as u64) as usize;
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if open.as_ref().is_none_or(|(n, _)| *n != i) {
            let file = File::open(&self.files[i])
                .map_err(|e| VfsError::from_io(e, self.files[i].display()))?;
            *open = Some((i, file));
        }
        let Some((_, file)) = open.as_mut() else {
            return Err(VfsError::new(ErrorKind::Other, "no open file"));
        };
        file.seek(SeekFrom::Start(within))
            .and_then(|_| file.read(&mut buf[..len]))
            .map_err(|e| VfsError::from_io(e, self.files[i].display()))
    }
}

/// Everything a remux opens, freed on every exit path.
struct Session<'a> {
    ff: &'a Ffmpeg,
    api: &'a RemuxApi,
    input: *mut AVFormatContext,
    meta: *mut AVFormatContext,
    output: *mut AVFormatContext,
    packet: *mut AVPacket,
    /// The input's AVIO; closed after `input` (in `drop`).
    io: Option<AvioContext>,
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        let f = &self.ff.functions;
        // SAFETY: each pointer is null or was produced by the matching
        // allocator and is freed exactly once here.
        unsafe {
            if !self.packet.is_null() {
                (f.av_packet_free)(&raw mut self.packet);
            }
            if !self.output.is_null() {
                if !(*self.output).pb.is_null() {
                    (self.api.avio_closep)(&raw mut (*self.output).pb);
                }
                (self.api.avformat_free_context)(self.output);
            }
            if !self.meta.is_null() {
                (f.avformat_close_input)(&raw mut self.meta);
            }
            if !self.input.is_null() {
                (f.avformat_close_input)(&raw mut self.input);
            }
        }
        self.io = None;
    }
}

impl Session<'_> {
    fn error(&self, what: &str, ret: c_int) -> ProbeError {
        let mut buf = [0 as c_char; 128];
        // SAFETY: `buf` holds 128 bytes; av_strerror NUL-terminates.
        let text = unsafe {
            (self.ff.functions.av_strerror)(ret, buf.as_mut_ptr(), buf.len());
            CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
        };
        let kind = if what.starts_with("open input") || what.starts_with("read") {
            ProbeErrorKind::InvalidData
        } else {
            ProbeErrorKind::Source
        };
        ProbeError::new(kind, format!("remux: {what}: {text}"))
    }
}

impl Ffmpeg {
    /// Remuxes `request.inputs` into one file without re-encoding (see the
    /// module docs).
    ///
    /// # Errors
    ///
    /// [`ProbeErrorKind::LibraryUnavailable`] without FFmpeg or a needed
    /// function, [`ProbeErrorKind::InvalidData`] if the inputs are not
    /// media, [`ProbeErrorKind::Source`] for write errors and
    /// [`ProbeErrorKind::Cancelled`].
    pub fn remux(&self, request: &RemuxRequest<'_>) -> Result<RemuxReport, ProbeError> {
        let api = self.remux_api()?;
        if request.inputs.is_empty() {
            return Err(ProbeError::new(ProbeErrorKind::InvalidData, "remux: no inputs"));
        }
        let files = ConcatFiles::new(request.inputs)
            .map_err(|e| ProbeError::new(ProbeErrorKind::Source, format!("remux input: {e}")))?;
        // SAFETY: the AVIO functions come from this loaded FFmpeg.
        let io = unsafe { AvioContext::new(self.functions.avio_api(), Arc::new(files), 256 * 1024) }
            .map_err(|e| ProbeError::new(ProbeErrorKind::LibraryUnavailable, e.to_string()))?;
        let mut s = Session {
            ff: self,
            api,
            input: null_mut(),
            meta: null_mut(),
            output: null_mut(),
            packet: null_mut(),
            io: Some(io),
        };
        // SAFETY: every call below follows the libavformat API contract;
        // struct fields are read and written through the prefixes mirrored
        // in `sys`, which are pinned against the supported headers.
        unsafe { remux_session(&mut s, request) }
    }
}

unsafe fn remux_session(
    s: &mut Session<'_>,
    request: &RemuxRequest<'_>,
) -> Result<RemuxReport, ProbeError> {
    let f = s.ff.functions;
    let api = *s.api;
    let mut report = RemuxReport::default();

    // Input: the segments as one stream through our AVIO.
    s.input = unsafe { (f.avformat_alloc_context)() };
    if s.input.is_null() {
        return Err(s.error("open input", -12));
    }
    let pb = s.io.as_ref().map_or(null_mut(), AvioContext::as_ptr);
    // SAFETY: `pb` is our live AVIOContext; avformat_open_input sets
    // AVFMT_FLAG_CUSTOM_IO and does not free it. On failure FFmpeg frees
    // the context and nulls `s.input`.
    let ret = unsafe {
        (*s.input).pb = pb;
        (f.avformat_open_input)(&raw mut s.input, null(), null(), null_mut())
    };
    if ret < 0 {
        return Err(s.error("open input", ret));
    }
    let ret = unsafe { (f.avformat_find_stream_info)(s.input, null_mut()) };
    if ret < 0 {
        return Err(s.error("read stream info", ret));
    }

    // Metadata and chapters.
    if let Some(meta) = request.metadata {
        let name = cstring("ffmetadata")?;
        let fmt = unsafe { (api.av_find_input_format)(name.as_ptr()) };
        if fmt.is_null() {
            return Err(ProbeError::new(
                ProbeErrorKind::LibraryUnavailable,
                "remux: FFmpeg has no ffmetadata demuxer",
            ));
        }
        let url = path_url(meta)?;
        let ret = unsafe {
            (f.avformat_open_input)(&raw mut s.meta, url.as_ptr(), fmt.cast(), null_mut())
        };
        if ret < 0 {
            return Err(s.error("open input ffmetadata", ret));
        }
    }

    // Output.
    let _ = std::fs::remove_file(request.output);
    let muxer = cstring(request.format.muxer())?;
    let out_url = path_url(request.output)?;
    let ret = unsafe {
        (api.avformat_alloc_output_context2)(
            &raw mut s.output,
            null(),
            muxer.as_ptr(),
            out_url.as_ptr(),
        )
    };
    if ret < 0 || s.output.is_null() {
        return Err(s.error("create output", ret.min(-1)));
    }
    let input = s.input;
    let output = s.output;

    // Streams: video and audio only.
    let n_in = unsafe { (*input).nb_streams } as usize;
    let mut map: Vec<Option<(c_int, AVRational)>> = vec![None; n_in];
    for (i, slot) in map.iter_mut().enumerate() {
        let in_st = unsafe { *(*input).streams.add(i) };
        let par = unsafe { (*in_st).codecpar };
        let kind = unsafe { (*par).codec_type };
        if kind != AVMEDIA_TYPE_VIDEO && kind != AVMEDIA_TYPE_AUDIO {
            continue;
        }
        if unsafe { (*in_st).disposition } & AV_DISPOSITION_ATTACHED_PIC != 0 {
            continue;
        }
        let out_st = unsafe { (api.avformat_new_stream)(output, null()) };
        if out_st.is_null() {
            return Err(s.error("new stream", -12));
        }
        let ret = unsafe { (api.avcodec_parameters_copy)((*out_st).codecpar, par) };
        if ret < 0 {
            return Err(s.error("copy codec parameters", ret));
        }
        unsafe {
            (*(*out_st).codecpar).codec_tag = 0;
            (*out_st).time_base = (*in_st).time_base;
        }
        *slot = Some((unsafe { (*out_st).index }, unsafe { (*in_st).time_base }));
        report.streams += 1;
    }
    if report.streams == 0 {
        return Err(ProbeError::new(
            ProbeErrorKind::InvalidData,
            "remux: no audio or video stream in the input",
        ));
    }

    // Tags and chapters from ffmetadata.
    if !s.meta.is_null() {
        let meta = s.meta;
        unsafe {
            (api.av_dict_copy)(&raw mut (*output).metadata, (*meta).metadata, 0);
            for i in 0..(*meta).nb_chapters as usize {
                let src = *(*meta).chapters.add(i);
                let ch = (api.av_mallocz)(std::mem::size_of::<AVChapter>()).cast::<AVChapter>();
                if ch.is_null() {
                    return Err(s.error("allocate chapter", -12));
                }
                (*ch).id = (*src).id;
                (*ch).time_base = (*src).time_base;
                (*ch).start = (*src).start;
                (*ch).end = (*src).end;
                (api.av_dict_copy)(&raw mut (*ch).metadata, (*src).metadata, 0);
                (api.av_dynarray_add)(
                    (&raw mut (*output).chapters).cast(),
                    (&raw mut (*output).nb_chapters).cast::<c_int>(),
                    ch.cast(),
                );
            }
            report.chapters = (*output).nb_chapters as u32;
        }
    }

    // Attachments (Matroska only).
    if request.format == RemuxFormat::Matroska {
        for att in request.attachments {
            let st = unsafe { (api.avformat_new_stream)(output, null()) };
            if st.is_null() {
                return Err(s.error("new attachment stream", -12));
            }
            unsafe {
                let par = (*st).codecpar;
                (*par).codec_type = AVMEDIA_TYPE_ATTACHMENT;
                set_extradata(&api, par, &att.data)
                    .ok_or_else(|| s.error("allocate attachment", -12))?;
                dict_set(&f, &raw mut (*st).metadata, "filename", &att.file_name)?;
                dict_set(&f, &raw mut (*st).metadata, "mimetype", &att.mime_type)?;
            }
            report.attachments += 1;
        }
    }

    // Header.
    let ret = unsafe { (api.avio_open)(&raw mut (*output).pb, out_url.as_ptr(), AVIO_FLAG_WRITE) };
    if ret < 0 {
        return Err(s.error("open output", ret));
    }
    let mut opts: Dict = null_mut();
    if request.format == RemuxFormat::Mp4 {
        // `use_metadata_tags`: keep the custom `TWITCH_*` tags.
        unsafe { dict_set(&f, &raw mut opts, "movflags", "+faststart+use_metadata_tags")? };
    }
    let ret = unsafe { (api.avformat_write_header)(output, &raw mut opts) };
    unsafe { (f.av_dict_free)(&raw mut opts) };
    if ret < 0 {
        return Err(s.error("write header", ret));
    }

    s.packet = unsafe { (f.av_packet_alloc)() };
    if s.packet.is_null() {
        return Err(s.error("allocate packet", -12));
    }
    let pkt = s.packet;

    // Packets.
    let start_us = match unsafe { (*input).start_time } {
        AV_NOPTS_VALUE => 0,
        t => t,
    };
    let end_us = request
        .end_ms
        .map(|ms| i64::try_from(ms.saturating_mul(1000)).unwrap_or(i64::MAX));
    let mut last_report_us = 0i64;
    let mut max_end_us = 0i64;
    loop {
        if request.cancel.is_some_and(|c| c()) {
            return Err(ProbeError::new(ProbeErrorKind::Cancelled, "remux cancelled"));
        }
        let ret = unsafe { (f.av_read_frame)(input, pkt) };
        if ret == AVERROR_EOF {
            break;
        }
        if ret < 0 {
            return Err(s.error("read packet", ret));
        }
        let idx = unsafe { (*pkt).stream_index } as usize;
        let Some(Some((out_index, in_tb))) = map.get(idx).copied() else {
            unsafe { (f.av_packet_unref)(pkt) };
            continue;
        };
        let offset = unsafe { (api.av_rescale_q)(start_us, MICROS, in_tb) };
        unsafe {
            if (*pkt).pts != AV_NOPTS_VALUE {
                (*pkt).pts -= offset;
            }
            if (*pkt).dts != AV_NOPTS_VALUE {
                (*pkt).dts -= offset;
            }
        }
        let ts = unsafe {
            if (*pkt).pts == AV_NOPTS_VALUE {
                (*pkt).dts
            } else {
                (*pkt).pts
            }
        };
        if ts != AV_NOPTS_VALUE {
            let us = unsafe { (api.av_rescale_q)(ts, in_tb, MICROS) };
            if end_us.is_some_and(|end| us >= end) {
                unsafe { (f.av_packet_unref)(pkt) };
                continue;
            }
            let dur = unsafe { (api.av_rescale_q)(packet_duration(pkt), in_tb, MICROS) };
            max_end_us = max_end_us.max(us.saturating_add(dur.max(0)));
        }
        let out_st = unsafe { *(*output).streams.add(out_index as usize) };
        unsafe {
            (*pkt).stream_index = out_index;
            (api.av_packet_rescale_ts)(pkt, in_tb, (*out_st).time_base);
        }
        let ret = unsafe { (api.av_interleaved_write_frame)(output, pkt) };
        if ret < 0 {
            return Err(s.error("write packet", ret));
        }
        report.packets += 1;
        if let Some(progress) = request.progress
            && max_end_us - last_report_us >= 10_000_000
        {
            last_report_us = max_end_us;
            progress(u64::try_from(max_end_us / 1000).unwrap_or(0));
        }
    }
    let ret = unsafe { (api.av_write_trailer)(output) };
    if ret < 0 {
        return Err(s.error("write trailer", ret));
    }
    let ret = unsafe { (api.avio_closep)(&raw mut (*output).pb) };
    if ret < 0 {
        return Err(s.error("close output", ret));
    }
    report.duration_ms = u64::try_from(max_end_us / 1000).unwrap_or(0);
    Ok(report)
}

/// `AVPacket.duration`: right after `side_data_elems` (an `int`) that
/// follows `side_data` after `flags`; 64-bit aligned. Stable since FFmpeg
/// 4.x: `flags` at 40, `side_data` at 48, `side_data_elems` at 56,
/// `duration` at 64 on 64-bit targets.
unsafe fn packet_duration(pkt: *const AVPacket) -> i64 {
    #[cfg(target_pointer_width = "64")]
    const OFFSET: usize = 64;
    #[cfg(target_pointer_width = "32")]
    const OFFSET: usize = 48;
    // SAFETY: `pkt` is a live AVPacket; the offset is that of `duration`.
    unsafe { pkt.cast::<u8>().add(OFFSET).cast::<i64>().read_unaligned() }
}

unsafe fn set_extradata(api: &RemuxApi, par: *mut AVCodecParameters, data: &[u8]) -> Option<()> {
    let size = c_int::try_from(data.len()).ok()?;
    // SAFETY: av_mallocz returns zeroed memory of the requested size; the
    // codec parameters own it from here on (freed with the context).
    unsafe {
        let buf = (api.av_mallocz)(data.len() + PADDING).cast::<u8>();
        if buf.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len());
        (*par).extradata = buf;
        (*par).extradata_size = size;
    }
    Some(())
}

unsafe fn dict_set(
    f: &crate::sys::Functions,
    dict: *mut Dict,
    key: &str,
    value: &str,
) -> Result<(), ProbeError> {
    let (k, v) = (cstring(key)?, cstring(value)?);
    // SAFETY: av_dict_set copies both strings (flags 0).
    let ret = unsafe { (f.av_dict_set)(dict, k.as_ptr(), v.as_ptr(), 0) };
    if ret < 0 {
        return Err(ProbeError::new(ProbeErrorKind::Source, "remux: av_dict_set failed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concatenated_files_read_across_boundaries() {
        let dir = std::env::temp_dir().join(format!("pa-probe-concat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files: Vec<PathBuf> = ["abc", "", "defg", "h"]
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let p = dir.join(format!("{i}.seg"));
                std::fs::write(&p, text).unwrap();
                p
            })
            .collect();
        let src = ConcatFiles::new(&files).unwrap();
        assert_eq!(src.size(), 8);
        let mut all = vec![0; 8];
        src.read_exact_at(0, &mut all).unwrap();
        assert_eq!(all, b"abcdefgh");
        let mut buf = [0; 4];
        assert_eq!(src.read_at(2, &mut buf).unwrap(), 1, "stops at a file end");
        assert_eq!(src.read_at(3, &mut buf).unwrap(), 4);
        assert_eq!(&buf, b"defg");
        assert_eq!(src.read_at(8, &mut buf).unwrap(), 0);
        assert!(ConcatFiles::new(&[dir.join("missing")]).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn formats() {
        assert_eq!(RemuxFormat::Matroska.muxer(), "matroska");
        assert_eq!(RemuxFormat::Mp4.extension(), "mp4");
    }
}
