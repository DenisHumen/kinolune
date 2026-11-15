// SPDX-License-Identifier: MPL-2.0
//
// How this installation updates itself (PKG-018, ADR-NNNN): which release
// asset it needs and who installs it. Full flavor only.

import 'dart:async';
import 'dart:ffi' show Abi;
import 'dart:io';

import 'package:flutter/services.dart';

import 'releases.dart';

/// What kind of installation this is.
enum UpdateKind {
  /// An Android APK: the app installs the update through PackageInstaller.
  androidApk,

  /// A Linux AppImage the user can write: the app replaces the file and
  /// restarts.
  appImage,

  /// The Linux tar.gz archive (or an AppImage in a folder the app cannot
  /// write): the app shows how to update by hand.
  linuxArchive,

  /// Windows and macOS until their installers exist (PKG-005, PKG-008):
  /// the release page.
  releasePage,

  /// Flatpak, Snap and the like update through their own store: the app
  /// does not check.
  managed,
}

/// The installation the app runs from.
class UpdateTarget {
  const UpdateTarget(this.kind, {this.abis = const [], this.appImagePath});

  final UpdateKind kind;

  /// Android ABIs of the device, preferred first (`Build.SUPPORTED_ABIS`).
  final List<String> abis;

  /// `$APPIMAGE`: the AppImage file the app was started from.
  final String? appImagePath;

  /// Detects the installation from the platform and the environment the
  /// AppImage runtime, Flatpak and Snap set.
  static Future<UpdateTarget> detect({
    AndroidUpdateBridge? android,
    Map<String, String>? environment,
  }) async {
    final env = environment ?? Platform.environment;
    if (Platform.isAndroid) {
      final info = await (android ?? AndroidUpdateBridge()).deviceInfo();
      return UpdateTarget(UpdateKind.androidApk, abis: info.abis);
    }
    if (Platform.isLinux) {
      if (env.containsKey('FLATPAK_ID') || env.containsKey('SNAP')) {
        return const UpdateTarget(UpdateKind.managed);
      }
      final appImage = env['APPIMAGE'];
      if (appImage != null && appImage.isNotEmpty) {
        return UpdateTarget(UpdateKind.appImage, appImagePath: appImage);
      }
      return const UpdateTarget(UpdateKind.linuxArchive);
    }
    if (Platform.isWindows || Platform.isMacOS) {
      return const UpdateTarget(UpdateKind.releasePage);
    }
    return const UpdateTarget(UpdateKind.managed);
  }

  /// The asset of [release] this installation needs; null when the release
  /// has none for it.
  ReleaseAsset? assetIn(AppRelease release) {
    final v = release.version.text;
    switch (kind) {
      case UpdateKind.androidApk:
        for (final abi in abis) {
          if (release.asset('kinolune-$v-android-$abi.apk') case final a?) {
            return a;
          }
        }
        return release.asset('kinolune-$v-android-universal.apk');
      case UpdateKind.appImage:
        return release.asset('kinolune-$v-${_linuxArch()}.AppImage');
      case UpdateKind.linuxArchive:
        return release.asset('kinolune-$v-linux-${_linuxArch()}.tar.gz');
      case UpdateKind.releasePage:
      case UpdateKind.managed:
        return null;
    }
  }

  static String _linuxArch() => switch (Abi.current()) {
    Abi.linuxArm64 => 'aarch64',
    _ => 'x86_64',
  };
}

/// The device, as the Android side reports it.
class AndroidDeviceInfo {
  const AndroidDeviceInfo({required this.abis, required this.canInstall});

  final List<String> abis;

  /// "Install unknown apps" is allowed for Kinolune.
  final bool canInstall;
}

/// How the signing certificate of a downloaded APK compares with the
/// installed app's.
enum ApkSignature {
  /// The same key (or a rotation of it): Android accepts the update.
  same,

  /// Another key: Android refuses the update (signatures do not match).
  different,

  /// Could not be read.
  unknown,
}

/// What PackageInstaller said.
enum AndroidInstallResult {
  /// Installed (seldom seen: Android stops the app to replace it).
  installed,

  /// The user cancelled the system dialog.
  aborted,

  /// Signatures do not match, or a downgrade.
  incompatible,

  /// "Install unknown apps" is off.
  blocked,

  /// Not enough space.
  storage,

  /// Anything else.
  failed,
}

/// The channel `app.kinolune/updates` of the full flavor (Kotlin
/// `UpdateBridge`, compiled into full builds only).
class AndroidUpdateBridge {
  AndroidUpdateBridge({MethodChannel? channel})
    : _channel = channel ?? const MethodChannel('app.kinolune/updates') {
    _channel.setMethodCallHandler(_onCall);
  }

  final MethodChannel _channel;
  Completer<(AndroidInstallResult, String?)>? _pending;

  Future<Object?> _onCall(MethodCall call) async {
    if (call.method == 'installResult') {
      final args = (call.arguments as Map?) ?? const {};
      final result = AndroidInstallResult.values.firstWhere(
        (r) => r.name == args['result'],
        orElse: () => AndroidInstallResult.failed,
      );
      _pending?.complete((result, args['message'] as String?));
      _pending = null;
    }
    return null;
  }

  Future<AndroidDeviceInfo> deviceInfo() async {
    try {
      final m = await _channel.invokeMapMethod<String, Object?>('deviceInfo');
      return AndroidDeviceInfo(
        abis: [for (final a in (m?['abis'] as List?) ?? const []) '$a'],
        canInstall: m?['canInstall'] == true,
      );
    } on PlatformException {
      return const AndroidDeviceInfo(abis: [], canInstall: false);
    } on MissingPluginException {
      return const AndroidDeviceInfo(abis: [], canInstall: false);
    }
  }

  Future<ApkSignature> signature(String apkPath) async {
    try {
      final r = await _channel.invokeMethod<String>('signature', {
        'path': apkPath,
      });
      return ApkSignature.values.firstWhere(
        (s) => s.name == r,
        orElse: () => ApkSignature.unknown,
      );
    } on PlatformException {
      return ApkSignature.unknown;
    }
  }

  /// Opens the system's "Install unknown apps" page for Kinolune.
  Future<void> openInstallSettings() async {
    try {
      await _channel.invokeMethod<void>('openInstallSettings');
    } on PlatformException {
      // Nothing to open: the user goes to the system settings by hand.
    }
  }

  /// Hands the APK to PackageInstaller and waits for its verdict (after
  /// the system confirmation dialog).
  Future<(AndroidInstallResult, String?)> install(String apkPath) async {
    _pending?.complete((AndroidInstallResult.aborted, null));
    final pending = _pending = Completer();
    try {
      await _channel.invokeMethod<void>('install', {'path': apkPath});
    } on PlatformException catch (e) {
      _pending = null;
      return (AndroidInstallResult.failed, e.message);
    }
    return pending.future;
  }
}
