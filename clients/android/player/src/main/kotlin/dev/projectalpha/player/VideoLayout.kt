// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import android.content.Context
import android.view.Gravity
import android.view.SurfaceView
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.annotation.MainThread
import androidx.annotation.OptIn
import androidx.media3.common.Player
import androidx.media3.common.VideoSize
import androidx.media3.common.util.UnstableApi
import androidx.media3.ui.AspectRatioFrameLayout

/**
 * The video picture of the player screen (ADR-0008, PLAY-AND-006): a [SurfaceView] (own SurfaceFlinger layer: HDR
 * and AFR untouched) and the [SubtitleOverlay] above it, both kept at the video's aspect ratio, centred in this view.
 * Without this MediaCodec stretches the picture to the surface, and subtitles positioned for the video would drift.
 */
@OptIn(UnstableApi::class)
class VideoLayout(context: Context) : FrameLayout(context) {
    val surface = SurfaceView(context)
    val subtitles = SubtitleOverlay(context)
    private val frame = AspectRatioFrameLayout(context)

    private val sizeListener = object : Player.Listener {
        override fun onVideoSizeChanged(videoSize: VideoSize) = applySize(videoSize)
    }

    init {
        val match = ViewGroup.LayoutParams.MATCH_PARENT
        frame.addView(surface, LayoutParams(match, match))
        frame.addView(subtitles, LayoutParams(match, match))
        addView(frame, LayoutParams(match, match, Gravity.CENTER))
    }

    /** Video and subtitles of [engine] go here. Main thread. */
    @MainThread
    fun bind(engine: Media3Engine) {
        engine.setVideoSurfaceView(surface)
        engine.setSubtitleOverlay(subtitles)
        engine.exoPlayer.addListener(sizeListener)
        applySize(engine.exoPlayer.videoSize)
    }

    private fun applySize(size: VideoSize) {
        if (size.width > 0 && size.height > 0) frame.setAspectRatio(size.width * size.pixelWidthHeightRatio / size.height)
    }
}
