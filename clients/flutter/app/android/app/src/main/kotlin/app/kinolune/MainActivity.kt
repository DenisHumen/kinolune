// SPDX-License-Identifier: MPL-2.0
package app.kinolune

import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.os.Bundle
import app.kinolune.player.PaPlayerPlugin
import app.kinolune.tv.TvBridge
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine

class MainActivity : FlutterActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        KinoluneEngine.activityCreated()
        // UI-025: a personal library on screen keeps the window out of Recents previews.
        KinoluneEngine.attachWindow(this)
        // UI-045: the system listening screen of voice search answers this activity.
        VoiceBridge.attachActivity(this)
        // PLAY-AND-009: a Watch Next card or a voice search that started the app; a restored activity has none.
        if (savedInstanceState == null) {
            TvBridge.onIntent(intent)
            // UI-035: files shared to Kinolune.
            ShareBridge.onIntent(intent)
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        TvBridge.onIntent(intent)
        ShareBridge.onIntent(intent)
    }

    // ADR-0068: the engine (the Dart app, its libmpv players and the music queue) belongs to the process, so
    // music goes on when the activity is gone (swiped away from Recents) and a new activity shows it live.
    override fun provideFlutterEngine(context: Context): FlutterEngine = KinoluneEngine.obtain(context)

    override fun shouldDestroyEngineWithHost(): Boolean = false

    // The channel `app.kinolune/device` lives on the engine (KinoluneEngine): it outlives this activity and also
    // starts without one for the background downloads (ADR-0072).

    // PLAY-AND-008: FlutterActivity gives plugins no PiP callback.
    override fun onPictureInPictureModeChanged(isInPictureInPictureMode: Boolean, newConfig: Configuration) {
        super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
        PaPlayerPlugin.onPictureInPictureModeChanged(isInPictureInPictureMode)
    }

    // UI-045: the text of the system listening screen (VoiceBridge); other results go to the plugins.
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (VoiceBridge.onActivityResult(requestCode, resultCode, data)) return
        super.onActivityResult(requestCode, resultCode, data)
    }

    override fun onDestroy() {
        VoiceBridge.detachActivity(this)
        val changing = isChangingConfigurations
        super.onDestroy()
        KinoluneEngine.activityDestroyed(changing)
    }
}
