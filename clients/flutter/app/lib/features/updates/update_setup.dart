// SPDX-License-Identifier: MPL-2.0
//
// Wiring of in-app updates (PKG-018) for main(): full flavor only.

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:pa_core/pa_core.dart' as core;

import '../../branding.dart';
import '../../flavor.dart';
import '../../services/preferences.dart';
import 'app_version.dart';
import 'installers.dart';
import 'releases.dart';
import 'update_controller.dart';
import 'update_platform.dart';

/// Base URL of a stand-in for the GitHub releases API, for manual checks
/// of the update flow against local files
/// (`--dart-define=KINOLUNE_UPDATES_API=http://10.0.2.2:8000/releases`).
/// A compile-time constant: release builds of CI never set it.
const kUpdatesApiOverride = String.fromEnvironment('KINOLUNE_UPDATES_API');

/// The verifier on the core (pa-update): SHA256SUMS always, the minisign
/// signature once [releaseKey] returns the owner's key.
ArtifactVerifier coreArtifactVerifier({String? Function()? releaseKey}) =>
    ({
      required File file,
      required String assetName,
      required List<int> sums,
      required String? signature,
    }) async {
      try {
        await core.updateVerifyArtifact(
          path: file.path,
          assetName: assetName,
          sha256Sums: Uint8List.fromList(sums),
          signature: signature,
          releaseKey: releaseKey?.call(),
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

/// The version of this build: `--build-name` of the release workflow,
/// read from the platform (versionName on Android, version.json on
/// Linux); [kAppVersion] if that fails.
Future<AppVersion> runningVersion() async {
  try {
    final info = await PackageInfo.fromPlatform();
    if (AppVersion.tryParse(info.version) case final v?) return v;
  } catch (e) {
    debugPrint('App version not read: $e');
  }
  return AppVersion.tryParse(kAppVersion)!;
}

/// Opens in-app updates: null in store builds, demo mode and installs
/// managed by a store (Flatpak, Snap). Reads the signed remote config
/// (release key, kill-switch) from its cache; no network until the daily
/// check, which runs in the background.
Future<UpdateController?> openUpdates({
  required String dataDir,
  required String cacheDir,
  required Preferences preferences,
  bool onRustCore = true,
}) async {
  if (!AppFlavor.selfUpdate || !onRustCore) return null;
  try {
    final android = Platform.isAndroid ? AndroidUpdateBridge() : null;
    final target = await UpdateTarget.detect(android: android);
    if (target.kind == UpdateKind.managed) return null;
    final version = await runningVersion();
    final config = core.CoreRemoteConfig.open(
      cacheDir: '$dataDir/remote-config',
      appVersion: version.text,
      platform: Platform.operatingSystem,
    );
    final UpdateInstaller? installer = switch (target.kind) {
      UpdateKind.androidApk => AndroidApkInstaller(android!),
      UpdateKind.appImage when canReplaceAppImage(target.appImagePath!) =>
        AppImageInstaller(target.appImagePath!),
      _ => null,
    };
    final api = Uri.tryParse(kUpdatesApiOverride);
    final controller = UpdateController(
      current: version,
      // A read-only AppImage updates like the archive: by hand.
      target: target.kind == UpdateKind.appImage && installer == null
          ? const UpdateTarget(UpdateKind.linuxArchive)
          : target,
      source: GitHubReleaseSource(
        api: api != null && api.hasScheme ? api : null,
      ),
      verify: coreArtifactVerifier(releaseKey: config.releasePublicKey),
      cacheDir: Directory('$cacheDir/updates'),
      preferences: preferences,
      installer: installer,
      installKilled: config.isKilled(feature: 'updates.install'),
    );
    // The daily check: at start and whenever the app comes back.
    AppLifecycleListener(
      onResume: () => unawaited(controller.maybeAutoCheck()),
    );
    unawaited(controller.start());
    return controller;
  } catch (e) {
    debugPrint('In-app updates unavailable: $e');
    return null;
  }
}
