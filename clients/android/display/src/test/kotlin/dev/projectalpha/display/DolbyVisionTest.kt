// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.display

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DolbyVisionTest {
    private val dvTv = setOf(HdrType.DOLBY_VISION, HdrType.HDR10, HdrType.HLG)
    private val hdr10Tv = setOf(HdrType.HDR10, HdrType.HLG)

    // Chromecast with Google TV: profiles 5 and 8; Shield: also 7 (MEL/FEL).
    private val chromecast = setOf(5, 8)
    private val shield = setOf(4, 5, 7, 8, 9)

    private fun p(profile: Int, compat: Int? = null) = DolbyVisionConfig(profile, 6, compat)

    @Test fun `codecs strings`() {
        assertEquals(DolbyVisionConfig(8, 6), DolbyVision.parseCodecs("dvhe.08.06"))
        assertEquals(DolbyVisionConfig(5, 9), DolbyVision.parseCodecs("dvh1.05.09"))
        assertEquals(DolbyVisionConfig(7, 6), DolbyVision.parseCodecs("hvc1.2.4.L153.B0, dvhe.07.06"))
        assertEquals(DolbyVisionConfig(10, 1), DolbyVision.parseCodecs("dav1.10.01"))
        assertEquals(DolbyVisionConfig(9, 5), DolbyVision.parseCodecs("dvav.09.05"))
        assertEquals(DolbyVisionConfig(8, null), DolbyVision.parseCodecs("dvhe.08"))
        assertNull(DolbyVision.parseCodecs("hvc1.2.4.L153.B0"))
        assertNull(DolbyVision.parseCodecs(null))
    }

    @Test fun `profile of a CodecProfileLevel constant`() {
        assertEquals(0, DolbyVision.profileOfCodecConstant(0x1))
        assertEquals(5, DolbyVision.profileOfCodecConstant(0x20))
        assertEquals(7, DolbyVision.profileOfCodecConstant(0x80))
        assertEquals(8, DolbyVision.profileOfCodecConstant(0x100))
        assertEquals(10, DolbyVision.profileOfCodecConstant(0x400))
        assertNull(DolbyVision.profileOfCodecConstant(0))
        assertNull(DolbyVision.profileOfCodecConstant(0x30))
        assertNull(DolbyVision.profileOfCodecConstant(0x800))
    }

    @Test fun `base layers`() {
        assertEquals(BaseLayer.NONE, p(5).baseLayer())
        assertEquals(BaseLayer.HDR10, p(7).baseLayer())
        assertEquals(BaseLayer.HDR10, p(8).baseLayer())
        assertEquals(BaseLayer.HDR10, p(8, compat = 1).baseLayer())
        assertEquals(BaseLayer.SDR, p(8, compat = 2).baseLayer())
        assertEquals(BaseLayer.HLG, p(8, compat = 4).baseLayer())
        assertEquals(BaseLayer.HDR10, p(8, compat = 6).baseLayer())
        assertEquals(BaseLayer.SDR, p(9).baseLayer())
        // The base layer's own VUI wins over the profile's guess.
        assertEquals(BaseLayer.HLG, p(8).baseLayer(HdrType.HLG, transferKnown = true))
        assertEquals(BaseLayer.SDR, p(8).baseLayer(null, transferKnown = true))
    }

    @Test fun `profile 8 on a DV TV with a DV decoder plays natively`() =
        assertEquals(DvPlayback.NATIVE, DolbyVision.decide(p(8), chromecast, dvTv, true).playback)

    @Test fun `profile 7 without a P7 decoder falls back to the HDR10 base layer`() =
        assertEquals(
            DvDecision(DvPlayback.BASE_LAYER, BaseLayer.HDR10, "no_dv_decoder_for_profile"),
            DolbyVision.decide(p(7), chromecast, dvTv, true),
        )

    @Test fun `profile 7 on a Shield plays natively`() =
        assertEquals(DvPlayback.NATIVE, DolbyVision.decide(p(7), shield, dvTv, true).playback)

    @Test fun `no DV on the display plays the base layer`() =
        assertEquals(
            DvDecision(DvPlayback.BASE_LAYER, BaseLayer.HDR10, "display_without_dv"),
            DolbyVision.decide(p(8), chromecast, hdr10Tv, true),
        )

    @Test fun `matching off plays the base layer`() =
        assertEquals(
            DvDecision(DvPlayback.BASE_LAYER, BaseLayer.HDR10, "matching_off"),
            DolbyVision.decide(p(8), chromecast, dvTv, false),
        )

    @Test fun `profile 8_4 falls back to HLG`() =
        assertEquals(BaseLayer.HLG, DolbyVision.decide(p(8, compat = 4), emptySet(), hdr10Tv, true).baseLayer)

    @Test fun `profile 5 uses the DV decoder even without DV on the display or with matching off`() {
        assertEquals(DvPlayback.NATIVE, DolbyVision.decide(p(5), chromecast, hdr10Tv, true).playback)
        assertEquals(DvPlayback.NATIVE, DolbyVision.decide(p(5), chromecast, dvTv, false).playback)
    }

    @Test fun `profile 5 without a DV decoder has wrong colors`() =
        assertEquals(
            DvDecision(DvPlayback.WRONG_COLORS, BaseLayer.NONE, "profile5_without_dv_decoder"),
            DolbyVision.decide(p(5), emptySet(), dvTv, true),
        )

    @Test fun `what the TV receives`() {
        assertEquals(HdrType.HDR10, DolbyVision.output(HdrType.HDR10, hdr10Tv))
        assertEquals(HdrType.HDR10, DolbyVision.output(HdrType.HDR10_PLUS, hdr10Tv))
        assertEquals(HdrType.HLG, DolbyVision.output(HdrType.HLG, hdr10Tv))
        assertEquals(HdrType.DOLBY_VISION, DolbyVision.output(HdrType.DOLBY_VISION, dvTv))
        assertNull(DolbyVision.output(HdrType.HDR10, emptySet()))
        assertNull(DolbyVision.output(null, dvTv))
    }

    @Test fun `android HDR types`() {
        assertEquals(HdrType.DOLBY_VISION, HdrType.fromAndroid(1))
        assertEquals(HdrType.HDR10, HdrType.fromAndroid(2))
        assertEquals(HdrType.HLG, HdrType.fromAndroid(3))
        assertEquals(HdrType.HDR10_PLUS, HdrType.fromAndroid(4))
        assertNull(HdrType.fromAndroid(-1))
    }
}
