// SPDX-License-Identifier: MPL-2.0
package app.kinolune.downloads

import android.content.Context
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.ForegroundInfo
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import java.util.concurrent.TimeUnit

/**
 * The WorkManager side of the background downloads (ADR-NNNN).
 *
 * A periodic kick (every 15 min, on the network the downloads need and with the battery not low) is scheduled while
 * a download would go on by itself: after the process was killed it starts the app's Flutter engine again, whose
 * job queue resumes the downloads, and then tries the foreground service. When Android 12+ refuses the service
 * (a start from the background), this worker itself holds the process while the downloads run, up to
 * [MAX_RUN_MS], and the next kick continues.
 */
class DownloadWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result {
        val main = Handler(Looper.getMainLooper())
        main.post { BackgroundDownloads.command(applicationContext, null, null) }
        val start = SystemClock.elapsedRealtime()
        var triedForeground = false
        while (!isStopped && SystemClock.elapsedRealtime() - start < MAX_RUN_MS) {
            try {
                Thread.sleep(POLL_MS)
            } catch (e: InterruptedException) {
                break
            }
            if (DownloadService.foreground) break
            val st = BackgroundDownloads.state()
            if (!st.known) {
                if (SystemClock.elapsedRealtime() - start > BackgroundDownloads.DART_TIMEOUT_MS) break
                continue
            }
            if (!DownloadPolicy.keepsEngine(st.snapshot)) break
            if (!triedForeground) {
                triedForeground = true
                try {
                    setForegroundAsync(foregroundInfo()).get()
                    Log.i(TAG, "worker holds the downloads in the foreground")
                } catch (e: Exception) {
                    // Not allowed from the background either: the worker's own run time holds the process.
                    Log.i(TAG, "worker runs the downloads without the foreground: $e")
                }
            }
            main.post { DownloadNotification.show(applicationContext, BackgroundDownloads.state().snapshot) }
        }
        val st = BackgroundDownloads.state()
        if (!isStopped && !DownloadService.foreground && st.known && DownloadPolicy.keepsEngine(st.snapshot)) {
            // Run time is up and the downloads go on: the next run follows right away.
            runNow(applicationContext, DownloadPolicy.kickNetwork(st.snapshot), replace = true)
        }
        return Result.success()
    }

    override fun getForegroundInfo(): ForegroundInfo = foregroundInfo()

    private fun foregroundInfo(): ForegroundInfo {
        val n = DownloadNotification.build(applicationContext, BackgroundDownloads.state().snapshot, ongoing = true)
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            ForegroundInfo(DownloadNotification.ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            ForegroundInfo(DownloadNotification.ID, n)
        }
    }

    companion object {
        private const val TAG = "KinoluneDownloads"
        private const val PERIODIC = "kinolune-downloads-kick"
        private const val NOW = "kinolune-downloads-now"
        private const val POLL_MS = 2_000L

        /** Less than WorkManager's 10 minutes, so the run ends by itself and is not counted as stopped. */
        const val MAX_RUN_MS = 9 * 60_000L

        /** What this process last asked WorkManager for: a [KickNetwork], [NONE], or null (not yet asked). */
        @Volatile
        private var scheduled: Any? = null
        private val NONE = Any()

        private fun constraints(network: KickNetwork) = Constraints.Builder()
            .setRequiredNetworkType(
                when (network) {
                    KickNetwork.UNMETERED -> NetworkType.UNMETERED
                    KickNetwork.CONNECTED -> NetworkType.CONNECTED
                },
            )
            .setRequiresBatteryNotLow(true)
            .build()

        /** The periodic kick for [network]; rescheduled only when that changes. */
        fun schedule(context: Context, network: KickNetwork) {
            if (scheduled == network) return
            scheduled = network
            val request = PeriodicWorkRequestBuilder<DownloadWorker>(15, TimeUnit.MINUTES)
                .setConstraints(constraints(network))
                .setInitialDelay(15, TimeUnit.MINUTES)
                .build()
            WorkManager.getInstance(context)
                .enqueueUniquePeriodicWork(PERIODIC, ExistingPeriodicWorkPolicy.UPDATE, request)
        }

        /** Runs the downloads now under WorkManager (the foreground service was refused). */
        fun runNow(context: Context, network: KickNetwork, replace: Boolean = false) {
            val request = OneTimeWorkRequestBuilder<DownloadWorker>()
                .setConstraints(constraints(network))
                .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
                .build()
            val policy = if (replace) ExistingWorkPolicy.APPEND_OR_REPLACE else ExistingWorkPolicy.KEEP
            WorkManager.getInstance(context).enqueueUniqueWork(NOW, policy, request)
            if (scheduled === NONE) scheduled = null
        }

        fun cancel(context: Context) {
            // Also when this process never scheduled it: a kick of an earlier process may still be there.
            if (scheduled === NONE) return
            scheduled = NONE
            val wm = WorkManager.getInstance(context)
            wm.cancelUniqueWork(PERIODIC)
            wm.cancelUniqueWork(NOW)
        }
    }
}
