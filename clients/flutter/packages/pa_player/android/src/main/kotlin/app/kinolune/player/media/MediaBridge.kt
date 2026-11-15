// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.media

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import androidx.annotation.OptIn
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import com.google.common.util.concurrent.ListenableFuture

/**
 * Process-wide glue between the Dart players and the system session (PLAY-AND-008): one [BridgePlayer] shared by
 * the [PlaybackService], audio focus ([AudioFocus]), and the channel sink that carries commands back to Dart.
 *
 * The service is started by connecting a [MediaController] to it when Dart first reports something to show, and
 * stopped when Dart clears the session. Everything runs on the main thread.
 */
@OptIn(UnstableApi::class)
object MediaBridge {
    private const val TAG = "KinoluneSession"
    private val main = Handler(Looper.getMainLooper())
    private var bridgePlayer: BridgePlayer? = null
    private var focus: AudioFocus? = null
    private var controller: ListenableFuture<MediaController>? = null

    /** Sends a command to Dart: `{action, positionMs?, reason?}`, to the engine that last reported a state. */
    private var sink: ((Map<String, Any?>) -> Unit)? = null
    private var owner: Any? = null

    fun player(): BridgePlayer = bridgePlayer ?: BridgePlayer(Looper.getMainLooper()) { action, args ->
        dispatch(action, args + mapOf("reason" to "session"))
    }.also { bridgePlayer = it }

    private fun dispatch(action: String, args: Map<String, Any?>) {
        Log.i(TAG, "command $action $args")
        val s = sink ?: return
        main.post { s(mapOf("action" to action) + args) }
    }

    /** Dart of [from] (a plugin instance, one per Flutter engine) reported [state] (method `update`). */
    fun update(context: Context, state: Map<*, *>, from: Any, commands: (Map<String, Any?>) -> Unit) {
        val ctx = context.applicationContext
        owner = from
        sink = commands
        val s = SessionSnapshot.fromMap(state, SystemClock.elapsedRealtime())
        player().update(s)
        focusFor(ctx).onState(s)
        ensureService(ctx)
    }

    /** [from] has nothing to show any more (method `clear`, engine gone): the notification goes, the service stops. */
    fun release(from: Any, context: Context) {
        if (owner !== from) return
        owner = null
        sink = null
        val ctx = context.applicationContext
        bridgePlayer?.update(SessionSnapshot.EMPTY)
        focus?.reset()
        controller?.let { MediaController.releaseFuture(it) }
        controller = null
        ctx.stopService(Intent(ctx, PlaybackService::class.java))
        Log.i(TAG, "session released")
        onSessionEnded?.invoke()
    }

    /**
     * Called on the main thread after the session is released: the app's engine holder destroys a Flutter engine
     * that no activity and no music needs any more (ADR-NNNN).
     */
    @JvmStatic
    var onSessionEnded: (() -> Unit)? = null

    /** Music owns the session (playing or paused): the engine that plays it must outlive the UI (ADR-NNNN). */
    @JvmStatic
    fun keepsEngineWithoutUi(): Boolean =
        BackgroundPolicy.keepsEngineWithoutUi(owner != null, bridgePlayer?.snapshot ?: SessionSnapshot.EMPTY)

    /** The user swiped the media notification away: the player that owns the session stops (ADR-NNNN). */
    fun onNotificationDismissed() {
        if (owner == null) return
        Log.i(TAG, "notification dismissed: stop")
        dispatch("stop", mapOf("reason" to "dismissed"))
    }

    /**
     * The task was swiped away from Recents. Music goes on (the engine outlives the activity); anything else
     * (a video, a live stream) is told to stop and the session ends. Returns true when the service should stop.
     */
    fun onTaskRemoved(context: Context): Boolean {
        val o = owner ?: return true
        if (!BackgroundPolicy.stopsOnTaskRemoved(true, bridgePlayer?.snapshot ?: SessionSnapshot.EMPTY)) {
            Log.i(TAG, "task removed: music goes on in the background")
            return false
        }
        Log.i(TAG, "task removed: not music, stopping")
        dispatch("stop", mapOf("reason" to "task_removed"))
        release(o, context)
        return true
    }

    private fun focusFor(ctx: Context): AudioFocus = focus ?: AudioFocus(ctx) { action, reason ->
        val name = when (action) {
            FocusPolicy.Action.PLAY -> "play"
            FocusPolicy.Action.PAUSE -> "pause"
            FocusPolicy.Action.DUCK -> "duck"
            FocusPolicy.Action.UNDUCK -> "unduck"
            else -> null
        }
        if (name != null) dispatch(name, mapOf("reason" to reason))
    }.also { focus = it }

    private fun ensureService(ctx: Context) {
        if (controller != null) return
        val token = SessionToken(ctx, ComponentName(ctx, PlaybackService::class.java))
        controller = MediaController.Builder(ctx, token).buildAsync()
    }

    internal fun onServiceCreated(service: PlaybackService) {
        Log.i(TAG, "service created")
    }

    internal fun onServiceDestroyed(service: PlaybackService) {
        Log.i(TAG, "service destroyed")
        controller?.let { MediaController.releaseFuture(it) }
        controller = null
    }
}
