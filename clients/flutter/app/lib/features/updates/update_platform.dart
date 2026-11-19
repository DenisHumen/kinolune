// SPDX-License-Identifier: MPL-2.0
//
// How this installation updates itself (PKG-018, PKG-022, ADR-0109): which
// release asset it needs and who installs it. Full flavor only.

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

  /// A copy a package manager installed (.deb, .rpm, Flatpak, Snap): the
  /// app only says a release exists and links its file (PKG-022).
  linuxPackage,

  /// Windows, installed by `kinolune-<v>-windows-x64-setup.exe` (Inno
  /// Setup leaves `unins000.exe` next to the exe): the app runs the new
  /// installer silently and restarts (PKG-022).
  windowsInstaller,

  /// The portable Windows zip: cannot install itself, only notifies.
  windowsPortable,

  /// A macOS `Kinolune.app`: the app swaps the bundle for the one in the
  /// DMG of its architecture and restarts (PKG-022).
  macApp,

  /// Anything else: the release page.
  releasePage,
}

/// The installation the app runs from.
class UpdateTarget {
  const UpdateTarget(
    this.kind, {
    this.abis = const [],
    this.appImagePath,
    this.appPath,
    this.arch = 'x86_64', // l10n-ignore
    this.packageType,
  });

  final UpdateKind kind;

  /// Android ABIs of the device, preferred first (`Build.SUPPORTED_ABIS`).
  final List<String> abis;

  /// `$APPIMAGE`: the AppImage file the app was started from.
  final String? appImagePath;

  /// The installed app: the install folder on Windows, `Kinolune.app` on
  /// macOS.
  final String? appPath;

  /// The architecture in the release file name: `x86_64` or `arm64` on
  /// macOS (an Intel build under Rosetta gets `arm64`), `x86_64` or
  /// `aarch64` on Linux.
  final String arch;

  /// `deb`, `rpm`, `flatpak` or `snap` for [UpdateKind.linuxPackage].
  final String? packageType;

  /// The app installs the update itself here.
  bool get selfInstalls => switch (kind) {
    UpdateKind.androidApk ||
    UpdateKind.appImage ||
    UpdateKind.windowsInstaller ||
    UpdateKind.macApp => true,
    _ => false,
  };

  /// With «Обновлять автоматически» the update is downloaded and verified
  /// without a tap. On Android (PKG-024) only that: the system dialog is
  /// offered later, when nothing plays ([appliesQuietly]).
  bool get downloadsInBackground => switch (kind) {
    UpdateKind.androidApk ||
    UpdateKind.appImage ||
    UpdateKind.windowsInstaller ||
    UpdateKind.macApp => true,
    _ => false,
  };

  /// The update is put in place without a question (PKG-024): at the next
  /// start, or by a restart once the app is idle. Android always asks.
  bool get appliesQuietly => switch (kind) {
    UpdateKind.appImage ||
    UpdateKind.windowsInstaller ||
    UpdateKind.macApp => true,
    _ => false,
  };

  /// Detects the installation from the platform, the executable's folder
  /// and the environment the AppImage runtime, Flatpak and Snap set.
  /// Everything is injectable for tests.
  static Future<UpdateTarget> detect({
    AndroidUpdateBridge? android,
    Map<String, String>? environment,
    String? operatingSystem,
    String? executable,
    bool Function(String path)? fileExists,
    String? Function(String path)? readFile,
    Abi? abi,
    bool Function()? rosetta,
  }) async {
    final env = environment ?? Platform.environment;
    final os = operatingSystem ?? Platform.operatingSystem;
    final exe = executable ?? Platform.resolvedExecutable;
    final exists = fileExists ?? (String p) => File(p).existsSync();
    final read = readFile ?? _readFile;
    final currentAbi = abi ?? Abi.current();
    switch (os) {
      case 'android':
        final info = await (android ?? AndroidUpdateBridge()).deviceInfo();
        return UpdateTarget(UpdateKind.androidApk, abis: info.abis);
      case 'linux':
        final arch = currentAbi == Abi.linuxArm64 ? 'aarch64' : 'x86_64';
        if (env.containsKey('FLATPAK_ID')) {
          return UpdateTarget(
            UpdateKind.linuxPackage,
            arch: arch,
            packageType: 'flatpak',
          );
        }
        if (env.containsKey('SNAP')) {
          return UpdateTarget(
            UpdateKind.linuxPackage,
            arch: arch,
            packageType: 'snap',
          );
        }
        final appImage = env['APPIMAGE'];
        if (appImage != null && appImage.isNotEmpty) {
          return UpdateTarget(
            UpdateKind.appImage,
            appImagePath: appImage,
            arch: arch,
          );
        }
        // A .deb or .rpm writes its kind next to the executable.
        final type = read('${_dirname(exe)}/package-type')?.trim();
        if (type == 'deb' || type == 'rpm') {
          return UpdateTarget(
            UpdateKind.linuxPackage,
            arch: arch,
            packageType: type,
          );
        }
        return UpdateTarget(UpdateKind.linuxArchive, arch: arch);
      case 'windows':
        final dir = _dirname(exe);
        if (exists('$dir\\unins000.exe')) {
          return UpdateTarget(UpdateKind.windowsInstaller, appPath: dir);
        }
        return const UpdateTarget(UpdateKind.windowsPortable);
      case 'macos':
        // …/Kinolune.app/Contents/MacOS/Kinolune
        final bundle = _dirname(_dirname(_dirname(exe)));
        final arch = macDmgArch(
          arm64: currentAbi == Abi.macosArm64,
          translated: (rosetta ?? runsUnderRosetta)(),
        );
        if (!bundle.endsWith('.app')) {
          return UpdateTarget(UpdateKind.releasePage, arch: arch);
        }
        return UpdateTarget(UpdateKind.macApp, appPath: bundle, arch: arch);
    }
    return const UpdateTarget(UpdateKind.releasePage);
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
        return release.asset('kinolune-$v-$arch.AppImage');
      case UpdateKind.linuxArchive:
        return release.asset('kinolune-$v-linux-$arch.tar.gz');
      case UpdateKind.linuxPackage:
        return switch (packageType) {
          'deb' => release.asset(
            'kinolune-$v-linux-${arch == 'aarch64' ? 'arm64' : 'amd64'}.deb',
          ),
          'rpm' => release.asset('kinolune-$v-linux-$arch.rpm'),
          _ => null,
        };
      case UpdateKind.windowsInstaller:
        return release.asset('kinolune-$v-windows-x64-setup.exe');
      case UpdateKind.windowsPortable:
        return release.asset('kinolune-$v-windows-x64.zip');
      case UpdateKind.macApp:
        return release.asset('kinolune-$v-macos-$arch.dmg');
      case UpdateKind.releasePage:
        return null;
    }
  }

  /// Where «Скачать» goes where the app does not install: the file of this
  /// platform, else the release page.
  Uri downloadLink(AppRelease release) => assetIn(release)?.url ?? release.page;

  @override
  String toString() =>
      '${kind.name}/$arch${packageType == null ? '' : '/$packageType'}';
}

/// The DMG architecture for a Mac: an Intel build running under Rosetta is
/// offered the Apple silicon image. It runs natively there, and the process
/// architecture alone would keep that Mac on the emulated build for good.
String macDmgArch({required bool arm64, required bool translated}) =>
    arm64 || translated ? 'arm64' : 'x86_64';

/// `sysctl.proc_translated`: 1 for a process Rosetta translates, 0 for a
/// native one; the key does not exist on Intel Macs.
bool runsUnderRosetta() {
  try {
    final r = Process.runSync('/usr/sbin/sysctl', [
      '-n',
      'sysctl.proc_translated',
    ]);
    return r.exitCode == 0 && '${r.stdout}'.trim() == '1';
  } on ProcessException {
    return false;
  }
}

String? _readFile(String path) {
  try {
    return File(path).readAsStringSync();
  } on FileSystemException {
    return null;
  }
}

String _dirname(String path) {
  final i = path.lastIndexOf(RegExp(r'[\\/]'));
  return i <= 0 ? path : path.substring(0, i);
}

/// The device, as the Android side reports it.
class AndroidDeviceInfo {
  const AndroidDeviceInfo({
    required this.abis,
    required this.canInstall,
    this.metered = true,
  });

  final List<String> abis;

  /// "Install unknown apps" is allowed for Kinolune.
  final bool canInstall;

  /// The active network is metered (mobile data, a hotspot) or unknown:
  /// no download in the background (PKG-024).
  final bool metered;
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
        metered: m?['metered'] != false,
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
