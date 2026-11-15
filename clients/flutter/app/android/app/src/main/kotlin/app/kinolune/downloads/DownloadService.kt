// SPDX-License-Identifier: MPL-2.0
package app.kinolune.downloads

import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.ServiceCompat

/**
 * The foreground service of type `dataSync` that keeps the process (and the Rust job queue in it) going while
 * offline copies download with no UI (ADR-NNNN). It does no work itself: the queue runs in the process, Dart
 * reports it through [BackgroundDownloads], and this service only holds the foreground and the progress
 * notification. Notification actions start it too (`ACTION_*`), which Android allows from the background.
 */
class DownloadService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        running = this
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // First: a service started with startForegroundService must be in the foreground within seconds.
        if (!goForeground()) {
            stopSelf()
            return START_NOT_STICKY
        }
        val action = when (intent?.action) {
            ACTION_PAUSE -> "pause"
            ACTION_RESUME -> "resume"
            ACTION_CANCEL -> "cancel"
            else -> null
        }
        val job = intent?.getLongExtra(EXTRA_JOB, -1)?.takeIf { it >= 0 }
        BackgroundDownloads.command(this, action, job)
        BackgroundDownloads.serviceStarted()
        // Not sticky: a restart from the background is not allowed on Android 12+; the WorkManager kick restarts.
        return START_NOT_STICKY
    }

    private fun goForeground(): Boolean {
        val n = DownloadNotification.build(this, BackgroundDownloads.state().snapshot, ongoing = true)
        return try {
            ServiceCompat.startForeground(
                this,
                DownloadNotification.ID,
                n,
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC else 0,
            )
            foreground = true
            true
        } catch (e: RuntimeException) {
            // Android 15 after the daily dataSync time, or a start the system no longer allows.
            Log.w(TAG, "cannot go foreground: $e")
            DownloadWorker.runNow(this, DownloadPolicy.kickNetwork(BackgroundDownloads.state().snapshot))
            false
        }
    }

    // Android 15: dataSync services get 6 hours a day; the rest goes through WorkManager.
    override fun onTimeout(startId: Int, fgsType: Int) {
        Log.w(TAG, "dataSync time is up, WorkManager takes over")
        DownloadWorker.schedule(this, DownloadPolicy.kickNetwork(BackgroundDownloads.state().snapshot))
        stop(keepNotification = true)
    }

    private fun stop(keepNotification: Boolean) {
        foreground = false
        ServiceCompat.stopForeground(
            this,
            if (keepNotification) ServiceCompat.STOP_FOREGROUND_DETACH else ServiceCompat.STOP_FOREGROUND_REMOVE,
        )
        stopSelf()
    }

    override fun onDestroy() {
        if (running === this) running = null
        foreground = false
        super.onDestroy()
    }

    companion object {
        private const val TAG = "KinoluneDownloads"
        const val ACTION_PAUSE = "app.kinolune.downloads.PAUSE"
        const val ACTION_RESUME = "app.kinolune.downloads.RESUME"
        const val ACTION_CANCEL = "app.kinolune.downloads.CANCEL"
        const val EXTRA_JOB = "job"

        private var running: DownloadService? = null

        /** In the foreground now (read by the worker from its thread). */
        @Volatile
        var foreground = false
            private set

        /** Refreshes the notification of a running service; false if none runs. */
        fun update(s: DownloadSnapshot): Boolean {
            val service = running ?: return false
            if (!foreground) return false
            DownloadNotification.show(service, s, ongoing = true)
            return true
        }

        /** Leaves the foreground and stops; the notification stays (detached) or goes. */
        fun leave(keepNotification: Boolean) {
            running?.stop(keepNotification)
        }
    }
}
