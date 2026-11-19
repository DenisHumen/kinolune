// SPDX-License-Identifier: MPL-2.0
//
// Wiring of in-app updates (PKG-018, PKG-022, PKG-024) for main(): full
// flavor only.

import 'dart:async';
import 'dart:io';
import 'dart:ui' show AppExitResponse;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:pa_core/pa_core.dart' as core;

import '../../branding.dart';
import '../../diagnostics/diagnostics.dart';
import '../../flavor.dart';
import '../../integration/player.dart' show PlaybackActivity;
import '../../services/preferences.dart';
import 'app_version.dart';
import 'installers.dart';
import 'releases.dart';
import 'update_controller.dart';
import 'update_platform.dart';
import 'update_rules.dart';

/// Base URL of a stand-in for the GitHub releases API, for manual checks
/// of the update flow against local files
/// (`--dart-define=KINOLUNE_UPDATES_API=http://10.0.2.2:8000/releases`)
/// and the CI update test. A compile-time constant: release builds of CI
/// never set it.
const kUpdatesApiOverride = String.fromEnvironment('KINOLUNE_UPDATES_API');

/// The minisign public key of a throwaway test release, used instead of
/// the release key of the signed remote config. Only the CI update test
/// (release-app.yml) builds with it; release builds never set it.
const kUpdatesTestKey = String.fromEnvironment('KINOLUNE_UPDATES_TEST_KEY');

/// The verifier on the core (pa-update): the minisign signature of
/// SHA256SUMS with [releaseKey], then the checksum. With
/// [requireSignature] (the app) a missing release key is a refusal too:
/// nothing is installed that the release key did not sign.
ArtifactVerifier coreArtifactVerifier({
  String? Function()? releaseKey,
  bool requireSignature = true,
}) =>
    ({
      required File file,
      required String assetName,
      required List<int> sums,
      required String? signature,
    }) async {
      final key = releaseKey?.call()?.trim();
      if (requireSignature && (key == null || key.isEmpty)) {
        throw const UpdateVerifyFailure(
          'no release key to check the signature with', // l10n-ignore
          signature: true,
        );
      }
      try {
        await core.updateVerifyArtifact(
          path: file.path,
          assetName: assetName,
          sha256Sums: Uint8List.fromList(sums),
          signature: signature,
          releaseKey: key,
        );
      } on core.UpdateVerifyError catch (e) {
        throw UpdateVerifyFailure(
          e.message,
          signature: switch (e.kind) {
            core.UpdateVerifyErrorKind.signatureMissing ||
            core.UpdateVerifyErrorKind.badSignature ||
            core.UpdateVerifyErrorKind.badKey => true,
            _ => false,
          },
        );
      }
    };

/// The version of this build: `KINOLUNE_VERSION` that the release workflow
/// passes with `--build-name` (the platforms cannot always carry a
/// pre-release: macOS bundles hold only numbers, so `0.1.0-alpha.3` would
/// read back as something else); else the platform's version; else
/// [kAppVersion].
Future<AppVersion> runningVersion() async {
  const built = String.fromEnvironment('KINOLUNE_VERSION');
  if (AppVersion.tryParse(built) case final v?) return v;
  try {
    final info = await PackageInfo.fromPlatform();
    if (AppVersion.tryParse(info.version) case final v?) return v;
  } catch (e) {
    debugPrint('App version not read: $e');
  }
  return AppVersion.tryParse(kAppVersion)!;
}

/// Status changes go to pa-log (dropped without the diagnostics opt-in)
/// and, in the CI update test, to `<dir>/update-smoke.log`.
UpdateLog updateLog({String? smokeDir}) {
  final smoke = smokeDir == null ? null : File('$smokeDir/update-smoke.log');
  return (level, message) {
    debugPrint('updates $level: $message');
    Diagnostics.instance.log(level, 'updates', message);
    try {
      smoke?.writeAsStringSync(
        '${DateTime.now().toUtc().toIso8601String()} $level $message\n',
        mode: FileMode.append,
        flush: true,
      );
    } on FileSystemException {
      // A test aid only.
    }
  };
}

/// Picks who installs updates for [target]; null where the app only
/// notifies.
UpdateInstaller? installerFor(
  UpdateTarget target, {
  AndroidUpdateBridge? android,
}) => switch (target.kind) {
  UpdateKind.androidApk => AndroidApkInstaller(android!),
  UpdateKind.appImage when canReplaceAppImage(target.appImagePath!) =>
    AppImageInstaller(target.appImagePath!),
  UpdateKind.windowsInstaller => WindowsSetupInstaller(),
  UpdateKind.macApp => MacAppInstaller(target.appPath!),
  _ => null,
};

/// The idle time before a silent restart in the CI update test: seconds
/// instead of minutes, the same path otherwise (PKG-024).
const kUpdateSmokeIdle = Duration(seconds: 15);

/// Feeds the controller what silent updates (PKG-024) wait for: every key
/// press, click and touch in the app, and whether the window is in front.
void watchActivity(UpdateController controller) {
  void onPointer(PointerEvent _) => controller.userActive();
  bool onKey(KeyEvent _) {
    controller.userActive();
    return false;
  }

  GestureBinding.instance.pointerRouter.addGlobalRoute(onPointer);
  HardwareKeyboard.instance.addHandler(onKey);
  final state = WidgetsBinding.instance.lifecycleState;
  if (state != null) controller.foreground = state == AppLifecycleState.resumed;
}

/// Opens in-app updates: null in store builds and demo mode. Reads the
/// signed remote config (release key, kill-switch) from its cache; the
/// first network request is the scheduled check five seconds after start.
/// [tv]: the TV form factor, where the app never restarts by itself.
Future<UpdateController?> openUpdates({
  required String dataDir,
  required String cacheDir,
  required Preferences preferences,
  bool onRustCore = true,
  bool tv = false,
}) async {
  if (!AppFlavor.selfUpdate || !onRustCore) return null;
  try {
    final android = Platform.isAndroid ? AndroidUpdateBridge() : null;
    final target = await UpdateTarget.detect(android: android);
    final version = await runningVersion();
    final config = core.CoreRemoteConfig.open(
      cacheDir: '$dataDir/remote-config',
      appVersion: version.text,
      platform: Platform.operatingSystem,
    );
    final installer = installerFor(target, android: android);
    final api = Uri.tryParse(kUpdatesApiOverride);
    final smokeDir = Platform.environment[kUpdateSmokeEnv];
    final smoke = smokeDir != null && smokeDir.isNotEmpty;
    final controller = UpdateController(
      current: version,
      // A read-only AppImage updates like the archive: by hand.
      target: target.kind == UpdateKind.appImage && installer == null
          ? UpdateTarget(UpdateKind.linuxArchive, arch: target.arch)
          : target,
      source: GitHubReleaseSource(
        api: api != null && api.hasScheme ? api : null,
      ),
      verify: coreArtifactVerifier(
        releaseKey: () => kUpdatesTestKey.isNotEmpty
            ? kUpdatesTestKey
            : config.releasePublicKey(),
      ),
      cacheDir: Directory('$cacheDir/updates'),
      preferences: preferences,
      installer: installer,
      installKilled: config.isKilled(feature: 'updates.install'),
      log: updateLog(smokeDir: smokeDir),
      playback: PlaybackActivity.busy,
      tv: tv,
      idleApply: smoke ? kUpdateSmokeIdle : UpdateSchedule.idleApply,
      requireForeground: !smoke,
      meteredNetwork: android == null
          ? null
          : () async => (await android.deviceInfo()).metered,
    );
    // The schedule, a look a minute after the app comes back, whether the
    // window is in front (PKG-024), and on Windows a staged installer runs
    // when the app quits.
    AppLifecycleListener(
      onResume: controller.resumed,
      onStateChange: (s) =>
          controller.foreground = s == AppLifecycleState.resumed,
      onExitRequested: Platform.isWindows
          ? () async {
              await controller.applyOnExit();
              return AppExitResponse.exit;
            }
          : null,
    );
    await controller.start();
    watchActivity(controller);
    controller.startSchedule();
    return controller;
  } catch (e) {
    debugPrint('In-app updates unavailable: $e');
    return null;
  }
}
