// SPDX-License-Identifier: MPL-2.0
package app.kinolune.tv

import android.app.SearchManager
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.util.Log
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.net.URLEncoder
import java.util.concurrent.Executors

/**
 * Android TV integration (PLAY-AND-009, ADR-NNNN), channel `app.kinolune/tv` on the app's engine:
 *
 * * `publishWatchNext(entries)` — Dart's Continue and Up Next rows go to the system Watch Next row ([WatchNext]);
 * * `takeLink()` — the `kinolune://` link the app was opened with (a Watch Next card, voice search), once;
 * * `link(uri)` — Dart is told of a link that arrives while it runs (a new intent of the running activity).
 *
 * Links ([TvLinks]): `kinolune://play/<id>` (resume an item), `kinolune://item/<id>` (its page),
 * `kinolune://twitch/vod/<id>?t=<ms>`, `kinolune://search?q=<text>` (from a search intent).
 */
object TvBridge {
    private const val TAG = "TvBridge"
    private const val CHANNEL = "app.kinolune/tv"

    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor { r -> Thread(r, "kinolune-watch-next").apply { isDaemon = true } }
    private var channel: MethodChannel? = null
    private var pending: String? = null

    fun attach(context: Context, engine: FlutterEngine) {
        val app = context.applicationContext
        val ch = MethodChannel(engine.dartExecutor.binaryMessenger, CHANNEL)
        channel = ch
        ch.setMethodCallHandler { call, result ->
            when (call.method) {
                "takeLink" -> {
                    result.success(pending)
                    pending = null
                }
                "watchNextSupported" -> result.success(WatchNext.supported(app))
                "publishWatchNext" -> {
                    val entries = (call.arguments as? List<*>).orEmpty()
                        .mapNotNull { (it as? Map<*, *>)?.let(WatchNextEntry::fromMap) }
                    io.execute {
                        try {
                            WatchNext.publish(app, entries)
                        } catch (e: Exception) {
                            Log.w(TAG, "Watch Next publish failed: $e")
                        }
                    }
                    result.success(null)
                }
                else -> result.notImplemented()
            }
        }
    }

    fun detach() {
        channel?.setMethodCallHandler(null)
        channel = null
    }

    /** An intent of the activity (its first or a new one): keeps its link for Dart and tells a running Dart. */
    fun onIntent(intent: Intent?) {
        val link = TvLinks.linkOf(intent?.action, intent?.dataString, intent?.getStringExtra(SearchManager.QUERY))
            ?: return
        Log.i(TAG, "link ${link.substringBefore('?')}")
        pending = link
        val ch = channel ?: return
        main.post {
            ch.invokeMethod(
                "link",
                link,
                object : MethodChannel.Result {
                    // Dart took it; otherwise (no shell yet) it stays for `takeLink`.
                    override fun success(result: Any?) {
                        if (result == true && pending == link) pending = null
                    }

                    override fun error(code: String, message: String?, details: Any?) {}
                    override fun notImplemented() {}
                },
            )
        }
    }
}

/** Links of intents (pure, unit-tested). */
object TvLinks {
    const val SCHEME = "kinolune"

    /** Google Assistant's "search for X on Kinolune" (with [SearchManager.QUERY]). */
    const val GMS_SEARCH = "com.google.android.gms.actions.SEARCH_ACTION"

    /** The app link of an intent, or null. */
    fun linkOf(action: String?, data: String?, query: String?): String? = when (action) {
        Intent.ACTION_VIEW -> data?.takeIf { it.startsWith("$SCHEME://") }
        Intent.ACTION_SEARCH, GMS_SEARCH ->
            query?.trim()?.takeIf { it.isNotEmpty() }?.let { "$SCHEME://search?q=" + URLEncoder.encode(it, "UTF-8") }
        else -> null
    }
}
