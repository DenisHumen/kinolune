// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.media

/**
 * What happens to playback when the app's UI goes away (ADR-NNNN): the activity is destroyed or the task is swiped
 * away from Recents while the Flutter engine, which runs the libmpv players, may live on without it.
 *
 * Music keeps the engine (and so the player and its queue) alive, playing or paused, until the user stops it from
 * the notification or the queue is stopped; a video or a live stream has no picture without the activity, so it
 * is stopped and the engine goes with the UI.
 */
object BackgroundPolicy {
    /** The engine may outlive the UI: something owns the session and it is music. */
    fun keepsEngineWithoutUi(owned: Boolean, snapshot: SessionSnapshot): Boolean =
        owned && !snapshot.isEmpty && snapshot.isMusic

    /** The task was swiped away: stop the session (a video), or let it go on (music). */
    fun stopsOnTaskRemoved(owned: Boolean, snapshot: SessionSnapshot): Boolean =
        owned && !keepsEngineWithoutUi(owned, snapshot)
}
