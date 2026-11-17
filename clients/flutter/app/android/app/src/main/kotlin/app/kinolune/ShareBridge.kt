// SPDX-License-Identifier: MPL-2.0
package app.kinolune

import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.OpenableColumns
import android.util.Log
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.io.File
import java.util.concurrent.Executors

/**
 * «Поделиться → Kinolune» (UI-035): files another app shares (`ACTION_SEND`, `ACTION_SEND_MULTIPLE`) are copied
 * into `cache/shared/<time>/` under their own names and handed to Dart, which uploads them to «Личное».
 *
 * Channel `app.kinolune/share`:
 * * `take()` — the paths of a share that arrived before Dart was listening (a cold start), or an empty list;
 * * `shared(paths)` — Dart is told of a share that arrives while it runs. Dart answers `true` once it took them;
 *   otherwise they wait for `take`.
 *
 * Copies run on a background thread: a video from the gallery can take a while.
 */
object ShareBridge {
    private const val TAG = "KinoluneShare"
    private const val CHANNEL = "app.kinolune/share"

    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor { r -> Thread(r, "kinolune-share").apply { isDaemon = true } }
    private lateinit var app: Context
    private var channel: MethodChannel? = null
    private val pending = mutableListOf<String>()

    fun attach(context: Context, engine: FlutterEngine) {
        app = context.applicationContext
        val ch = MethodChannel(engine.dartExecutor.binaryMessenger, CHANNEL)
        channel = ch
        ch.setMethodCallHandler { call, result ->
            when (call.method) {
                "take" -> {
                    result.success(ArrayList(pending))
                    pending.clear()
                }
                else -> result.notImplemented()
            }
        }
    }

    /** The shared files of [intent], if it is a share; `true` if it was one. */
    fun onIntent(intent: Intent?): Boolean {
        val uris = urisOf(intent)
        if (uris.isEmpty()) return false
        val resolver = app.contentResolver
        io.execute {
            val dir = File(app.cacheDir, "shared/${System.currentTimeMillis()}").apply { mkdirs() }
            val paths = uris.mapNotNull { copy(resolver, it, dir) }
            Log.i(TAG, "${paths.size} of ${uris.size} shared files copied")
            if (paths.isNotEmpty()) main.post { deliver(paths) }
        }
        return true
    }

    private fun deliver(paths: List<String>) {
        pending.addAll(paths)
        val ch = channel ?: return
        ch.invokeMethod(
            "shared",
            ArrayList(paths),
            object : MethodChannel.Result {
                override fun success(result: Any?) {
                    if (result == true) pending.removeAll(paths.toSet())
                }

                override fun error(code: String, message: String?, details: Any?) {}
                override fun notImplemented() {}
            },
        )
    }

    @Suppress("DEPRECATION")
    private fun urisOf(intent: Intent?): List<Uri> = when (intent?.action) {
        Intent.ACTION_SEND -> listOfNotNull(
            if (Build.VERSION.SDK_INT >= 33) {
                intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
            } else {
                intent.getParcelableExtra(Intent.EXTRA_STREAM)
            },
        )
        Intent.ACTION_SEND_MULTIPLE -> (
            if (Build.VERSION.SDK_INT >= 33) {
                intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
            } else {
                intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM)
            }
            ).orEmpty()
        else -> emptyList()
    }

    private fun copy(resolver: ContentResolver, uri: Uri, dir: File): String? = try {
        val name = ShareNames.safe(displayName(resolver, uri) ?: uri.lastPathSegment)
        val out = ShareNames.free(dir, name)
        resolver.openInputStream(uri)?.use { input -> out.outputStream().use { input.copyTo(it, 1 shl 16) } }
            ?: return null
        out.absolutePath
    } catch (e: Exception) {
        Log.w(TAG, "cannot read a shared file: ${e.javaClass.simpleName}")
        null
    }

    private fun displayName(resolver: ContentResolver, uri: Uri): String? =
        if (uri.scheme == ContentResolver.SCHEME_CONTENT) {
            resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                if (c.moveToFirst()) c.getString(0) else null
            }
        } else {
            uri.lastPathSegment
        }
}

/** File names of shared files (pure, unit-tested). */
object ShareNames {
    /** One path component without separators or control characters; `file` if nothing is left. */
    fun safe(name: String?): String {
        val clean = name.orEmpty().substringAfterLast('/').substringAfterLast('\\')
            .filter { it >= ' ' && it != ':' }
            .trim()
        return if (clean.isEmpty() || clean == "." || clean == "..") "file" else clean
    }

    /** [name] in [dir], or `name (2).ext`… if it is taken. */
    fun free(dir: File, name: String): File {
        var f = File(dir, name)
        var n = 2
        val dot = name.lastIndexOf('.').takeIf { it > 0 } ?: name.length
        while (f.exists()) {
            f = File(dir, "${name.substring(0, dot)} ($n)${name.substring(dot)}")
            n++
        }
        return f
    }
}
