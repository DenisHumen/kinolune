// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.display

import android.app.Activity
import android.hardware.display.DisplayManager
import android.media.MediaCodecList
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.Display
import android.view.Surface

/** What the display and the decoders can do (PLAY-AND-005): for the decisions, the stats overlay and the logs. */
data class DisplayCapabilities(
    val sdk: Int,
    val current: ModeSpec,
    val modes: List<ModeSpec>,
    val hdr: Set<HdrType>,
    /** Dolby Vision profiles of the device's DV decoders (empty: no DV decoder). */
    val dolbyVisionProfiles: Set<Int>,
) {
    fun toMap(): Map<String, Any> = mapOf(
        "sdk" to sdk,
        "mode" to current.toString(),
        "modes" to modes.map { it.toString() },
        "hdr" to hdr.map { it.id },
        "dolby_vision_profiles" to dolbyVisionProfiles.sorted(),
    )

    companion object {
        @Suppress("DEPRECATION")
        fun read(display: Display): DisplayCapabilities {
            // API 34 moved the HDR types to the mode (the current mode's types are the ones the TV takes now).
            val hdrTypes = if (Build.VERSION.SDK_INT >= 34) {
                display.mode.supportedHdrTypes
            } else {
                display.hdrCapabilities?.supportedHdrTypes ?: IntArray(0)
            }
            return DisplayCapabilities(
                sdk = Build.VERSION.SDK_INT,
                current = display.mode.toSpec(),
                modes = display.supportedModes.map { it.toSpec() },
                hdr = hdrTypes.mapNotNull(HdrType::fromAndroid).toSet(),
                dolbyVisionProfiles = dolbyVisionDecoderProfiles(),
            )
        }

        /** Profiles that the device's Dolby Vision decoders advertise. */
        fun dolbyVisionDecoderProfiles(): Set<Int> = runCatching {
            MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
                .filter { !it.isEncoder && it.supportedTypes.any { t -> t.equals(MIME_DOLBY_VISION, ignoreCase = true) } }
                .flatMap { codec ->
                    codec.getCapabilitiesForType(MIME_DOLBY_VISION).profileLevels.mapNotNull {
                        DolbyVision.profileOfCodecConstant(it.profile)
                    }
                }
                .toSet()
        }.getOrDefault(emptySet())

        const val MIME_DOLBY_VISION = "video/dolby-vision"
    }
}

internal fun Display.Mode.toSpec() = ModeSpec(modeId, physicalWidth, physicalHeight, refreshRate)

/**
 * Auto frame rate for one player window (PLAY-AND-005, ADR-NNNN).
 *
 * - The mode switch goes through `WindowManager.LayoutParams.preferredDisplayModeId` (API 23+, what Kodi does): it
 *   is the only way on Android TV 9–11, can also change the resolution, and works for any window — the Media3
 *   `SurfaceView` player and the media_kit texture player alike. The system drops the preference when the window
 *   goes away; [restore] puts the previous mode back explicitly (the window may outlive the player).
 * - [hintSurface] adds `Surface.setFrameRate` (API 30+) with seamless-only switching: on displays with seamless
 *   rates (VRR, 120 Hz panels) the platform picks a matching rate without an HDMI resync.
 * - [match] calls back when the display reports the new mode (or after [SWITCH_TIMEOUT_MS]), so the caller may
 *   hold playback while the TV resyncs.
 */
class DisplayModeMatcher(private val activity: Activity) : DisplayManager.DisplayListener {
    private val displayManager = activity.getSystemService(DisplayManager::class.java)
    private val handler = Handler(Looper.getMainLooper())
    private var originalModeId = 0
    private var targetModeId = 0
    private var pending: ((ModeSpec?) -> Unit)? = null

    @Suppress("DEPRECATION")
    fun display(): Display? = runCatching {
        if (Build.VERSION.SDK_INT >= 30) activity.display else activity.windowManager.defaultDisplay
    }.getOrNull()

    fun capabilities(): DisplayCapabilities? = display()?.let(DisplayCapabilities::read)

    /**
     * Switches the display to the best mode for a [fps] video ([width]x[height] for the resolution option) and calls
     * [onDone] with the mode in use afterwards (null: the display was not touched). Main thread.
     */
    fun match(fps: Float, width: Int, height: Int, options: DisplayMatchOptions, onDone: (ModeSpec?) -> Unit = {}) {
        val d = display()
        if (options.frameRate == FrameRateMatch.OFF || d == null) return onDone(null)
        val current = d.mode.toSpec()
        val target = AfrModeSelector.select(
            d.supportedModes.map { it.toSpec() },
            current,
            fps,
            width,
            height,
            options.resolution,
        )
        Log.i(TAG, "match fps=$fps ${width}x$height from $current to ${target ?: "none"}")
        if (target == null || target.id == current.id) return onDone(target)
        finish(null)
        if (originalModeId == 0) originalModeId = current.id
        targetModeId = target.id
        pending = onDone
        displayManager?.registerDisplayListener(this, handler)
        setPreferredMode(target.id)
        handler.postDelayed({ finish(display()?.mode?.toSpec()) }, SWITCH_TIMEOUT_MS)
    }

    /** `Surface.setFrameRate` for the video surface: seamless switches only (API 30+; no-op before). */
    fun hintSurface(surface: Surface?, fps: Float, options: DisplayMatchOptions) {
        if (surface == null || !surface.isValid || Build.VERSION.SDK_INT < 30) return
        val rate = if (options.frameRate == FrameRateMatch.OFF || fps.isNaN() || fps <= 0f) 0f else FrameRates.snap(fps)
        runCatching {
            if (Build.VERSION.SDK_INT >= 31) {
                surface.setFrameRate(rate, Surface.FRAME_RATE_COMPATIBILITY_FIXED_SOURCE, Surface.CHANGE_FRAME_RATE_ONLY_IF_SEAMLESS)
            } else {
                surface.setFrameRate(rate, Surface.FRAME_RATE_COMPATIBILITY_FIXED_SOURCE)
            }
        }.onFailure { Log.w(TAG, "setFrameRate failed", it) }
    }

    /** Puts back the mode that was active before the first switch (player exit). Main thread. */
    fun restore() {
        finish(null)
        displayManager?.unregisterDisplayListener(this)
        if (originalModeId == 0) return
        val id = originalModeId
        originalModeId = 0
        targetModeId = 0
        Log.i(TAG, "restore mode $id")
        // 0 = no preference: the system goes back to its default (normally the mode the launcher used).
        setPreferredMode(if (display()?.supportedModes?.any { it.modeId == id } == true) id else 0)
    }

    override fun onDisplayChanged(displayId: Int) {
        val d = display() ?: return
        if (displayId == d.displayId && d.mode.modeId == targetModeId) finish(d.mode.toSpec())
    }

    override fun onDisplayAdded(displayId: Int) = Unit
    override fun onDisplayRemoved(displayId: Int) = Unit

    private fun finish(mode: ModeSpec?) {
        val callback = pending ?: return
        pending = null
        handler.removeCallbacksAndMessages(null)
        displayManager?.unregisterDisplayListener(this)
        callback(mode)
    }

    private fun setPreferredMode(id: Int) {
        val window = activity.window ?: return
        window.attributes = window.attributes.apply { preferredDisplayModeId = id }
    }

    companion object {
        private const val TAG = "PaDisplay"

        /** TVs take 1–5 s to resync HDMI; after that the player goes on regardless. */
        const val SWITCH_TIMEOUT_MS = 6000L
    }
}
