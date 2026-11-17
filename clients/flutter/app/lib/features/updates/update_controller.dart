// SPDX-License-Identifier: MPL-2.0
//
// In-app updates of the full flavor (PKG-018, PKG-022, ADR-NNNN): five
// seconds after launch and then every six hours (and on "Check for
// updates") the app reads the GitHub releases of the project, offers a
// newer `app-v*` release with its changelog, downloads the asset of this
// installation, checks it against the release's signed SHA256SUMS and
// installs it. With «Обновлять автоматически» desktop installs download and
// verify a release in the background and wait for «Перезапустить».

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

import '../../services/preferences.dart';
import 'app_version.dart';
import 'installers.dart';
import 'releases.dart';
import 'update_platform.dart';
import 'update_rules.dart';

export 'update_rules.dart' show UpdateMode;

/// Checks a download against the release's `SHA256SUMS` and signature;
/// throws [UpdateVerifyFailure] when it does not match. The app uses the
/// core (`update_verify_artifact`, pa-update).
typedef ArtifactVerifier = Future<void> Function({
  required File file,
  required String assetName,
  required List<int> sums,
  required String? signature,
});

/// Where status changes go: pa-log (CORE-008) in the app.
typedef UpdateLog = void Function(String level, String message);

void _debugLog(String level, String message) =>
    debugPrint('updates $level: $message');

/// A download that failed verification.
class UpdateVerifyFailure implements Exception {
  const UpdateVerifyFailure(this.message, {this.signature = false});

  final String message;

  /// The signature (not the checksum) failed or is missing.
  final bool signature;

  @override
  String toString() => 'UpdateVerifyFailure: $message';
}

/// Where the update flow is.
enum UpdatePhase {
  idle,
  checking,
  upToDate,
  available,
  downloading,
  verifying,
  installing,

  /// The AppImage or Kinolune.app was replaced, or the Windows installer
  /// is staged; «Перезапустить» runs the new version.
  restartNeeded,

  /// PackageInstaller took the APK; Android replaces the app.
  installed,

  /// macOS: the app's folder is read-only, the verified DMG was opened for
  /// the user.
  handedOver,
  failed,
}

/// Why the last step failed.
enum UpdateProblem {
  /// No network, GitHub down, a broken response.
  network,

  /// GitHub's limit for anonymous requests; try in an hour.
  rateLimited,

  /// The release has no file for this installation.
  noAsset,

  /// The download does not match SHA256SUMS: corrupt or tampered.
  checksum,

  /// The signature of SHA256SUMS is missing or wrong.
  signature,

  /// The APK is signed with another key (the CI debug key of alpha
  /// builds): uninstall and reinstall.
  signatureMismatch,

  /// "Install unknown apps" is off for Kinolune.
  needsPermission,

  /// The user cancelled the system dialog or the download.
  aborted,

  /// The AppImage's (or Kinolune.app's) folder is read-only.
  notWritable,

  /// The remote config stopped in-app updates (`updates.install`).
  killed,
  installFailed,
}

/// Preference keys of in-app updates.
abstract final class UpdatePrefs {
  /// 'false' turns the scheduled checks off; absent = on.
  static const autoCheck = 'updates_auto_check';

  /// `stable` or `beta`; absent = beta while the app is a pre-release.
  static const channel = 'updates_channel';

  /// Milliseconds since the epoch of the last check GitHub answered.
  static const lastCheck = 'updates_last_check';

  /// Tag of the release whose banner the user closed.
  static const dismissed = 'updates_dismissed';

  /// `auto` or `notify` (PKG-022); absent = auto.
  static const mode = 'updates_mode';
}

class UpdateController extends ChangeNotifier {
  UpdateController({
    required this.current,
    required this.target,
    required this.source,
    required this.verify,
    required this.cacheDir,
    required this.preferences,
    this.installer,
    UpdateHttp? http,
    this.installKilled = false,
    DateTime Function()? clock,
    UpdateLog? log,
    this.restartWhenReady = false,
  }) : http = http ?? UpdateHttp(),
       clock = clock ?? DateTime.now,
       log = log ?? _debugLog;

  /// The running version.
  final AppVersion current;
  final UpdateTarget target;
  final ReleaseSource source;
  final ArtifactVerifier verify;

  /// Downloads go to `<cacheDir>` (`<cache>/updates`), except the
  /// AppImage's (next to it, [UpdateInstaller.downloadFile]).
  final Directory cacheDir;
  final Preferences preferences;

  /// Null where the app does not install updates itself (tar.gz, the
  /// Windows zip, packages): the UI links the file or the release page.
  final UpdateInstaller? installer;
  final UpdateHttp http;

  /// A kill-switch of the signed remote config (`updates.install`): the app
  /// still says a release exists but leaves installing to the user.
  final bool installKilled;
  final DateTime Function() clock;

  /// Every status change, without progress ticks.
  final UpdateLog log;

  /// The CI update test ([kUpdateSmokeEnv]): «Перезапустить» is pressed
  /// as soon as an update is ready.
  final bool restartWhenReady;

  UpdatePhase _phase = UpdatePhase.idle;
  UpdatePhase get phase => _phase;

  AppRelease? _release;

  /// The newest release above [current], once a check found one.
  AppRelease? get release => _release;

  UpdateProblem? _problem;
  UpdateProblem? get problem => _problem;

  /// Technical detail of [problem] (logs, the About screen).
  String? detail;

  int _received = 0;
  int? _total;

  /// 0..1 while downloading, null if the size is unknown.
  double? get progress =>
      _total == null || _total == 0 ? null : _received / _total!;

  bool _autoCheck = true;

  /// Scheduled checks are on.
  bool get autoCheck => _autoCheck;

  UpdateMode _mode = UpdateMode.auto;

  /// «Обновлять автоматически» or «Только сообщать».
  UpdateMode get mode => _mode;

  UpdateChannel? _channel;
  UpdateChannel get channel =>
      _channel ??
      (current.isPrerelease ? UpdateChannel.beta : UpdateChannel.stable);

  DateTime? _lastCheck;

  /// The last check GitHub answered (scheduled or asked for).
  DateTime? get lastCheck => _lastCheck;

  /// The last scheduled attempt, answered or not.
  DateTime? _lastTried;
  DateTime? _lastTick;

  String? _dismissed;
  bool _cancelled = false;
  bool _busy = false;

  /// A release whose background download failed in this session: not
  /// fetched again on its own (a tampered file would be downloaded every
  /// six hours otherwise). «Скачать» still tries.
  String? _backgroundFailed;
  final List<Timer> _timers = [];
  Timer? _ticker;

  /// The app installs updates itself here (and the remote config allows
  /// it).
  bool get canInstall => installer != null && !installKilled;

  /// Where a check nobody asked for must not interfere.
  UpdateActivity get activity => switch (_phase) {
    UpdatePhase.checking => UpdateActivity.checking,
    UpdatePhase.downloading ||
    UpdatePhase.verifying ||
    UpdatePhase.installing => UpdateActivity.downloading,
    UpdatePhase.restartNeeded ||
    UpdatePhase.installed ||
    UpdatePhase.handedOver => UpdateActivity.readyToRestart,
    _ => _busy ? UpdateActivity.checking : UpdateActivity.idle,
  };

  /// Show the banner across the app: a release the user has not closed it
  /// for, or a download that runs.
  bool get bannerVisible {
    final r = _release;
    if (r == null || r.tag == _dismissed) return false;
    return switch (_phase) {
      UpdatePhase.idle || UpdatePhase.checking || UpdatePhase.upToDate => false,
      _ => true,
    };
  }

  /// Loads the settings and removes leftovers of earlier downloads. The
  /// checks themselves run from [startSchedule] (and [check]).
  Future<void> start() async {
    _autoCheck = await preferences.read(UpdatePrefs.autoCheck) != 'false';
    _mode = switch (await preferences.read(UpdatePrefs.mode)) {
      'notify' => UpdateMode.notify,
      _ => UpdateMode.auto,
    };
    _channel = switch (await preferences.read(UpdatePrefs.channel)) {
      'stable' => UpdateChannel.stable,
      'beta' => UpdateChannel.beta,
      _ => null,
    };
    final last = int.tryParse(
      await preferences.read(UpdatePrefs.lastCheck) ?? '',
    );
    _lastCheck = last == null
        ? null
        : DateTime.fromMillisecondsSinceEpoch(last, isUtc: true);
    _dismissed = await preferences.read(UpdatePrefs.dismissed);
    _cleanDownloads();
    log(
      'info',
      'Running $current ($target), mode ${_mode.name}, '
          'checks ${_autoCheck ? 'on' : 'off'}',
    );
    notifyListeners();
  }

  void _cleanDownloads() {
    try {
      if (cacheDir.existsSync()) cacheDir.deleteSync(recursive: true);
      // An AppImage download interrupted by a crash or power loss.
      if (installer case final AppImageInstaller i) {
        for (final f in [
          i.downloadFile(cacheDir, ''),
          File('${i.downloadFile(cacheDir, '').path}.part'),
        ]) {
          if (f.existsSync()) f.deleteSync();
        }
      }
    } on FileSystemException catch (e) {
      log('warn', 'Old update downloads not removed: $e'); // l10n-ignore
    }
    // The bundle a macOS update replaced, and an interrupted copy.
    if (installer case final MacAppInstaller i) i.cleanUp();
  }

  /// Starts the schedule: the first look [UpdateSchedule.launchDelay]
  /// after launch, then a [UpdateSchedule.tick] that checks whenever
  /// [checkDue] says so (wall clock: a laptop that slept through a check
  /// makes it soon after waking).
  void startSchedule() {
    _timers.add(Timer(UpdateSchedule.launchDelay, () => unawaited(tick())));
    _ticker ??= Timer.periodic(UpdateSchedule.tick, (_) => unawaited(tick()));
  }

  /// The app came back to the foreground (or the computer woke up): a look
  /// once the network had time to return.
  void resumed() {
    _timers.removeWhere((t) => !t.isActive);
    _timers.add(Timer(UpdateSchedule.resumeDelay, () => unawaited(tick())));
  }

  /// One step of the schedule: checks when one is due and allowed.
  Future<void> tick() async {
    final now = clock();
    final previous = _lastTick;
    _lastTick = now;
    if (previous != null && sleptBetween(previous, now)) {
      // Woken from sleep: Wi-Fi may not be back yet.
      log('info', 'Woke from sleep; checking in a minute'); // l10n-ignore
      resumed();
      return;
    }
    if (!_autoCheck) return;
    if (!checkDue(now, _lastCheck, _lastTried)) return;
    await backgroundCheck();
  }

  /// A check the scheduler makes, as opposed to one somebody asked for:
  /// it leaves a running download or a pending «Перезапустить» alone and
  /// is quiet — no "checking…", and a failure goes to the log only. With
  /// «Обновлять автоматически» a found release is downloaded and verified
  /// right away where the app installs it itself.
  Future<void> backgroundCheck() async {
    if (_busy || !mayCheckInBackground(activity)) return;
    _lastTried = clock();
    await _check(quiet: true);
    final r = _release;
    if (r != null &&
        _phase == UpdatePhase.available &&
        _mode == UpdateMode.auto &&
        target.downloadsInBackground &&
        canInstall &&
        r.tag != _backgroundFailed) {
      await update(quiet: true);
    }
  }

  /// Looks for a newer release, out loud: "checking…" and any failure is
  /// shown.
  Future<void> check() => _check(quiet: false);

  Future<void> _check({required bool quiet}) async {
    if (_busy) return;
    _busy = true;
    final before = _phase;
    if (!quiet) _set(UpdatePhase.checking);
    try {
      final releases = await source.releases();
      _lastCheck = clock().toUtc();
      await preferences.write(
        UpdatePrefs.lastCheck,
        '${_lastCheck!.millisecondsSinceEpoch}',
      );
      final found = newestUpdate(releases, current, channel);
      if (quiet && found?.tag == _release?.tag && before == _phase) {
        // Nothing new: no status change to report.
        if (found == null && _phase == UpdatePhase.idle) {
          _set(UpdatePhase.upToDate);
        }
        return;
      }
      _release = found;
      _set(found == null ? UpdatePhase.upToDate : UpdatePhase.available);
    } on UpdateNetworkException catch (e) {
      if (quiet) {
        log('warn', 'Scheduled update check failed: ${e.message}'); // l10n-ignore
      } else {
        _fail(
          e.rateLimited ? UpdateProblem.rateLimited : UpdateProblem.network,
          e.message,
        );
      }
    } finally {
      _busy = false;
    }
  }

  /// Downloads, verifies and installs [release]. [quiet]: started by the
  /// schedule; a failure goes to the log and the offer stays.
  Future<void> update({bool quiet = false}) async {
    final r = _release;
    if (r == null || _busy) return;
    final inst = installer;
    if (inst == null || installKilled) {
      _fail(installKilled ? UpdateProblem.killed : UpdateProblem.noAsset);
      return;
    }
    final asset = target.assetIn(r);
    final sumsAsset = r.asset('SHA256SUMS');
    if (asset == null || sumsAsset == null) {
      if (quiet) {
        log(
          'warn',
          'Release ${r.tag} has no ${asset == null ? target : 'SHA256SUMS'} file',
        );
        _backgroundFailed = r.tag;
        return;
      }
      _fail(UpdateProblem.noAsset, asset == null ? null : 'no SHA256SUMS');
      return;
    }
    _busy = true;
    _cancelled = false;
    _received = 0;
    _total = asset.size;
    File? file;
    try {
      _set(UpdatePhase.downloading);
      final sums = await http.getBytes(sumsAsset.url, maxBytes: 64 << 10);
      String? signature;
      if (r.asset('SHA256SUMS.minisig') case final sig?) {
        signature = String.fromCharCodes(
          await http.getBytes(sig.url, maxBytes: 16 << 10),
        );
      }
      await cacheDir.create(recursive: true);
      file = inst.downloadFile(cacheDir, asset.name);
      if (file.existsSync()) file.deleteSync();
      await http.download(
        asset.url,
        file,
        cancelled: () => _cancelled,
        onProgress: (received, total) {
          _received = received;
          _total = total ?? asset.size;
          notifyListeners();
        },
      );
      if (asset.size case final size? when file.lengthSync() != size) {
        throw UpdateVerifyFailure('size ${file.lengthSync()} != $size');
      }
      _set(UpdatePhase.verifying);
      await verify(
        file: file,
        assetName: asset.name,
        sums: sums,
        signature: signature,
      );
      log('info', 'Verified ${asset.name}');
      _set(UpdatePhase.installing);
      final outcome = await inst.install(file);
      switch (outcome) {
        case InstallOutcome.installed:
          _set(UpdatePhase.installed);
        case InstallOutcome.restartNeeded:
          // The AppImage now, or the staged Windows installer.
          file = null;
          _set(UpdatePhase.restartNeeded);
          if (restartWhenReady) unawaited(restart());
        case InstallOutcome.openedDmg:
          file = null; // Open in Finder.
          _set(UpdatePhase.handedOver);
        case InstallOutcome.needsPermission:
          _fail(UpdateProblem.needsPermission);
        case InstallOutcome.signatureMismatch:
          _fail(UpdateProblem.signatureMismatch);
        case InstallOutcome.aborted:
          _fail(UpdateProblem.aborted);
        case InstallOutcome.notWritable:
          _fail(UpdateProblem.notWritable, inst.lastError);
        case InstallOutcome.failed:
          _fail(UpdateProblem.installFailed, inst.lastError);
      }
    } on UpdateCancelled {
      _fail(UpdateProblem.aborted);
    } on UpdateNetworkException catch (e) {
      _fail(
        e.message.startsWith('disk:') && target.kind == UpdateKind.appImage
            ? UpdateProblem.notWritable
            : e.rateLimited
            ? UpdateProblem.rateLimited
            : UpdateProblem.network,
        e.message,
      );
    } on UpdateVerifyFailure catch (e) {
      log('warn', 'Update rejected: ${e.message}'); // l10n-ignore
      _fail(
        e.signature ? UpdateProblem.signature : UpdateProblem.checksum,
        e.message,
      );
    } finally {
      // A rejected or unused download never stays on disk. An APK stays
      // until the next start: PackageInstaller may still read it.
      final f = file;
      if (f != null &&
          _phase != UpdatePhase.installed &&
          _phase != UpdatePhase.installing &&
          f.existsSync()) {
        try {
          f.deleteSync();
        } on FileSystemException catch (e) {
          log('warn', 'Update download not removed: $e'); // l10n-ignore
        }
      }
      _busy = false;
      if (quiet && _phase == UpdatePhase.failed) {
        // Nobody asked: the offer stays, «Скачать» shows what went wrong.
        _backgroundFailed = r.tag;
        _problem = null;
        detail = null;
        _set(UpdatePhase.available);
      }
    }
  }

  /// Stops a running download.
  void cancel() => _cancelled = true;

  /// Runs the new version: restarts into the replaced AppImage or
  /// Kinolune.app, or runs the staged Windows installer.
  Future<void> restart() async {
    if (_phase != UpdatePhase.restartNeeded) return;
    log('info', 'Restarting into ${_release?.version}'); // l10n-ignore
    await installer?.restart();
  }

  /// The app is quitting: a staged Windows installer runs now, without
  /// starting the app again.
  Future<void> applyOnExit() async {
    if (_phase != UpdatePhase.restartNeeded) return;
    if (await installer?.applyOnExit() ?? false) {
      log('info', 'Installing ${_release?.version} on exit');
    }
  }

  /// Opens "Install unknown apps" for Kinolune (Android).
  Future<void> openInstallPermission() async {
    if (installer case final AndroidApkInstaller i) {
      await i.bridge.openInstallSettings();
    }
  }

  Future<void> setAutoCheck(bool on) async {
    _autoCheck = on;
    await preferences.write(UpdatePrefs.autoCheck, on ? null : 'false');
    log('info', 'Scheduled checks ${on ? 'on' : 'off'}'); // l10n-ignore
    notifyListeners();
  }

  Future<void> setMode(UpdateMode m) async {
    _mode = m;
    await preferences.write(UpdatePrefs.mode, m.name);
    log('info', 'Update mode ${m.name}'); // l10n-ignore
    notifyListeners();
  }

  Future<void> setChannel(UpdateChannel c) async {
    _channel = c;
    await preferences.write(UpdatePrefs.channel, c.name);
    // The offer depends on the channel: a pre-release is no update on
    // stable.
    if (_release case final r? when c == UpdateChannel.stable && r.prerelease) {
      _release = null;
      _set(UpdatePhase.idle);
    }
    notifyListeners();
  }

  /// Hides the banner for the offered release (About still shows it).
  Future<void> dismiss() async {
    final r = _release;
    if (r == null) return;
    _dismissed = r.tag;
    await preferences.write(UpdatePrefs.dismissed, r.tag);
    notifyListeners();
  }

  void _set(UpdatePhase phase) {
    final changed = phase != _phase;
    _phase = phase;
    if (phase != UpdatePhase.failed) {
      _problem = null;
      detail = null;
    }
    if (changed) {
      final v = _release?.version;
      log(
        'info',
        'Update status: ${phase.name}' // l10n-ignore
            '${v != null && phase != UpdatePhase.upToDate ? ' ($v)' : ''}',
      );
    }
    notifyListeners();
  }

  void _fail(UpdateProblem problem, [String? detail]) {
    _problem = problem;
    this.detail = detail;
    _phase = UpdatePhase.failed;
    log(
      'warn',
      'Update failed: ${problem.name}${detail == null ? '' : ' — $detail'}', // l10n-ignore
    );
    notifyListeners();
  }

  @override
  void dispose() {
    for (final t in _timers) {
      t.cancel();
    }
    _ticker?.cancel();
    super.dispose();
  }
}
