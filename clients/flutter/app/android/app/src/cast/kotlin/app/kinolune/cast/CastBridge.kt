// SPDX-License-Identifier: MPL-2.0
package app.kinolune.cast

import android.content.Context
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.mediarouter.media.MediaRouteSelector
import androidx.mediarouter.media.MediaRouter
import com.google.android.gms.cast.CastDevice
import com.google.android.gms.cast.CastMediaControlIntent
import com.google.android.gms.cast.MediaInfo
import com.google.android.gms.cast.MediaLoadRequestData
import com.google.android.gms.cast.MediaMetadata
import com.google.android.gms.cast.MediaSeekOptions
import com.google.android.gms.cast.MediaStatus
import com.google.android.gms.cast.framework.CastContext
import com.google.android.gms.cast.framework.CastSession
import com.google.android.gms.cast.framework.SessionManagerListener
import com.google.android.gms.common.ConnectionResult
import com.google.android.gms.common.GoogleApiAvailability
import com.google.android.gms.common.images.WebImage
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

/**
 * PLAY-012 (ADR-NNNN): the Google Cast sender on the channel `app.kinolune/cast` (lib/cast/cast_platform.dart).
 *
 * The device list is our own Flutter dialog, so there is no MediaRouteButton (and no AppCompat theme): the media
 * router discovers the Cast devices that run the Default Media Receiver, Dart gets them as `devices`, and `connect`
 * selects the route — the Cast framework then starts the session by itself. `load` hands the receiver the URL Dart
 * chose (the LAN relay or the media server's HLS); `status` is polled by Dart every second for the progress sync.
 *
 * Builds without the Cast SDK (F-Droid) get src/nocast instead: the same object, `available` false.
 * Main thread only (the method channel and the media router run there).
 */
object CastBridge {
    private const val TAG = "CastBridge"
    private const val CHANNEL = "app.kinolune/cast"
    private const val CONNECT_TIMEOUT_MS = 20_000L

    private val main = Handler(Looper.getMainLooper())
    private var app: Context? = null
    private var channel: MethodChannel? = null
    private var cast: CastContext? = null
    private var castFailed = false
    private var router: MediaRouter? = null
    private var discovering = false

    private val selector: MediaRouteSelector by lazy {
        MediaRouteSelector.Builder()
            .addControlCategory(
                CastMediaControlIntent.categoryForCast(CastMediaControlIntent.DEFAULT_MEDIA_RECEIVER_APPLICATION_ID),
            )
            .build()
    }

    /** A `connect` waiting for its session. */
    private var pendingConnect: MethodChannel.Result? = null
    private val connectTimeout = Runnable { finishConnect(null) }

    fun attach(context: Context, engine: FlutterEngine) {
        app = context.applicationContext
        channel = MethodChannel(engine.dartExecutor.binaryMessenger, CHANNEL).also {
            it.setMethodCallHandler(::onCall)
        }
    }

    fun detach() {
        stopDiscovery()
        finishConnect(null)
        channel?.setMethodCallHandler(null)
        channel = null
    }

    private fun onCall(call: MethodCall, result: MethodChannel.Result) {
        try {
            when (call.method) {
                "available" -> result.success(castContext() != null)
                "startDiscovery" -> {
                    startDiscovery()
                    result.success(null)
                }
                "stopDiscovery" -> {
                    stopDiscovery()
                    result.success(null)
                }
                "connect" -> connect(call.argument<String>("id"), result)
                "load" -> load(call, result)
                "status" -> result.success(status())
                "play" -> {
                    client()?.play()
                    result.success(null)
                }
                "pause" -> {
                    client()?.pause()
                    result.success(null)
                }
                "seek" -> {
                    val ms = call.argument<Number>("positionMs")?.toLong() ?: 0L
                    client()?.seek(MediaSeekOptions.Builder().setPosition(ms).build())
                    result.success(null)
                }
                "stop" -> {
                    castContext()?.sessionManager?.endCurrentSession(true)
                    result.success(null)
                }
                else -> result.notImplemented()
            }
        } catch (e: Exception) {
            Log.w(TAG, "${call.method} failed", e)
            result.error("cast", e.message, null)
        }
    }

    /** The Cast framework, or null without Google Play services (it then stays off for the process). */
    @Suppress("DEPRECATION")
    private fun castContext(): CastContext? {
        cast?.let { return it }
        if (castFailed) return null
        val ctx = app ?: return null
        castFailed = true
        if (GoogleApiAvailability.getInstance().isGooglePlayServicesAvailable(ctx) != ConnectionResult.SUCCESS) {
            Log.i(TAG, "no Google Play services: casting is off")
            return null
        }
        return try {
            CastContext.getSharedInstance(ctx).also {
                it.sessionManager.addSessionManagerListener(sessions, CastSession::class.java)
                cast = it
                castFailed = false
            }
        } catch (e: Exception) {
            Log.w(TAG, "Cast framework not available", e)
            null
        }
    }

    private fun client() = castContext()?.sessionManager?.currentCastSession?.remoteMediaClient

    // ------------------------------------------------------------------ discovery

    private val routes = object : MediaRouter.Callback() {
        override fun onRouteAdded(router: MediaRouter, route: MediaRouter.RouteInfo) = publish()
        override fun onRouteRemoved(router: MediaRouter, route: MediaRouter.RouteInfo) = publish()
        override fun onRouteChanged(router: MediaRouter, route: MediaRouter.RouteInfo) = publish()
    }

    private fun startDiscovery() {
        if (discovering || castContext() == null) return
        val ctx = app ?: return
        val r = router ?: MediaRouter.getInstance(ctx).also { router = it }
        r.addCallback(selector, routes, MediaRouter.CALLBACK_FLAG_REQUEST_DISCOVERY)
        discovering = true
        publish()
    }

    private fun stopDiscovery() {
        if (!discovering) return
        router?.removeCallback(routes)
        discovering = false
    }

    private fun castRoutes(): List<MediaRouter.RouteInfo> {
        val r = router ?: return emptyList()
        return r.routes.filter { !it.isDefault && it.isEnabled && it.matchesSelector(selector) }
    }

    private fun publish() {
        val list = castRoutes().map { route ->
            val device = CastDevice.getFromBundle(route.extras)
            mapOf(
                "id" to route.id,
                "name" to route.name,
                "tech" to "googleCast",
                "model" to device?.modelName,
            )
        }
        channel?.invokeMethod("devices", list)
    }

    // ------------------------------------------------------------------ session

    private fun connect(id: String?, result: MethodChannel.Result) {
        val ctx = app
        if (id == null || ctx == null || castContext() == null) {
            result.success(mapOf("ok" to false))
            return
        }
        val r = router ?: MediaRouter.getInstance(ctx).also { router = it }
        val route = r.routes.firstOrNull { it.id == id }
        if (route == null) {
            result.success(mapOf("ok" to false))
            return
        }
        // Already connected to it: nothing to wait for.
        val session = cast?.sessionManager?.currentCastSession
        if (session != null && session.isConnected && route.isSelected) {
            result.success(connected(session))
            return
        }
        finishConnect(null)
        pendingConnect = result
        main.postDelayed(connectTimeout, CONNECT_TIMEOUT_MS)
        r.selectRoute(route)
    }

    private fun connected(session: CastSession): Map<String, Any?> {
        val device = session.castDevice
        @Suppress("DEPRECATION")
        val address = device?.inetAddress?.hostAddress
        return mapOf("ok" to true, "address" to address, "model" to device?.modelName)
    }

    private fun finishConnect(session: CastSession?) {
        main.removeCallbacks(connectTimeout)
        val pending = pendingConnect ?: return
        pendingConnect = null
        pending.success(if (session != null) connected(session) else mapOf("ok" to false))
    }

    private val sessions = object : SessionManagerListener<CastSession> {
        override fun onSessionStarting(session: CastSession) = Unit
        override fun onSessionStarted(session: CastSession, sessionId: String) = finishConnect(session)
        override fun onSessionStartFailed(session: CastSession, error: Int) = finishConnect(null)
        override fun onSessionEnding(session: CastSession) = Unit
        override fun onSessionEnded(session: CastSession, error: Int) {
            finishConnect(null)
            channel?.invokeMethod("sessionEnded", null)
        }
        override fun onSessionResuming(session: CastSession, sessionId: String) = Unit
        override fun onSessionResumed(session: CastSession, wasSuspended: Boolean) = finishConnect(session)
        override fun onSessionResumeFailed(session: CastSession, error: Int) = finishConnect(null)
        override fun onSessionSuspended(session: CastSession, reason: Int) = Unit
    }

    // ------------------------------------------------------------------ media

    private fun load(call: MethodCall, result: MethodChannel.Result) {
        val client = client()
        val url = call.argument<String>("url")
        if (client == null || url == null) {
            result.success(false)
            return
        }
        val metadata = MediaMetadata(MediaMetadata.MEDIA_TYPE_MOVIE).apply {
            putString(MediaMetadata.KEY_TITLE, call.argument<String>("title") ?: "")
            call.argument<String>("subtitle")?.let { putString(MediaMetadata.KEY_SUBTITLE, it) }
            call.argument<String>("imageUrl")?.let { addImage(WebImage(Uri.parse(it))) }
        }
        val duration = call.argument<Number>("durationMs")?.toLong() ?: 0L
        @Suppress("DEPRECATION")
        val info = MediaInfo.Builder(url)
            .setStreamType(MediaInfo.STREAM_TYPE_BUFFERED)
            .setContentType(call.argument<String>("contentType") ?: "video/mp4")
            .setMetadata(metadata)
            .apply { if (duration > 0) setStreamDuration(duration) }
            .build()
        val request = MediaLoadRequestData.Builder()
            .setMediaInfo(info)
            .setAutoplay(true)
            .setCurrentTime(call.argument<Number>("positionMs")?.toLong() ?: 0L)
            .build()
        client.load(request).setResultCallback { r ->
            if (!r.status.isSuccess) Log.w(TAG, "load failed: ${r.status.statusCode}")
            result.success(r.status.isSuccess)
        }
    }

    private fun status(): Map<String, Any?>? {
        val client = client() ?: return null
        val state = when (client.playerState) {
            MediaStatus.PLAYER_STATE_PLAYING -> "playing"
            MediaStatus.PLAYER_STATE_PAUSED -> "paused"
            MediaStatus.PLAYER_STATE_BUFFERING -> "buffering"
            MediaStatus.PLAYER_STATE_LOADING -> "loading"
            else -> "idle"
        }
        val idle = when (client.idleReason) {
            MediaStatus.IDLE_REASON_FINISHED -> "finished"
            MediaStatus.IDLE_REASON_CANCELED -> "cancelled"
            MediaStatus.IDLE_REASON_INTERRUPTED -> "interrupted"
            MediaStatus.IDLE_REASON_ERROR -> "error"
            else -> "none"
        }
        return mapOf(
            "state" to state,
            "positionMs" to client.approximateStreamPosition,
            "durationMs" to client.streamDuration,
            "idleReason" to idle,
        )
    }
}
