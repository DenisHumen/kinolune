// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import android.util.Log
import com.sun.jna.Native
import com.sun.jna.NativeLong
import com.sun.jna.Pointer

/**
 * libass (ISC) through JNA (PLAY-AND-006, ADR-0135).
 *
 * There is no separate copy of libass in the APK: our LGPL `libmpv.so` (tools/mpv, shipped by
 * `media_kit_libs_android_video` in every APK) links libass 0.17 statically and exports its `ass_*` API on Android
 * (no `--exclude-libs` there, unlike Linux; `AssSubtitlesTest.libassComesFromLibmpv` notices if that changes).
 * Direct mapping, like the UniFFI bindings of the core.
 *
 * When `libmpv.so` is missing (a build without it) [available] is false and the engine leaves SSA/ASS to Media3's
 * own parser (text with basic styles, no karaoke).
 */
internal object LibAss {
    /** `ASS_FONTPROVIDER_NONE`: our Android build has no system font provider; fonts come from a directory. */
    const val FONTPROVIDER_NONE = 0

    val available: Boolean by lazy {
        try {
            Native.register(LibAss::class.java, "mpv")
            true
        } catch (e: Throwable) {
            Log.w(TAG, "libass is not available (libmpv.so): ASS goes through Media3", e)
            false
        }
    }

    @JvmStatic external fun ass_library_init(): Pointer?
    @JvmStatic external fun ass_library_done(library: Pointer)
    @JvmStatic external fun ass_set_fonts_dir(library: Pointer, dir: String?)
    @JvmStatic external fun ass_set_extract_fonts(library: Pointer, extract: Int)
    @JvmStatic external fun ass_add_font(library: Pointer, name: String, data: ByteArray, size: Int)

    @JvmStatic external fun ass_renderer_init(library: Pointer): Pointer?
    @JvmStatic external fun ass_renderer_done(renderer: Pointer)
    @JvmStatic external fun ass_set_frame_size(renderer: Pointer, w: Int, h: Int)
    @JvmStatic external fun ass_set_storage_size(renderer: Pointer, w: Int, h: Int)
    @JvmStatic external fun ass_set_fonts(
        renderer: Pointer,
        defaultFont: String?,
        defaultFamily: String?,
        fontProvider: Int,
        config: String?,
        update: Int,
    )
    @JvmStatic external fun ass_set_cache_limits(renderer: Pointer, glyphMax: Int, bitmapMaxSizeMb: Int)

    /** `ASS_Image *ass_render_frame(ASS_Renderer *, ASS_Track *, long long now, int *detect_change)`. */
    @JvmStatic external fun ass_render_frame(renderer: Pointer, track: Pointer, nowMs: Long, detectChange: IntArray): Pointer?

    @JvmStatic external fun ass_new_track(library: Pointer): Pointer?
    @JvmStatic external fun ass_free_track(track: Pointer)
    @JvmStatic external fun ass_process_codec_private(track: Pointer, data: ByteArray, size: Int)
    @JvmStatic external fun ass_process_chunk(track: Pointer, data: ByteArray, size: Int, timecodeMs: Long, durationMs: Long)

    /** Parses a whole .ass/.ssa file; the buffer is changed in place (JNA passes a copy). */
    @JvmStatic external fun ass_read_memory(library: Pointer, buf: ByteArray, size: NativeLong, codepage: String?): Pointer?

    /**
     * Field offsets of `ASS_Image` (ass.h): `int w, h, stride; unsigned char *bitmap; uint32_t color;
     * int dst_x, dst_y; struct ass_image *next; int type;`, laid out by the C ABI of this process.
     */
    object Image {
        private val ptr = Native.POINTER_SIZE
        const val W = 0L
        const val H = 4L
        const val STRIDE = 8L
        val BITMAP: Long = align(12L, ptr)
        val COLOR: Long = BITMAP + ptr
        val DST_X: Long = COLOR + 4
        val DST_Y: Long = COLOR + 8
        val NEXT: Long = align(COLOR + 12, ptr)

        private fun align(offset: Long, to: Int): Long = (offset + to - 1) / to * to
    }

    private const val TAG = "LibAss"
}
