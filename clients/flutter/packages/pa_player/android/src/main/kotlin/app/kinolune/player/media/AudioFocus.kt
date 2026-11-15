// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.media

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ApplicationInfo
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log

/**
 * Audio focus and "becoming noisy" for the media session (PLAY-AND-008). libmpv's audio output does not ask for
 * focus itself, so this does it for whatever the Dart player reports as playing, following [FocusPolicy];
 * [onAction] gets PLAY / PAUSE / DUCK / UNDUCK for the Dart player.
 *
 * While playing, `ACTION_AUDIO_BECOMING_NOISY` (headphones unplugged, Bluetooth headset gone) pauses. Debuggable
 * builds also take [DEBUG_NOISY] from `adb shell am broadcast`, since the shell may not send the system broadcast.
 */
class AudioFocus(context: Context, private val onAction: (FocusPolicy.Action, String) -> Unit) {
    private val ctx = context.applicationContext
    private val audio = ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    private val main = Handler(Looper.getMainLooper())
    private val policy = FocusPolicy()
    private var request: AudioFocusRequest? = null
    private var movie = false
    private var noisyRegistered = false

    private val listener = AudioManager.OnAudioFocusChangeListener { change ->
        val c = when (change) {
            AudioManager.AUDIOFOCUS_GAIN,
            AudioManager.AUDIOFOCUS_GAIN_TRANSIENT,
            AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK,
            AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE -> FocusPolicy.Change.GAIN
            AudioManager.AUDIOFOCUS_LOSS -> FocusPolicy.Change.LOSS
            AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> FocusPolicy.Change.LOSS_TRANSIENT
            AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> FocusPolicy.Change.LOSS_TRANSIENT_CAN_DUCK
            else -> null
        } ?: return@OnAudioFocusChangeListener
        Log.i(TAG, "focus change $change -> $c")
        perform(policy.onFocusChange(c), "focus")
    }

    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY || intent.action == DEBUG_NOISY) {
                Log.i(TAG, "becoming noisy: pause")
                onAction(FocusPolicy.Action.PAUSE, "noisy")
            }
        }
    }

    /** The Dart player's state changed. */
    fun onState(s: SessionSnapshot) {
        movie = !s.isMusic
        val playing = s.playing && !s.ended
        perform(policy.onPlaying(playing), "state")
        setNoisyReceiver(playing)
    }

    /** The session was cleared. */
    fun reset() {
        perform(policy.reset(), "reset")
        setNoisyReceiver(false)
    }

    private fun perform(actions: List<FocusPolicy.Action>, reason: String) {
        for (a in actions) {
            when (a) {
                FocusPolicy.Action.REQUEST -> {
                    val granted = requestFocus()
                    Log.i(TAG, "focus request granted=$granted")
                    perform(policy.onRequestResult(granted), "refused")
                }
                FocusPolicy.Action.ABANDON -> abandonFocus()
                else -> main.post { onAction(a, reason) }
            }
        }
    }

    private fun attributes(): AudioAttributes = AudioAttributes.Builder()
        .setUsage(AudioAttributes.USAGE_MEDIA)
        .setContentType(if (movie) AudioAttributes.CONTENT_TYPE_MOVIE else AudioAttributes.CONTENT_TYPE_MUSIC)
        .build()

    private fun requestFocus(): Boolean {
        val result = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val r = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                .setAudioAttributes(attributes())
                // We duck ourselves (FocusPolicy), the system must not do it on top.
                .setWillPauseWhenDucked(true)
                .setOnAudioFocusChangeListener(listener, main)
                .build()
            request = r
            audio.requestAudioFocus(r)
        } else {
            @Suppress("DEPRECATION")
            audio.requestAudioFocus(listener, AudioManager.STREAM_MUSIC, AudioManager.AUDIOFOCUS_GAIN)
        }
        return result == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
    }

    private fun abandonFocus() {
        Log.i(TAG, "focus abandoned")
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            request?.let { audio.abandonAudioFocusRequest(it) }
            request = null
        } else {
            @Suppress("DEPRECATION")
            audio.abandonAudioFocus(listener)
        }
    }

    private fun setNoisyReceiver(on: Boolean) {
        if (on == noisyRegistered) return
        noisyRegistered = on
        if (!on) {
            ctx.unregisterReceiver(noisy)
            return
        }
        val filter = IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY)
        val debuggable = (ctx.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0
        if (debuggable) filter.addAction(DEBUG_NOISY)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            ctx.registerReceiver(
                noisy,
                filter,
                if (debuggable) Context.RECEIVER_EXPORTED else Context.RECEIVER_NOT_EXPORTED,
            )
        } else {
            ctx.registerReceiver(noisy, filter)
        }
    }

    companion object {
        private const val TAG = "KinoluneSession"

        /** Debug builds: `adb shell am broadcast -a app.kinolune.debug.BECOMING_NOISY`. */
        const val DEBUG_NOISY = "app.kinolune.debug.BECOMING_NOISY"
    }
}
