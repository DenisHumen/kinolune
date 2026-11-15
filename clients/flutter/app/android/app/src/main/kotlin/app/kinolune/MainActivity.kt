// SPDX-License-Identifier: MPL-2.0
package app.kinolune

import android.content.Context
import android.content.res.Configuration
import android.os.Bundle
import app.kinolune.player.PaPlayerPlugin
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine

class MainActivity : FlutterActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        KinoluneEngine.activityCreated()
    }

    // ADR-NNNN: the engine (the Dart app, its libmpv players and the music queue) belongs to the process, so
    // music goes on when the activity is gone (swiped away from Recents) and a new activity shows it live.
    override fun provideFlutterEngine(context: Context): FlutterEngine = KinoluneEngine.obtain(context)

    override fun shouldDestroyEngineWithHost(): Boolean = false

    // The channel `app.kinolune/device` lives on the engine (KinoluneEngine): it outlives this activity and also
    // starts without one for the background downloads (ADR-NNNN).

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
}
