// SPDX-License-Identifier: MPL-2.0
package app.kinolune

import android.app.ActivityManager
import android.app.ApplicationExitInfo
import android.content.ContentValues
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Log
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.io.File
import java.util.concurrent.Executors

/**
 * Diagnostics (CORE-008, ADR-NNNN), channel `app.kinolune/diagnostics` on the app's engine:
 *
 * * `exitReasons(sinceMs)` — crashes, native crashes and ANRs of earlier runs that the system remembers
 *   (`ApplicationExitInfo`, Android 11+). Native crashes of libmpv or the core end here: ART owns the fatal
 *   signals, so the core does not install its own handlers on Android. Dart saves them as crash reports only
 *   when the user opted in.
 * * `saveToDownloads(path, name)` — copies the diagnostic report into the shared Downloads (Android 10+) so the
 *   user can attach it to a GitHub issue; returns `Download/<name>` or null.
 *
 * Nothing is sent anywhere.
 */
object DiagnosticsBridge {
    private const val TAG = "DiagnosticsBridge"
    private const val CHANNEL = "app.kinolune/diagnostics"
    private const val MAX_TRACE = 64 * 1024

    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor { r -> Thread(r, "kinolune-diagnostics").apply { isDaemon = true } }

    fun attach(context: Context, engine: FlutterEngine) {
        val app = context.applicationContext
        MethodChannel(engine.dartExecutor.binaryMessenger, CHANNEL).setMethodCallHandler { call, result ->
            when (call.method) {
                "exitReasons" -> {
                    val since = (call.argument<Number>("sinceMs") ?: 0).toLong()
                    io.execute {
                        val list = try {
                            exitReasons(app, since)
                        } catch (e: Exception) {
                            Log.w(TAG, "exit reasons not read: $e")
                            emptyList()
                        }
                        main.post { result.success(list) }
                    }
                }
                "saveToDownloads" -> {
                    val path = call.argument<String>("path")
                    val name = call.argument<String>("name")
                    if (path == null || name == null) {
                        result.error("args", "path and name are required", null)
                        return@setMethodCallHandler
                    }
                    io.execute {
                        val saved = try {
                            saveToDownloads(app, File(path), name)
                        } catch (e: Exception) {
                            Log.w(TAG, "report not saved to Downloads: $e")
                            null
                        }
                        main.post { result.success(saved) }
                    }
                }
                else -> result.notImplemented()
            }
        }
    }

    private fun exitReasons(context: Context, sinceMs: Long): List<Map<String, Any?>> {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return emptyList()
        val am = context.getSystemService(ActivityManager::class.java) ?: return emptyList()
        return am.getHistoricalProcessExitReasons(null, 0, 16)
            .filter { it.timestamp > sinceMs }
            .mapNotNull { info ->
                val reason = when (info.reason) {
                    ApplicationExitInfo.REASON_CRASH -> "crash"
                    ApplicationExitInfo.REASON_CRASH_NATIVE -> "native-crash"
                    ApplicationExitInfo.REASON_ANR -> "anr"
                    else -> return@mapNotNull null
                }
                mapOf(
                    "time" to info.timestamp,
                    "reason" to reason,
                    "status" to info.status,
                    "description" to info.description,
                    // ANR traces are text; native tombstones are protobuf (Android 12+) and are left out.
                    "trace" to if (info.reason == ApplicationExitInfo.REASON_ANR) trace(info) else null,
                )
            }
    }

    private fun trace(info: ApplicationExitInfo): String? = try {
        info.traceInputStream?.use { s ->
            val bytes = s.readNBytesCompat(MAX_TRACE)
            String(bytes, Charsets.UTF_8)
        }
    } catch (e: Exception) {
        null
    }

    private fun java.io.InputStream.readNBytesCompat(max: Int): ByteArray {
        val out = java.io.ByteArrayOutputStream()
        val buf = ByteArray(8192)
        while (out.size() < max) {
            val n = read(buf, 0, minOf(buf.size, max - out.size()))
            if (n < 0) break
            out.write(buf, 0, n)
        }
        return out.toByteArray()
    }

    private fun saveToDownloads(context: Context, file: File, name: String): String? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return null
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.MIME_TYPE, "application/zip")
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: return null
        try {
            resolver.openOutputStream(uri)?.use { out -> file.inputStream().use { it.copyTo(out) } }
                ?: throw java.io.IOException("no output stream")
            values.clear()
            values.put(MediaStore.Downloads.IS_PENDING, 0)
            resolver.update(uri, values, null, null)
        } catch (e: Exception) {
            resolver.delete(uri, null, null)
            throw e
        }
        return "Download/$name"
    }
}
