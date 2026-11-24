// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player

import android.app.Activity
import android.content.Context
import android.graphics.Typeface
import android.provider.Settings
import android.system.ErrnoException
import android.system.Os
import android.view.WindowManager
import android.view.accessibility.CaptioningManager
import app.kinolune.player.display.DisplayChannel
import app.kinolune.player.media.MediaBridge
import app.kinolune.player.media.OutputPicker
import app.kinolune.player.pip.PipController
import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.embedding.engine.plugins.activity.ActivityAware
import io.flutter.embedding.engine.plugins.activity.ActivityPluginBinding
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.PluginRegistry
import java.io.File

/**
 * Android side of `pa_player` (PLAY-007): channel `app.kinolune/captioning`, method `captionStyle` — the
 * system caption settings ([CaptioningManager]) that the player's subtitle overlay follows in "system style"
 * mode. Colors are ARGB ints; a key is absent when the user's style leaves it unset.
 *
 * Same channel, method `subtitleFonts` (PLAY-006): libass in our libmpv has no font provider on Android, so a few
 * system fonts (Roboto, Noto Sans CJK, Arabic, Hebrew, Thai) are linked into one directory for `sub-fonts-dir`.
 * Returns `{dir, files}`; links, not copies: the fonts stay where the system keeps them.
 *
 * Channel `app.kinolune/screen` (UI-013): `brightness` reads and `setBrightness(0..1)` sets the brightness of the
 * player's window (the OSD's vertical swipe); `setBrightness(-1)` gives it back to the system setting. Only the
 * window changes, so no WRITE_SETTINGS permission is needed and leaving the app restores the system brightness.
 *
 * Channel `app.kinolune/media_session` (PLAY-AND-008): `update {…}` and `clear` from Dart drive the system media
 * session ([MediaBridge]); the session calls back with `command {action, positionMs?, reason}`.
 *
 * Channel `app.kinolune/pip` (PLAY-AND-008): `isSupported`, `configure`, `enter`; events `changed` ([PipController]).
 *
 * Channel `app.kinolune/audio_output` (MUS-005): `showPicker` opens the system audio output picker ([OutputPicker]).
 * Channel `app.kinolune/display` (PLAY-AND-005): display mode switching for the video's frame rate ([DisplayChannel]).
 */
class PaPlayerPlugin : FlutterPlugin, ActivityAware, MethodChannel.MethodCallHandler {
    private var channel: MethodChannel? = null
    private var screenChannel: MethodChannel? = null
    private var sessionChannel: MethodChannel? = null
    private var pipChannel: MethodChannel? = null
    private var outputChannel: MethodChannel? = null
    private var context: Context? = null
    private var activity: Activity? = null
    private var binding: ActivityPluginBinding? = null
    private val pip = PipController { args -> pipChannel?.invokeMethod("changed", args) }
    private val leaveHint = PluginRegistry.UserLeaveHintListener { pip.onUserLeaveHint() }
    private val display = DisplayChannel()

    override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        context = binding.applicationContext
        channel = MethodChannel(binding.binaryMessenger, CHANNEL).also { it.setMethodCallHandler(this) }
        screenChannel = MethodChannel(binding.binaryMessenger, SCREEN_CHANNEL).also { it.setMethodCallHandler(this) }
        sessionChannel = MethodChannel(binding.binaryMessenger, SESSION_CHANNEL).also { it.setMethodCallHandler(this) }
        pipChannel = MethodChannel(binding.binaryMessenger, PIP_CHANNEL).also { it.setMethodCallHandler(this) }
        outputChannel = MethodChannel(binding.binaryMessenger, OUTPUT_CHANNEL).also { it.setMethodCallHandler(this) }
        display.attachToEngine(binding.binaryMessenger)
    }

    override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel?.setMethodCallHandler(null)
        channel = null
        screenChannel?.setMethodCallHandler(null)
        screenChannel = null
        sessionChannel?.setMethodCallHandler(null)
        sessionChannel = null
        pipChannel?.setMethodCallHandler(null)
        pipChannel = null
        outputChannel?.setMethodCallHandler(null)
        outputChannel = null
        display.detachFromEngine()
        // The Dart players are gone with the engine: nothing to show or control any more. Other engines (the
        // background one of workmanager) never own the session, so their detach leaves it alone.
        MediaBridge.release(this, binding.applicationContext)
        context = null
    }

    private fun attach(b: ActivityPluginBinding) {
        activity = b.activity
        binding = b
        b.addOnUserLeaveHintListener(leaveHint)
        pip.attach(b.activity)
        display.attachToActivity(b.activity)
    }

    private fun detach(gone: Boolean) {
        binding?.removeOnUserLeaveHintListener(leaveHint)
        binding = null
        activity = null
        pip.detach(gone)
        display.detachFromActivity(gone)
    }

    override fun onAttachedToActivity(binding: ActivityPluginBinding) = attach(binding)

    override fun onDetachedFromActivityForConfigChanges() = detach(gone = false)

    override fun onReattachedToActivityForConfigChanges(binding: ActivityPluginBinding) = attach(binding)

    override fun onDetachedFromActivity() = detach(gone = true)

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
            "update" -> {
                val ctx = context
                val state = call.arguments as? Map<*, *>
                val ch = sessionChannel
                if (ctx != null && state != null && ch != null) {
                    MediaBridge.update(ctx, state, this) { args -> ch.invokeMethod("command", args) }
                }
                result.success(null)
            }
            "clear" -> {
                context?.let { MediaBridge.release(this, it) }
                result.success(null)
            }
            "isSupported" -> result.success(pip.isSupported())
            "showPicker" -> {
                val ctx: Context? = activity ?: context
                result.success(ctx != null && OutputPicker.show(ctx))
            }
            "configure" -> {
                val args = call.arguments as? Map<*, *>
                pip.configure(
                    args?.get("autoEnter") == true,
                    (args?.get("width") as? Number)?.toInt(),
                    (args?.get("height") as? Number)?.toInt(),
                )
                result.success(null)
            }
            "enter" -> {
                val args = call.arguments as? Map<*, *>
                result.success(
                    pip.enter((args?.get("width") as? Number)?.toInt(), (args?.get("height") as? Number)?.toInt()),
                )
            }
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
            "subtitleFonts" -> {
                val ctx = context
                if (ctx == null) {
                    result.success(null)
                    return
                }
                try {
                    result.success(subtitleFonts(ctx))
                } catch (e: Exception) {
                    result.error("fonts", e.message, null)
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
        const val SESSION_CHANNEL = "app.kinolune/media_session"
        const val PIP_CHANNEL = "app.kinolune/pip"
        const val OUTPUT_CHANNEL = "app.kinolune/audio_output"

        /** Call from the activity's `onPictureInPictureModeChanged` (FlutterActivity gives plugins no hook). */
        @JvmStatic
        fun onPictureInPictureModeChanged(inPip: Boolean) {
            PipController.current?.onModeChanged(inPip)
        }

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

        /** System fonts libass gets on Android, by file name, in order of preference. */
        private val SUBTITLE_FONTS = listOf(
            "Roboto-Regular.ttf",
            "NotoSansCJK-Regular.ttc",
            "NotoNaskhArabic-Regular.ttf",
            "NotoSansHebrew-Regular.ttf",
            "NotoSansThai-Regular.ttf",
        )
        private val FONT_DIRS = listOf("/system/fonts", "/product/fonts")

        /** (Re)creates `<no-backup files>/subfonts` with links to the system fonts that exist on this device. */
        fun subtitleFonts(ctx: Context): Map<String, Any?> {
            val dir = File(ctx.noBackupFilesDir, "subfonts")
            dir.mkdirs()
            // Links from an older system image may point to missing files.
            dir.listFiles()?.forEach { it.delete() }
            val linked = ArrayList<String>()
            for (name in SUBTITLE_FONTS) {
                val source = FONT_DIRS.map { File(it, name) }.firstOrNull { it.canRead() } ?: continue
                try {
                    Os.symlink(source.path, File(dir, name).path)
                    linked.add(name)
                } catch (e: ErrnoException) {
                    // Skip this font; the others still help.
                }
            }
            return mapOf("dir" to dir.path, "files" to linked)
        }

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
