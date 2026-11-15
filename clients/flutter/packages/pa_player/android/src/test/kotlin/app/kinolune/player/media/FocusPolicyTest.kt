// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.media

import app.kinolune.player.media.FocusPolicy.Action
import app.kinolune.player.media.FocusPolicy.Change
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class FocusPolicyTest {
    private fun playing(): FocusPolicy {
        val p = FocusPolicy()
        assertEquals(listOf(Action.REQUEST), p.onPlaying(true))
        assertEquals(emptyList<Action>(), p.onRequestResult(true))
        return p
    }

    @Test
    fun requestsOnPlayAndAbandonsOnUserPause() {
        val p = playing()
        assertTrue(p.held)
        assertEquals(listOf(Action.ABANDON), p.onPlaying(false))
        assertFalse(p.held)
        // Playing again asks again.
        assertEquals(listOf(Action.REQUEST), p.onPlaying(true))
    }

    @Test
    fun refusedRequestPauses() {
        val p = FocusPolicy()
        p.onPlaying(true)
        assertEquals(listOf(Action.PAUSE), p.onRequestResult(false))
    }

    @Test
    fun callPausesAndResumesAfterwards() {
        val p = playing()
        assertEquals(listOf(Action.PAUSE), p.onFocusChange(Change.LOSS_TRANSIENT))
        // The player pauses because of it: the focus is kept for the gain.
        assertEquals(emptyList<Action>(), p.onPlaying(false))
        assertEquals(listOf(Action.PLAY), p.onFocusChange(Change.GAIN))
        assertEquals(emptyList<Action>(), p.onPlaying(true))
    }

    @Test
    fun userPlayDuringCallCancelsAutoResume() {
        val p = playing()
        p.onFocusChange(Change.LOSS_TRANSIENT)
        p.onPlaying(false)
        p.onPlaying(true)
        assertEquals(emptyList<Action>(), p.onFocusChange(Change.GAIN))
    }

    @Test
    fun permanentLossPausesForGood() {
        val p = playing()
        assertEquals(listOf(Action.PAUSE), p.onFocusChange(Change.LOSS))
        assertFalse(p.held)
        assertEquals(emptyList<Action>(), p.onPlaying(false))
        assertEquals(emptyList<Action>(), p.onFocusChange(Change.GAIN).filter { it == Action.PLAY })
    }

    @Test
    fun duckAndUnduck() {
        val p = playing()
        assertEquals(listOf(Action.DUCK), p.onFocusChange(Change.LOSS_TRANSIENT_CAN_DUCK))
        assertEquals(emptyList<Action>(), p.onFocusChange(Change.LOSS_TRANSIENT_CAN_DUCK))
        assertEquals(listOf(Action.UNDUCK), p.onFocusChange(Change.GAIN))
    }

    @Test
    fun userPauseWhileDuckedRestoresVolume() {
        val p = playing()
        p.onFocusChange(Change.LOSS_TRANSIENT_CAN_DUCK)
        assertEquals(listOf(Action.UNDUCK, Action.ABANDON), p.onPlaying(false))
    }

    @Test
    fun resetGivesEverythingBack() {
        val p = playing()
        p.onFocusChange(Change.LOSS_TRANSIENT_CAN_DUCK)
        assertEquals(listOf(Action.UNDUCK, Action.ABANDON), p.reset())
        assertFalse(p.held)
    }
}
