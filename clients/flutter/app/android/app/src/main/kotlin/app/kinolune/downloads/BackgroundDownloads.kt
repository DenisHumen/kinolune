// SPDX-License-Identifier: MPL-2.0
package app.kinolune.downloads

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.os.SystemClock
import android.util.Log
import androidx.core.content.ContextCompat
import app.kinolune.KinoluneEngine
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

/**
 * Background downloads on Android (ADR-NNNN): the bridge between the job queue in Dart and the platform.
 *
 * The offline copies (NET-019) run in the Rust job queue of the process; Dart reports what is in it (`update`,
 * a [DownloadSnapshot]) and this object keeps the process going: a foreground service of type `dataSync`
 * ([DownloadService]) with a progress notification while something downloads, the app's Flutter engine kept
 * without an activity like for music (ADR-0068), and a WorkManager kick ([DownloadWorker]) that restarts the engine
 * after the process died and stands in when Android 12+ refuses to start the service from the background. Pause,
 * resume and cancel from the notification come back here and go to Dart (`command`), after Dart is up.
 *
 * Channel `app.kinolune/downloads`. Main thread only, except [state].
 */
object BackgroundDownloads {
    private const val TAG = "KinoluneDownloads"
    const val CHANNEL = "app.kinolune/downloads"

    /** How long a service started for a command waits for Dart to report the queue. */
    const val DART_TIMEOUT_MS = 60_000L

    private val main = Handler(Looper.getMainLooper())
    private lateinit var app: Context
    private var channel: MethodChannel? = null
    private var dartReady = false
    private val pending = ArrayDeque<Pair<String, Long?>>()
    private var powerReceiver: BroadcastReceiver? = null

    /** Called when the downloads no longer need the engine (KinoluneEngine may shut it down). */
    var onIdle: (() -> Unit)? = null

    /** What Dart last reported, and whether it reported since its engine started. */
    data class State(val snapshot: DownloadSnapshot, val known: Boolean, val blockedSinceMs: Long?)

    @Volatile
    private var current = State(DownloadSnapshot.EMPTY, known = false, blockedSinceMs = null)

    /** Readable from any thread (the worker polls it). */
    fun state(): State = current

    /** A command or a kick started the engine and waits for Dart's first report. */
    private var awaitingDart = false

    private val dartTimeout = Runnable {
        awaitingDart = false
        if (!current.known) {
            Log.w(TAG, "Dart did not report the queue: stopping")
            DownloadService.leave(keepNotification = false)
            onIdle?.invoke()
        }
    }

    /** A new engine: Dart will say `ready` once its queue is open. */
    fun attach(context: Context, engine: FlutterEngine) {
        app = context.applicationContext
        dartReady = false
        current = current.copy(known = false)
        channel = MethodChannel(engine.dartExecutor.binaryMessenger, CHANNEL).also {
            it.setMethodCallHandler(::onCall)
        }
        watchPowerSave()
    }

    fun detach() {
        channel?.setMethodCallHandler(null)
        channel = null
        dartReady = false
        current = current.copy(known = false)
    }

    private fun onCall(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "ready" -> {
                dartReady = true
                result.success(mapOf("powerSave" to powerSave()))
                while (pending.isNotEmpty()) {
                    val (action, job) = pending.removeFirst()
                    send(action, job)
                }
            }
            "update" -> {
                val snapshot = DownloadSnapshot.fromMap(call.arguments as? Map<*, *> ?: emptyMap<String, Any>())
                update(snapshot)
                result.success(null)
            }
            else -> result.notImplemented()
        }
    }

    /** The queue as Dart sees it: where the downloads run now. */
    fun update(snapshot: DownloadSnapshot) {
        val now = SystemClock.elapsedRealtime()
        val before = current
        val blockedSince = if (DownloadPolicy.waitsForNetworkOnly(snapshot)) {
            before.blockedSinceMs.takeIf { before.known } ?: now
        } else {
            null
        }
        current = State(snapshot, known = true, blockedSinceMs = blockedSince)
        awaitingDart = false
        main.removeCallbacks(dartTimeout)
        apply()
        // Re-check once the network grace runs out with nothing else changing.
        if (blockedSince != null) {
            main.removeCallbacks(graceCheck)
            main.postDelayed(graceCheck, DownloadPolicy.NETWORK_GRACE_MS - (now - blockedSince) + 1_000)
        }
    }

    private val graceCheck = Runnable { if (current.known) apply() }

    private fun apply() {
        val s = current.snapshot
        val host = DownloadPolicy.host(s, current.blockedSinceMs, SystemClock.elapsedRealtime())
        if (!::app.isInitialized) return
        when (host) {
            DownloadHost.STOP -> {
                DownloadService.leave(keepNotification = false)
                DownloadNotification.cancel(app)
                DownloadWorker.cancel(app)
            }
            DownloadHost.FOREGROUND -> {
                if (!DownloadService.update(s)) startService()
            }
            DownloadHost.NOTIFY -> {
                DownloadService.leave(keepNotification = true)
                DownloadNotification.show(app, s)
            }
        }
        if (DownloadPolicy.needsKick(s)) DownloadWorker.schedule(app, DownloadPolicy.kickNetwork(s))
        else if (host != DownloadHost.STOP) DownloadWorker.cancel(app)
        if (!DownloadPolicy.keepsEngine(s)) onIdle?.invoke()
    }

    /** Starts the foreground service; Android 12+ may refuse from the background: WorkManager then runs it. */
    private fun startService() {
        // After a refusal the next try waits: progress updates come every second.
        val now = SystemClock.elapsedRealtime()
        if (now - refusedAtMs < RETRY_SERVICE_MS) return
        try {
            ContextCompat.startForegroundService(app, Intent(app, DownloadService::class.java))
            refusedAtMs = Long.MIN_VALUE / 2
        } catch (e: IllegalStateException) {
            // ForegroundServiceStartNotAllowedException (API 31) is an IllegalStateException.
            refusedAtMs = now
            Log.w(TAG, "foreground service not allowed now, WorkManager runs the downloads: $e")
            if (!DownloadWorker.running) DownloadWorker.runNow(app, DownloadPolicy.kickNetwork(current.snapshot))
        }
    }

    /** When the system last refused the service. */
    private var refusedAtMs = Long.MIN_VALUE / 2

    /** How long after a refusal the service is not tried again (the app may come to the front meanwhile). */
    private const val RETRY_SERVICE_MS = 30_000L

    /** True while the downloads want the engine kept without an activity (or a command waits for Dart). */
    fun keepsEngine(): Boolean =
        awaitingDart || current.known && DownloadPolicy.keepsEngine(current.snapshot)

    /**
     * A notification action (`pause`, `resume`, `cancel`; [jobId] for cancel) or a kick (`null` action): the
     * engine is started if needed and Dart gets the command once it is up.
     */
    fun command(context: Context, action: String?, jobId: Long?) {
        app = context.applicationContext
        KinoluneEngine.ensureRunning(app)
        if (!current.known) {
            awaitingDart = true
            main.removeCallbacks(dartTimeout)
            main.postDelayed(dartTimeout, DART_TIMEOUT_MS)
        }
        if (action == null) {
            if (current.known) apply()
            return
        }
        if (dartReady) send(action, jobId) else pending.addLast(action to jobId)
    }

    private fun send(action: String, jobId: Long?) {
        channel?.invokeMethod("command", mapOf("action" to action, "jobId" to jobId))
    }

    /** Called by [DownloadService] once it is in the foreground: re-applies the policy (it may leave at once). */
    fun serviceStarted() {
        if (current.known) apply()
    }

    private fun powerSave(): Boolean {
        val pm = app.getSystemService(Context.POWER_SERVICE) as PowerManager
        return pm.isPowerSaveMode
    }

    private fun watchPowerSave() {
        if (powerReceiver != null) return
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                channel?.invokeMethod("powerSave", powerSave())
            }
        }
        ContextCompat.registerReceiver(
            app,
            receiver,
            IntentFilter(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
        powerReceiver = receiver
    }
}
