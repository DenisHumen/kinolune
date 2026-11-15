// SPDX-License-Identifier: MPL-2.0
package app.kinolune.updates

import android.content.Context
import io.flutter.embedding.engine.FlutterEngine

/**
 * Store flavor (PKG-018, ADR-NNNN): no in-app updates. The store updates the app; this stub only keeps
 * [app.kinolune.KinoluneEngine] the same in both flavors. The full flavor's bridge is in src/full.
 */
object UpdateBridge {
    @Suppress("UNUSED_PARAMETER")
    fun attach(context: Context, engine: FlutterEngine) = Unit

    fun detach() = Unit
}
