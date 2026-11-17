// SPDX-License-Identifier: MPL-2.0
package app.kinolune

import org.junit.Assert.assertEquals
import org.junit.Test
import java.io.File
import java.nio.file.Files

class ShareNamesTest {
    @Test
    fun namesStayOneComponent() {
        assertEquals("clip.mp4", ShareNames.safe("clip.mp4"))
        assertEquals("evil.mp4", ShareNames.safe("../../evil.mp4"))
        assertEquals("a.mp4", ShareNames.safe("C:\\x\\a.mp4"))
        assertEquals("file", ShareNames.safe(null))
        assertEquals("file", ShareNames.safe(".."))
        assertEquals("ab.mp4", ShareNames.safe("a\u0000b.mp4"))
    }

    @Test
    fun aTakenNameGetsANumber() {
        val dir: File = Files.createTempDirectory("share").toFile()
        File(dir, "v.mp4").writeText("x")
        File(dir, "v (2).mp4").writeText("x")
        assertEquals("v (3).mp4", ShareNames.free(dir, "v.mp4").name)
        assertEquals("w.mp4", ShareNames.free(dir, "w.mp4").name)
        dir.deleteRecursively()
    }
}
