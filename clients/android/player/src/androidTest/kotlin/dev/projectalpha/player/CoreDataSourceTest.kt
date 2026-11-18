// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import android.net.Uri
import androidx.media3.common.PlaybackException
import androidx.media3.datasource.DataSourceException
import androidx.media3.datasource.DataSpec
import androidx.media3.datasource.DefaultDataSource
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.projectalpha.core.MediaSource
import dev.projectalpha.core.OpenRequest
import dev.projectalpha.core.streamTestServeDir
import org.junit.AfterClass
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * PLAY-AND-003: [CoreDataSource] reads proxy URLs of the core in-process, falls back to HTTP through the proxy when
 * the URL is not served by this core, and the engine plays through both paths. A test proxy over the fixture folder
 * (`streamTestServeDir`, core built with the `testing` feature) stands in for the library's proxy.
 */
@RunWith(AndroidJUnit4::class)
class CoreDataSourceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val file: File = File(Uri.parse(Fixtures.uri("h264.mkv")).path!!)
    private val bytes: ByteArray by lazy { file.readBytes() }

    @Test
    fun readsTheFileDirectlyFromTheCore() {
        val factory = CoreDataSource.Factory(context)
        val url = "$base/h264.mkv"
        assertArrayEquals(bytes, readAll(factory, url))
        assertArrayEquals(bytes.copyOfRange(1000, 51_000), readAll(factory, url, 1000, 50_000))
        // A range longer than the rest of the file ends at the end.
        assertArrayEquals(bytes.copyOfRange(bytes.size - 10, bytes.size), readAll(factory, url, bytes.size - 10L, 100))
        assertEquals(3L, factory.counters.directOpens.get())
        assertEquals(0L, factory.counters.fallbackOpens.get())
        assertEquals(bytes.size + 50_000L + 10, factory.counters.directBytes.get())

        assertOpenFails(factory, url, bytes.size + 1L, PlaybackException.ERROR_CODE_IO_READ_POSITION_OUT_OF_RANGE)
        assertOpenFails(factory, "$base/missing.mkv", 0, PlaybackException.ERROR_CODE_IO_FILE_NOT_FOUND)
    }

    @Test
    fun fallsBackToHttpWhenTheUrlIsNotServedHere() {
        // Another port in front of the same proxy: the core does not know it, so the URL goes over HTTP.
        PortForwarder(port(base)).use { fwd ->
            val factory = CoreDataSource.Factory(context)
            val url = "$base/h264.mkv".replace(":${port(base)}/", ":${fwd.port}/")
            assertArrayEquals(bytes, readAll(factory, url))
            assertArrayEquals(bytes.copyOfRange(5000, 6000), readAll(factory, url, 5000, 1000))
            assertEquals(0L, factory.counters.directOpens.get())
            assertEquals(2L, factory.counters.fallbackOpens.get())
        }
        // `direct = false` always reads over HTTP.
        val http = CoreDataSource.Factory(context, direct = false)
        assertArrayEquals(bytes, readAll(http, "$base/h264.mkv"))
        assertEquals(1L, http.counters.fallbackOpens.get())
        // Other URIs go to the upstream source as they are.
        val files = CoreDataSource.Factory(context)
        assertArrayEquals(bytes, readAll(files, Fixtures.uri("h264.mkv")))
        assertEquals(0L, files.counters.directOpens.get())
    }

    @Test
    fun theEnginePlaysThroughTheCoreAndThroughTheProxy() {
        for (direct in listOf(true, false)) {
            val factory = CoreDataSource.Factory(context, DefaultDataSource.Factory(context), direct)
            EngineUnderTest { Media3Engine.player(it, factory) }.use { under ->
                val engine = under.engine
                engine.openMedia(OpenRequest(listOf(MediaSource("$base/h264.mkv", null)), 0u, 0uL, true))
                under.advance(0)
                var s = engine.state()
                assertTrue("loaded (direct=$direct): $s", s.loaded && s.playing && s.error == null)
                under.advance(1_500)
                engine.seek(8_000uL)
                under.advance(1_000)
                s = engine.state()
                assertTrue("playing after seek (direct=$direct): $s", s.playing && s.positionMs >= 8_500uL)
                val (rendered, _) = onMain { engine.videoOutput() }
                assertTrue("rendered frames: $rendered", rendered >= 20)
                if (direct) {
                    assertTrue(factory.counters.directOpens.get() >= 1)
                    assertEquals(0L, factory.counters.fallbackOpens.get())
                } else {
                    assertEquals(0L, factory.counters.directOpens.get())
                    assertTrue(factory.counters.fallbackOpens.get() >= 1)
                }
            }
        }
    }

    @Test
    fun aMissingFileIsASourceError() {
        EngineUnderTest().use { under ->
            under.engine.openMedia(OpenRequest(listOf(MediaSource("$base/missing.mkv", null)), 0u, 0uL, true))
            under.advance(0)
            val error = under.engine.state().error
            assertTrue("error: $error", error is dev.projectalpha.core.EngineException.Source)
        }
    }

    private fun readAll(factory: CoreDataSource.Factory, url: String, position: Long = 0, length: Long = -1): ByteArray {
        val ds = factory.createDataSource()
        try {
            ds.open(DataSpec.Builder().setUri(url).setPosition(position).setLength(length).build())
            val out = java.io.ByteArrayOutputStream()
            val buf = ByteArray(37_000) // not a multiple of the chunk: reads cross chunk borders
            while (true) {
                val n = ds.read(buf, 0, buf.size)
                if (n < 0) break
                out.write(buf, 0, n)
            }
            return out.toByteArray()
        } finally {
            ds.close()
        }
    }

    private fun assertOpenFails(factory: CoreDataSource.Factory, url: String, position: Long, code: Int) {
        val ds = factory.createDataSource()
        try {
            ds.open(DataSpec.Builder().setUri(url).setPosition(position).build())
            fail("open $url at $position should fail")
        } catch (e: DataSourceException) {
            assertEquals(code, e.reason)
        } finally {
            ds.close()
        }
    }

    companion object {
        private val base: String by lazy {
            val dir = File(Uri.parse(Fixtures.uri("h264.mkv")).path!!).parentFile!!
            streamTestServeDir(dir.absolutePath)
        }

        private fun port(url: String) = Uri.parse(url).port

        @JvmStatic
        @AfterClass
        fun stopServers() = dev.projectalpha.core.streamTestStopAll()
    }
}
