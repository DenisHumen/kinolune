// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.media

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.media.MediaRouter2
import android.os.Build
import android.provider.Settings

/**
 * MUS-005: the system audio output picker for Now Playing. The app does not route audio itself (libmpv plays to the
 * default device), so this only opens what the system offers:
 * - Android 14+: the output switcher of the media session ([MediaRouter2.showSystemOutputSwitcher]);
 * - Android 10–13: the volume panel, which has the «Play on» row;
 * - older, or when neither exists (some TVs): the sound settings.
 * Returns false when nothing could be opened.
 */
object OutputPicker {
    fun show(context: Context): Boolean {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            try {
                if (MediaRouter2.getInstance(context).showSystemOutputSwitcher()) return true
            } catch (e: RuntimeException) {
                // Falls back to the panel below.
            }
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && start(context, Intent(Settings.Panel.ACTION_VOLUME))) {
            return true
        }
        return start(context, Intent(Settings.ACTION_SOUND_SETTINGS))
    }

    private fun start(context: Context, intent: Intent): Boolean = try {
        // The application context needs a new task; an activity context opens it over the app.
        if (context !is android.app.Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(intent)
        true
    } catch (e: ActivityNotFoundException) {
        false
    } catch (e: SecurityException) {
        false
    }
}
