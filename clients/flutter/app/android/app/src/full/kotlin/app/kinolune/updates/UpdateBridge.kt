// SPDX-License-Identifier: MPL-2.0
package app.kinolune.updates

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.content.pm.Signature
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.io.File
import java.util.concurrent.Executors

/**
 * In-app updates of the full flavor (PKG-018, ADR-NNNN), channel `app.kinolune/updates` on the app's engine. Compiled into
 * full builds only (src/full, build.gradle.kts), together with REQUEST_INSTALL_PACKAGES.
 *
 * * `deviceInfo()` — `{abis: Build.SUPPORTED_ABIS, canInstall}`: the ABI picks the release APK, `canInstall` is "Install
 *   unknown apps" for Kinolune.
 * * `signature(path)` — `same`, `different` or `unknown`: whether the downloaded APK is signed with the installed app's key.
 *   Android refuses an update signed with another key, and alpha releases are signed with a throw-away CI debug key until the
 *   owner adds the release keystore (PKG-009); Dart then explains how to reinstall instead of failing in the system dialog.
 * * `install(path)` — writes the APK (already checked against SHA256SUMS by Dart and the core) into a PackageInstaller session
 *   and commits it. The system shows its confirmation; the outcome comes back as `installResult({result, message})`.
 * * `openInstallSettings()` — the system page "Install unknown apps" for Kinolune.
 */
object UpdateBridge {
    private const val TAG = "UpdateBridge"
    private const val CHANNEL = "app.kinolune/updates"

    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor { r -> Thread(r, "kinolune-updates").apply { isDaemon = true } }
    private var channel: MethodChannel? = null

    fun attach(context: Context, engine: FlutterEngine) {
        val app = context.applicationContext
        val ch = MethodChannel(engine.dartExecutor.binaryMessenger, CHANNEL)
        channel = ch
        ch.setMethodCallHandler { call, result ->
            when (call.method) {
                "deviceInfo" -> result.success(
                    mapOf("abis" to Build.SUPPORTED_ABIS.toList(), "canInstall" to canInstall(app)),
                )
                "signature" -> {
                    val path = call.argument<String>("path")
                    if (path == null) {
                        result.error("args", "path is required", null)
                        return@setMethodCallHandler
                    }
                    io.execute {
                        val verdict = try {
                            compareSignature(app, path)
                        } catch (e: Exception) {
                            Log.w(TAG, "signature not read: $e")
                            "unknown"
                        }
                        main.post { result.success(verdict) }
                    }
                }
                "install" -> {
                    val path = call.argument<String>("path")
                    if (path == null) {
                        result.error("args", "path is required", null)
                        return@setMethodCallHandler
                    }
                    io.execute {
                        try {
                            install(app, File(path))
                            main.post { result.success(null) }
                        } catch (e: Exception) {
                            Log.w(TAG, "install not started: $e")
                            main.post { result.error("install", e.message, null) }
                        }
                    }
                }
                "openInstallSettings" -> {
                    openInstallSettings(app)
                    result.success(null)
                }
                else -> result.notImplemented()
            }
        }
    }

    fun detach() {
        channel?.setMethodCallHandler(null)
        channel = null
    }

    /** Hands the outcome of an install session to Dart. */
    internal fun report(result: String, message: String?) {
        Log.i(TAG, "install result: $result ${message.orEmpty()}")
        main.post { channel?.invokeMethod("installResult", mapOf("result" to result, "message" to message)) }
    }

    private fun canInstall(context: Context): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.O || context.packageManager.canRequestPackageInstalls()

    private fun openInstallSettings(context: Context) {
        val intent = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}"))
        } else {
            Intent(Settings.ACTION_SECURITY_SETTINGS)
        }
        try {
            context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        } catch (e: Exception) {
            Log.w(TAG, "install settings not opened: $e")
        }
    }

    private fun install(context: Context, apk: File) {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            setSize(apk.length())
        }
        val id = installer.createSession(params)
        installer.openSession(id).use { session ->
            apk.inputStream().use { input ->
                session.openWrite("base.apk", 0, apk.length()).use { out ->
                    input.copyTo(out, 1 shl 20)
                    session.fsync(out)
                }
            }
            val flags = PendingIntent.FLAG_UPDATE_CURRENT or
                (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0)
            val status = PendingIntent.getBroadcast(
                context,
                id,
                Intent(context, InstallResultReceiver::class.java).setPackage(context.packageName),
                flags,
            )
            session.commit(status.intentSender)
        }
    }

    /**
     * Whether [path] is signed by the key the installed app is signed with now (or by a newer key in its rotation lineage,
     * APK Signature Scheme v3): exactly what Android checks before it replaces the app.
     */
    @Suppress("DEPRECATION")
    private fun compareSignature(context: Context, path: String): String {
        val pm = context.packageManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            val flags = PackageManager.GET_SIGNING_CERTIFICATES
            val archive = pm.getPackageArchiveInfo(path, flags)?.signingInfo ?: return "unknown"
            val installed = pm.getPackageInfo(context.packageName, flags).signingInfo ?: return "unknown"
            val current: Set<Signature> = if (installed.hasMultipleSigners()) {
                installed.apkContentsSigners.toSet()
            } else {
                setOfNotNull(installed.signingCertificateHistory.lastOrNull())
            }
            val offered: Set<Signature> = if (archive.hasMultipleSigners()) {
                archive.apkContentsSigners.toSet()
            } else {
                archive.signingCertificateHistory.toSet()
            }
            if (current.isEmpty() || offered.isEmpty()) return "unknown"
            return if (offered.containsAll(current)) "same" else "different"
        }
        val archive: PackageInfo = pm.getPackageArchiveInfo(path, PackageManager.GET_SIGNATURES) ?: return "unknown"
        val installed = pm.getPackageInfo(context.packageName, PackageManager.GET_SIGNATURES)
        val a = archive.signatures?.toSet().orEmpty()
        val b = installed.signatures?.toSet().orEmpty()
        if (a.isEmpty() || b.isEmpty()) return "unknown"
        return if (a == b) "same" else "different"
    }
}

/** The status of an install session (PKG-018); declared in src/full/AndroidManifest.xml. */
class InstallResultReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
        val message = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)
        if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
            val confirm: Intent? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
            } else {
                @Suppress("DEPRECATION")
                intent.getParcelableExtra(Intent.EXTRA_INTENT)
            }
            if (confirm == null) {
                UpdateBridge.report("failed", "no confirmation intent")
                return
            }
            try {
                context.startActivity(confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            } catch (e: Exception) {
                UpdateBridge.report("failed", e.message)
            }
            return
        }
        UpdateBridge.report(
            when (status) {
                PackageInstaller.STATUS_SUCCESS -> "installed"
                PackageInstaller.STATUS_FAILURE_ABORTED -> "aborted"
                // INSTALL_FAILED_UPDATE_INCOMPATIBLE: another signing key (or a downgrade).
                PackageInstaller.STATUS_FAILURE_CONFLICT -> "incompatible"
                PackageInstaller.STATUS_FAILURE_BLOCKED -> "blocked"
                PackageInstaller.STATUS_FAILURE_STORAGE -> "storage"
                else -> "failed"
            },
            message,
        )
    }
}
