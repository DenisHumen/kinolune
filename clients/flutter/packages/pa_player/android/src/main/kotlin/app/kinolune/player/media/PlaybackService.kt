// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.media

import android.app.PendingIntent
import android.content.Intent
import androidx.annotation.OptIn
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService

/**
 * The media session of the app (PLAY-AND-008): a Media3 [MediaSessionService] (foreground service of type
 * `mediaPlayback`) over the [BridgePlayer] of [MediaBridge]. Media3 posts the media notification with artwork and
 * controls, keeps the service in the foreground while something plays (background music, screen off), and routes
 * media buttons (Bluetooth, wired headset, `adb shell input keyevent KEYCODE_MEDIA_*`) and lock-screen / system
 * controls to the player.
 */
@OptIn(UnstableApi::class)
class PlaybackService : MediaSessionService() {
    private var session: MediaSession? = null

    override fun onCreate() {
        super.onCreate()
        val player = MediaBridge.player()
        val b = MediaSession.Builder(this, player).setId(SESSION_ID)
        packageManager.getLaunchIntentForPackage(packageName)?.let { launch ->
            launch.flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_REORDER_TO_FRONT
            b.setSessionActivity(
                PendingIntent.getActivity(
                    this,
                    0,
                    launch,
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                ),
            )
        }
        session = b.build()
        MediaBridge.onServiceCreated(this)
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    /**
     * The app was swiped away from Recents (ADR-NNNN). The app keeps its Flutter engine (and the libmpv players in
     * it) alive without the activity while music owns the session, so music goes on and the notification keeps
     * controlling it; a video or a live stream is stopped and the service with it.
     */
    override fun onTaskRemoved(rootIntent: Intent?) {
        if (MediaBridge.onTaskRemoved(this)) stopSelf()
    }

    override fun onDestroy() {
        MediaBridge.onServiceDestroyed(this)
        session?.release()
        session = null
        super.onDestroy()
    }

    companion object {
        const val SESSION_ID = "kinolune"
    }
}
