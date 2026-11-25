// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.audio

/**
 * «Звук» (PLAY-AND-004): [AUTO] sends Dolby and DTS untouched to a receiver that takes them (passthrough), [DECODE]
 * always decodes to PCM (a receiver that misbehaves, a soundbar without DTS, volume control in the app).
 */
enum class AudioOutputMode(val id: String) {
    AUTO("auto"),
    DECODE("decode"),
    ;

    companion object {
        /** `auto` / `decode` from the Dart side; anything else is [AUTO]. */
        fun fromId(id: Any?): AudioOutputMode = if (id == DECODE.id) DECODE else AUTO
    }
}

/**
 * A compressed format an HDMI (or S/PDIF, ARC, eARC) output can take as it is, with its `AudioFormat.ENCODING_*`
 * value and the name mpv uses for it in `audio-spdif`.
 */
enum class BitstreamFormat(val id: String, val encoding: Int, val mpvSpdif: String) {
    AC3("ac3", 5, "ac3"),
    E_AC3("eac3", 6, "eac3"),

    /** E-AC-3 with Dolby Atmos (joint object coding): the same bitstream, mpv sends it as `eac3`. */
    E_AC3_JOC("eac3_joc", 18, "eac3"),
    DTS("dts", 7, "dts"),
    DTS_HD("dts_hd", 8, "dts-hd"),

    /** Dolby TrueHD, with or without Atmos (Atmos rides inside the TrueHD stream). */
    DOLBY_TRUEHD("truehd", 14, "truehd"),
    ;

    companion object {
        /** `AudioFormat.ENCODING_*` → format; PCM, AAC, AC-4, IEC61937 and unknown values are not ours. */
        fun fromEncoding(encoding: Int): BitstreamFormat? = entries.firstOrNull { it.encoding == encoding }

        fun fromId(id: String): BitstreamFormat? = entries.firstOrNull { it.id == id }
    }
}

/** What the current audio output takes untouched (from `AudioCapabilities` of HDMI, see `AudioOutputProbe`). */
data class AudioOutputCaps(
    val formats: Set<BitstreamFormat> = emptySet(),
    /** Most PCM channels the output takes (2 for most phones, 8 for an HDMI receiver), 0 if unknown. */
    val maxPcmChannels: Int = 0,
    /** A digital output (HDMI, ARC, eARC, S/PDIF) is connected. */
    val digitalOutput: Boolean = false,
) {
    fun toMap(): Map<String, Any> = mapOf(
        "formats" to formats.map { it.id },
        "max_pcm_channels" to maxPcmChannels,
        "digital_output" to digitalOutput,
    )

    companion object {
        /** From the `AudioFormat.ENCODING_*` values the platform reports; PCM and unknown encodings are dropped. */
        fun fromEncodings(
            encodings: Collection<Int>,
            maxPcmChannels: Int = 0,
            digitalOutput: Boolean = false,
        ): AudioOutputCaps = AudioOutputCaps(
            encodings.mapNotNull(BitstreamFormat::fromEncoding).toSet(),
            maxPcmChannels,
            digitalOutput,
        )
    }
}

/** The codec of an audio track, as far as passthrough cares. */
enum class StreamCodec {
    AC3,
    E_AC3,

    /** E-AC-3 with Atmos (JOC). */
    E_AC3_JOC,
    TRUEHD,
    DTS,

    /** DTS-HD Master Audio or High Resolution: a DTS core plus an extension. */
    DTS_HD,

    /** DTS Express (LBR): no DTS core, passes only as DTS-HD. */
    DTS_EXPRESS,

    /** Anything else (AAC, FLAC, Opus, PCM…): always decoded, there is nothing to pass through. */
    OTHER,
    ;

    companion object {
        /** From a Media3 `sampleMimeType` (`audio/ac3`, `audio/eac3-joc`, `audio/true-hd`, `audio/vnd.dts.hd`…). */
        fun fromMime(mime: String?, codecs: String? = null): StreamCodec = when (mime?.lowercase()) {
            "audio/ac3" -> AC3
            "audio/eac3" -> E_AC3
            "audio/eac3-joc" -> E_AC3_JOC
            "audio/true-hd" -> TRUEHD
            "audio/vnd.dts" -> DTS
            "audio/vnd.dts.hd" -> if (codecs?.lowercase()?.contains("dtse") == true) DTS_EXPRESS else DTS_HD
            "audio/vnd.dts.hd;profile=lbr" -> DTS_EXPRESS
            else -> OTHER
        }

        /**
         * From mpv's `audio-codec-name` and the track's `codec-profile` (`dts` + `DTS-HD MA` → [DTS_HD], `eac3` +
         * `Dolby Digital Plus + Dolby Atmos` → [E_AC3_JOC]).
         */
        fun fromMpv(codec: String?, profile: String? = null): StreamCodec {
            val p = profile?.lowercase().orEmpty()
            return when (codec?.lowercase()) {
                "ac3" -> AC3
                "eac3" -> if ("atmos" in p) E_AC3_JOC else E_AC3
                "truehd" -> TRUEHD
                // FFmpeg's profile names: `DTS`, `DTS-ES`, `DTS 96/24`, `DTS-HD HRA`, `DTS-HD MA`,
                // `DTS-HD MA + DTS:X`, `DTS Express`.
                "dts", "dca" -> when {
                    "express" in p -> DTS_EXPRESS
                    p.startsWith("dts-hd") -> DTS_HD
                    else -> DTS
                }
                else -> OTHER
            }
        }
    }
}

/** How a track leaves the device. */
enum class AudioRoute {
    /** Untouched to the receiver, which decodes it (shows «Dolby Atmos», «DTS-HD MA»…). */
    PASSTHROUGH,

    /** Decoded on the device to PCM (multichannel LPCM over HDMI, or stereo on a phone). */
    DECODE,
}

/** The decision for one track and why (for the player info and logs). */
data class AudioDecision(val route: AudioRoute, val format: BitstreamFormat?, val reason: String)

object AudioPassthrough {
    /**
     * Passthrough or decoding for one track:
     *
     * - «Всегда декодировать» ([AudioOutputMode.DECODE]) and codecs without a bitstream form (AAC, FLAC…) decode.
     * - Otherwise the track passes untouched when the output takes its format. Two fallbacks keep the receiver in
     *   charge: E-AC-3 with Atmos goes as plain E-AC-3 (the same bitstream; a receiver without Atmos plays the
     *   5.1/7.1 bed), DTS-HD as its DTS core (a soundbar with DTS but not DTS-HD: 5.1 core instead of PCM).
     * - TrueHD has no core in MKV, AC-3 cannot become E-AC-3: without a matching format they decode.
     */
    fun decide(codec: StreamCodec, caps: AudioOutputCaps, mode: AudioOutputMode): AudioDecision {
        if (mode == AudioOutputMode.DECODE) return decode("always_decode_setting")
        val f = caps.formats
        fun pass(format: BitstreamFormat, reason: String) = AudioDecision(AudioRoute.PASSTHROUGH, format, reason)
        return when (codec) {
            StreamCodec.OTHER -> decode("not_a_bitstream_codec")
            StreamCodec.AC3 -> when {
                BitstreamFormat.AC3 in f -> pass(BitstreamFormat.AC3, "output_takes_ac3")
                else -> decode("output_without_ac3")
            }
            StreamCodec.E_AC3 -> when {
                BitstreamFormat.E_AC3 in f -> pass(BitstreamFormat.E_AC3, "output_takes_eac3")
                BitstreamFormat.E_AC3_JOC in f -> pass(BitstreamFormat.E_AC3_JOC, "output_takes_eac3_joc")
                else -> decode("output_without_eac3")
            }
            StreamCodec.E_AC3_JOC -> when {
                BitstreamFormat.E_AC3_JOC in f -> pass(BitstreamFormat.E_AC3_JOC, "output_takes_eac3_joc")
                BitstreamFormat.E_AC3 in f -> pass(BitstreamFormat.E_AC3, "atmos_as_eac3")
                else -> decode("output_without_eac3")
            }
            StreamCodec.TRUEHD -> when {
                BitstreamFormat.DOLBY_TRUEHD in f -> pass(BitstreamFormat.DOLBY_TRUEHD, "output_takes_truehd")
                else -> decode("output_without_truehd")
            }
            StreamCodec.DTS -> when {
                BitstreamFormat.DTS in f -> pass(BitstreamFormat.DTS, "output_takes_dts")
                // A DTS-HD receiver takes plain DTS too.
                BitstreamFormat.DTS_HD in f -> pass(BitstreamFormat.DTS, "output_takes_dts_hd")
                else -> decode("output_without_dts")
            }
            StreamCodec.DTS_HD -> when {
                BitstreamFormat.DTS_HD in f -> pass(BitstreamFormat.DTS_HD, "output_takes_dts_hd")
                BitstreamFormat.DTS in f -> pass(BitstreamFormat.DTS, "dts_core_only")
                else -> decode("output_without_dts")
            }
            StreamCodec.DTS_EXPRESS -> when {
                BitstreamFormat.DTS_HD in f -> pass(BitstreamFormat.DTS_HD, "output_takes_dts_hd")
                else -> decode("dts_express_needs_dts_hd")
            }
        }
    }

    /**
     * mpv's `audio-spdif` for this output: the codecs it may pass through (`ac3,eac3,dts,dts-hd,truehd`), empty for
     * «Всегда декодировать» or an output without passthrough. mpv picks per track: with `dts` but not `dts-hd` a
     * DTS-HD track goes as its core, like [decide].
     */
    fun mpvSpdif(caps: AudioOutputCaps, mode: AudioOutputMode): String {
        if (mode == AudioOutputMode.DECODE) return ""
        val f = caps.formats
        val codecs = buildList {
            if (BitstreamFormat.AC3 in f) add("ac3")
            if (BitstreamFormat.E_AC3 in f || BitstreamFormat.E_AC3_JOC in f) add("eac3")
            if (BitstreamFormat.DTS in f || BitstreamFormat.DTS_HD in f) add("dts")
            if (BitstreamFormat.DTS_HD in f) add("dts-hd")
            if (BitstreamFormat.DOLBY_TRUEHD in f) add("truehd")
        }
        return codecs.joinToString(",")
    }

    /**
     * Whether what reached the `AudioTrack` (Media3 `AudioTrackConfig.encoding`) is a bitstream (passthrough)
     * rather than PCM. The player info shows this, not the intent: a receiver that refused a format shows PCM.
     */
    fun isBitstream(encoding: Int): Boolean = encoding > 0 && encoding !in PCM_ENCODINGS

    /**
     * PCM encodings: `AudioFormat.ENCODING_PCM_16BIT` (2), `_8BIT` (3), `_FLOAT` (4), `_24BIT_PACKED` (21),
     * `_32BIT` (22) and Media3's big-endian `C.ENCODING_PCM_*_BIG_ENDIAN`.
     */
    private val PCM_ENCODINGS = setOf(2, 3, 4, 21, 22, 0x10000000, 0x50000000, 0x60000000)

    private fun decode(reason: String) = AudioDecision(AudioRoute.DECODE, null, reason)
}
