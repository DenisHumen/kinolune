// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.audio

import android.content.Context
import android.content.IntentFilter
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.os.Build

/**
 * What the current audio output takes untouched (PLAY-AND-004), the way Media3's `AudioCapabilities` reads it, for
 * the media_kit player (mpv `audio-spdif`) and the player info. Media3 itself reads `AudioCapabilities` in its sink.
 *
 * - API 33+: `AudioManager.getDirectProfilesForAttributes` — the formats the routed device plays directly (what the
 *   HDMI EDID of the TV or receiver lists, filtered by the system's «Объёмный звук» setting).
 * - API 29–32: `AudioTrack.isDirectPlaybackSupported` per format.
 * - Always also: the encodings of connected HDMI / ARC / eARC / S/PDIF outputs (`AudioDeviceInfo.getEncodings`) and
 *   the sticky `ACTION_HDMI_AUDIO_PLUG` broadcast (Android TV 7–9 boxes report only there).
 */
object AudioOutputProbe {
    private val MEDIA: AudioAttributes = AudioAttributes.Builder()
        .setUsage(AudioAttributes.USAGE_MEDIA)
        .setContentType(AudioAttributes.CONTENT_TYPE_MOVIE)
        .build()

    /** Output device types that carry a bitstream to a receiver or TV. */
    private val DIGITAL_TYPES: Set<Int> = buildSet {
        add(AudioDeviceInfo.TYPE_HDMI)
        add(AudioDeviceInfo.TYPE_HDMI_ARC)
        add(AudioDeviceInfo.TYPE_LINE_DIGITAL)
        if (Build.VERSION.SDK_INT >= 31) add(AudioDeviceInfo.TYPE_HDMI_EARC)
    }

    fun capabilities(context: Context): AudioOutputCaps {
        val am = context.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        val encodings = HashSet<Int>()
        var maxChannels = 0

        val digital = am.getDevices(AudioManager.GET_DEVICES_OUTPUTS).filter { it.type in DIGITAL_TYPES }
        for (device in digital) {
            encodings += device.encodings.toList()
            maxChannels = maxOf(maxChannels, device.channelCounts.maxOrNull() ?: 0)
        }

        // Sticky: the last HDMI plug state, with the sink's encodings (EDID).
        val plug = runCatching {
            context.registerReceiver(null, IntentFilter(AudioManager.ACTION_HDMI_AUDIO_PLUG))
        }.getOrNull()
        val plugged = plug?.getIntExtra(AudioManager.EXTRA_AUDIO_PLUG_STATE, 0) == 1
        if (plug != null && plugged) {
            plug.getIntArrayExtra(AudioManager.EXTRA_ENCODINGS)?.let { encodings += it.toList() }
            maxChannels = maxOf(maxChannels, plug.getIntExtra(AudioManager.EXTRA_MAX_CHANNEL_COUNT, 0))
        }

        when {
            Build.VERSION.SDK_INT >= 33 -> runCatching {
                for (profile in am.getDirectProfilesForAttributes(MEDIA)) {
                    encodings += profile.format
                    if (!AudioPassthrough.isBitstream(profile.format)) {
                        maxChannels = maxOf(maxChannels, profile.channelMasks.maxOfOrNull { Integer.bitCount(it) } ?: 0)
                    }
                }
            }
            Build.VERSION.SDK_INT >= 29 -> for (format in BitstreamFormat.entries) {
                if (directPlayback(format)) encodings += format.encoding
            }
        }
        return AudioOutputCaps.fromEncodings(encodings, maxChannels, digital.isNotEmpty() || plugged)
    }

    /** API 29+: whether the routed output plays [format] directly (TrueHD and DTS-HD at 7.1, the rest at 5.1). */
    private fun directPlayback(format: BitstreamFormat): Boolean {
        if (Build.VERSION.SDK_INT < 29) return false
        val mask = when (format) {
            BitstreamFormat.DOLBY_TRUEHD, BitstreamFormat.DTS_HD -> AudioFormat.CHANNEL_OUT_7POINT1_SURROUND
            else -> AudioFormat.CHANNEL_OUT_5POINT1
        }
        return runCatching {
            AudioTrack.isDirectPlaybackSupported(
                AudioFormat.Builder().setEncoding(format.encoding).setSampleRate(48_000).setChannelMask(mask).build(),
                MEDIA,
            )
        }.getOrDefault(false)
    }
}
