// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import android.os.SystemClock
import android.util.Log
import androidx.annotation.OptIn
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DataSpec
import androidx.media3.exoplayer.analytics.AnalyticsListener
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.projectalpha.core.MediaSource
import dev.projectalpha.core.OpenRequest
import dev.projectalpha.core.streamTestServeSmb
import dev.projectalpha.core.streamTestStopAll
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * PLAY-AND-003 "done when", emulator part: a high-bitrate remux from Samba plays without stalls through
 * [CoreDataSource] (direct, in-process) and through the HTTP proxy (fallback); throughput of both paths.
 *
 * Runs only with a Samba server given as instrumentation arguments (skipped otherwise):
 * `smbServer` (`host:port`, the emulator reaches the host as 10.0.2.2), `smbShare`, `smbUser`, `smbPassword`,
 * `smbFile` (path in the share); optional `playSeconds` (default 60) and `readMb` (default 1024). Results go to
 * logcat with the tag `PLAYAND003` as `key=value` lines.
 */
@RunWith(AndroidJUnit4::class)
@OptIn(UnstableApi::class)
class SambaStreamingTest {
    private val args = InstrumentationRegistry.getArguments()
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    private fun arg(name: String): String? = args.getString(name)?.takeIf { it.isNotEmpty() }

    private val url: String by lazy {
        val base = streamTestServeSmb(arg("smbServer")!!, arg("smbShare")!!, arg("smbUser")!!, arg("smbPassword")!!)
        "$base/" + arg("smbFile")!!.split('/').joinToString("/") { android.net.Uri.encode(it) }
    }

    @After
    fun stop() = streamTestStopAll()

    @Test
    fun throughputDirectAndThroughTheProxy() {
        assumeTrue("no Samba server given", arg("smbServer") != null)
        val limit = (arg("readMb")?.toLong() ?: 1024L) * MB
        // Alternate the paths, so caches on either side help both the same.
        for (round in 1..2) {
            for (direct in listOf(true, false)) {
                val factory = CoreDataSource.Factory(context, direct = direct)
                val ds = factory.createDataSource()
                val started = SystemClock.elapsedRealtime()
                var total = 0L
                try {
                    ds.open(DataSpec.Builder().setUri(url).setLength(limit).build())
                    val buf = ByteArray(64 * 1024) // ExoPlayer's extractors read about this much at a time
                    while (true) {
                        val n = ds.read(buf, 0, buf.size)
                        if (n < 0) break
                        total += n
                    }
                } finally {
                    ds.close()
                }
                val ms = (SystemClock.elapsedRealtime() - started).coerceAtLeast(1)
                val mbit = total * 8.0 / 1e6 / (ms / 1000.0)
                report("read path=${path(direct)} round=$round bytes=$total ms=$ms mbit_s=${"%.1f".format(mbit)}")
                assertEquals(if (direct) 1L else 0L, factory.counters.directOpens.get())
            }
        }
    }

    @Test
    fun playsWithoutStallsDirectAndThroughTheProxy() {
        assumeTrue("no Samba server given", arg("smbServer") != null)
        val seconds = arg("playSeconds")?.toLong() ?: 60L
        for (direct in listOf(true, false)) {
            val factory = CoreDataSource.Factory(context, direct = direct)
            val watch = Watch()
            EngineUnderTest { Media3Engine.player(it, factory) }.use { under ->
                val engine = under.engine
                onMain { engine.exoPlayer.addAnalyticsListener(watch) }
                val opened = SystemClock.elapsedRealtime()
                engine.openMedia(OpenRequest(listOf(MediaSource(url, null)), 0u, 0uL, true))
                under.advance(0)
                val startMs = SystemClock.elapsedRealtime() - opened
                var s = engine.state()
                assertTrue("loaded (${path(direct)}): $s", s.loaded && s.playing && s.error == null)

                // Plain playback: the position must keep up with the wall clock, no rebuffering.
                val before = s.positionMs.toLong()
                val wallStart = SystemClock.elapsedRealtime()
                watch.counting = true
                under.advance(seconds * 1000)
                watch.counting = false
                s = engine.state()
                val played = s.positionMs.toLong() - before
                val wall = SystemClock.elapsedRealtime() - wallStart
                val (rendered, decoder) = onMain { engine.videoOutput() }

                // Seeks across the file: time until playback resumes.
                val duration = s.durationMs.toLong()
                val seekTimes = listOf(0.3, 0.6, 0.1).map { f ->
                    val t0 = SystemClock.elapsedRealtime()
                    engine.seek((duration * f).toLong().toULong())
                    under.advance(0)
                    val ms = SystemClock.elapsedRealtime() - t0
                    under.advance(2_000)
                    ms
                }
                s = engine.state()
                val stats = engine.stats()
                report(
                    "play path=${path(direct)} start_ms=$startMs played_ms=$played wall_ms=$wall " +
                        "rebuffers=${watch.rebuffers} rebuffer_ms=${watch.rebufferMs} dropped=${watch.dropped} " +
                        "rendered=$rendered decoder=$decoder size=${stats.videoWidth}x${stats.videoHeight} " +
                        "codec=${stats.videoCodec} bitrate_estimate=${watch.bitrate} seek_ms=${seekTimes.joinToString(",")} " +
                        "direct_opens=${factory.counters.directOpens.get()} fallback_opens=${factory.counters.fallbackOpens.get()} " +
                        "error=${s.error}",
                )
                assertTrue("still playing after seeks (${path(direct)}): $s", s.playing && s.error == null)
                assertEquals("rebuffers while playing (${path(direct)})", 0, watch.rebuffers)
                assertTrue("position kept up (${path(direct)}): $played ms in $wall ms", played >= wall * 0.95)
                if (direct) {
                    assertEquals(0L, factory.counters.fallbackOpens.get())
                } else {
                    assertEquals(0L, factory.counters.directOpens.get())
                }
            }
        }
    }

    /** Rebuffers (READY → BUFFERING while playing), dropped frames and the bandwidth estimate. */
    private class Watch : AnalyticsListener {
        @Volatile var counting = false
        var rebuffers = 0
        var rebufferMs = 0L
        var dropped = 0L
        var bitrate = 0L
        private var bufferingSince = 0L

        override fun onPlaybackStateChanged(eventTime: AnalyticsListener.EventTime, state: Int) {
            if (state == Player.STATE_BUFFERING && counting) {
                rebuffers++
                bufferingSince = SystemClock.elapsedRealtime()
            } else if (state == Player.STATE_READY && bufferingSince > 0) {
                rebufferMs += SystemClock.elapsedRealtime() - bufferingSince
                bufferingSince = 0
            }
        }

        override fun onDroppedVideoFrames(eventTime: AnalyticsListener.EventTime, droppedFrames: Int, elapsedMs: Long) {
            dropped += droppedFrames
        }

        override fun onBandwidthEstimate(
            eventTime: AnalyticsListener.EventTime,
            totalLoadTimeMs: Int,
            totalBytesLoaded: Long,
            bitrateEstimate: Long,
        ) {
            bitrate = bitrateEstimate
        }
    }

    private fun path(direct: Boolean) = if (direct) "direct" else "proxy"

    private fun report(line: String) {
        Log.i(TAG, line)
        println("$TAG $line")
    }

    companion object {
        const val TAG = "PLAYAND003"
        const val MB = 1024L * 1024
    }
}
