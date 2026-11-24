// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.display

import kotlin.math.abs
import kotlin.math.roundToInt

/** A display mode decoupled from `android.view.Display.Mode`, so the selection is plain JVM code (PLAY-AND-005). */
data class ModeSpec(val id: Int, val width: Int, val height: Int, val refreshRate: Float) {
    override fun toString(): String = "${width}x$height@${"%.3f".format(java.util.Locale.ROOT, refreshRate)}"
}

/** «Частота кадров»: switch the display to the video's rate, or never touch it. */
enum class FrameRateMatch { AUTO, OFF }

/**
 * What the player may do with the display (settings «Частота кадров», «Разрешение как у видео»,
 * «Совпадение динамического диапазона»; ADR-0147).
 *
 * - [frameRate]: AUTO switches the refresh rate to the video's (and asks for seamless switches through
 *   `Surface.setFrameRate`), OFF leaves the display alone.
 * - [resolution]: also switch the resolution to the video's (as Kodi's "whitelist"); off by default — TVs
 *   usually upscale better than the box.
 * - [dynamicRange]: Dolby Vision goes to the TV as Dolby Vision where the decoder and the display can; off plays
 *   the HDR10/HLG base layer instead (no DV handshake on the TV).
 */
data class DisplayMatchOptions(
    val frameRate: FrameRateMatch = FrameRateMatch.AUTO,
    val resolution: Boolean = false,
    val dynamicRange: Boolean = true,
) {
    companion object {
        /** From the map the Dart side sends (`frame_rate`: `auto`/`off`, `resolution`, `dynamic_range`). */
        fun fromMap(map: Map<*, *>?): DisplayMatchOptions {
            if (map == null) return DisplayMatchOptions()
            return DisplayMatchOptions(
                frameRate = if (map["frame_rate"] == "off") FrameRateMatch.OFF else FrameRateMatch.AUTO,
                resolution = map["resolution"] == true,
                dynamicRange = map["dynamic_range"] != false,
            )
        }
    }
}

/**
 * Picks the display mode for auto frame rate (AFR, "match content frame rate"), from the PLAY-AND-001 spike.
 *
 * Policy (Kodi's defaults):
 * - keep the current resolution unless [select]'s `matchResolution` asks for the video's: then the smallest mode
 *   that holds the whole picture (1920x800 → 1920x1080, 3840x1600 → 3840x2160);
 * - a refresh rate equal to the video rate, otherwise the smallest integer multiple (24 → 48 → 72 → 120 Hz);
 * - if the current mode already is an exact multiple, do not switch (no HDMI resync for nothing);
 * - 23.976 is not "matched" by 24.000 (or 59.94 by 60) unless no exact mode exists (a loose second pass): the
 *   0.1 % error is a repeated frame every ~42 s.
 */
object AfrModeSelector {
    /** Relative error of an exact match: 23.976 vs 24.000 is 0.1 %, so it is not one. */
    internal const val STRICT_TOLERANCE = 0.0005f

    /** Relative error of a fallback match (24.000 Hz for 23.976 fps). */
    internal const val LOOSE_TOLERANCE = 0.002f

    /**
     * The mode to switch to for a [videoFps] video, `current` itself when it already fits, or null when nothing fits
     * (the display is left as it is). [videoWidth]/[videoHeight] matter only with [matchResolution].
     */
    fun select(
        modes: List<ModeSpec>,
        current: ModeSpec,
        videoFps: Float,
        videoWidth: Int = 0,
        videoHeight: Int = 0,
        matchResolution: Boolean = false,
    ): ModeSpec? {
        if (videoFps.isNaN() || videoFps <= 1f) return null
        val fps = FrameRates.snap(videoFps)
        val size = if (matchResolution && videoWidth > 0 && videoHeight > 0) {
            targetSize(modes, videoWidth, videoHeight)
        } else {
            null
        } ?: (current.width to current.height)
        val sized = modes.filter { it.width == size.first && it.height == size.second }
        val sameAsCurrent = size.first == current.width && size.second == current.height
        if (sameAsCurrent && multipleOf(current.refreshRate, fps, STRICT_TOLERANCE) != null) return current
        best(sized, fps, STRICT_TOLERANCE)?.let { return it }
        if (sameAsCurrent && multipleOf(current.refreshRate, fps, LOOSE_TOLERANCE) != null) return current
        best(sized, fps, LOOSE_TOLERANCE)?.let { return it }
        if (sameAsCurrent) return null
        // The video's resolution has no mode for its rate: the rate matters more, so keep the current resolution.
        return select(modes, current, fps)
            ?: sized.minByOrNull { abs(it.refreshRate - current.refreshRate) }
    }

    /** The smallest resolution of [modes] that holds a [width]x[height] picture, or the largest one. */
    internal fun targetSize(modes: List<ModeSpec>, width: Int, height: Int): Pair<Int, Int>? {
        val sizes = modes.map { it.width to it.height }.distinct().sortedBy { it.first.toLong() * it.second }
        return sizes.firstOrNull { it.first >= width && it.second >= height } ?: sizes.lastOrNull()
    }

    private fun best(modes: List<ModeSpec>, fps: Float, tolerance: Float): ModeSpec? =
        modes.mapNotNull { mode -> multipleOf(mode.refreshRate, fps, tolerance)?.let { k -> mode to k } }
            .minWithOrNull(
                compareBy<Pair<ModeSpec, Int>> { it.second }.thenBy { relativeError(it.first.refreshRate, fps, it.second) },
            )
            ?.first

    /** k if `refresh ≈ k * fps` within [tolerance], otherwise null. */
    internal fun multipleOf(refresh: Float, fps: Float, tolerance: Float): Int? {
        if (fps <= 0f || refresh <= 0f) return null
        val k = (refresh / fps).roundToInt()
        if (k < 1) return null
        return if (relativeError(refresh, fps, k) <= tolerance) k else null
    }

    private fun relativeError(refresh: Float, fps: Float, k: Int): Float = abs(refresh - k * fps) / refresh
}

/** Standard video frame rates. */
object FrameRates {
    val STANDARD = floatArrayOf(
        23.976f, 24f, 25f, 29.97f, 30f, 47.952f, 48f, 50f, 59.94f, 60f, 100f, 119.88f, 120f,
    )

    /**
     * Snaps to a standard rate within 0.5 %. Containers often give rates like 24.900398 (a 25 fps MP4 from ffmpeg,
     * derived from sample count and duration) or 23.9760246, which would otherwise match neither 25 nor 50 Hz.
     */
    fun snap(fps: Float): Float {
        if (fps.isNaN() || fps <= 0f) return fps
        return STANDARD.minByOrNull { abs(it - fps) }?.takeIf { abs(it - fps) / it < 0.005f } ?: fps
    }
}
