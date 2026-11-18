// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

/**
 * Fonts attached to a Matroska file (PLAY-AND-006, ADR-0135). Media3 skips MKV attachments, but ASS typesetting
 * (anime above all) names fonts that only live there, so libass gets them from here.
 *
 * A minimal EBML walk: the top level of the Segment up to the first Cluster, and the Attachments element found there
 * or through the SeekHead (mkvmerge writes it before the clusters, other muxers after them). Only fonts are read
 * (by MIME type or extension), at most [MAX_TOTAL_BYTES] in all.
 */
internal object MkvAttachments {
    class Font(val name: String, val data: ByteArray)

    /** Reads up to `length` bytes at `position` of the file; fewer at its end. */
    fun interface Source {
        fun read(position: Long, length: Int): ByteArray
    }

    const val MAX_TOTAL_BYTES = 64L * 1024 * 1024
    private const val WINDOW = 64 * 1024
    private const val MAX_TOP_LEVEL = 256

    private const val ID_EBML = 0x1A45DFA3L
    private const val ID_SEGMENT = 0x18538067L
    private const val ID_SEEK_HEAD = 0x114D9B74L
    private const val ID_SEEK = 0x4DBBL
    private const val ID_SEEK_ID = 0x53ABL
    private const val ID_SEEK_POSITION = 0x53ACL
    private const val ID_ATTACHMENTS = 0x1941A469L
    private const val ID_ATTACHED_FILE = 0x61A7L
    private const val ID_FILE_NAME = 0x466EL
    private const val ID_FILE_MIME = 0x4660L
    private const val ID_FILE_DATA = 0x465CL
    private const val ID_CLUSTER = 0x1F43B675L

    private val FONT_MIMES = setOf(
        "application/x-truetype-font", "application/x-font-ttf", "application/x-font-otf",
        "application/x-font", "application/vnd.ms-opentype", "application/font-sfnt", "application/font-woff",
        "font/ttf", "font/otf", "font/sfnt", "font/collection", "font/woff", "font/woff2",
    )
    private val FONT_EXTENSIONS = setOf("ttf", "otf", "ttc", "otc")

    fun isFont(name: String, mime: String?): Boolean =
        mime?.lowercase()?.trim() in FONT_MIMES || name.substringAfterLast('.', "").lowercase() in FONT_EXTENSIONS

    /** The fonts of the file; empty when it is not Matroska or has none. Throws only what [source] throws. */
    fun fonts(source: Source): List<Font> {
        val r = Reader(source)
        val ebml = r.header(0) ?: return emptyList()
        if (ebml.id != ID_EBML || ebml.size < 0) return emptyList()
        val segment = r.header(ebml.dataStart + ebml.size) ?: return emptyList()
        if (segment.id != ID_SEGMENT) return emptyList()
        val segmentStart = segment.dataStart
        val segmentEnd = if (segment.size < 0) Long.MAX_VALUE else segmentStart + segment.size

        var attachmentsAt: Long? = null
        var pos = segmentStart
        for (i in 0 until MAX_TOP_LEVEL) {
            if (pos >= segmentEnd) break
            val e = r.header(pos) ?: break
            when (e.id) {
                ID_ATTACHMENTS -> return attachments(r, e)
                ID_SEEK_HEAD -> if (e.size in 0..WINDOW.toLong()) {
                    seekTarget(r, e, ID_ATTACHMENTS)?.let { attachmentsAt = segmentStart + it }
                }
                ID_CLUSTER -> break
            }
            if (e.size < 0) break
            pos = e.dataStart + e.size
        }
        val at = attachmentsAt ?: return emptyList()
        val e = r.header(at) ?: return emptyList()
        return if (e.id == ID_ATTACHMENTS) attachments(r, e) else emptyList()
    }

    private fun seekTarget(r: Reader, seekHead: Element, wanted: Long): Long? {
        for (seek in r.children(seekHead)) {
            if (seek.id != ID_SEEK) continue
            var id: Long? = null
            var position: Long? = null
            for (c in r.children(seek)) {
                when (c.id) {
                    ID_SEEK_ID -> id = r.uint(c)
                    ID_SEEK_POSITION -> position = r.uint(c)
                }
            }
            if (id == wanted && position != null) return position
        }
        return null
    }

    private fun attachments(r: Reader, attachments: Element): List<Font> {
        val fonts = ArrayList<Font>()
        var total = 0L
        for (file in r.children(attachments)) {
            if (file.id != ID_ATTACHED_FILE) continue
            var name: String? = null
            var mime: String? = null
            var data: Element? = null
            for (c in r.children(file)) {
                when (c.id) {
                    ID_FILE_NAME -> name = r.string(c)
                    ID_FILE_MIME -> mime = r.string(c)
                    ID_FILE_DATA -> data = c
                }
            }
            val n = name ?: continue
            val d = data ?: continue
            if (!isFont(n, mime) || d.size <= 0 || total + d.size > MAX_TOTAL_BYTES) continue
            val bytes = r.bytes(d.dataStart, d.size.toInt())
            if (bytes.size.toLong() != d.size) break // truncated file
            total += d.size
            fonts.add(Font(n, bytes))
        }
        return fonts
    }

    /** An element: [size] is -1 for "unknown size". */
    private class Element(val id: Long, val dataStart: Long, val size: Long)

    private class Reader(private val source: Source) {
        private var windowStart = 0L
        private var window = ByteArray(0)

        fun bytes(position: Long, length: Int): ByteArray {
            if (position >= windowStart && position + length <= windowStart + window.size) {
                val from = (position - windowStart).toInt()
                return window.copyOfRange(from, from + length)
            }
            return source.read(position, length)
        }

        private fun byteAt(position: Long): Int? {
            if (position < windowStart || position >= windowStart + window.size) {
                window = source.read(position, WINDOW)
                windowStart = position
                if (window.isEmpty()) return null
            }
            return window[(position - windowStart).toInt()].toInt() and 0xFF
        }

        /** Element header at [position], or null at the end of the file or on a malformed header. */
        fun header(position: Long): Element? {
            var p = position
            val first = byteAt(p) ?: return null
            val idLength = Integer.numberOfLeadingZeros(first) - 24 + 1
            if (idLength > 4) return null
            var id = first.toLong()
            for (i in 1 until idLength) id = (id shl 8) or (byteAt(++p) ?: return null).toLong()
            val sizeFirst = byteAt(++p) ?: return null
            val sizeLength = Integer.numberOfLeadingZeros(sizeFirst) - 24 + 1
            if (sizeLength > 8) return null
            var size = (sizeFirst and (0xFF shr sizeLength)).toLong()
            var allOnes = size == (0xFF shr sizeLength).toLong()
            for (i in 1 until sizeLength) {
                val b = byteAt(++p) ?: return null
                size = (size shl 8) or b.toLong()
                allOnes = allOnes && b == 0xFF
            }
            return Element(id, p + 1, if (allOnes) -1 else size)
        }

        fun children(parent: Element): Sequence<Element> = sequence {
            if (parent.size < 0) return@sequence
            val end = parent.dataStart + parent.size
            var p = parent.dataStart
            while (p < end) {
                val e = header(p) ?: break
                if (e.size < 0 || e.dataStart + e.size > end) break
                yield(e)
                p = e.dataStart + e.size
            }
        }

        fun uint(e: Element): Long? {
            if (e.size !in 1..8) return null
            var v = 0L
            for (i in 0 until e.size) v = (v shl 8) or (byteAt(e.dataStart + i) ?: return null).toLong()
            return v
        }

        fun string(e: Element): String? {
            if (e.size !in 0..4096) return null
            return String(bytes(e.dataStart, e.size.toInt()), Charsets.UTF_8).trimEnd('\u0000')
        }
    }
}
