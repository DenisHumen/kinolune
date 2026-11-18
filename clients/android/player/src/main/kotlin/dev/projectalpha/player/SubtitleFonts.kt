// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import java.io.File

/**
 * System fonts for libass on Android (PLAY-AND-006, ADR-0135): the same set and fallback rules as the libmpv path
 * (ADR-0056, `SubtitleFontDir` in pa_player). libass has no font provider here, so it gets these files, the fonts
 * attached to the MKV and a default family by the track language.
 */
internal object SubtitleFonts {
    const val ROBOTO = "Roboto-Regular.ttf"
    const val CJK = "NotoSansCJK-Regular.ttc"
    const val ARABIC = "NotoNaskhArabic-Regular.ttf"
    const val HEBREW = "NotoSansHebrew-Regular.ttf"
    const val THAI = "NotoSansThai-Regular.ttf"

    private val NAMES = listOf(ROBOTO, CJK, ARABIC, HEBREW, THAI)
    private val DIRS = listOf("/system/fonts", "/product/fonts")

    /** The fonts of [NAMES] this device has, by file name. */
    fun system(dirs: List<String> = DIRS): Map<String, File> = buildMap {
        for (name in NAMES) {
            dirs.map { File(it, name) }.firstOrNull { it.canRead() }?.let { put(name, it) }
        }
    }

    /**
     * The family libass falls back to for a track in [language] (BCP 47 from Media3 or ISO 639-2): taken for a font
     * that is missing and for every glyph the requested font lacks. Noto Sans CJK covers Latin, Greek, Cyrillic and
     * CJK, so it is the default (in the variant of the language); Arabic, Hebrew and Thai get their Noto font;
     * Roboto where the device has no Noto CJK.
     */
    fun fallbackFamily(language: String?, files: Set<String>): String {
        val tag = language?.trim()?.lowercase().orEmpty()
        val base = tag.split('-', '_').first()
        if (ARABIC in files && base in setOf("ar", "ara", "fa", "fas", "per", "ur", "urd")) return "Noto Naskh Arabic"
        if (HEBREW in files && base in setOf("he", "iw", "heb")) return "Noto Sans Hebrew"
        if (THAI in files && base in setOf("th", "tha")) return "Noto Sans Thai"
        if (CJK !in files) return "Roboto"
        if (base in setOf("zh", "chi", "zho")) {
            val traditional = tag.contains("hant") || tag.contains("tw") || tag.contains("hk")
            return if (traditional) "Noto Sans CJK TC" else "Noto Sans CJK SC"
        }
        if (base in setOf("ko", "kor")) return "Noto Sans CJK KR"
        return "Noto Sans CJK JP"
    }
}
