// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.audio

import android.content.Context
import dev.projectalpha.audio.AudioOutputMode
import dev.projectalpha.audio.AudioOutputProbe
import dev.projectalpha.audio.AudioPassthrough
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

/**
 * Channel `app.kinolune/audio_passthrough` (PLAY-AND-004): passthrough for the media_kit player.
 *
 * - `capabilities` → `{formats, max_pcm_channels, digital_output}` of the current output ([AudioOutputProbe]);
 * - `spdif {mode}` → mpv's `audio-spdif` for this output and «Звук» (`auto`/`decode`): `ac3,eac3,dts,dts-hd,truehd`
 *   or a part of it, empty when everything is decoded ([AudioPassthrough.mpvSpdif]).
 *
 * mpv's `audiotrack` output sends what `audio-spdif` lists as IEC 61937 to the `AudioTrack`; the rest it decodes with
 * our LGPL FFmpeg (TrueHD, DTS-HD MA, E-AC-3 to multichannel PCM).
 */
class AudioOutputChannel : MethodChannel.MethodCallHandler {
    private var channel: MethodChannel? = null
    private var context: Context? = null

    fun attachToEngine(messenger: BinaryMessenger, context: Context) {
        this.context = context
        channel = MethodChannel(messenger, CHANNEL).also { it.setMethodCallHandler(this) }
    }

    fun detachFromEngine() {
        channel?.setMethodCallHandler(null)
        channel = null
        context = null
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        val ctx = context ?: return result.success(null)
        when (call.method) {
            "capabilities" -> result.success(runCatching { AudioOutputProbe.capabilities(ctx).toMap() }.getOrNull())
            "spdif" -> {
                val mode = AudioOutputMode.fromId(call.argument<String>("mode"))
                if (mode == AudioOutputMode.DECODE) return result.success("")
                val caps = runCatching { AudioOutputProbe.capabilities(ctx) }.getOrNull()
                result.success(caps?.let { AudioPassthrough.mpvSpdif(it, mode) } ?: "")
            }
            else -> result.notImplemented()
        }
    }

    companion object {
        const val CHANNEL = "app.kinolune/audio_passthrough"
    }
}
