// SPDX-License-Identifier: MPL-2.0
package app.kinolune.cast

import android.content.Context
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/**
 * Builds without the Google Cast SDK (F-Droid, `--dart-define=KINOLUNE_GOOGLE_CAST=false`; PLAY-012, ADR-NNNN): the
 * channel answers that casting is not available, so Dart shows no cast button. The real bridge is in src/cast.
 */
object CastBridge {
    private var channel: MethodChannel? = null

    fun attach(context: Context, engine: FlutterEngine) {
        channel = MethodChannel(engine.dartExecutor.binaryMessenger, "app.kinolune/cast").also {
            it.setMethodCallHandler { call, result ->
                if (call.method == "available") result.success(false) else result.notImplemented()
            }
        }
    }

    fun detach() {
        channel?.setMethodCallHandler(null)
        channel = null
    }
}
