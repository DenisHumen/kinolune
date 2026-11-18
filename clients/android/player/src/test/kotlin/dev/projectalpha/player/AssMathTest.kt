// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** PLAY-AND-006: Matroska SSA samples, blending of libass images, choosing the picture for a vsync. */
class AssMathTest {
    @Test
    fun matroskaSampleBecomesALibassChunk() {
        val sample = "Dialogue: 0:00:00:00,0:00:02:50,7,0,Default,,0,0,0,,{\\k4}Привет".toByteArray()
        val chunk = AssMath.chunk(sample, sample.size)!!
        assertEquals(2_500L, chunk.durationMs)
        assertEquals("7,0,Default,,0,0,0,,{\\k4}Привет", String(sample, chunk.offset, sample.size - chunk.offset))
    }

    @Test
    fun sampleSizeLimitsTheParse() {
        val sample = "Dialogue: 0:00:00:00,1:02:03:04,1,0,S,,0,0,0,,x".toByteArray() + ByteArray(16)
        val chunk = AssMath.chunk(sample, sample.size - 16)!!
        assertEquals(((1 * 60 + 2) * 60 + 3) * 1000L + 40, chunk.durationMs)
    }

    @Test
    fun otherSamplesAreRejected() {
        for (s in listOf("", "Comment: 0:00:00:00,0:00:01:00,x", "Dialogue: 0:00:00:00", "Dialogue: a,b,c")) {
            assertNull(s, AssMath.chunk(s.toByteArray(), s.length))
        }
    }

    @Test
    fun timecodes() {
        assertEquals(1_230L, AssMath.timecodeMs("0:00:01:23"))
        assertEquals(3_723_450L, AssMath.timecodeMs("1:02:03.45"))
        assertNull(AssMath.timecodeMs("1:02:03"))
        assertNull(AssMath.timecodeMs("x:00:00:00"))
    }

    @Test
    fun opaqueImageCoversWithItsColour() {
        val dst = IntArray(4 * 2)
        // libass colour 0xRRGGBBTT: red, opaque.
        AssMath.blend(dst, 4, 10, 20, byteArrayOf(-1, -1, 0, 0), 2, 1, 4, 0xFF000000.toInt(), 11, 21)
        // Box origin (10, 20): the image at (11, 21) lands in row 1, columns 1–2. Memory order RGBA → 0xAABBGGRR.
        assertEquals(listOf(0, 0, 0, 0, 0, 0xFF0000FF.toInt(), 0xFF0000FF.toInt(), 0), dst.toList())
    }

    @Test
    fun translucentImagesArePremultipliedAndStack() {
        val dst = IntArray(1)
        // Green at half transparency (TT = 0x80 → opacity 127).
        AssMath.blend(dst, 1, 0, 0, byteArrayOf(-1), 1, 1, 1, 0x00FF0080, 0, 0)
        val a1 = dst[0] ushr 24
        assertEquals(127, a1)
        assertEquals(127, (dst[0] ushr 8) and 0xFF) // premultiplied green
        assertEquals(0, dst[0] and 0xFF)
        // Opaque blue over it replaces it.
        AssMath.blend(dst, 1, 0, 0, byteArrayOf(-1), 1, 1, 1, 0x0000FF00, 0, 0)
        assertEquals(0xFFFF0000.toInt(), dst[0])
    }

    @Test
    fun coverageScalesAlphaAndClipsToTheBox() {
        val dst = IntArray(2)
        AssMath.blend(dst, 2, 0, 0, byteArrayOf(0x40, -1, -1), 3, 1, 3, 0xFFFFFF00.toInt(), 0, 0)
        assertEquals(0x40, dst[0] ushr 24)
        assertEquals(0x40, dst[0] and 0xFF)
        assertEquals(0xFFFFFFFF.toInt(), dst[1])
        val transparent = IntArray(1)
        AssMath.blend(transparent, 1, 0, 0, byteArrayOf(-1), 1, 1, 1, 0xFFFFFFFF.toInt(), 0, 0)
        assertEquals(0, transparent[0])
    }

    @Test
    fun picturesFollowTheFramesOnScreen() {
        val period = 16_666_667L
        val vsync = 1_000_000_000L
        val present = AssMath.presentTime(vsync, period)
        assertEquals(vsync + 2 * period, present)
        // Released before the vsync, before the present time, exactly at it and after it.
        val releases = listOf(vsync - 30_000_000, vsync + period, present, present + 1)
        assertEquals(2, AssMath.due(releases, present))
        assertEquals(-1, AssMath.due(listOf(present + 1), present))
        assertEquals(-1, AssMath.due(emptyList(), present))
    }
}
