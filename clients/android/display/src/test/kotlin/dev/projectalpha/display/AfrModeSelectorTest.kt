// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.display

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AfrModeSelectorTest {
    // A typical 4K Android TV mode list (refresh rates as real devices report them).
    private val uhd60 = ModeSpec(1, 3840, 2160, 60f)
    private val uhd5994 = ModeSpec(2, 3840, 2160, 59.94006f)
    private val uhd50 = ModeSpec(3, 3840, 2160, 50f)
    private val uhd24 = ModeSpec(4, 3840, 2160, 24f)
    private val uhd23976 = ModeSpec(5, 3840, 2160, 23.976025f)
    private val uhd25 = ModeSpec(6, 3840, 2160, 25f)
    private val uhd2997 = ModeSpec(7, 3840, 2160, 29.97003f)
    private val fhd60 = ModeSpec(10, 1920, 1080, 60f)
    private val fhd23976 = ModeSpec(11, 1920, 1080, 23.976025f)
    private val fhd50 = ModeSpec(12, 1920, 1080, 50f)
    private val hd60 = ModeSpec(13, 1280, 720, 60f)
    private val tv = listOf(uhd60, uhd5994, uhd50, uhd24, uhd23976, uhd25, uhd2997, fhd60, fhd23976, fhd50, hd60)

    private fun pick(fps: Float, current: ModeSpec = uhd60, modes: List<ModeSpec> = tv) =
        AfrModeSelector.select(modes, current, fps)

    @Test fun `23_976 picks 23_976 Hz, not 24 Hz`() = assertEquals(uhd23976, pick(23.976f))

    @Test fun `24 picks 24 Hz, not 23_976 Hz`() = assertEquals(uhd24, pick(24f))

    @Test fun `25 picks 25 Hz`() = assertEquals(uhd25, pick(25f))

    @Test fun `29_97 picks 29_97 Hz`() = assertEquals(uhd2997, pick(29.97f))

    @Test fun `50 picks 50 Hz`() = assertEquals(uhd50, pick(50f))

    @Test fun `59_94 from 60 Hz switches to 59_94 Hz`() = assertEquals(uhd5994, pick(59.94f))

    @Test fun `60 keeps 60 Hz`() = assertEquals(uhd60, pick(60f))

    @Test fun `29_97 keeps a current 59_94 Hz mode (exact multiple)`() =
        assertEquals(uhd5994, pick(29.97f, current = uhd5994))

    @Test fun `25 picks 50 Hz when there is no 25 Hz mode`() =
        assertEquals(uhd50, pick(25f, modes = listOf(uhd60, uhd50, uhd24)))

    @Test fun `24 prefers the smallest multiple, 48 Hz before 120 Hz`() {
        val m48 = ModeSpec(20, 3840, 2160, 48f)
        val m120 = ModeSpec(21, 3840, 2160, 120f)
        assertEquals(m48, pick(24f, modes = listOf(uhd60, m120, m48)))
        assertEquals(m120, pick(24f, modes = listOf(uhd60, m120)))
    }

    @Test fun `120 Hz panel stays at 120 Hz for 24 fps`() {
        val m120 = ModeSpec(21, 3840, 2160, 120f)
        assertEquals(m120, pick(24f, current = m120, modes = listOf(m120, uhd24)))
    }

    @Test fun `23_976 falls back to 24 Hz when it is the only 24p mode`() =
        assertEquals(uhd24, pick(23.976f, modes = listOf(uhd60, uhd24)))

    @Test fun `23_976 on a current 24 Hz mode switches to an exact 23_976 Hz mode`() =
        assertEquals(uhd23976, pick(23.976f, current = uhd24))

    @Test fun `23_976 keeps a current 24 Hz mode when nothing is exact`() =
        assertEquals(uhd24, pick(23.976f, current = uhd24, modes = listOf(uhd60, uhd24)))

    @Test fun `no matching mode leaves the display alone`() = assertNull(pick(24f, modes = listOf(uhd60, uhd50)))

    @Test fun `unknown frame rate leaves the display alone`() {
        assertNull(pick(0f))
        assertNull(pick(-1f))
        assertNull(pick(Float.NaN))
    }

    @Test fun `container rates are snapped to standard ones`() {
        assertEquals(uhd25, pick(24.900398f))
        assertEquals(uhd23976, pick(23.9760246f))
    }

    @Test fun `resolution is kept without the resolution option`() =
        assertEquals(fhd23976, AfrModeSelector.select(tv, fhd60, 23.976f, 3840, 2160, matchResolution = false))

    @Test fun `resolution option picks the smallest mode that holds the picture`() {
        assertEquals(fhd23976, AfrModeSelector.select(tv, uhd60, 23.976f, 1920, 800, matchResolution = true))
        assertEquals(uhd23976, AfrModeSelector.select(tv, fhd60, 23.976f, 3840, 1600, matchResolution = true))
        assertEquals(fhd50, AfrModeSelector.select(tv, uhd60, 25f, 1440, 1080, matchResolution = true))
    }

    @Test fun `resolution option keeps the current resolution when the video's has no mode for the rate`() =
        // 720p has only 60 Hz: the rate wins over the resolution.
        assertEquals(uhd23976, AfrModeSelector.select(tv, uhd60, 23.976f, 1280, 720, matchResolution = true))

    @Test fun `resolution option without the rate anywhere changes only the resolution`() =
        assertEquals(fhd60, AfrModeSelector.select(listOf(uhd60, fhd60), uhd60, 24f, 1920, 1080, matchResolution = true))

    @Test fun `a picture larger than every mode takes the largest`() =
        assertEquals(3840 to 2160, AfrModeSelector.targetSize(tv, 7680, 4320))

    @Test fun multiples() {
        assertEquals(1, AfrModeSelector.multipleOf(24f, 24f, AfrModeSelector.STRICT_TOLERANCE))
        assertEquals(5, AfrModeSelector.multipleOf(120f, 24f, AfrModeSelector.STRICT_TOLERANCE))
        assertEquals(2, AfrModeSelector.multipleOf(59.94006f, 29.97f, AfrModeSelector.STRICT_TOLERANCE))
        assertNull(AfrModeSelector.multipleOf(24f, 23.976f, AfrModeSelector.STRICT_TOLERANCE))
        assertEquals(1, AfrModeSelector.multipleOf(24f, 23.976f, AfrModeSelector.LOOSE_TOLERANCE))
        assertNull(AfrModeSelector.multipleOf(60f, 25f, AfrModeSelector.LOOSE_TOLERANCE))
    }

    @Test fun `options from the Dart map`() {
        val d = DisplayMatchOptions.fromMap(null)
        assertEquals(FrameRateMatch.AUTO, d.frameRate)
        assertFalse(d.resolution)
        assertTrue(d.dynamicRange)
        val o = DisplayMatchOptions.fromMap(mapOf("frame_rate" to "off", "resolution" to true, "dynamic_range" to false))
        assertEquals(DisplayMatchOptions(FrameRateMatch.OFF, resolution = true, dynamicRange = false), o)
    }
}
