// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import android.net.Uri
import androidx.media3.datasource.DataSpec
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.projectalpha.core.streamTestServeDir
import dev.projectalpha.core.streamTestStopAll
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * PLAY-AND-003: the UniFFI bindings bound to the app's core (`libpa_frb.so`, [CoreLibrary.preferAppCore]) work, and
 * only that copy of the core is loaded. Needs `libpa_frb.so` (built with `pa-ffi/testing`) in the test APK and a
 * process of its own: run it alone (`-e class dev.projectalpha.player.AppCoreTest`, argument `appCore=1`), because the
 * bindings pick their library once per process. Skipped otherwise.
 */
@RunWith(AndroidJUnit4::class)
class AppCoreTest {
    @Test
    fun bindsTheBindingsToTheAppCore() {
        assumeTrue("run alone with appCore=1", InstrumentationRegistry.getArguments().getString("appCore") == "1")
        assertTrue("libpa_frb.so in the APK", CoreLibrary.preferAppCore())
        assertEquals(CoreLibrary.APP_CORE, System.getProperty(CoreLibrary.OVERRIDE_PROPERTY))

        val file = File(Uri.parse(Fixtures.uri("h264.mkv")).path!!)
        val base = streamTestServeDir(file.parentFile!!.absolutePath)
        try {
            val factory = CoreDataSource.Factory(InstrumentationRegistry.getInstrumentation().targetContext)
            val ds = factory.createDataSource()
            ds.open(DataSpec.Builder().setUri("$base/h264.mkv").setPosition(100).setLength(1000).build())
            val buf = ByteArray(1000)
            var n = 0
            while (n < buf.size) {
                val r = ds.read(buf, n, buf.size - n)
                if (r < 0) break
                n += r
            }
            ds.close()
            assertArrayEquals(file.readBytes().copyOfRange(100, 1100), buf)
            assertEquals(1L, factory.counters.directOpens.get())
        } finally {
            streamTestStopAll()
        }

        val maps = File("/proc/self/maps").readText()
        assertTrue("libpa_frb.so is mapped", maps.contains("libpa_frb.so"))
        assertFalse("libpa_ffi.so is not mapped", maps.contains("libpa_ffi.so"))
    }
}
