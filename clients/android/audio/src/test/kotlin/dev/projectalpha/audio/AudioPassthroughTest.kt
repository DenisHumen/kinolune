// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.audio

import dev.projectalpha.audio.BitstreamFormat.AC3
import dev.projectalpha.audio.BitstreamFormat.DOLBY_TRUEHD
import dev.projectalpha.audio.BitstreamFormat.DTS
import dev.projectalpha.audio.BitstreamFormat.DTS_HD
import dev.projectalpha.audio.BitstreamFormat.E_AC3
import dev.projectalpha.audio.BitstreamFormat.E_AC3_JOC
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AudioPassthroughTest {
    // Shield + Atmos receiver: everything. A Dolby-only soundbar over ARC. A phone: nothing.
    private val receiver = AudioOutputCaps(setOf(AC3, E_AC3, E_AC3_JOC, DOLBY_TRUEHD, DTS, DTS_HD), 8, true)
    private val dolbySoundbar = AudioOutputCaps(setOf(AC3, E_AC3), 2, true)
    private val dtsCoreOnly = AudioOutputCaps(setOf(AC3, DTS), 6, true)
    private val phone = AudioOutputCaps(emptySet(), 2, false)

    private fun decide(codec: StreamCodec, caps: AudioOutputCaps, mode: AudioOutputMode = AudioOutputMode.AUTO) =
        AudioPassthrough.decide(codec, caps, mode)

    private fun pass(format: BitstreamFormat, reason: String) = AudioDecision(AudioRoute.PASSTHROUGH, format, reason)
    private fun decode(reason: String) = AudioDecision(AudioRoute.DECODE, null, reason)

    @Test fun `a receiver that takes everything gets every bitstream as it is`() {
        assertEquals(pass(DOLBY_TRUEHD, "output_takes_truehd"), decide(StreamCodec.TRUEHD, receiver))
        assertEquals(pass(E_AC3_JOC, "output_takes_eac3_joc"), decide(StreamCodec.E_AC3_JOC, receiver))
        assertEquals(pass(E_AC3, "output_takes_eac3"), decide(StreamCodec.E_AC3, receiver))
        assertEquals(pass(AC3, "output_takes_ac3"), decide(StreamCodec.AC3, receiver))
        assertEquals(pass(DTS_HD, "output_takes_dts_hd"), decide(StreamCodec.DTS_HD, receiver))
        assertEquals(pass(DTS, "output_takes_dts"), decide(StreamCodec.DTS, receiver))
        assertEquals(pass(DTS_HD, "output_takes_dts_hd"), decide(StreamCodec.DTS_EXPRESS, receiver))
    }

    @Test fun `always decode wins over any receiver`() {
        for (codec in StreamCodec.entries) {
            assertEquals(decode("always_decode_setting"), decide(codec, receiver, AudioOutputMode.DECODE))
        }
    }

    @Test fun `a device without passthrough decodes everything to LPCM`() {
        assertEquals(decode("output_without_truehd"), decide(StreamCodec.TRUEHD, phone))
        assertEquals(decode("output_without_dts"), decide(StreamCodec.DTS_HD, phone))
        assertEquals(decode("output_without_dts"), decide(StreamCodec.DTS, phone))
        assertEquals(decode("output_without_eac3"), decide(StreamCodec.E_AC3_JOC, phone))
        assertEquals(decode("output_without_ac3"), decide(StreamCodec.AC3, phone))
        assertEquals("", AudioPassthrough.mpvSpdif(phone, AudioOutputMode.AUTO))
    }

    @Test fun `codecs without a bitstream form are always decoded`() {
        assertEquals(decode("not_a_bitstream_codec"), decide(StreamCodec.OTHER, receiver))
    }

    @Test fun `Atmos in E-AC-3 goes as plain E-AC-3 to a receiver without JOC`() {
        assertEquals(pass(E_AC3, "atmos_as_eac3"), decide(StreamCodec.E_AC3_JOC, dolbySoundbar))
        // and plain E-AC-3 to an output that only lists JOC
        val jocOnly = AudioOutputCaps(setOf(E_AC3_JOC))
        assertEquals(pass(E_AC3_JOC, "output_takes_eac3_joc"), decide(StreamCodec.E_AC3, jocOnly))
    }

    @Test fun `TrueHD has no core to fall back to`() {
        assertEquals(decode("output_without_truehd"), decide(StreamCodec.TRUEHD, dolbySoundbar))
    }

    @Test fun `DTS-HD goes as its DTS core, DTS Express does not`() {
        assertEquals(pass(DTS, "dts_core_only"), decide(StreamCodec.DTS_HD, dtsCoreOnly))
        assertEquals(decode("dts_express_needs_dts_hd"), decide(StreamCodec.DTS_EXPRESS, dtsCoreOnly))
        assertEquals(decode("output_without_dts"), decide(StreamCodec.DTS_HD, dolbySoundbar))
        // A DTS-HD-only list still takes plain DTS.
        assertEquals(pass(DTS, "output_takes_dts_hd"), decide(StreamCodec.DTS, AudioOutputCaps(setOf(DTS_HD))))
    }

    @Test fun `mpv audio-spdif follows the output`() {
        assertEquals("ac3,eac3,dts,dts-hd,truehd", AudioPassthrough.mpvSpdif(receiver, AudioOutputMode.AUTO))
        assertEquals("ac3,eac3", AudioPassthrough.mpvSpdif(dolbySoundbar, AudioOutputMode.AUTO))
        assertEquals("ac3,dts", AudioPassthrough.mpvSpdif(dtsCoreOnly, AudioOutputMode.AUTO))
        assertEquals("eac3", AudioPassthrough.mpvSpdif(AudioOutputCaps(setOf(E_AC3_JOC)), AudioOutputMode.AUTO))
        assertEquals("", AudioPassthrough.mpvSpdif(receiver, AudioOutputMode.DECODE))
    }

    @Test fun `capabilities from AudioFormat encodings`() {
        // PCM 16 (2), AC3 (5), E_AC3 (6), DTS (7), DTS_HD (8), IEC61937 (13), TRUEHD (14), AC4 (17), E_AC3_JOC (18).
        val caps = AudioOutputCaps.fromEncodings(listOf(2, 5, 6, 7, 8, 13, 14, 17, 18), 8, true)
        assertEquals(setOf(AC3, E_AC3, DTS, DTS_HD, DOLBY_TRUEHD, E_AC3_JOC), caps.formats)
        val map = caps.toMap()
        assertEquals(setOf("ac3", "eac3", "dts", "dts_hd", "truehd", "eac3_joc"), (map["formats"] as List<*>).toSet())
        assertEquals(8, map["max_pcm_channels"])
        assertEquals(true, map["digital_output"])
        assertEquals(AudioOutputCaps(), AudioOutputCaps.fromEncodings(listOf(2, 4, 13)))
    }

    @Test fun `stream codec from Media3 MIME types`() {
        assertEquals(StreamCodec.AC3, StreamCodec.fromMime("audio/ac3"))
        assertEquals(StreamCodec.E_AC3, StreamCodec.fromMime("audio/eac3"))
        assertEquals(StreamCodec.E_AC3_JOC, StreamCodec.fromMime("audio/eac3-joc"))
        assertEquals(StreamCodec.TRUEHD, StreamCodec.fromMime("audio/true-hd"))
        assertEquals(StreamCodec.DTS, StreamCodec.fromMime("audio/vnd.dts"))
        assertEquals(StreamCodec.DTS_HD, StreamCodec.fromMime("audio/vnd.dts.hd"))
        assertEquals(StreamCodec.DTS_EXPRESS, StreamCodec.fromMime("audio/vnd.dts.hd;profile=lbr"))
        assertEquals(StreamCodec.DTS_EXPRESS, StreamCodec.fromMime("audio/vnd.dts.hd", "dtse"))
        assertEquals(StreamCodec.OTHER, StreamCodec.fromMime("audio/mp4a-latm"))
        assertEquals(StreamCodec.OTHER, StreamCodec.fromMime(null))
    }

    @Test fun `stream codec from mpv codec names and FFmpeg profiles`() {
        assertEquals(StreamCodec.TRUEHD, StreamCodec.fromMpv("truehd", "Dolby TrueHD + Dolby Atmos"))
        assertEquals(StreamCodec.E_AC3_JOC, StreamCodec.fromMpv("eac3", "Dolby Digital Plus + Dolby Atmos"))
        assertEquals(StreamCodec.E_AC3, StreamCodec.fromMpv("eac3"))
        assertEquals(StreamCodec.DTS_HD, StreamCodec.fromMpv("dts", "DTS-HD MA + DTS:X"))
        assertEquals(StreamCodec.DTS_HD, StreamCodec.fromMpv("dts", "DTS-HD HRA"))
        assertEquals(StreamCodec.DTS, StreamCodec.fromMpv("dts", "DTS-ES"))
        assertEquals(StreamCodec.DTS_EXPRESS, StreamCodec.fromMpv("dts", "DTS Express"))
        assertEquals(StreamCodec.AC3, StreamCodec.fromMpv("ac3"))
        assertEquals(StreamCodec.OTHER, StreamCodec.fromMpv("mlp"))
        assertEquals(StreamCodec.OTHER, StreamCodec.fromMpv("aac"))
    }

    @Test fun `what reached the AudioTrack`() {
        assertTrue(AudioPassthrough.isBitstream(14)) // TrueHD
        assertTrue(AudioPassthrough.isBitstream(18)) // E-AC-3 JOC
        assertTrue(AudioPassthrough.isBitstream(13)) // IEC61937
        assertFalse(AudioPassthrough.isBitstream(2)) // PCM 16
        assertFalse(AudioPassthrough.isBitstream(4)) // float
        assertFalse(AudioPassthrough.isBitstream(0x50000000)) // Media3 PCM 24 big-endian
        assertFalse(AudioPassthrough.isBitstream(-1)) // Format.NO_VALUE
    }

    @Test fun `mode and format ids`() {
        assertEquals(AudioOutputMode.DECODE, AudioOutputMode.fromId("decode"))
        assertEquals(AudioOutputMode.AUTO, AudioOutputMode.fromId("auto"))
        assertEquals(AudioOutputMode.AUTO, AudioOutputMode.fromId(null))
        assertEquals(DTS_HD, BitstreamFormat.fromId("dts_hd"))
        assertNull(BitstreamFormat.fromId("aac"))
    }
}
