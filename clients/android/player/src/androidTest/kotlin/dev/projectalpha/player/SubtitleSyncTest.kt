// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import android.graphics.Bitmap
import android.os.SystemClock
import android.util.Log
import androidx.test.platform.app.InstrumentationRegistry
import dev.projectalpha.core.MediaSource
import dev.projectalpha.core.OpenRequest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import kotlin.math.abs

/**
 * PLAY-AND-006 "done when": ASS karaoke is in sync with the video within ±1 frame.
 *
 * karaoke.mkv (tools/make-fixtures.sh) shows its frame number in binary at the top, and its ASS track sings one 12 px
 * square per frame (`\k4` at 25 fps). Screenshots of the display (SurfaceFlinger: the video layer and the overlay as
 * the viewer sees them) give, for the same instant, the video frame N and the karaoke position K; K − N must stay
 * within one frame of its value on still frames (paused after a seek), which is the libass timing itself.
 */
class SubtitleSyncTest {
    private class Sample(val frame: Int, val karaoke: Int) {
        val diff get() = karaoke - frame
    }

    @Test
    fun karaokeFollowsTheVideoFrames() {
        assumeTrue("libass (libmpv.so) must be in the test APK", LibAss.available)
        EngineUnderTest().use { under ->
            val engine = under.engine
            engine.openMedia(OpenRequest(listOf(MediaSource(Fixtures.uri("karaoke.mkv"), null)), 0u, 0uL, false))
            under.advance(0)
            val ass = engine.state().subtitleTracks.single { it.codec == "ass" }
            assertEquals("ru", ass.language)
            engine.setSubtitleTrack(ass.id)
            under.advance(300)
            assertEquals(ass.id, engine.state().subtitleTrack)

            // Still frames: the offset of the libass timing on this sample.
            val still = listOf(1_120L, 2_440L, 4_800L, 6_960L, 8_000L).map { position ->
                engine.seek(position.toULong())
                under.advance(600)
                val s = capture(under)
                assertNotNull("paused at $position ms: no frame number or karaoke bar on screen", s)
                Log.i(TAG, "paused at $position ms: frame ${s!!.frame}, karaoke ${s.karaoke}")
                assertEquals("paused at $position ms", position / 40, s.frame.toLong())
                s
            }
            val offset = still.first().diff
            assertTrue("still frames: ${still.map { it.diff }}", still.all { it.diff == offset } && abs(offset) <= 1)

            // Playing: every screenshot within one frame of that offset.
            engine.seek(0uL)
            under.advance(300)
            engine.play()
            val playing = ArrayList<Sample>()
            val deadline = SystemClock.elapsedRealtime() + 20_000
            while (SystemClock.elapsedRealtime() < deadline && engine.state().positionMs < 9_400uL && !engine.state().completed) {
                capture(under)?.let { playing.add(it) }
                Thread.sleep(37)
            }
            val diffs = playing.map { it.diff - offset }
            Log.i(TAG, "playing: ${playing.size} screenshots, frames ${playing.firstOrNull()?.frame}..${playing.lastOrNull()?.frame}, K-N-offset histogram ${diffs.groupingBy { it }.eachCount().toSortedMap()}")
            assertTrue("too few screenshots while playing: ${playing.size}", playing.size >= 15)
            assertTrue("video did not advance: ${playing.map { it.frame }}", playing.last().frame - playing.first().frame >= 100)
            val off = playing.filter { abs(it.diff - offset) > 1 }
            assertTrue("out of ±1 frame: ${off.map { "N=${it.frame} K=${it.karaoke}" }}", off.isEmpty())
        }
    }

    /** Frame number and karaoke position on screen now, or null when either cannot be read. */
    private fun capture(under: EngineUnderTest): Sample? {
        val (left, top, w, h) = onMain {
            val loc = IntArray(2)
            under.activity.video.surface.getLocationOnScreen(loc)
            listOf(loc[0], loc[1], under.activity.video.surface.width, under.activity.video.surface.height)
        }
        val shot = InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot() ?: return null
        try {
            val sx = w / 320.0
            val sy = h / 240.0
            fun pixel(x: Double, y: Double): Int = shot.getPixel((left + x * sx).toInt(), (top + y * sy).toInt())
            var frame = 0
            for (bit in 0 until 8) {
                val p = pixel(40.0 * bit + 20, 20.0)
                val luma = ((p shr 16 and 0xFF) + (p shr 8 and 0xFF) + (p and 0xFF)) / 3
                if (luma > 128) frame = frame or (1 shl bit)
            }
            val sung = sung(shot, left, top, sx, sy) ?: return null
            // The bar restarts every second (25 frames): take the second closest to the video frame.
            val index = sung - 1
            val second = Math.round((frame - index) / 25.0).toInt()
            return Sample(frame, second * 25 + index)
        } finally {
            shot.recycle()
        }
    }

    private fun sung(shot: Bitmap, left: Int, top: Int, sx: Double, sy: Double): Int? {
        val y = (top + (Bar.Y + 0.5) * sy).toInt()
        val x0 = (left + 4 * sx).toInt()
        val x1 = (left + 316 * sx).toInt()
        val row = IntArray(x1 - x0)
        shot.getPixels(row, 0, row.size, x0, y, row.size, 1)
        var green = 0
        var magenta = 0
        for (p in row) {
            val r = p shr 16 and 0xFF
            val g = p shr 8 and 0xFF
            val b = p and 0xFF
            if (Bar.isGreen(r, g, b)) green++ else if (Bar.isMagenta(r, g, b)) magenta++
        }
        val square = Bar.SQUARE * sx
        if (green + magenta < Bar.SQUARES * square * 0.9) return null
        return Math.round(green / square).toInt()
    }

    private companion object {
        const val TAG = "PLAYAND006"
    }
}
