// SPDX-License-Identifier: MPL-2.0
//! Raw C ABI of the decoding and scaling functions [`crate::FrameGrabber`]
//! uses (PLAY-009), loaded separately from the prober's [`crate::sys`]
//! table: probing must keep working with an FFmpeg that has no libswscale.
//!
//! Only the `AVFrame` prefix up to `format` is mirrored. Those fields have
//! not moved since FFmpeg 1.x; the offsets are pinned by the test below
//! against the FFmpeg 7.1, 8.0 and 9.0 headers. Codec contexts stay opaque:
//! decoder options go through the `avcodec_open2` dictionary.

#![allow(non_camel_case_types, non_snake_case, missing_docs)]

use std::ffi::{c_int, c_void};
use std::path::Path;

use crate::sys::{AVCodecParameters, AVDictionary, AVFormatContext, AVPacket};

/// `AV_PIX_FMT_RGB24`.
pub const AV_PIX_FMT_RGB24: c_int = 2;
/// `AV_PKT_FLAG_KEY`.
pub const AV_PKT_FLAG_KEY: c_int = 1;
/// `AVSEEK_FLAG_BACKWARD`.
pub const AVSEEK_FLAG_BACKWARD: c_int = 1;
/// `AVSEEK_FLAG_BYTE`.
pub const AVSEEK_FLAG_BYTE: c_int = 2;
/// `enum AVDiscard`: `AVDISCARD_ALL`.
pub const AVDISCARD_ALL: c_int = 48;
/// `SWS_AREA` (area averaging: good quality for large downscales).
pub const SWS_AREA: c_int = 1 << 5;
/// `SWS_CS_*` of `sws_getCoefficients`.
pub const SWS_CS_ITU709: c_int = 1;
pub const SWS_CS_DEFAULT: c_int = 5;
pub const SWS_CS_BT2020: c_int = 9;
/// `AVCOL_SPC_*`.
pub const AVCOL_SPC_BT709: c_int = 1;
pub const AVCOL_SPC_BT2020_NCL: c_int = 9;
pub const AVCOL_SPC_BT2020_CL: c_int = 10;
/// `AVCOL_RANGE_JPEG` (full range).
pub const AVCOL_RANGE_JPEG: c_int = 2;
/// `AVCOL_TRC_SMPTE2084` (PQ).
pub const AVCOL_TRC_SMPTE2084: c_int = 16;

/// `AVERROR(EAGAIN)`: `EAGAIN` is 35 on Apple platforms, 11 elsewhere
/// (Linux, Android, Windows' CRT).
pub const AVERROR_EAGAIN: c_int = if cfg!(target_vendor = "apple") {
    -35
} else {
    -11
};
/// `AVERROR_EOF` (`-MKTAG('E','O','F',' ')`).
pub const AVERROR_EOF: c_int = -0x2046_4F45;

/// `AVFrame` prefix, up to `format`.
#[repr(C)]
pub struct AVFrame {
    pub data: [*mut u8; 8],
    pub linesize: [c_int; 8],
    pub extended_data: *mut *mut u8,
    pub width: c_int,
    pub height: c_int,
    pub nb_samples: c_int,
    pub format: c_int,
}

/// Opaque `AVCodec`, `AVCodecContext` and `SwsContext`.
pub type Codec = c_void;
pub type CodecContext = c_void;
pub type SwsContext = c_void;

type Dict = *mut AVDictionary;

/// Every function frame extraction calls.
#[derive(Clone, Copy)]
pub struct DecodeFunctions {
    // libavutil
    pub av_frame_alloc: unsafe extern "C" fn() -> *mut AVFrame,
    pub av_frame_free: unsafe extern "C" fn(*mut *mut AVFrame),
    pub av_frame_unref: unsafe extern "C" fn(*mut AVFrame),
    // libavcodec
    pub avcodec_find_decoder: unsafe extern "C" fn(c_int) -> *const Codec,
    pub avcodec_alloc_context3: unsafe extern "C" fn(*const Codec) -> *mut CodecContext,
    pub avcodec_free_context: unsafe extern "C" fn(*mut *mut CodecContext),
    pub avcodec_parameters_to_context:
        unsafe extern "C" fn(*mut CodecContext, *const AVCodecParameters) -> c_int,
    pub avcodec_open2: unsafe extern "C" fn(*mut CodecContext, *const Codec, *mut Dict) -> c_int,
    pub avcodec_send_packet: unsafe extern "C" fn(*mut CodecContext, *const AVPacket) -> c_int,
    pub avcodec_receive_frame: unsafe extern "C" fn(*mut CodecContext, *mut AVFrame) -> c_int,
    pub avcodec_flush_buffers: unsafe extern "C" fn(*mut CodecContext),
    // libavformat
    pub av_seek_frame: unsafe extern "C" fn(*mut AVFormatContext, c_int, i64, c_int) -> c_int,
    // libswscale
    pub sws_getContext: unsafe extern "C" fn(
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        *mut c_void,
        *mut c_void,
        *const f64,
    ) -> *mut SwsContext,
    pub sws_freeContext: unsafe extern "C" fn(*mut SwsContext),
    pub sws_scale: unsafe extern "C" fn(
        *mut SwsContext,
        *const *const u8,
        *const c_int,
        c_int,
        c_int,
        *const *mut u8,
        *const c_int,
    ) -> c_int,
    pub sws_getCoefficients: unsafe extern "C" fn(c_int) -> *const c_int,
    pub sws_setColorspaceDetails: unsafe extern "C" fn(
        *mut SwsContext,
        *const c_int,
        c_int,
        *const c_int,
        c_int,
        c_int,
        c_int,
        c_int,
    ) -> c_int,
}

/// [`DecodeFunctions`] with the libswscale they partly come from.
pub struct DecodeApi {
    _swscale: libloading::Library,
    pub functions: DecodeFunctions,
}

impl DecodeApi {
    /// Loads libswscale from `swscale` and resolves every function.
    ///
    /// # Safety
    ///
    /// The libraries are FFmpeg's libavutil, libavcodec and libavformat of
    /// one release line, and `swscale` names libswscale of the same line.
    pub(crate) unsafe fn load(
        avutil: &libloading::Library,
        avcodec: &libloading::Library,
        avformat: &libloading::Library,
        swscale: &Path,
    ) -> Result<Self, String> {
        // SAFETY: loading runs libswscale's initialisers only.
        let sws = unsafe { libloading::Library::new(swscale) }
            .map_err(|e| format!("{}: {e}", swscale.display()))?;
        macro_rules! sym {
            // SAFETY: the caller guarantees the library is the FFmpeg one,
            // whose function has the C type of the field it is assigned to.
            ($lib:expr, $name:literal) => {
                unsafe { get($lib, concat!($name, "\0")) }?
            };
        }
        let functions = DecodeFunctions {
            av_frame_alloc: sym!(avutil, "av_frame_alloc"),
            av_frame_free: sym!(avutil, "av_frame_free"),
            av_frame_unref: sym!(avutil, "av_frame_unref"),
            avcodec_find_decoder: sym!(avcodec, "avcodec_find_decoder"),
            avcodec_alloc_context3: sym!(avcodec, "avcodec_alloc_context3"),
            avcodec_free_context: sym!(avcodec, "avcodec_free_context"),
            avcodec_parameters_to_context: sym!(avcodec, "avcodec_parameters_to_context"),
            avcodec_open2: sym!(avcodec, "avcodec_open2"),
            avcodec_send_packet: sym!(avcodec, "avcodec_send_packet"),
            avcodec_receive_frame: sym!(avcodec, "avcodec_receive_frame"),
            avcodec_flush_buffers: sym!(avcodec, "avcodec_flush_buffers"),
            av_seek_frame: sym!(avformat, "av_seek_frame"),
            sws_getContext: sym!(&sws, "sws_getContext"),
            sws_freeContext: sym!(&sws, "sws_freeContext"),
            sws_scale: sym!(&sws, "sws_scale"),
            sws_getCoefficients: sym!(&sws, "sws_getCoefficients"),
            sws_setColorspaceDetails: sym!(&sws, "sws_setColorspaceDetails"),
        };
        Ok(Self {
            _swscale: sws,
            functions,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    /// `offsetof(AVFrame, …)` against the FFmpeg 7.1, 8.0 and 9.0 headers,
    /// 64-bit targets.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn frame_offsets_match_ffmpeg_headers() {
        assert_eq!(offset_of!(AVFrame, linesize), 64);
        assert_eq!(offset_of!(AVFrame, extended_data), 96);
        assert_eq!(offset_of!(AVFrame, width), 104);
        assert_eq!(offset_of!(AVFrame, height), 108);
        assert_eq!(offset_of!(AVFrame, format), 116);
    }

    #[test]
    fn error_codes() {
        assert_eq!(AVERROR_EOF, -i32::from_le_bytes(*b"EOF "));
        const { assert!(AVERROR_EAGAIN < 0) };
    }
}
