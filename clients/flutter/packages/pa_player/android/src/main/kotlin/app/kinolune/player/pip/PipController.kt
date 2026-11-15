// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.pip

import android.app.Activity
import android.app.AppOpsManager
import android.app.Application
import android.app.PictureInPictureParams
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.util.Log
import android.util.Rational
import androidx.annotation.RequiresApi

/**
 * Picture-in-picture for the video player (PLAY-AND-008), channel `app.kinolune/pip`.
 *
 * - `configure {autoEnter, width, height}`: while a video plays the system may enter PiP by itself when the user
 *   leaves the app — `setAutoEnterEnabled` on Android 12+, [onUserLeaveHint] on 8–11.
 * - `enter {width, height}`: the OSD button.
 * - Events to Dart, `changed {inPip, dismissed}`: `dismissed` when the PiP window was closed rather than expanded
 *   (the activity did not come back to the foreground), so the player pauses.
 *
 * The activity declares `supportsPictureInPicture` and handles orientation / size changes itself, so a rotation
 * neither recreates it nor leaves PiP. The play / pause / next controls of the PiP window come from the media
 * session ([app.kinolune.player.media.PlaybackService]).
 */
class PipController(private val send: (Map<String, Any?>) -> Unit) {
    private var activity: Activity? = null
    private var autoEnter = false
    private var aspect = Rational(16, 9)
    private var resumed = true
    private val main = Handler(Looper.getMainLooper())

    private val lifecycle = object : Application.ActivityLifecycleCallbacks {
        override fun onActivityResumed(a: Activity) {
            if (a === activity) resumed = true
        }

        override fun onActivityPaused(a: Activity) {
            if (a === activity) resumed = false
        }

        override fun onActivityCreated(a: Activity, b: Bundle?) {}
        override fun onActivityStarted(a: Activity) {}
        override fun onActivityStopped(a: Activity) {}
        override fun onActivitySaveInstanceState(a: Activity, b: Bundle) {}
        override fun onActivityDestroyed(a: Activity) {}
    }

    fun attach(a: Activity) {
        if (activity === a) return
        detach()
        activity = a
        resumed = true
        a.application.registerActivityLifecycleCallbacks(lifecycle)
        current = this
        if (autoEnter) apply()
    }

    fun detach() {
        activity?.application?.unregisterActivityLifecycleCallbacks(lifecycle)
        activity = null
        if (current === this) current = null
    }

    fun isSupported(): Boolean {
        val a = activity ?: return false
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return false
        if (!a.packageManager.hasSystemFeature(PackageManager.FEATURE_PICTURE_IN_PICTURE)) return false
        // The user may turn PiP off for the app in the system settings.
        val ops = a.getSystemService(Context.APP_OPS_SERVICE) as AppOpsManager
        val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            ops.unsafeCheckOpNoThrow(AppOpsManager.OPSTR_PICTURE_IN_PICTURE, Process.myUid(), a.packageName)
        } else {
            @Suppress("DEPRECATION")
            ops.checkOpNoThrow(AppOpsManager.OPSTR_PICTURE_IN_PICTURE, Process.myUid(), a.packageName)
        }
        return mode == AppOpsManager.MODE_ALLOWED
    }

    fun configure(enabled: Boolean, width: Int?, height: Int?) {
        autoEnter = enabled
        setAspect(width, height)
        apply()
    }

    fun enter(width: Int?, height: Int?): Boolean {
        val a = activity ?: return false
        if (!isSupported()) return false
        setAspect(width, height)
        return try {
            a.enterPictureInPictureMode(params(autoEnter))
        } catch (e: IllegalStateException) {
            Log.w(TAG, "enterPictureInPictureMode: ${e.message}")
            false
        }
    }

    /** Android 8–11: Home or Recents while a video plays enters PiP. */
    fun onUserLeaveHint() {
        if (!autoEnter || Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) return
        val a = activity ?: return
        if (a.isInPictureInPictureMode || !isSupported()) return
        try {
            a.enterPictureInPictureMode(params(true))
        } catch (e: IllegalStateException) {
            Log.w(TAG, "auto PiP: ${e.message}")
        }
    }

    fun onModeChanged(inPip: Boolean) {
        Log.i(TAG, "PiP mode $inPip")
        if (inPip) {
            send(mapOf("inPip" to true, "dismissed" to false))
            return
        }
        // Expanded: the activity resumes right away. Closed: it stays stopped.
        main.postDelayed({ send(mapOf("inPip" to false, "dismissed" to !resumed)) }, DISMISS_CHECK_MS)
    }

    private fun setAspect(width: Int?, height: Int?) {
        if (width == null || height == null || width <= 0 || height <= 0) return
        // The system accepts 1:2.39 .. 2.39:1.
        val r = (width.toDouble() / height).coerceIn(1 / 2.39 + 0.001, 2.39 - 0.001)
        aspect = Rational((r * 10_000).toInt(), 10_000)
    }

    private fun apply() {
        val a = activity ?: return
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O || !isSupported()) return
        try {
            a.setPictureInPictureParams(params(autoEnter))
        } catch (e: IllegalStateException) {
            Log.w(TAG, "setPictureInPictureParams: ${e.message}")
        }
    }

    @RequiresApi(Build.VERSION_CODES.O)
    private fun params(auto: Boolean): PictureInPictureParams {
        val b = PictureInPictureParams.Builder().setAspectRatio(aspect)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            b.setAutoEnterEnabled(auto)
            // Video: the system may crossfade instead of resizing seamlessly.
            b.setSeamlessResizeEnabled(false)
        }
        return b.build()
    }

    companion object {
        private const val TAG = "KinolunePip"
        private const val DISMISS_CHECK_MS = 400L

        /** The controller of the attached activity, for [Activity.onPictureInPictureModeChanged]. */
        @Volatile
        var current: PipController? = null
            private set
    }
}
