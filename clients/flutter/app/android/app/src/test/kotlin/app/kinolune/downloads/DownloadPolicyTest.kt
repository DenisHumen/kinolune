// SPDX-License-Identifier: MPL-2.0
package app.kinolune.downloads

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DownloadPolicyTest {
    private val running = DownloadSnapshot(active = 2, runnable = 2, jobId = 7, title = "Film", bytesDone = 50, bytesTotal = 200)
    private val blocked = DownloadSnapshot(active = 1, blockedByNetwork = 1, wifiOnly = true)
    private val paused = DownloadSnapshot(active = 1, paused = 1)
    private val held = DownloadSnapshot(active = 1, held = 1)

    @Test
    fun emptyQueueStopsEverything() {
        assertEquals(DownloadHost.STOP, DownloadPolicy.host(DownloadSnapshot.EMPTY, null, 0))
        assertFalse(DownloadPolicy.keepsEngine(DownloadSnapshot.EMPTY))
        assertFalse(DownloadPolicy.needsKick(DownloadSnapshot.EMPTY))
    }

    @Test
    fun runningDownloadsHoldTheForegroundAndTheEngine() {
        assertEquals(DownloadHost.FOREGROUND, DownloadPolicy.host(running, null, 0))
        assertTrue(DownloadPolicy.keepsEngine(running))
        assertTrue(DownloadPolicy.needsKick(running))
        assertEquals(DownloadPolicy.Status.DOWNLOADING, DownloadPolicy.status(running))
        assertFalse(DownloadPolicy.offersResume(running))
        assertEquals(250, DownloadPolicy.progressPermille(running))
    }

    @Test
    fun waitingForWifiKeepsTheForegroundForTheGraceOnly() {
        assertTrue(DownloadPolicy.waitsForNetworkOnly(blocked))
        assertEquals(DownloadHost.FOREGROUND, DownloadPolicy.host(blocked, 1_000, 1_000 + 60_000))
        assertEquals(
            DownloadHost.NOTIFY,
            DownloadPolicy.host(blocked, 1_000, 1_000 + DownloadPolicy.NETWORK_GRACE_MS),
        )
        // The kick then waits for Wi-Fi; the engine stays to resume when it is back.
        assertTrue(DownloadPolicy.needsKick(blocked))
        assertTrue(DownloadPolicy.keepsEngine(blocked))
        assertEquals(KickNetwork.UNMETERED, DownloadPolicy.kickNetwork(blocked))
        assertEquals(KickNetwork.CONNECTED, DownloadPolicy.kickNetwork(blocked.copy(wifiOnly = false)))
        assertEquals(DownloadPolicy.Status.WAITING_WIFI, DownloadPolicy.status(blocked))
        assertEquals(DownloadPolicy.Status.WAITING_NETWORK, DownloadPolicy.status(blocked.copy(wifiOnly = false)))
    }

    @Test
    fun pausedByTheUserOnlyNotifies() {
        assertEquals(DownloadHost.NOTIFY, DownloadPolicy.host(paused, null, 0))
        assertFalse(DownloadPolicy.keepsEngine(paused))
        assertFalse(DownloadPolicy.needsKick(paused))
        assertTrue(DownloadPolicy.offersResume(paused))
        assertEquals(DownloadPolicy.Status.PAUSED, DownloadPolicy.status(paused))
    }

    @Test
    fun batterySaverHoldsOnlyWithoutVisibleUi() {
        assertTrue(DownloadPolicy.holds(powerSave = true, uiVisible = false))
        assertFalse(DownloadPolicy.holds(powerSave = true, uiVisible = true))
        assertFalse(DownloadPolicy.holds(powerSave = false, uiVisible = false))
        assertEquals(DownloadHost.NOTIFY, DownloadPolicy.host(held, null, 0))
        assertFalse(DownloadPolicy.keepsEngine(held))
        // A kick comes back to resume them once Battery Saver is off.
        assertTrue(DownloadPolicy.needsKick(held))
        assertEquals(DownloadPolicy.Status.HELD, DownloadPolicy.status(held))
    }

    @Test
    fun aRunningDownloadWinsOverPausedOnes() {
        val mixed = DownloadSnapshot(active = 3, runnable = 1, paused = 1, blockedByNetwork = 1)
        assertEquals(DownloadHost.FOREGROUND, DownloadPolicy.host(mixed, null, 0))
        assertEquals(DownloadPolicy.Status.DOWNLOADING, DownloadPolicy.status(mixed))
    }

    @Test
    fun unknownSizeHasNoProgress() {
        assertNull(DownloadPolicy.progressPermille(running.copy(bytesTotal = null)))
        assertNull(DownloadPolicy.progressPermille(running.copy(bytesTotal = 0)))
    }

    @Test
    fun snapshotFromDart() {
        val s = DownloadSnapshot.fromMap(
            mapOf(
                "active" to 3, "runnable" to 1, "blockedByNetwork" to 1, "paused" to 1, "held" to 0,
                "jobId" to 42, "title" to "Ep 1", "bytesDone" to 10L, "bytesTotal" to 4_000_000_000L,
                "wifiOnly" to false,
            ),
        )
        assertEquals(DownloadSnapshot(3, 1, 1, 1, 0, 42, "Ep 1", 10, 4_000_000_000L, false), s)
        assertEquals(DownloadSnapshot.EMPTY, DownloadSnapshot.fromMap(emptyMap<String, Any>()))
    }
}
