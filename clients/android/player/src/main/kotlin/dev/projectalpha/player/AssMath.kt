// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

/**
 * Pure parts of the ASS overlay (PLAY-AND-006, ADR-0135), covered by JUnit: Matroska SSA samples → libass chunks,
 * blending libass images into a bitmap, which video frame is on screen at a vsync.
 */
internal object AssMath {
    private val DIALOGUE = "Dialogue: ".toByteArray(Charsets.US_ASCII)

    /** One event for `ass_process_chunk`: `ReadOrder,Layer,Style,…,Text` starts at [offset] of the sample. */
    data class Chunk(val offset: Int, val durationMs: Long)

    /**
     * A Matroska SSA/ASS sample as Media3's MatroskaExtractor writes it: `Dialogue: 0:00:00:00,H:MM:SS:CC,` (start
     * relative to the sample time, end = the block duration) followed by the block itself, `ReadOrder,Layer,Style,
     * Name,MarginL,MarginR,MarginV,Effect,Text`, which is what libass takes for a Matroska track. Null when the
     * sample does not look like that.
     */
    fun chunk(data: ByteArray, size: Int): Chunk? {
        if (size < DIALOGUE.size || (DIALOGUE.indices).any { data[it] != DIALOGUE[it] }) return null
        val firstComma = indexOf(data, ',', DIALOGUE.size, size)
        if (firstComma < 0) return null
        val secondComma = indexOf(data, ',', firstComma + 1, size)
        if (secondComma < 0) return null
        val start = timecodeMs(String(data, DIALOGUE.size, firstComma - DIALOGUE.size, Charsets.US_ASCII)) ?: return null
        val end = timecodeMs(String(data, firstComma + 1, secondComma - firstComma - 1, Charsets.US_ASCII)) ?: return null
        return Chunk(secondComma + 1, (end - start).coerceAtLeast(0))
    }

    /** `H:MM:SS:CC` (Media3's Matroska form) or `H:MM:SS.CC` (ASS) in milliseconds. */
    fun timecodeMs(s: String): Long? {
        val parts = s.trim().split(':', '.')
        if (parts.size != 4) return null
        val n = parts.map { it.toLongOrNull() ?: return null }
        if (n.any { it < 0 }) return null
        return ((n[0] * 60 + n[1]) * 60 + n[2]) * 1000 + n[3] * 10
    }

    private fun indexOf(data: ByteArray, c: Char, from: Int, to: Int): Int {
        for (i in from until to) if (data[i] == c.code.toByte()) return i
        return -1
    }

    /**
     * Blends one libass image into [dst], premultiplied pixels of a [dstW]-wide box whose top left is at
     * ([originX], [originY]) of the frame, as Android's ARGB_8888 memory reads them as little-endian ints
     * (`0xAABBGGRR`, so `Bitmap.copyPixelsFromBuffer` takes them as is).
     *
     * [coverage] is the 8-bit alpha map (`stride` bytes per row), [color] libass's `0xRRGGBBTT` with TT the
     * transparency (0 = opaque), ([x], [y]) the image position in the frame.
     */
    fun blend(
        dst: IntArray,
        dstW: Int,
        originX: Int,
        originY: Int,
        coverage: ByteArray,
        w: Int,
        h: Int,
        stride: Int,
        color: Int,
        x: Int,
        y: Int,
    ) {
        val r = color ushr 24
        val g = (color ushr 16) and 0xFF
        val b = (color ushr 8) and 0xFF
        val opacity = 255 - (color and 0xFF)
        if (opacity == 0) return
        val dstH = dst.size / dstW
        for (row in 0 until h) {
            val dy = y + row - originY
            if (dy !in 0 until dstH) continue
            val src = row * stride
            val line = dy * dstW
            for (col in 0 until w) {
                val cov = coverage[src + col].toInt() and 0xFF
                if (cov == 0) continue
                val dx = x + col - originX
                if (dx !in 0 until dstW) continue
                val k = div255(cov * opacity)
                if (k == 0) continue
                val i = line + dx
                val d = dst[i]
                val inv = 255 - k
                val outR = div255(r * k) + div255((d and 0xFF) * inv)
                val outG = div255(g * k) + div255(((d ushr 8) and 0xFF) * inv)
                val outB = div255(b * k) + div255(((d ushr 16) and 0xFF) * inv)
                val outA = k + div255((d ushr 24) * inv)
                dst[i] = (outA shl 24) or (outB shl 16) or (outG shl 8) or outR
            }
        }
    }

    /** Rounded `v / 255` for `v` in 0..65025. */
    private fun div255(v: Int): Int = (v + 128 + ((v + 128) shr 8)) shr 8

    /**
     * Which of the pictures rendered for released video frames (release times in [releases], oldest first) belongs on
     * screen when the overlay drawn now is presented at [presentNs]: SurfaceFlinger shows then the latest video frame
     * released by that time. -1: none is due yet.
     */
    fun due(releases: List<Long>, presentNs: Long): Int {
        var due = -1
        for (i in releases.indices) if (releases[i] <= presentNs) due = i
        return due
    }

    /**
     * When a draw started at the vsync [vsyncNs] reaches the display without frame timelines (API < 33): HWUI
     * records it in this frame, its RenderThread renders it for the next vsync and SurfaceFlinger presents it one
     * vsync after that. Measured on the emulator: one period lagged the video by a frame in a third of the
     * screenshots, two matched every one (ADR-0135).
     */
    fun presentTime(vsyncNs: Long, periodNs: Long): Long = vsyncNs + 2 * periodNs
}
