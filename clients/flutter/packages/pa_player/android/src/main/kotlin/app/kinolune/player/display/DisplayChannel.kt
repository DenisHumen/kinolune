// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.display

import android.app.Activity
import dev.projectalpha.display.DisplayMatchOptions
import dev.projectalpha.display.DisplayModeMatcher
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

/**
 * Channel `app.kinolune/display` (PLAY-AND-005, ADR-0147): auto frame rate for the media_kit player.
 *
 * - `capabilities` → `{sdk, mode, modes, hdr, dolby_vision_profiles}` of the display (or null without a window);
 * - `match {fps, width, height, options}` switches the window's display mode ([DisplayModeMatcher]) and answers
 *   with the mode in use once the TV reports it (`"3840x2160@23.976"`, null: the display was not touched);
 * - `restore` puts the mode from before the first switch back (player closed).
 *
 * The media_kit video is a Flutter texture inside the app's own window, so the mode applies to the whole app while
 * the player is open; [restore] ends it. HDR does not pass through that texture (ADR-0008): the window stays SDR.
 */
class DisplayChannel : MethodChannel.MethodCallHandler {
    private var channel: MethodChannel? = null
    private var matcher: DisplayModeMatcher? = null
    private var activity: Activity? = null

    fun attachToEngine(messenger: BinaryMessenger) {
        channel = MethodChannel(messenger, CHANNEL).also { it.setMethodCallHandler(this) }
    }

    fun detachFromEngine() {
        channel?.setMethodCallHandler(null)
        channel = null
    }

    fun attachToActivity(activity: Activity) {
        if (this.activity === activity) return
        this.activity = activity
        matcher = DisplayModeMatcher(activity)
    }

    /** The activity is gone for good: its window takes the preference with it. */
    fun detachFromActivity(gone: Boolean) {
        if (!gone) return
        matcher?.restore()
        matcher = null
        activity = null
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        val m = matcher
        when (call.method) {
            "capabilities" -> result.success(m?.capabilities()?.toMap())
            "match" -> {
                if (m == null) return result.success(null)
                val fps = (call.argument<Number>("fps") ?: 0).toFloat()
                val width = call.argument<Number>("width")?.toInt() ?: 0
                val height = call.argument<Number>("height")?.toInt() ?: 0
                val options = DisplayMatchOptions.fromMap(call.argument<Map<*, *>>("options"))
                var answered = false
                m.match(fps, width, height, options) { mode ->
                    if (!answered) {
                        answered = true
                        result.success(mode?.toString())
                    }
                }
            }
            "restore" -> {
                m?.restore()
                result.success(null)
            }
            else -> result.notImplemented()
        }
    }

    companion object {
        const val CHANNEL = "app.kinolune/display"
    }
}
