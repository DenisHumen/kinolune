// SPDX-License-Identifier: MPL-2.0
//
// Installing a verified update (PKG-018, PKG-022, ADR-0109): PackageInstaller
// on Android, an atomic replacement of the AppImage on Linux and of
// Kinolune.app on macOS, the silent Inno Setup installer on Windows.

import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

import 'update_platform.dart';

/// Set by the CI update test (release-app.yml, `windows-update`,
/// `macos-update`): a folder for the update log, and a short idle time
/// (PKG-024) so the silent restart comes within seconds. Unset for people.
const kUpdateSmokeEnv = 'KINOLUNE_UPDATE_SMOKE';

/// What an install attempt ended with.
enum InstallOutcome {
  /// Done; nothing else to do (Android usually stops the app first).
  installed,

  /// The AppImage or Kinolune.app was replaced, or the Windows installer
  /// is staged: restart to run the new version.
  restartNeeded,

  /// Kinolune.app sits in a folder the app cannot write: the verified DMG
  /// was opened for the user to drag the app over (macOS).
  openedDmg,

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

  /// After [InstallOutcome.restartNeeded]: runs the new version and quits
  /// this one.
  Future<void> restart() async {}

  /// The app is quitting with an update ready: apply it without starting
  /// the app again (Windows). True when something was started.
  Future<bool> applyOnExit() async => false;

  /// The system asks the user before it installs (Android): a silent update
  /// stops at a verified download and offers «Установить» (PKG-024).
  bool get asksUser => false;
}

/// PackageInstaller through [AndroidUpdateBridge].
class AndroidApkInstaller extends UpdateInstaller {
  AndroidApkInstaller(this.bridge);

  final AndroidUpdateBridge bridge;
  String? _error;

  @override
  String? get lastError => _error;

  @override
  bool get asksUser => true;

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
  @override
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

/// Runs a program and waits: `hdiutil`, `ditto`, `xattr` (tests fake it).
typedef UpdateCommand = Future<ProcessResult> Function(
  String executable,
  List<String> args,
);

Future<ProcessResult> _runCommand(String executable, List<String> args) =>
    Process.run(executable, args);

/// Starts a program that outlives the app.
typedef UpdateLauncher = Future<void> Function(
  String executable,
  List<String> args,
);

Future<void> _startDetached(String executable, List<String> args) =>
    Process.start(executable, args, mode: ProcessStartMode.detached);

/// Quits the app after the update was handed over (tests replace it).
typedef UpdateQuit = void Function();

void _quitApp() => exit(0);

/// Windows, installed by our Inno Setup installer (PKG-022): the verified
/// `…-setup.exe` waits in the cache; «Перезапустить» runs it silently over
/// the installed copy and quits, and the installer starts the new version.
/// When the app quits with the installer staged, it runs too, without the
/// restart.
class WindowsSetupInstaller extends UpdateInstaller {
  WindowsSetupInstaller({
    UpdateLauncher launch = _startDetached,
    UpdateQuit quit = _quitApp,
  }) : _launcher = launch,
       _quitter = quit;

  final UpdateLauncher _launcher;
  final UpdateQuit _quitter;
  File? _staged;
  String? _error;

  /// The verified installer, once [install] staged it.
  File? get staged => _staged;

  @override
  String? get lastError => _error;

  /// Arguments of the silent update. `/UPDATE=1` makes the installer wait
  /// until this process has quit (packaging/app/windows/kinolune.iss);
  /// `/RELAUNCH=1` starts the new version at the end.
  static List<String> arguments({required bool relaunch}) => [
    '/VERYSILENT',
    '/SUPPRESSMSGBOXES',
    '/NORESTART',
    '/UPDATE=1',
    if (relaunch) '/RELAUNCH=1',
  ];

  @override
  Future<InstallOutcome> install(File verified) async {
    _error = null;
    if (!verified.existsSync()) {
      _error = 'the installer is gone'; // l10n-ignore
      return InstallOutcome.failed;
    }
    _staged = verified;
    return InstallOutcome.restartNeeded;
  }

  @override
  Future<void> restart() async {
    final setup = _staged;
    if (setup == null || !setup.existsSync()) return;
    await _launcher(setup.path, arguments(relaunch: true));
    _staged = null;
    _quitter();
  }

  @override
  Future<bool> applyOnExit() async {
    final setup = _staged;
    if (setup == null || !setup.existsSync()) return false;
    try {
      await _launcher(setup.path, arguments(relaunch: false));
    } on ProcessException catch (e) {
      _error = '$e';
      return false;
    }
    _staged = null;
    return true;
  }
}

/// macOS (PKG-022): mounts the verified DMG, copies `Kinolune.app` next to
/// the installed one, takes the quarantine off that verified copy only and
/// swaps the two bundles in one `renamex_np(RENAME_SWAP)`. The running app
/// keeps its old bundle (moved aside, removed on the next start) until
/// «Перезапустить». Where the app's folder is not writable, the DMG is
/// opened for the user instead.
class MacAppInstaller extends UpdateInstaller {
  MacAppInstaller(
    this.appPath, {
    UpdateCommand run = _runCommand,
    UpdateLauncher launch = _startDetached,
    UpdateQuit quit = _quitApp,
    bool Function(String from, String to)? swap,
    Map<String, String>? environment,
  }) : _runner = run,
       _launcher = launch,
       _quitter = quit,
       _swap = swap ?? swapPaths,
       _env = environment ?? Platform.environment;

  /// The installed `Kinolune.app`.
  final String appPath;
  final UpdateCommand _runner;
  final UpdateLauncher _launcher;
  final UpdateQuit _quitter;
  final bool Function(String from, String to) _swap;
  final Map<String, String> _env;
  String? _error;

  @override
  String? get lastError => _error;

  String get _folder => File(appPath).parent.path;
  String get _name => appPath.substring(appPath.lastIndexOf('/') + 1);

  /// The copy being prepared; after the swap it holds the old bundle.
  String get stagingPath => '$_folder/.$_name.update-$pid';

  /// Where the old bundle waits for the next start.
  String get oldPath => '$_folder/.$_name.old-$pid';

  /// Removes what earlier updates left next to the app (an old bundle
  /// stays until the next start: the app ran from it).
  void cleanUp() {
    try {
      for (final e in Directory(_folder).listSync()) {
        final n = e.path.substring(e.path.lastIndexOf('/') + 1);
        if (n.startsWith('.$_name.old-') || n.startsWith('.$_name.update-')) {
          e.deleteSync(recursive: true);
        }
      }
    } on FileSystemException {
      // Tried again on the next start.
    }
  }

  Future<bool> _ok(String exe, List<String> args) async {
    final r = await _runner(exe, args);
    if (r.exitCode == 0) return true;
    final what = args.isEmpty ? exe : '$exe ${args.first}';
    _error = '$what: ${r.stderr}'.trim();
    return false;
  }

  @override
  Future<InstallOutcome> install(File verified) async {
    _error = null;
    if (!canWriteFolder(_folder) || !canWriteFolder(appPath)) {
      // As in the owner's other app: hand over the verified image.
      if (await _ok('/usr/bin/open', [verified.path])) {
        return InstallOutcome.openedDmg;
      }
      return InstallOutcome.notWritable;
    }
    final mount = Directory.systemTemp.createTempSync('kinolune-update-');
    final staging = Directory(stagingPath);
    var mounted = false;
    try {
      if (!await _ok('/usr/bin/hdiutil', [
        'attach',
        '-nobrowse',
        '-readonly',
        '-noautoopen',
        '-mountpoint',
        mount.path,
        verified.path,
      ])) {
        return InstallOutcome.failed;
      }
      mounted = true;
      final source = '${mount.path}/$_name';
      if (!File('$source/Contents/MacOS/Kinolune').existsSync()) {
        _error = 'no $_name in the disk image'; // l10n-ignore
        return InstallOutcome.failed;
      }
      if (staging.existsSync()) staging.deleteSync(recursive: true);
      if (!await _ok('/usr/bin/ditto', [source, staging.path])) {
        return InstallOutcome.failed;
      }
    } finally {
      if (mounted) {
        await _runner('/usr/bin/hdiutil', ['detach', '-force', mount.path]);
      }
      try {
        mount.deleteSync();
      } on FileSystemException {
        // A temp folder; the system cleans it.
      }
    }
    // The copy came out of a verified image: only it loses the quarantine.
    await _runner('/usr/bin/xattr', [
      '-dr',
      'com.apple.quarantine',
      staging.path,
    ]);
    if (!_swap(staging.path, appPath)) {
      _error = 'the new $_name was not moved into place'; // l10n-ignore
      if (staging.existsSync()) staging.deleteSync(recursive: true);
      return InstallOutcome.failed;
    }
    // The staging path now holds the old bundle: out of the way until the
    // next start.
    try {
      staging.renameSync(oldPath);
    } on FileSystemException {
      // cleanUp() finds it under the staging name too.
    }
    return InstallOutcome.restartNeeded;
  }

  @override
  Future<void> restart() async {
    // The CI update test reads the new version's log through the
    // environment, which `open` does not pass on.
    final script = _env.containsKey(kUpdateSmokeEnv)
        ? r'sleep 1; exec "$0/Contents/MacOS/Kinolune"'
        : r'sleep 1; exec /usr/bin/open -n "$0"';
    await _launcher('/bin/sh', ['-c', script, appPath]);
    _quitter();
  }
}

/// Whether the folder [path] takes new entries.
bool canWriteFolder(String path) {
  final probe = File('$path/.kinolune-write-test-$pid'); // l10n-ignore
  try {
    probe.writeAsStringSync('');
    probe.deleteSync();
    return true;
  } on FileSystemException {
    return false;
  }
}

/// Swaps two folders. On macOS one `renamex_np(RENAME_SWAP)`: at any
/// moment both names point to the old entries or both to the new ones.
/// Elsewhere (and if the call is missing) two renames, undone when the
/// second fails.
bool swapPaths(String a, String b) {
  if (Platform.isMacOS) {
    try {
      return _renameSwap(a, b);
    } on ArgumentError {
      // No renamex_np in this libc: the two renames below.
    }
  }
  final aside = '$b.swap-$pid';
  try {
    Directory(b).renameSync(aside);
  } on FileSystemException {
    return false;
  }
  try {
    Directory(a).renameSync(b);
  } on FileSystemException {
    Directory(aside).renameSync(b);
    return false;
  }
  Directory(aside).renameSync(a);
  return true;
}

typedef _RenamexNative = Int32 Function(
  Pointer<Utf8> from,
  Pointer<Utf8> to,
  Uint32 flags,
);
typedef _Renamex = int Function(
  Pointer<Utf8> from,
  Pointer<Utf8> to,
  int flags,
);

/// `RENAME_SWAP` of `stdio.h` on macOS.
const _renameSwapFlag = 0x2;

bool _renameSwap(String a, String b) {
  final renamex = DynamicLibrary.process()
      .lookupFunction<_RenamexNative, _Renamex>('renamex_np');
  final from = a.toNativeUtf8();
  final to = b.toNativeUtf8();
  try {
    return renamex(from, to, _renameSwapFlag) == 0;
  } finally {
    malloc
      ..free(from)
      ..free(to);
  }
}
