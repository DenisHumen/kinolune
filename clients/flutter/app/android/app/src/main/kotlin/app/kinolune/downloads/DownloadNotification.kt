// SPDX-License-Identifier: MPL-2.0
package app.kinolune.downloads

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.text.format.Formatter
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import app.kinolune.R

/** The progress notification of the background downloads, with Pause / Resume and Cancel (ADR-NNNN). */
object DownloadNotification {
    const val ID = 0x4b4c_0019
    private const val CHANNEL_ID = "downloads"

    fun build(context: Context, s: DownloadSnapshot, ongoing: Boolean): Notification {
        ensureChannel(context)
        val b = NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setShowWhen(false)
            .setCategory(NotificationCompat.CATEGORY_PROGRESS)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setOngoing(ongoing)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
        context.packageManager.getLaunchIntentForPackage(context.packageName)?.let { launch ->
            b.setContentIntent(
                PendingIntent.getActivity(context, 0, launch, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT),
            )
        }
        if (s.active <= 0) {
            return b.setContentTitle(context.getString(R.string.downloads_preparing)).setProgress(0, 0, true).build()
        }
        val title = s.title ?: context.getString(R.string.downloads_title)
        val status = DownloadPolicy.status(s)
        b.setContentTitle(
            when (status) {
                DownloadPolicy.Status.DOWNLOADING -> title
                DownloadPolicy.Status.WAITING_WIFI -> context.getString(R.string.downloads_waiting_wifi)
                DownloadPolicy.Status.WAITING_NETWORK -> context.getString(R.string.downloads_waiting_network)
                DownloadPolicy.Status.HELD -> context.getString(R.string.downloads_held)
                DownloadPolicy.Status.PAUSED -> context.getString(R.string.downloads_paused)
            },
        )
        val parts = mutableListOf<String>()
        if (status != DownloadPolicy.Status.DOWNLOADING) parts += title
        val total = s.bytesTotal
        parts += if (total != null && total > 0) {
            context.getString(
                R.string.downloads_bytes,
                Formatter.formatShortFileSize(context, s.bytesDone),
                Formatter.formatShortFileSize(context, total),
            )
        } else {
            Formatter.formatShortFileSize(context, s.bytesDone)
        }
        if (s.active > 1) parts += context.resources.getQuantityString(R.plurals.downloads_more, s.active - 1, s.active - 1)
        b.setContentText(parts.joinToString(" · "))
        if (status == DownloadPolicy.Status.DOWNLOADING) {
            val p = DownloadPolicy.progressPermille(s)
            if (p != null) b.setProgress(1000, p, false) else b.setProgress(0, 0, true)
        } else {
            b.setSmallIcon(android.R.drawable.stat_sys_download_done)
        }
        if (DownloadPolicy.offersResume(s)) {
            b.addAction(0, context.getString(R.string.downloads_resume), action(context, DownloadService.ACTION_RESUME, null))
        } else {
            b.addAction(0, context.getString(R.string.downloads_pause), action(context, DownloadService.ACTION_PAUSE, null))
        }
        b.addAction(0, context.getString(R.string.downloads_cancel), action(context, DownloadService.ACTION_CANCEL, s.jobId))
        return b.build()
    }

    private fun action(context: Context, action: String, jobId: Long?): PendingIntent {
        val intent = Intent(context, DownloadService::class.java).setAction(action)
        if (jobId != null) intent.putExtra(DownloadService.EXTRA_JOB, jobId)
        val flags = PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        // From a notification Android lets the app start a foreground service even in the background.
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            PendingIntent.getForegroundService(context, action.hashCode(), intent, flags)
        } else {
            PendingIntent.getService(context, action.hashCode(), intent, flags)
        }
    }

    /** Shows or refreshes the notification (without the permission it is silently not shown). */
    fun show(context: Context, s: DownloadSnapshot, ongoing: Boolean = false) {
        val nm = NotificationManagerCompat.from(context)
        if (!nm.areNotificationsEnabled()) return
        try {
            nm.notify(ID, build(context, s, ongoing))
        } catch (e: SecurityException) {
            // POST_NOTIFICATIONS refused on Android 13+.
        }
    }

    fun cancel(context: Context) {
        NotificationManagerCompat.from(context).cancel(ID)
    }

    private fun ensureChannel(context: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val nm = context.getSystemService(NotificationManager::class.java)
        if (nm.getNotificationChannel(CHANNEL_ID) != null) return
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, context.getString(R.string.downloads_channel), NotificationManager.IMPORTANCE_LOW),
        )
    }
}
