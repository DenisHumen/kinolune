// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.media

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BackgroundPolicyTest {
    private val music = SessionSnapshot(kind = "music", id = "t1", title = "Track", playing = true)

    @Test
    fun musicKeepsTheEngineWhetherPlayingOrPaused() {
        assertTrue(BackgroundPolicy.keepsEngineWithoutUi(true, music))
        assertTrue(BackgroundPolicy.keepsEngineWithoutUi(true, music.copy(playing = false)))
        assertFalse(BackgroundPolicy.stopsOnTaskRemoved(true, music))
        assertFalse(BackgroundPolicy.stopsOnTaskRemoved(true, music.copy(playing = false)))
    }

    @Test
    fun videoAndLiveGoWithTheUi() {
        for (kind in listOf("video", "live")) {
            val s = music.copy(kind = kind)
            assertFalse(BackgroundPolicy.keepsEngineWithoutUi(true, s))
            assertTrue(BackgroundPolicy.stopsOnTaskRemoved(true, s))
        }
    }

    @Test
    fun nothingOwnedNothingKept() {
        assertFalse(BackgroundPolicy.keepsEngineWithoutUi(false, music))
        assertFalse(BackgroundPolicy.keepsEngineWithoutUi(true, SessionSnapshot.EMPTY))
        assertFalse(BackgroundPolicy.stopsOnTaskRemoved(false, music))
    }
}
