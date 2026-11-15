// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.media

/**
 * Audio focus decisions of the media session (PLAY-AND-008), without Android types so that it is unit-tested.
 *
 * - The player starts playing: request focus; refused (a call is ringing) → pause.
 * - Permanent loss (another player started): pause and forget the focus.
 * - Transient loss (a call, a voice assistant): pause, and play again when the focus comes back, unless the user
 *   paused or played meanwhile.
 * - "May duck" (navigation prompt, notification sound): play quieter until the focus comes back. The request sets
 *   `willPauseWhenDucked`, so the system leaves ducking to us instead of lowering the volume itself.
 * - The user pauses: give the focus up, except while a transient loss is pending (the gain resumes then).
 */
class FocusPolicy {
    enum class Change { GAIN, LOSS, LOSS_TRANSIENT, LOSS_TRANSIENT_CAN_DUCK }

    enum class Action { REQUEST, ABANDON, PLAY, PAUSE, DUCK, UNDUCK }

    var held = false
        private set
    var pausedTransiently = false
        private set
    var ducked = false
        private set
    private var playing = false

    /** The player reported [nowPlaying]. */
    fun onPlaying(nowPlaying: Boolean): List<Action> {
        if (nowPlaying == playing) return emptyList()
        playing = nowPlaying
        val out = ArrayList<Action>()
        if (nowPlaying) {
            // The user played during the loss: the gain must not toggle anything.
            pausedTransiently = false
            if (!held) out.add(Action.REQUEST)
        } else if (held && !pausedTransiently) {
            if (ducked) {
                ducked = false
                out.add(Action.UNDUCK)
            }
            held = false
            out.add(Action.ABANDON)
        }
        return out
    }

    /** Result of a [Action.REQUEST]. */
    fun onRequestResult(granted: Boolean): List<Action> {
        held = granted
        return if (granted || !playing) emptyList() else listOf(Action.PAUSE)
    }

    fun onFocusChange(change: Change): List<Action> {
        val out = ArrayList<Action>()
        when (change) {
            Change.GAIN -> {
                held = true
                if (ducked) {
                    ducked = false
                    out.add(Action.UNDUCK)
                }
                if (pausedTransiently) {
                    pausedTransiently = false
                    out.add(Action.PLAY)
                }
            }
            Change.LOSS -> {
                held = false
                pausedTransiently = false
                if (ducked) {
                    ducked = false
                    out.add(Action.UNDUCK)
                }
                if (playing) out.add(Action.PAUSE)
            }
            Change.LOSS_TRANSIENT -> {
                if (playing) {
                    pausedTransiently = true
                    out.add(Action.PAUSE)
                }
            }
            Change.LOSS_TRANSIENT_CAN_DUCK -> {
                if (playing && !ducked) {
                    ducked = true
                    out.add(Action.DUCK)
                }
            }
        }
        return out
    }

    /** The session was cleared: nothing plays any more. */
    fun reset(): List<Action> {
        val out = ArrayList<Action>()
        if (ducked) out.add(Action.UNDUCK)
        if (held) out.add(Action.ABANDON)
        held = false
        pausedTransiently = false
        ducked = false
        playing = false
        return out
    }
}
