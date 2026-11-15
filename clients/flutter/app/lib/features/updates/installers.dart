// SPDX-License-Identifier: MPL-2.0
//
// Installing a verified update (PKG-018, ADR-NNNN): PackageInstaller on
// Android, an atomic replacement of the AppImage on Linux.

import 'dart:io';

import 'update_platform.dart';

/// What an install attempt ended with.
enum InstallOutcome {
  /// Done; nothing else to do (Android usually stops the app first).
  installed,

  /// The AppImage was replaced: restart to run the new version.
  restartNeeded,

  /// The system asks the user to allow installs from Kinolune first.
  needsPermission,

  /// The APK is signed with another key than the installed app: Android
  /// would refuse it. Reinstalling is the only way (ADR-NNNN, PKG-009).
  signatureMismatch,

  /// The user cancelled the system dialog.
  aborted,

  /// The AppImage's folder is not writable.
  notWritable,

  /// Anything else ([UpdateInstaller.lastError] says what).
  failed,
}

/// Installs a verified download.
abstract class UpdateInstaller {
  /// Where to download [assetName]. The AppImage installer downloads next
  /// to the AppImage, so the verified file itself is renamed into place.
  File downloadFile(Directory cache, String assetName) =>
      File('${cache.path}/$assetName');

  Future<InstallOutcome> install(File verified);

  /// Detail of the last [InstallOutcome.failed], for the UI and logs.
  String? get lastError => null;
}

/// PackageInstaller through [AndroidUpdateBridge].
class AndroidApkInstaller extends UpdateInstaller {
  AndroidApkInstaller(this.bridge);

  final AndroidUpdateBridge bridge;
  String? _error;

  @override
  String? get lastError => _error;

  @override
  Future<InstallOutcome> install(File verified) async {
    _error = null;
    // A release signed with another key cannot replace this app: say so
    // before the user goes through the system dialog for nothing.
    if (await bridge.signature(verified.path) == ApkSignature.different) {
      return InstallOutcome.signatureMismatch;
    }
    if (!(await bridge.deviceInfo()).canInstall) {
      return InstallOutcome.needsPermission;
    }
    final (result, message) = await bridge.install(verified.path);
    _error = message;
    return switch (result) {
      AndroidInstallResult.installed => InstallOutcome.installed,
      AndroidInstallResult.aborted => InstallOutcome.aborted,
      AndroidInstallResult.incompatible => InstallOutcome.signatureMismatch,
      AndroidInstallResult.blocked => InstallOutcome.needsPermission,
      AndroidInstallResult.storage ||
      AndroidInstallResult.failed => InstallOutcome.failed,
    };
  }
}

/// Replaces the running AppImage ([path], `$APPIMAGE`) and restarts it.
class AppImageInstaller extends UpdateInstaller {
  AppImageInstaller(this.path, {this.relaunch = relaunchAppImage});

  final String path;
  final Future<void> Function(String path) relaunch;
  String? _error;

  @override
  String? get lastError => _error;

  /// A hidden file in the AppImage's folder: the same file system, so the
  /// final rename is atomic.
  @override
  File downloadFile(Directory cache, String assetName) {
    final target = File(path);
    final name = target.uri.pathSegments.last;
    return File('${target.parent.path}/.$name.update');
  }

  @override
  Future<InstallOutcome> install(File verified) async {
    _error = null;
    try {
      await replaceAppImage(File(path), verified);
      return InstallOutcome.restartNeeded;
    } on FileSystemException catch (e) {
      _error = e.toString();
      return InstallOutcome.notWritable;
    } on ProcessException catch (e) {
      _error = e.toString();
      return InstallOutcome.failed;
    }
  }

  /// Starts the new AppImage and quits this one.
  Future<void> restart() => relaunch(path);
}

/// Checks that the folder of [appImage] takes new files (the update is
/// downloaded there).
bool canReplaceAppImage(String appImage) {
  final target = File(appImage);
  if (!target.existsSync()) return false;
  final probe = File(
    '${target.parent.path}/.kinolune-write-test-$pid', // l10n-ignore
  );
  try {
    probe.writeAsStringSync('');
    probe.deleteSync();
    return true;
  } on FileSystemException {
    return false;
  }
}

/// Atomically replaces [target] with [update] (a verified download in the
/// same folder): makes it executable like [target], syncs it to disk and
/// renames it over [target]. A crash at any point leaves either the old or
/// the new AppImage, never a broken one. The running app keeps its mounted
/// copy of the old file until it exits.
Future<void> replaceAppImage(File target, File update) async {
  if (update.parent.absolute.path != target.parent.absolute.path) {
    throw FileSystemException(
      'the update is not in the AppImage folder', // l10n-ignore
      update.path,
    );
  }
  final oldMode = (await target.stat()).mode & 0x1ff;
  // At least u+rwx, and r-x for whoever could run the old one.
  final mode = (oldMode | 0x1c0) | ((oldMode & 0x24) >> 2);
  final chmod = await Process.run('chmod', [
    mode.toRadixString(8),
    update.path,
  ]);
  if (chmod.exitCode != 0) {
    throw ProcessException('chmod', [update.path], '${chmod.stderr}');
  }
  final raf = await update.open(mode: FileMode.append);
  try {
    await raf.flush(); // fsync
  } finally {
    await raf.close();
  }
  await update.rename(target.path);
}

/// Variables of the AppImage runtime and our AppRun that point into the
/// old mount; the new AppImage sets its own.
const _appImageEnv = {
  'APPDIR',
  'APPIMAGE',
  'ARGV0',
  'OWD',
  'LIBMPV_LIBRARY_PATH',
};

/// Starts [path] a second after this process exits, then exits.
Future<void> relaunchAppImage(String path) async {
  final env = {
    for (final e in Platform.environment.entries)
      if (!_appImageEnv.contains(e.key)) e.key: e.value,
  };
  await Process.start(
    '/bin/sh',
    ['-c', r'sleep 1; exec "$0"', path],
    mode: ProcessStartMode.detached,
    environment: env,
    includeParentEnvironment: false,
  );
  exit(0);
}
