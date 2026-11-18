// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import android.os.SystemClock
import androidx.media3.common.Player
import androidx.media3.common.text.CueGroup
import dev.projectalpha.core.MediaSource
import dev.projectalpha.core.OpenRequest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

/** PLAY-AND-006: PGS through Media3 into the overlay's SubtitleView; side-loaded ASS through libass. */
class SubtitleTracksTest {
    @Test
    fun pgsBitmapsReachTheOverlay() {
        EngineUnderTest().use { under ->
            val engine = under.engine
            engine.openMedia(OpenRequest(listOf(MediaSource(Fixtures.uri("pgs.mkv"), null)), 0u, 0uL, false))
            under.advance(0)
            val pgs = engine.state().subtitleTracks.single()
            assertEquals("hdmv_pgs_subtitle", pgs.codec)
            engine.setSubtitleTrack(pgs.id)
            under.advance(0)

            // Bitmap width of the shown cue by position: event 1 has one block (60 px), event 2 two (100 px).
            val seen = ArrayList<Pair<Long, Int?>>()
            val listener = object : Player.Listener {
                override fun onCues(cueGroup: CueGroup) {
                    seen.add(engine.exoPlayer.currentPosition to cueGroup.cues.firstOrNull()?.bitmap?.width)
                }
            }
            onMain { engine.exoPlayer.addListener(listener) }
            engine.play()
            val deadline = SystemClock.elapsedRealtime() + 15_000
            while (!engine.state().completed && SystemClock.elapsedRealtime() < deadline) Thread.sleep(100)
            onMain { engine.exoPlayer.removeListener(listener) }

            val widths = seen.map { it.second }
            assertTrue("cues: $seen", widths.contains(60) && widths.contains(100))
            for ((position, width) in seen) {
                when (width) {
                    60 -> assertTrue("event 1 at $position ms", position in 900L..2_600L)
                    100 -> assertTrue("event 2 at $position ms", position in 3_400L..5_100L)
                }
            }
            // Cleared between the events, and the SubtitleView of the overlay got the cues.
            assertTrue("cleared after event 1: $seen", seen.any { it.second == null && it.first in 2_300L..3_600L })
            assertTrue(onMain { under.activity.video.subtitles.cues.isAttachedToWindow })
        }
    }

    @Test
    fun sideLoadedAssGoesToLibass() {
        assumeTrue(LibAss.available)
        EngineUnderTest().use { under ->
            val engine = under.engine
            engine.openMedia(OpenRequest(listOf(MediaSource(Fixtures.uri("h264.mkv"), null)), 0u, 2_000uL, false))
            under.advance(0)
            engine.addExternalSubtitle(Fixtures.uri("karaoke.ass"), "Караоке")
            under.advance(500)
            val s = engine.state()
            val ext = s.subtitleTracks.single { it.external }
            assertEquals("ass", ext.codec)
            assertEquals(ext.id, s.subtitleTrack)

            // Paused at 2 s: the overlay shows the libass picture once the file is read.
            engine.seek(2_100uL)
            val deadline = SystemClock.elapsedRealtime() + 5_000
            while (!onMain { under.activity.video.subtitles.ass.showsPicture } && SystemClock.elapsedRealtime() < deadline) {
                Thread.sleep(50)
            }
            assertTrue("no libass picture", onMain { under.activity.video.subtitles.ass.showsPicture })

            engine.setSubtitleTrack(null)
            under.advance(300)
            assertNull(engine.state().subtitleTrack)
            val cleared = SystemClock.elapsedRealtime() + 2_000
            while (onMain { under.activity.video.subtitles.ass.showsPicture } && SystemClock.elapsedRealtime() < cleared) Thread.sleep(50)
            assertTrue("picture left after turning subtitles off", !onMain { under.activity.video.subtitles.ass.showsPicture })
        }
    }
}
