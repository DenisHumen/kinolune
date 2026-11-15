// SPDX-License-Identifier: MPL-2.0
package app.kinolune

import android.app.UiModeManager
import android.content.Context
import android.content.pm.PackageManager
import android.content.res.Configuration
import app.kinolune.player.PaPlayerPlugin
import android.os.Build
import android.os.Bundle
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        KinoluneEngine.activityCreated()
    }

    // ADR-NNNN: the engine (the Dart app, its libmpv players and the music queue) belongs to the process, so
    // music goes on when the activity is gone (swiped away from Recents) and a new activity shows it live.
    override fun provideFlutterEngine(context: Context): FlutterEngine = KinoluneEngine.obtain(context)

    override fun shouldDestroyEngineWithHost(): Boolean = false

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, DEVICE_CHANNEL)
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "isTelevision" -> result.success(isTelevision())
                    "deviceName" -> result.success(Build.MODEL)
                    "multicastLock" -> {
                        KinoluneEngine.setMulticastLock(applicationContext, call.arguments as? Boolean ?: false)
                        result.success(null)
                    }
                    else -> result.notImplemented()
                }
            }
    }

    // The engine outlives this activity: its handler must not keep the activity.
    override fun cleanUpFlutterEngine(flutterEngine: FlutterEngine) {
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, DEVICE_CHANNEL).setMethodCallHandler(null)
        super.cleanUpFlutterEngine(flutterEngine)
    }

    // PLAY-AND-008: FlutterActivity gives plugins no PiP callback.
    override fun onPictureInPictureModeChanged(isInPictureInPictureMode: Boolean, newConfig: Configuration) {
        super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
        PaPlayerPlugin.onPictureInPictureModeChanged(isInPictureInPictureMode)
    }

    override fun onDestroy() {
        val changing = isChangingConfigurations
        super.onDestroy()
        KinoluneEngine.activityDestroyed(changing)
    }

    private fun isTelevision(): Boolean {
        val uiMode = getSystemService(Context.UI_MODE_SERVICE) as UiModeManager
        if (uiMode.currentModeType == Configuration.UI_MODE_TYPE_TELEVISION) return true
        return packageManager.hasSystemFeature(PackageManager.FEATURE_LEANBACK)
    }

    private companion object {
        const val DEVICE_CHANNEL = "app.kinolune/device"
    }
}
