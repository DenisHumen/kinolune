// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayOutputStream

/** PLAY-AND-006: fonts attached to a Matroska file, before the clusters or through the SeekHead. */
class MkvAttachmentsTest {
    private val font = ByteArray(70_000) { (it % 251).toByte() } // spans more than one read window

    @Test
    fun fontsBeforeTheClusters() {
        val file = mkv(seekHead = false, attachmentsFirst = true)
        val fonts = MkvAttachments.fonts(source(file))
        assertEquals(listOf("Fancy.ttf", "Other.OTF"), fonts.map { it.name })
        assertArrayEquals(font, fonts[0].data)
    }

    @Test
    fun fontsAfterTheClustersThroughTheSeekHead() {
        val file = mkv(seekHead = true, attachmentsFirst = false)
        assertEquals(listOf("Fancy.ttf", "Other.OTF"), MkvAttachments.fonts(source(file)).map { it.name })
    }

    @Test
    fun afterTheClustersWithoutSeekHeadIsNotSearched() {
        assertTrue(MkvAttachments.fonts(source(mkv(seekHead = false, attachmentsFirst = false))).isEmpty())
    }

    @Test
    fun notMatroska() {
        assertTrue(MkvAttachments.fonts(source(ByteArray(1000) { 0x47 })).isEmpty())
        assertTrue(MkvAttachments.fonts(source(ByteArray(0))).isEmpty())
    }

    @Test
    fun fontsByMimeOrExtension() {
        assertTrue(MkvAttachments.isFont("a.bin", "application/x-truetype-font"))
        assertTrue(MkvAttachments.isFont("A.TTC", null))
        assertTrue(!MkvAttachments.isFont("cover.jpg", "image/jpeg"))
    }

    private fun source(file: ByteArray) = MkvAttachments.Source { position, length ->
        if (position >= file.size) ByteArray(0) else file.copyOfRange(position.toInt(), minOf(file.size.toLong(), position + length).toInt())
    }

    // ---- A tiny EBML writer --------------------------------------------------------------------------------------

    private fun mkv(seekHead: Boolean, attachmentsFirst: Boolean): ByteArray {
        val attachments = element(
            0x1941A469,
            element(0x61A7, element(0x466E, "Fancy.ttf".toByteArray()), element(0x4660, "application/x-truetype-font".toByteArray()), element(0x465C, font)) +
                element(0x61A7, element(0x466E, "cover.jpg".toByteArray()), element(0x4660, "image/jpeg".toByteArray()), element(0x465C, ByteArray(10))) +
                element(0x61A7, element(0x466E, "Other.OTF".toByteArray()), element(0x465C, ByteArray(5) { 7 })),
        )
        val info = element(0x1549A966, element(0x2AD7B1, byteArrayOf(0x0F, 0x42, 0x40)))
        val cluster = element(0x1F43B675, element(0xE7, byteArrayOf(0)), ByteArray(200))
        // SeekHead with a fixed-size entry: 8-byte position, so its size does not depend on the value.
        fun seek(position: Long) = element(
            0x114D9B74,
            element(0x4DBB, element(0x53AB, byteArrayOf(0x19, 0x41, 0xA4.toByte(), 0x69)), element(0x53AC, long8(position))),
        )
        val placeholder = if (seekHead) seek(0) else ByteArray(0)
        val body = if (attachmentsFirst) info + attachments + cluster else info + cluster + attachments
        val attachmentsAt = placeholder.size + body.size - (if (attachmentsFirst) attachments.size + cluster.size else attachments.size)
        val head = if (seekHead) seek(attachmentsAt.toLong()) else ByteArray(0)
        val segment = element(0x18538067, head + body)
        return element(0x1A45DFA3, element(0x4282, "matroska".toByteArray())) + segment
    }

    private fun long8(v: Long) = ByteArray(8) { (v ushr (56 - 8 * it)).toByte() }

    private fun element(id: Long, vararg children: ByteArray): ByteArray {
        val data = ByteArrayOutputStream().apply { children.forEach { write(it) } }.toByteArray()
        val out = ByteArrayOutputStream()
        val idBytes = when {
            id > 0xFFFFFF -> 4
            id > 0xFFFF -> 3
            id > 0xFF -> 2
            else -> 1
        }
        for (i in idBytes - 1 downTo 0) out.write((id ushr (8 * i)).toInt() and 0xFF)
        // 8-byte size: 0x01 marker then 7 bytes.
        out.write(0x01)
        for (i in 6 downTo 0) out.write((data.size.toLong() ushr (8 * i)).toInt() and 0xFF)
        out.write(data)
        return out.toByteArray()
    }
}
