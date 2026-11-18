// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import androidx.media3.common.Format
import androidx.media3.common.MimeTypes
import androidx.media3.extractor.text.SubtitleParser
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** PLAY-AND-006: libass fallback family by language (same rules as ADR-0056) and which tracks go to libass. */
class SubtitleFontsTest {
    private val all = setOf(SubtitleFonts.ROBOTO, SubtitleFonts.CJK, SubtitleFonts.ARABIC, SubtitleFonts.HEBREW, SubtitleFonts.THAI)

    @Test
    fun familyByLanguage() {
        assertEquals("Noto Sans CJK JP", SubtitleFonts.fallbackFamily("ru", all))
        assertEquals("Noto Sans CJK JP", SubtitleFonts.fallbackFamily(null, all))
        assertEquals("Noto Sans CJK JP", SubtitleFonts.fallbackFamily("jpn", all))
        assertEquals("Noto Sans CJK SC", SubtitleFonts.fallbackFamily("zh", all))
        assertEquals("Noto Sans CJK TC", SubtitleFonts.fallbackFamily("zh-Hant", all))
        assertEquals("Noto Sans CJK TC", SubtitleFonts.fallbackFamily("zh-TW", all))
        assertEquals("Noto Sans CJK KR", SubtitleFonts.fallbackFamily("ko", all))
        assertEquals("Noto Naskh Arabic", SubtitleFonts.fallbackFamily("fa", all))
        assertEquals("Noto Sans Hebrew", SubtitleFonts.fallbackFamily("he", all))
        assertEquals("Noto Sans Thai", SubtitleFonts.fallbackFamily("tha", all))
    }

    @Test
    fun withoutNotoCjkRoboto() {
        assertEquals("Roboto", SubtitleFonts.fallbackFamily("ja", setOf(SubtitleFonts.ROBOTO)))
        assertEquals("Roboto", SubtitleFonts.fallbackFamily("ar", setOf(SubtitleFonts.ROBOTO)))
    }

    @Test
    fun assTracksGoToLibass() {
        val embedded = Format.Builder().setSampleMimeType(MimeTypes.TEXT_SSA).build()
        val sideLoaded = Format.Builder().setSampleMimeType(MimeTypes.TEXT_SSA).setId("ext-2").build()
        val embeddedRaw = embedded.buildUpon().setInitializationData(listOf("Format: …".toByteArray(), "[Script Info]".toByteArray())).build()
        val external = Format.Builder().setSampleMimeType(MimeTypes.APPLICATION_MEDIA3_CUES).setCodecs(MimeTypes.TEXT_SSA).setId("1:ext-2").build()
        val srt = Format.Builder().setSampleMimeType(MimeTypes.APPLICATION_SUBRIP).build()
        val cues = Format.Builder().setSampleMimeType(MimeTypes.APPLICATION_MEDIA3_CUES).setCodecs(MimeTypes.APPLICATION_SUBRIP).build()
        assertTrue(AssRenderer.isAss(embedded))
        assertTrue(AssRenderer.isAss(external))
        assertFalse(AssRenderer.isAss(srt))
        assertFalse(AssRenderer.isAss(cues))
        assertEquals("ext-2", AssRenderer.externalId(external))

        val parsers = AssRenderer.parserFactory()
        assertFalse(parsers.supportsFormat(embeddedRaw))
        // A side-loaded file gets a parser without cues: a seekable track for Media3, the file goes to libass.
        assertTrue(parsers.supportsFormat(sideLoaded))
        var cueCount = 0
        parsers.create(sideLoaded).parse("[Script Info]".toByteArray(), SubtitleParser.OutputOptions.allCues()) { cueCount++ }
        assertEquals(0, cueCount)
        assertTrue(parsers.supportsFormat(srt))
        assertTrue(parsers.supportsFormat(Format.Builder().setSampleMimeType(MimeTypes.APPLICATION_PGS).build()))
    }

    @Test
    fun sideLoadedAssKeepsItsCodec() {
        val external = Format.Builder().setSampleMimeType(MimeTypes.APPLICATION_MEDIA3_CUES).setCodecs(MimeTypes.TEXT_SSA).build()
        assertEquals("ass", Media3Mapping.codecName(external))
    }
}
