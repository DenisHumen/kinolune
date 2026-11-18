// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import android.app.Activity
import android.content.Context
import android.graphics.Color
import android.os.Bundle
import android.os.SystemClock
import android.view.Gravity
import android.view.SurfaceView
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.widget.FrameLayout
import androidx.media3.exoplayer.ExoPlayer
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import dev.projectalpha.core.ContractFixture
import dev.projectalpha.core.MediaSource
import dev.projectalpha.core.PlayerEngine
import dev.projectalpha.core.PlayerEngineHarness
import java.io.File
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import kotlin.concurrent.thread

/** The video picture of the player Activity (ADR-0008): SurfaceView and subtitle overlay ([VideoLayout]). */
class SurfaceActivity : Activity() {
    lateinit var video: VideoLayout
    val surfaceView: SurfaceView get() = video.surface

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        video = VideoLayout(this)
        val root = FrameLayout(this)
        root.setBackgroundColor(Color.BLACK)
        root.addView(video, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT, Gravity.CENTER))
        setContentView(root)
    }
}

/** Runs [block] on the main thread and returns its result. */
fun <T> onMain(block: () -> T): T {
    var result: Result<T>? = null
    InstrumentationRegistry.getInstrumentation().runOnMainSync { result = runCatching(block) }
    return result!!.getOrThrow()
}

/** Test media from tools/make-fixtures.sh, copied from the test APK assets to the cache directory. */
object Fixtures {
    /** Durations as Media3 reports them (12 s of video; AAC adds a few ms). */
    const val DURATION_MS = 12_000uL

    private val dir: File by lazy {
        val context = InstrumentationRegistry.getInstrumentation().context
        val target = InstrumentationRegistry.getInstrumentation().targetContext
        File(target.cacheDir, "fixtures").apply {
            mkdirs()
            for (name in listOf("h264.mkv", "hevc.mkv", "external.srt", "karaoke.mkv", "karaoke.ass", "pgs.mkv")) {
                context.assets.open(name).use { input -> File(this, name).outputStream().use { input.copyTo(it) } }
            }
        }
    }

    fun uri(name: String): String = "file://" + File(dir, name).absolutePath

    /** H.264 and HEVC: the contract plays both codecs. */
    fun contract() = ContractFixture(
        items = listOf(MediaSource(uri("h264.mkv"), "H.264"), MediaSource(uri("hevc.mkv"), "HEVC")),
        durationsMs = listOf(DURATION_MS, DURATION_MS),
        broken = MediaSource(uri("missing.mkv"), null),
        externalSubtitle = uri("external.srt"),
    )
}

/** A [Media3Engine] rendering into [SurfaceActivity]. Close it after the test. */
class EngineUnderTest(
    playerFactory: ((Context) -> ExoPlayer)? = null,
) : AutoCloseable {
    private val scenario = ActivityScenario.launch(SurfaceActivity::class.java)
    val engine: Media3Engine = onMain {
        Media3Engine(InstrumentationRegistry.getInstrumentation().targetContext, playerFactory)
    }

    lateinit var activity: SurfaceActivity
        private set

    init {
        scenario.onActivity {
            activity = it
            it.video.bind(engine)
        }
    }

    /**
     * Waits [ms] of wall time, then until the engine has finished the commands sent so far (loading, seeking,
     * track changes), then publishes the current position: what the contract means by `advance`.
     */
    fun advance(ms: Long) {
        if (ms > 0) Thread.sleep(ms)
        val deadline = SystemClock.elapsedRealtime() + SETTLE_TIMEOUT_MS
        while (!onMain { engine.isSettled() } && SystemClock.elapsedRealtime() < deadline) Thread.sleep(20)
        onMain { engine.publishPosition() }
    }

    override fun close() {
        runCatching { engine.closeEngine() }
        scenario.close()
    }

    companion object {
        const val SETTLE_TIMEOUT_MS = 20_000L
    }
}

/** The contract harness of pa-ffi over a real [Media3Engine]. */
class Media3Harness(private val under: EngineUnderTest) : PlayerEngineHarness {
    override fun engine(): PlayerEngine = under.engine
    override fun fixture(): ContractFixture = Fixtures.contract()
    override fun advance(ms: ULong) = under.advance(ms.toLong())
}

/**
 * A TCP forwarder on a free loopback port in front of `127.0.0.1:[target]`: the same proxy under a port the core
 * does not know, so [CoreDataSource] has to read the URL over HTTP (PLAY-AND-003 fallback).
 */
class PortForwarder(private val target: Int) : AutoCloseable {
    private val server = ServerSocket(0, 16, InetAddress.getByName("127.0.0.1"))
    val port: Int = server.localPort

    init {
        thread(isDaemon = true, name = "forwarder") {
            while (!server.isClosed) {
                val client = runCatching { server.accept() }.getOrNull() ?: break
                val upstream = Socket("127.0.0.1", target)
                pump(client, upstream)
                pump(upstream, client)
            }
        }
    }

    private fun pump(from: Socket, to: Socket) = thread(isDaemon = true) {
        runCatching { from.getInputStream().copyTo(to.getOutputStream()) }
        runCatching { to.shutdownOutput() }
        runCatching { from.close() }
    }

    override fun close() = server.close()
}
