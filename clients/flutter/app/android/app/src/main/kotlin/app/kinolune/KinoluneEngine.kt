// SPDX-License-Identifier: MPL-2.0
package app.kinolune

import android.content.Context
import android.net.wifi.WifiManager
import android.os.Handler
import android.os.Looper
import android.util.Log
import app.kinolune.player.media.MediaBridge
import io.flutter.embedding.engine.FlutterEngine

/**
 * The app's one Flutter engine, owned by the process rather than by [MainActivity] (ADR-NNNN).
 *
 * The libmpv players, the music queue and the whole Dart app run in this engine. While music owns the system
 * media session (playing or paused, [MediaBridge.keepsEngineWithoutUi]) the engine outlives the activity: the app
 * swiped away from Recents keeps playing in the foreground service of `pa_player`, the notification still
 * controls it, and a new activity attaches to the same engine and shows the live queue and position.
 *
 * Otherwise the engine goes with the last activity, as a FlutterActivity's own engine would; once the music
 * session ends with no activity (stopped from the notification, the queue stopped), the engine is destroyed after
 * [IDLE_GRACE_MS], which gives Dart time to save the queue (MUS-003).
 *
 * Main thread only.
 */
object KinoluneEngine {
    private const val TAG = "KinoluneEngine"

    /** Delay before an engine left with no UI and no music is destroyed. */
    const val IDLE_GRACE_MS = 5_000L

    private val main = Handler(Looper.getMainLooper())
    private var engine: FlutterEngine? = null
    private var activities = 0
    private var multicastLock: WifiManager.MulticastLock? = null
    private val idleCheck = Runnable { destroyIfIdle() }

    /** The engine for an activity: the running one, or a new one (the activity runs its Dart entrypoint). */
    fun obtain(context: Context): FlutterEngine {
        engine?.let {
            Log.i(TAG, "activity attaches to the running engine")
            return it
        }
        val e = FlutterEngine(context.applicationContext)
        engine = e
        MediaBridge.onSessionEnded = { schedule(IDLE_GRACE_MS) }
        return e
    }

    fun activityCreated() {
        activities++
        main.removeCallbacks(idleCheck)
    }

    fun activityDestroyed(changingConfigurations: Boolean) {
        activities = (activities - 1).coerceAtLeast(0)
        if (!changingConfigurations) schedule(0)
    }

    private fun schedule(delayMs: Long) {
        main.removeCallbacks(idleCheck)
        main.postDelayed(idleCheck, delayMs)
    }

    private fun destroyIfIdle() {
        val e = engine ?: return
        if (activities > 0) return
        if (MediaBridge.keepsEngineWithoutUi()) {
            Log.i(TAG, "no activity: the engine stays for the music")
            return
        }
        Log.i(TAG, "no activity and no music: engine destroyed")
        engine = null
        MediaBridge.onSessionEnded = null
        setMulticastLock(null, false)
        e.destroy()
    }

    /**
     * mDNS of LAN sync (UI-019) and source discovery (NET-020): Wi-Fi drops multicast without this lock. Dart
     * counts the holders; the lock lives as long as the engine, not the activity.
     */
    fun setMulticastLock(context: Context?, hold: Boolean) {
        if (hold) {
            if (multicastLock == null && context != null) {
                val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
                multicastLock = wifi.createMulticastLock("kinolune-mdns").apply {
                    setReferenceCounted(false)
                    acquire()
                }
            }
        } else {
            multicastLock?.release()
            multicastLock = null
        }
    }
}
