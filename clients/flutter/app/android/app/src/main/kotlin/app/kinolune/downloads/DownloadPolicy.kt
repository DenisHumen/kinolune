// SPDX-License-Identifier: MPL-2.0
package app.kinolune.downloads

/**
 * The downloads of the job queue as Dart reports them (ADR-NNNN): counts by what keeps them from running and the
 * download shown in the notification.
 */
data class DownloadSnapshot(
    /** Unfinished downloads: queued, running, retrying, waiting or paused. */
    val active: Int = 0,
    /** Downloads that run, or will run without the user (queued, retrying, waiting for space). */
    val runnable: Int = 0,
    /** Downloads waiting for a network ("Wi-Fi only" on mobile data, or offline). */
    val blockedByNetwork: Int = 0,
    /** Paused by the user. */
    val paused: Int = 0,
    /** Paused by the app for Battery Saver while it has no visible UI; resumed when that ends. */
    val held: Int = 0,
    val jobId: Long? = null,
    val title: String? = null,
    val bytesDone: Long = 0,
    val bytesTotal: Long? = null,
    val wifiOnly: Boolean = true,
) {
    companion object {
        val EMPTY = DownloadSnapshot()

        @Suppress("UNCHECKED_CAST")
        fun fromMap(m: Map<*, *>): DownloadSnapshot {
            fun int(k: String) = (m[k] as? Number)?.toInt() ?: 0
            return DownloadSnapshot(
                active = int("active"),
                runnable = int("runnable"),
                blockedByNetwork = int("blockedByNetwork"),
                paused = int("paused"),
                held = int("held"),
                jobId = (m["jobId"] as? Number)?.toLong(),
                title = m["title"] as? String,
                bytesDone = (m["bytesDone"] as? Number)?.toLong() ?: 0,
                bytesTotal = (m["bytesTotal"] as? Number)?.toLong(),
                wifiOnly = m["wifiOnly"] as? Boolean ?: true,
            )
        }
    }
}

/** How the downloads are kept going while the app has no UI. */
enum class DownloadHost {
    /** Nothing to download: no service, no notification, no scheduled kick. */
    STOP,

    /** A foreground service of type `dataSync` with an ongoing notification. */
    FOREGROUND,

    /** Only a notification (paused, held, or waiting for a network too long); a WorkManager kick resumes. */
    NOTIFY,
}

/** The network the WorkManager kick waits for. */
enum class KickNetwork { UNMETERED, CONNECTED }

/** Pure decisions of the background downloads, unit-tested (`DownloadPolicyTest`). */
object DownloadPolicy {
    /** How long a service waits in the foreground for the network before it leaves that to WorkManager. */
    const val NETWORK_GRACE_MS = 10 * 60_000L

    /**
     * Where the downloads run. [blockedSinceMs] is when the queue started to wait for the network only, or null.
     */
    fun host(s: DownloadSnapshot, blockedSinceMs: Long?, nowMs: Long): DownloadHost = when {
        s.active <= 0 -> DownloadHost.STOP
        s.runnable > 0 -> DownloadHost.FOREGROUND
        s.blockedByNetwork > 0 && (blockedSinceMs == null || nowMs - blockedSinceMs < NETWORK_GRACE_MS) ->
            DownloadHost.FOREGROUND
        else -> DownloadHost.NOTIFY
    }

    /** Only waiting for the network: the moment that started is kept for [NETWORK_GRACE_MS]. */
    fun waitsForNetworkOnly(s: DownloadSnapshot): Boolean = s.runnable <= 0 && s.blockedByNetwork > 0

    /** The Flutter engine (and the queue in it) stays without an activity while something will run by itself. */
    fun keepsEngine(s: DownloadSnapshot): Boolean = s.runnable > 0 || s.blockedByNetwork > 0

    /**
     * A periodic WorkManager kick is scheduled while a download would go on by itself: it restarts the app's
     * engine (and so the queue) after the process was killed, and when the service could not start.
     */
    fun needsKick(s: DownloadSnapshot): Boolean = s.runnable > 0 || s.blockedByNetwork > 0 || s.held > 0

    fun kickNetwork(s: DownloadSnapshot): KickNetwork =
        if (s.wifiOnly) KickNetwork.UNMETERED else KickNetwork.CONNECTED

    /** Battery Saver holds downloads only while no UI of the app is visible. */
    fun holds(powerSave: Boolean, uiVisible: Boolean): Boolean = powerSave && !uiVisible

    /** What the notification says. */
    enum class Status { DOWNLOADING, WAITING_WIFI, WAITING_NETWORK, PAUSED, HELD }

    fun status(s: DownloadSnapshot): Status = when {
        s.runnable > 0 -> Status.DOWNLOADING
        s.blockedByNetwork > 0 -> if (s.wifiOnly) Status.WAITING_WIFI else Status.WAITING_NETWORK
        s.held > 0 -> Status.HELD
        else -> Status.PAUSED
    }

    /** Pause in the notification while something runs or waits; Resume when everything is paused. */
    fun offersResume(s: DownloadSnapshot): Boolean = s.runnable <= 0 && s.blockedByNetwork <= 0

    /** 0–1000 for the progress bar, or null (unknown size). */
    fun progressPermille(s: DownloadSnapshot): Int? {
        val total = s.bytesTotal ?: return null
        if (total <= 0) return null
        return ((s.bytesDone.coerceIn(0, total) * 1000) / total).toInt()
    }
}
