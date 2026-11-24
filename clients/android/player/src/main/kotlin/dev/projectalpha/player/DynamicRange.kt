// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import android.content.Context
import android.os.Handler
import android.util.Log
import androidx.annotation.OptIn
import androidx.media3.common.C
import androidx.media3.common.Format
import androidx.media3.common.MimeTypes
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.mediacodec.MediaCodecInfo
import androidx.media3.exoplayer.mediacodec.MediaCodecSelector
import androidx.media3.exoplayer.video.MediaCodecVideoRenderer
import androidx.media3.exoplayer.video.VideoRendererEventListener
import dev.projectalpha.display.DisplayCapabilities
import dev.projectalpha.display.DisplayMatchOptions
import dev.projectalpha.display.DolbyVision
import dev.projectalpha.display.DvDecision
import dev.projectalpha.display.DvPlayback
import dev.projectalpha.display.HdrType

/**
 * What the video renderer needs to choose between native Dolby Vision and the base layer (PLAY-AND-005, ADR-NNNN).
 * Written on the main thread, read on the playback thread.
 */
class DynamicRangePolicy {
    @Volatile var options: DisplayMatchOptions = DisplayMatchOptions()

    /** The display and decoders; null until the player has a window (then DV is decided by the decoders only). */
    @Volatile var capabilities: DisplayCapabilities? = null

    /** The last decision for a Dolby Vision stream (stats overlay). */
    @Volatile var lastDecision: DvDecision? = null

    /** Decides for a Dolby Vision [format]; null for anything else. */
    fun decide(format: Format): DvDecision? {
        if (format.sampleMimeType != MimeTypes.VIDEO_DOLBY_VISION) return null
        val config = DolbyVision.parseCodecs(format.codecs) ?: return null
        val caps = capabilities
        val profiles = caps?.dolbyVisionProfiles ?: DisplayCapabilities.dolbyVisionDecoderProfiles()
        // Without a window the display is unknown: assume it takes DV, the decoder alone decides.
        val display = caps?.hdr ?: setOf(HdrType.DOLBY_VISION, HdrType.HDR10)
        val transfer = format.colorInfo?.colorTransfer
        val baseLayer = config.baseLayer(
            transfer = when (transfer) {
                C.COLOR_TRANSFER_HLG -> HdrType.HLG
                C.COLOR_TRANSFER_ST2084 -> HdrType.HDR10
                else -> null
            },
            transferKnown = transfer != null && transfer != Format.NO_VALUE,
        )
        return DolbyVision.decide(config, profiles, display, options.dynamicRange, baseLayer).also {
            lastDecision = it
            Log.i(TAG, "dolby vision profile ${config.profile}: $it")
        }
    }

    companion object {
        private const val TAG = "PaDynamicRange"

        /** The HDR format of a video [format]: Dolby Vision by MIME type or codecs, else by its color transfer. */
        fun hdrType(format: Format?): HdrType? {
            if (format == null) return null
            if (format.sampleMimeType == MimeTypes.VIDEO_DOLBY_VISION || DolbyVision.parseCodecs(format.codecs) != null) {
                return HdrType.DOLBY_VISION
            }
            return when (format.colorInfo?.colorTransfer) {
                C.COLOR_TRANSFER_ST2084 -> HdrType.HDR10
                C.COLOR_TRANSFER_HLG -> HdrType.HLG
                else -> null
            }
        }
    }
}

/**
 * Offers the HEVC decoders after the Dolby Vision ones for every DV stream (PLAY-AND-005). Media3 falls back to
 * the base-layer decoder by itself for profiles 4, 8, 9 and 10, but not for 7 (UHD Blu-ray): without this a P7 file
 * on a box whose DV decoder does not take P7 (Chromecast with Google TV) would have no decoder at all. The HEVC
 * decoder ignores the enhancement-layer NAL units and shows the HDR10 base layer.
 */
@OptIn(UnstableApi::class)
class DolbyVisionFallbackSelector(private val base: MediaCodecSelector = MediaCodecSelector.DEFAULT) : MediaCodecSelector {
    override fun getDecoderInfos(
        mimeType: String,
        requiresSecureDecoder: Boolean,
        requiresTunnelingDecoder: Boolean,
    ): List<MediaCodecInfo> {
        val infos = base.getDecoderInfos(mimeType, requiresSecureDecoder, requiresTunnelingDecoder)
        if (mimeType != MimeTypes.VIDEO_DOLBY_VISION) return infos
        val hevc = base.getDecoderInfos(MimeTypes.VIDEO_H265, requiresSecureDecoder, requiresTunnelingDecoder)
        return (infos + hevc).distinctBy { it.name }
    }
}

/**
 * Media3's video renderer with the Dolby Vision decision of [policy] (PLAY-AND-005): native DV keeps Media3's
 * order (the DV decoder first); the base layer drops the DV decoders, so the plain decoder shows HDR10/HLG/SDR.
 */
@OptIn(UnstableApi::class)
class DynamicRangeVideoRenderer(builder: MediaCodecVideoRenderer.Builder, private val policy: DynamicRangePolicy) : MediaCodecVideoRenderer(builder) {
    override fun getDecoderInfos(
        mediaCodecSelector: MediaCodecSelector,
        format: Format,
        requiresSecureDecoder: Boolean,
    ): List<MediaCodecInfo> {
        val infos = super.getDecoderInfos(mediaCodecSelector, format, requiresSecureDecoder)
        val decision = policy.decide(format) ?: return infos
        if (decision.playback != DvPlayback.BASE_LAYER) return infos
        val plain = infos.filter { it.mimeType != MimeTypes.VIDEO_DOLBY_VISION }
        // Nothing to fall back to (an unusual DV codec): play it natively rather than not at all.
        return plain.ifEmpty { infos }
    }

    companion object {
        /** Built like `DefaultRenderersFactory` builds its video renderer (decoder fallback on, 50 frames). */
        fun create(
            context: Context,
            selector: MediaCodecSelector,
            enableDecoderFallback: Boolean,
            eventHandler: Handler,
            eventListener: VideoRendererEventListener,
            allowedJoiningTimeMs: Long,
            policy: DynamicRangePolicy,
        ): DynamicRangeVideoRenderer = DynamicRangeVideoRenderer(
            MediaCodecVideoRenderer.Builder(context)
                .setMediaCodecSelector(selector)
                .setEnableDecoderFallback(enableDecoderFallback)
                .setEventHandler(eventHandler)
                .setEventListener(eventListener)
                .setAllowedJoiningTimeMs(allowedJoiningTimeMs)
                .setMaxDroppedFramesToNotify(MAX_DROPPED_VIDEO_FRAME_COUNT_TO_NOTIFY),
            policy,
        )

        /** `DefaultRenderersFactory.MAX_DROPPED_VIDEO_FRAME_COUNT_TO_NOTIFY`. */
        private const val MAX_DROPPED_VIDEO_FRAME_COUNT_TO_NOTIFY = 50
    }
}
