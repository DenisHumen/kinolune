// SPDX-License-Identifier: MPL-2.0
package app.kinolune.cast

import android.content.Context
import androidx.annotation.Keep
import com.google.android.gms.cast.CastMediaControlIntent
import com.google.android.gms.cast.framework.CastOptions
import com.google.android.gms.cast.framework.OptionsProvider
import com.google.android.gms.cast.framework.SessionProvider

/**
 * PLAY-012 (ADR-NNNN): Cast options — Google's Default Media Receiver, so no receiver app of our own and no Cast
 * developer account. Named in src/cast/AndroidManifest.xml; the framework creates it by reflection.
 */
@Keep
class CastOptionsProvider : OptionsProvider {
    override fun getCastOptions(context: Context): CastOptions =
        CastOptions.Builder()
            .setReceiverApplicationId(CastMediaControlIntent.DEFAULT_MEDIA_RECEIVER_APPLICATION_ID)
            .build()

    override fun getAdditionalSessionProviders(context: Context): List<SessionProvider>? = null
}
