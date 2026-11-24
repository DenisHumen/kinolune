// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import androidx.annotation.OptIn
import androidx.media3.common.Format
import androidx.media3.common.MimeTypes
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.audio.AudioSink
import androidx.media3.exoplayer.audio.ForwardingAudioSink
import dev.projectalpha.audio.AudioOutputMode

/**
 * «Звук» for the Media3 engine (PLAY-AND-004, ADR-NNNN); read by the audio sink on the playback thread, so a change
 * applies from the next audio format (the next item, track or seek that re-creates the `AudioTrack`).
 */
class AudioOutputPolicy {
    @Volatile var mode: AudioOutputMode = AudioOutputMode.AUTO
}

/**
 * Media3's audio sink with «Всегда декодировать»: compressed formats are refused, so neither renderer sends a
 * bitstream — `MediaCodecAudioRenderer` decodes with a platform decoder, the FFmpeg extension (`FfmpegAudioRenderer`,
 * our LGPL FFmpeg) decodes TrueHD, DTS and the rest. PCM is untouched. In «Авто» the sink decides by the HDMI
 * `AudioCapabilities` (with Media3's own fallbacks: E-AC-3 JOC as E-AC-3, DTS-HD as its DTS core), the same rules as
 * `AudioPassthrough.decide` for mpv.
 */
@OptIn(UnstableApi::class)
internal class PolicyAudioSink(sink: AudioSink, private val policy: AudioOutputPolicy) : ForwardingAudioSink(sink) {
    override fun supportsFormat(format: Format): Boolean = getFormatSupport(format) != AudioSink.SINK_FORMAT_UNSUPPORTED

    override fun getFormatSupport(format: Format): Int =
        if (refuses(format)) AudioSink.SINK_FORMAT_UNSUPPORTED else super.getFormatSupport(format)

    private fun refuses(format: Format): Boolean =
        policy.mode == AudioOutputMode.DECODE && format.sampleMimeType != MimeTypes.AUDIO_RAW
}
