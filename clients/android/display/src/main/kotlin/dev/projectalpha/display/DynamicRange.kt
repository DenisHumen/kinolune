// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.display

/** HDR formats a display or a stream may have (`Display.HdrCapabilities.HDR_TYPE_*`). */
enum class HdrType(val id: String) {
    DOLBY_VISION("dolby_vision"),
    HDR10("hdr10"),
    HDR10_PLUS("hdr10_plus"),
    HLG("hlg"),
    ;

    companion object {
        /** `Display.HdrCapabilities.HDR_TYPE_*` (1 DV, 2 HDR10, 3 HLG, 4 HDR10+); unknown types are dropped. */
        fun fromAndroid(type: Int): HdrType? = when (type) {
            1 -> DOLBY_VISION
            2 -> HDR10
            3 -> HLG
            4 -> HDR10_PLUS
            else -> null
        }
    }
}

/** The base layer under Dolby Vision: what a plain HEVC/AVC/AV1 decoder shows without the RPU. */
enum class BaseLayer {
    /** Profile 5: IPTPQc2, no compatible base layer — green and purple without a DV decoder. */
    NONE,
    HDR10,
    HLG,
    SDR,
}

/** A Dolby Vision stream: `profile` (4, 5, 7, 8, 9, 10…), `level`, and the base-layer compatibility id if known. */
data class DolbyVisionConfig(val profile: Int, val level: Int? = null, val compatibilityId: Int? = null) {
    /**
     * The base layer: by the compatibility id (`dv_bl_signal_compatibility_id`) when known, otherwise by the
     * profile; [transfer] is the base layer's own transfer from its VUI (`hdr10`, `hlg` or null for SDR) and wins
     * over the profile's guess.
     */
    fun baseLayer(transfer: HdrType? = null, transferKnown: Boolean = false): BaseLayer = when {
        profile == 5 -> BaseLayer.NONE
        compatibilityId == 0 -> BaseLayer.NONE
        compatibilityId == 1 || compatibilityId == 6 -> BaseLayer.HDR10
        compatibilityId == 2 -> BaseLayer.SDR
        compatibilityId == 4 -> BaseLayer.HLG
        transferKnown -> when (transfer) {
            HdrType.HLG -> BaseLayer.HLG
            null -> BaseLayer.SDR
            else -> BaseLayer.HDR10
        }
        // Profile 7 (UHD Blu-ray) and 8.1 carry HDR10; 4 and 9 an SDR base layer; 10 is mostly 10.1 (HDR10).
        profile == 4 || profile == 9 -> BaseLayer.SDR
        else -> BaseLayer.HDR10
    }
}

/** How a Dolby Vision stream is played. */
enum class DvPlayback {
    /** By the device's Dolby Vision decoder: the TV gets Dolby Vision (or the device maps it to its output). */
    NATIVE,

    /** By the plain decoder, base layer only: the TV gets HDR10, HLG or SDR. */
    BASE_LAYER,

    /** Profile 5 without a DV decoder: plays, but with wrong (green/purple) colors. */
    WRONG_COLORS,
}

/** The decision for one Dolby Vision stream and why (for the stats overlay and logs). */
data class DvDecision(val playback: DvPlayback, val baseLayer: BaseLayer, val reason: String)

object DolbyVision {
    private val CODECS = Regex("""^(dvhe|dvh1|dvav|dva1|dav1)\.(\d{1,2})(?:\.(\d{1,2}))?""", RegexOption.IGNORE_CASE)

    /** `dvhe.08.06`, `dvh1.05.06`, `dav1.10.09`… (RFC 6381 codecs of Dolby Vision) → profile and level. */
    fun parseCodecs(codecs: String?): DolbyVisionConfig? {
        if (codecs == null) return null
        val m = codecs.split(',').map { it.trim() }.firstNotNullOfOrNull { CODECS.find(it) } ?: return null
        return DolbyVisionConfig(profile = m.groupValues[2].toInt(), level = m.groupValues[3].toIntOrNull())
    }

    /**
     * Profile number of a `MediaCodecInfo.CodecProfileLevel.DolbyVisionProfile*` constant: the constants are bit
     * flags, `DvavPer` (0x1) = profile 0 … `DvheSt` (0x100) = profile 8, `Dvav110` (0x400) = profile 10.
     */
    fun profileOfCodecConstant(constant: Int): Int? {
        if (constant <= 0 || constant and (constant - 1) != 0) return null
        return Integer.numberOfTrailingZeros(constant).takeIf { it <= 10 }
    }

    /**
     * Native Dolby Vision or the base layer, from what the device's DV decoders can do ([decoderProfiles]), what
     * the display reports ([display], `Display.HdrCapabilities`) and the user's «Совпадение динамического
     * диапазона» ([matchDynamicRange]).
     *
     * - Native when allowed, a decoder takes the profile and the display reports Dolby Vision.
     * - Otherwise the base layer: HDR10 (profiles 7, 8.1, 8.6), HLG (8.4) or SDR (8.2, 9) through the plain decoder.
     *   Profile 7's enhancement layer is dropped: what a box without a P7 decoder does anyway.
     * - Profile 5 has no base layer: a DV decoder is the only way to the right colors, so it is used even when the
     *   display does not report DV (the device maps DV to its HDR10/SDR output) or the user turned matching off.
     *   Without one it plays with wrong colors; libmpv with libplacebo (`vo=gpu-next`) could reshape it, our
     *   media_kit output (`vo=gpu`) cannot (ADR-0147).
     */
    fun decide(
        stream: DolbyVisionConfig,
        decoderProfiles: Set<Int>,
        display: Set<HdrType>,
        matchDynamicRange: Boolean,
        baseLayer: BaseLayer = stream.baseLayer(),
    ): DvDecision {
        val decoder = stream.profile in decoderProfiles
        val displayDv = HdrType.DOLBY_VISION in display
        return when {
            matchDynamicRange && decoder && displayDv ->
                DvDecision(DvPlayback.NATIVE, baseLayer, "dv_decoder_and_display")
            baseLayer != BaseLayer.NONE -> DvDecision(
                DvPlayback.BASE_LAYER,
                baseLayer,
                when {
                    !matchDynamicRange -> "matching_off"
                    !decoder -> "no_dv_decoder_for_profile"
                    else -> "display_without_dv"
                },
            )
            decoder -> DvDecision(DvPlayback.NATIVE, baseLayer, "profile5_needs_dv_decoder")
            else -> DvDecision(DvPlayback.WRONG_COLORS, baseLayer, "profile5_without_dv_decoder")
        }
    }

    /**
     * What the TV receives for a stream: [content] (null = SDR) if the display takes it, HDR10 for HDR10+ on an
     * HDR10-only display (the static metadata stays), otherwise SDR (the device tone-maps).
     */
    fun output(content: HdrType?, display: Set<HdrType>): HdrType? = when {
        content == null -> null
        content in display -> content
        content == HdrType.HDR10_PLUS && HdrType.HDR10 in display -> HdrType.HDR10
        content == HdrType.DOLBY_VISION && HdrType.HDR10 in display -> HdrType.HDR10
        else -> null
    }
}
