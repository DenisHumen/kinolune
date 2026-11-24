// SPDX-License-Identifier: MPL-2.0
package app.kinolune

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.speech.RecognizerIntent
import android.util.Log
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.lang.ref.WeakReference

/**
 * Voice input for Search (UI-045, ADR-NNNN), channel `app.kinolune/voice` on the app's engine:
 *
 * * `available()` — a system speech recognizer is installed (Android TV and Google TV have one);
 * * `listen({language, prompt})` — opens the system listening screen ([RecognizerIntent.ACTION_RECOGNIZE_SPEECH])
 *   and answers with the best text, or null when the user said nothing or cancelled.
 *
 * The system recognizer records and recognizes; the app gets text only and needs no microphone permission. The
 * engine outlives the activity (ADR-0068), so [MainActivity] tells this bridge which activity is in front and
 * hands it the activity result.
 */
object VoiceBridge {
    private const val TAG = "VoiceBridge"
    private const val CHANNEL = "app.kinolune/voice"

    /** Request code of the listening screen. */
    const val REQUEST = 0x4b56

    private var activity: WeakReference<Activity>? = null
    private var pending: MethodChannel.Result? = null

    fun attach(context: Context, engine: FlutterEngine) {
        val app = context.applicationContext
        MethodChannel(engine.dartExecutor.binaryMessenger, CHANNEL).setMethodCallHandler { call, result ->
            when (call.method) {
                "available" -> result.success(recognizer(null, null).resolveActivity(app.packageManager) != null)
                "listen" -> listen(call.argument<String>("language"), call.argument<String>("prompt"), result)
                else -> result.notImplemented()
            }
        }
    }

    fun attachActivity(a: Activity) {
        activity = WeakReference(a)
    }

    fun detachActivity(a: Activity) {
        if (activity?.get() === a) activity = null
        // The listening screen cannot answer a gone activity.
        pending?.success(null)
        pending = null
    }

    private fun recognizer(language: String?, prompt: String?) =
        Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
            putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_WEB_SEARCH)
            putExtra(RecognizerIntent.EXTRA_MAX_RESULTS, 1)
            if (!language.isNullOrEmpty()) putExtra(RecognizerIntent.EXTRA_LANGUAGE, language)
            if (!prompt.isNullOrEmpty()) putExtra(RecognizerIntent.EXTRA_PROMPT, prompt)
        }

    private fun listen(language: String?, prompt: String?, result: MethodChannel.Result) {
        val a = activity?.get()
        if (a == null) {
            result.success(null)
            return
        }
        // A second request replaces the first one.
        pending?.success(null)
        pending = result
        try {
            a.startActivityForResult(recognizer(language, prompt), REQUEST)
        } catch (e: ActivityNotFoundException) {
            Log.w(TAG, "no speech recognizer")
            pending = null
            result.success(null)
        }
    }

    /** The answer of the listening screen; false for another request code. */
    fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?): Boolean {
        if (requestCode != REQUEST) return false
        val r = pending ?: return true
        pending = null
        val text = if (resultCode == Activity.RESULT_OK) {
            data?.getStringArrayListExtra(RecognizerIntent.EXTRA_RESULTS)?.firstOrNull()
        } else {
            null
        }
        r.success(text)
        return true
    }
}
