// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import android.net.Uri
import android.os.SystemClock
import androidx.media3.datasource.DefaultDataSource
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File

/**
 * PLAY-AND-006: libass from our libmpv.so through JNA. The test APK carries libmpv and its FFmpeg libraries
 * (`-PpaTestJniDir`), as every Kinolune APK does through media_kit_libs_android_video.
 */
class AssSubtitlesTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun libassComesFromLibmpv() {
        assertTrue("libass (libmpv.so) must be in the test APK: -PpaTestJniDir", LibAss.available)
    }

    @Test
    fun karaokeSyllablesTurnPrimaryFrameByFrame() {
        assumeTrue(LibAss.available)
        AssSubtitles().use { subs ->
            subs.fetchExternal("ext-0", Fixtures.uri("karaoke.ass"), DefaultDataSource.Factory(context))
            val deadline = SystemClock.elapsedRealtime() + 5_000
            while (!subs.startExternal("ext-0", "ru")) {
                assertTrue("external file not read", SystemClock.elapsedRealtime() < deadline)
                Thread.sleep(20)
            }
            for (second in listOf(0, 3, 9)) {
                for (syllable in listOf(0, 1, 12, 24)) {
                    val t = second * 1000L + syllable * 40L + 20
                    val image = render(subs, t)
                    assertEquals("sung squares at $t ms", syllable + 1, Bar.sung(image, scale = 1.0))
                }
            }
            // The rotated sign in DejaVu Sans (a fallback family when the font is not attached) is drawn too.
            val sign = render(subs, 2_000)
            assertTrue("sign above the bar: box from y=${sign.y}", sign.y < 100)
        }
    }

    @Test
    fun fontsAttachedToTheMkvAreFound() {
        val file = File(Uri.parse(Fixtures.uri("karaoke.mkv")).path!!)
        val bytes = file.readBytes()
        val fonts = MkvAttachments.fonts { position, length ->
            if (position >= bytes.size) ByteArray(0) else bytes.copyOfRange(position.toInt(), minOf(bytes.size.toLong(), position + length).toInt())
        }
        assertEquals(listOf("DejaVuSans.ttf"), fonts.map { it.name })
        assertTrue(fonts[0].data.size > 100_000)
    }

    private fun render(subs: AssSubtitles, timeMs: Long): AssSubtitles.Image {
        // Reset the change detection: a fresh size forces a full picture.
        subs.render(timeMs, 1, 1, 320, 240)
        val r = subs.render(timeMs, 320, 240, 320, 240)
        assertTrue("picture at $timeMs ms: $r", r is AssSubtitles.Result.Changed)
        return (r as AssSubtitles.Result.Changed).image
    }
}

/** The karaoke bar of karaoke.mkv/.ass: 25 squares 12 px wide from x = 10, rows 150–174 (PlayRes 320×240). */
internal object Bar {
    const val X = 10
    const val Y = 162
    const val SQUARE = 12
    const val SQUARES = 25

    fun isGreen(r: Int, g: Int, b: Int) = g > 180 && r < 90 && b < 90
    fun isMagenta(r: Int, g: Int, b: Int) = r > 180 && b > 180 && g < 90

    /** Squares in the primary colour (sung) in a libass picture rendered at PlayRes size times [scale]. */
    fun sung(image: AssSubtitles.Image, scale: Double): Int {
        val row = (Y * scale).toInt() - image.y
        require(row in 0 until image.height) { "bar row outside the picture" }
        var green = 0
        var magenta = 0
        for (x in 0 until image.width) {
            val p = image.pixels[row * image.width + x]
            val r = p and 0xFF
            val g = (p ushr 8) and 0xFF
            val b = (p ushr 16) and 0xFF
            if (isGreen(r, g, b)) green++ else if (isMagenta(r, g, b)) magenta++
        }
        require(green + magenta > SQUARES * SQUARE * scale * 0.9) { "no bar: $green green, $magenta magenta" }
        return Math.round(green / (SQUARE * scale)).toInt()
    }
}
