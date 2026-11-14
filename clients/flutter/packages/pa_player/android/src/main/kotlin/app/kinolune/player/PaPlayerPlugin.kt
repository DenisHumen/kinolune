// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player

import android.app.Activity
import android.content.Context
import android.graphics.Typeface
import android.provider.Settings
import android.view.WindowManager
import android.view.accessibility.CaptioningManager
import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.embedding.engine.plugins.activity.ActivityAware
import io.flutter.embedding.engine.plugins.activity.ActivityPluginBinding
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

/**
 * Android side of `pa_player` (PLAY-007): channel `app.kinolune/captioning`, method `captionStyle` — the
 * system caption settings ([CaptioningManager]) that the player's subtitle overlay follows in "system style"
 * mode. Colors are ARGB ints; a key is absent when the user's style leaves it unset.
 *
 * Channel `app.kinolune/screen` (UI-013): `brightness` reads and `setBrightness(0..1)` sets the brightness of the
 * player's window (the OSD's vertical swipe); `setBrightness(-1)` gives it back to the system setting. Only the
 * window changes, so no WRITE_SETTINGS permission is needed and leaving the app restores the system brightness.
 */
class PaPlayerPlugin : FlutterPlugin, ActivityAware, MethodChannel.MethodCallHandler {
    private var channel: MethodChannel? = null
    private var screenChannel: MethodChannel? = null
    private var context: Context? = null
    private var activity: Activity? = null

    override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        context = binding.applicationContext
        channel = MethodChannel(binding.binaryMessenger, CHANNEL).also { it.setMethodCallHandler(this) }
        screenChannel = MethodChannel(binding.binaryMessenger, SCREEN_CHANNEL).also { it.setMethodCallHandler(this) }
    }

    override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel?.setMethodCallHandler(null)
        channel = null
        screenChannel?.setMethodCallHandler(null)
        screenChannel = null
        context = null
    }

    override fun onAttachedToActivity(binding: ActivityPluginBinding) {
        activity = binding.activity
    }

    override fun onDetachedFromActivityForConfigChanges() {
        activity = null
    }

    override fun onReattachedToActivityForConfigChanges(binding: ActivityPluginBinding) {
        activity = binding.activity
    }

    override fun onDetachedFromActivity() {
        activity = null
    }

    /** Window brightness 0..1; the system setting when the window follows it. */
    private fun brightness(act: Activity): Double {
        val own = act.window.attributes.screenBrightness
        if (own >= 0f) return own.toDouble()
        return try {
            Settings.System.getInt(act.contentResolver, Settings.System.SCREEN_BRIGHTNESS) / 255.0
        } catch (e: Settings.SettingNotFoundException) {
            0.5
        }
    }

    private fun setBrightness(act: Activity, value: Double) {
        val lp = act.window.attributes
        lp.screenBrightness = if (value < 0) {
            WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE
        } else {
            // 0 would turn the backlight off on some devices.
            value.coerceIn(0.01, 1.0).toFloat()
        }
        act.window.attributes = lp
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "brightness" -> {
                val act = activity
                if (act == null) result.error("screen", "no activity", null) else result.success(brightness(act))
            }
            "setBrightness" -> {
                val act = activity
                val value = (call.arguments as? Number)?.toDouble()
                if (act == null || value == null) {
                    result.error("screen", "no activity or value", null)
                } else {
                    act.runOnUiThread {
                        setBrightness(act, value)
                        result.success(null)
                    }
                }
            }
            "captionStyle" -> {
                val ctx = context
                if (ctx == null) {
                    result.success(null)
                    return
                }
                try {
                    result.success(captionStyle(ctx))
                } catch (e: RuntimeException) {
                    result.error("captioning", e.message, null)
                }
            }
            else -> result.notImplemented()
        }
    }

    companion object {
        const val CHANNEL = "app.kinolune/captioning"
        const val SCREEN_CHANNEL = "app.kinolune/screen"

        /** Android captioning typefaces (Settings > Accessibility > Caption preferences > Text font). */
        private val TYPEFACES = listOf(
            "sans-serif-condensed",
            "sans-serif-monospace",
            "serif-monospace",
            "sans-serif-smallcaps",
            "serif",
            "monospace",
            "casual",
            "cursive",
            "sans-serif",
        )

        fun captionStyle(ctx: Context): Map<String, Any?> {
            val cm = ctx.getSystemService(Context.CAPTIONING_SERVICE) as CaptioningManager
            val style = cm.userStyle
            val m = HashMap<String, Any?>()
            m["enabled"] = cm.isEnabled
            m["fontScale"] = cm.fontScale.toDouble()
            if (style.hasForegroundColor()) m["foregroundColor"] = style.foregroundColor
            if (style.hasBackgroundColor()) m["backgroundColor"] = style.backgroundColor
            if (style.hasWindowColor()) m["windowColor"] = style.windowColor
            if (style.hasEdgeType()) m["edgeType"] = style.edgeType
            if (style.hasEdgeColor()) m["edgeColor"] = style.edgeColor
            m["typeface"] = typefaceName(ctx, style.typeface)
            m["locale"] = cm.locale?.toLanguageTag()
            return m
        }

        /**
         * Name of the caption typeface. `CaptionStyle` only exposes a [Typeface], made with
         * `Typeface.create(name, NORMAL)`, which returns the cached system instance: compare against those.
         * The setting itself is a fallback (hidden settings may not be readable).
         */
        private fun typefaceName(ctx: Context, typeface: Typeface?): String? {
            if (typeface != null) {
                for (name in TYPEFACES) {
                    val candidate = Typeface.create(name, Typeface.NORMAL)
                    // A family missing on the device comes back as the default one.
                    if (candidate == Typeface.DEFAULT && name != "sans-serif") continue
                    if (candidate == typeface) return name
                }
            }
            return try {
                Settings.Secure.getString(ctx.contentResolver, "accessibility_captioning_typeface")
            } catch (e: SecurityException) {
                null
            }
        }
    }
}
