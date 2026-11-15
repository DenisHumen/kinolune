// SPDX-License-Identifier: MPL-2.0
package app.kinolune

import android.app.UiModeManager
import android.content.Context
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import app.kinolune.downloads.BackgroundDownloads
import app.kinolune.player.media.MediaBridge
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.embedding.engine.dart.DartExecutor
import io.flutter.plugin.common.MethodChannel

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
 * [IDLE_GRACE_MS], which gives Dart time to save the queue (MUS-003); after a video or live session it goes at once.
 *
 * Main thread only.
 */
object KinoluneEngine {
    private const val TAG = "KinoluneEngine"

    /** Delay before an engine left with no UI and no music is destroyed. */
    const val IDLE_GRACE_MS = 5_000L

    /** How long Dart gets to dispose its players before the engine is destroyed anyway. */
    private const val SHUTDOWN_TIMEOUT_MS = 3_000L
    private const val SHUTDOWN_CHANNEL = "app.kinolune/engine"
    private const val DEVICE_CHANNEL = "app.kinolune/device"

    private val main = Handler(Looper.getMainLooper())
    private var engine: FlutterEngine? = null
    private var shuttingDown: FlutterEngine? = null
    private var activities = 0
    private var multicastLock: WifiManager.MulticastLock? = null
    private val idleCheck = Runnable { destroyIfIdle() }

    /** The engine for an activity: the running one, or a new one (the activity runs its Dart entrypoint). */
    fun obtain(context: Context): FlutterEngine {
        // An activity came back while the engine was shutting down: its players are gone, start afresh.
        shuttingDown?.let { finishShutdown(it) }
        engine?.let {
            Log.i(TAG, "activity attaches to the running engine")
            return it
        }
        return create(context)
    }

    /**
     * Background downloads (ADR-NNNN): a notification action or a WorkManager kick needs the app's Dart (its job
     * queue) with no activity. Starts the engine headless and runs `main()`; an activity opened later attaches to
     * it and does not run the entrypoint again.
     */
    fun ensureRunning(context: Context) {
        shuttingDown?.let { finishShutdown(it) }
        if (engine != null) return
        Log.i(TAG, "starting the engine without an activity for the downloads")
        val e = create(context)
        e.dartExecutor.executeDartEntrypoint(DartExecutor.DartEntrypoint.createDefault())
        // Nothing may come of it (no downloads): the usual idle check then shuts it down.
        schedule(BackgroundDownloads.DART_TIMEOUT_MS)
    }

    private fun create(context: Context): FlutterEngine {
        val app = context.applicationContext
        val e = FlutterEngine(app)
        engine = e
        MediaBridge.onSessionEnded = { wasMusic -> schedule(if (wasMusic) IDLE_GRACE_MS else 0) }
        BackgroundDownloads.onIdle = { schedule(IDLE_GRACE_MS) }
        BackgroundDownloads.attach(app, e)
        // On the engine, not the activity: a headless start (downloads) asks for the form factor too.
        MethodChannel(e.dartExecutor.binaryMessenger, DEVICE_CHANNEL).setMethodCallHandler { call, result ->
            when (call.method) {
                "isTelevision" -> result.success(isTelevision(app))
                "deviceName" -> result.success(Build.MODEL)
                "multicastLock" -> {
                    setMulticastLock(app, call.arguments as? Boolean ?: false)
                    result.success(null)
                }
                else -> result.notImplemented()
            }
        }
        return e
    }

    private fun isTelevision(context: Context): Boolean {
        val uiMode = context.getSystemService(Context.UI_MODE_SERVICE) as UiModeManager
        if (uiMode.currentModeType == Configuration.UI_MODE_TYPE_TELEVISION) return true
        return context.packageManager.hasSystemFeature(PackageManager.FEATURE_LEANBACK)
    }

    fun activityCreated() {
        activities++
        main.removeCallbacks(idleCheck)
        checkAt = Long.MAX_VALUE
    }

    fun activityDestroyed(changingConfigurations: Boolean) {
        activities = (activities - 1).coerceAtLeast(0)
        if (!changingConfigurations) schedule(0)
    }

    /** Uptime of the pending check; an earlier one is not postponed. */
    private var checkAt = Long.MAX_VALUE

    private fun schedule(delayMs: Long) {
        val at = SystemClock.uptimeMillis() + delayMs
        if (at >= checkAt) return
        main.removeCallbacks(idleCheck)
        checkAt = at
        main.postAtTime(idleCheck, at)
    }

    private fun destroyIfIdle() {
        checkAt = Long.MAX_VALUE
        val e = engine ?: return
        if (activities > 0 || shuttingDown != null) return
        if (MediaBridge.keepsEngineWithoutUi()) {
            Log.i(TAG, "no activity: the engine stays for the music")
            return
        }
        if (BackgroundDownloads.keepsEngine()) {
            Log.i(TAG, "no activity: the engine stays for the downloads")
            return
        }
        Log.i(TAG, "no activity, no music and no downloads: shutting the engine down")
        shuttingDown = e
        // libmpv players must be disposed before their isolate dies, or their threads call back into freed code.
        val finish = Runnable { finishShutdown(e) }
        main.postDelayed(finish, SHUTDOWN_TIMEOUT_MS)
        MethodChannel(e.dartExecutor.binaryMessenger, SHUTDOWN_CHANNEL).invokeMethod(
            "shutdown",
            null,
            object : MethodChannel.Result {
                override fun success(result: Any?) = done()
                override fun error(code: String, message: String?, details: Any?) = done()
                override fun notImplemented() = done()
                private fun done() {
                    main.removeCallbacks(finish)
                    finishShutdown(e)
                }
            },
        )
    }

    private fun finishShutdown(e: FlutterEngine) {
        if (shuttingDown !== e) return
        shuttingDown = null
        if (engine === e) engine = null
        Log.i(TAG, "engine destroyed")
        MediaBridge.onSessionEnded = null
        BackgroundDownloads.detach()
        MethodChannel(e.dartExecutor.binaryMessenger, DEVICE_CHANNEL).setMethodCallHandler(null)
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
