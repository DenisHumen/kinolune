// SPDX-License-Identifier: MPL-2.0
//
// In-app updates of the full flavor (PKG-018, PKG-022, ADR-0109): five
// seconds after launch and then every six hours (and on "Check for
// updates") the app reads the GitHub releases of the project, offers a
// newer `app-v*` release with its changelog, downloads the asset of this
// installation, checks it against the release's signed SHA256SUMS and
// installs it. With «Обновлять автоматически» the update is silent
// (PKG-024): downloaded and verified in the background while nothing
// plays, put in place at the next start or by a restart once the app is
// idle, with a short «Что нового» afterwards; Android, whose system dialog
// cannot be skipped, offers «Установить» when nothing plays.

import 'dart:async';
import 'dart:convert';
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

  /// Android (PKG-024): the APK is downloaded and verified in the
  /// background; «Установить» hands it to the system dialog.
  readyToInstall,

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

  /// PKG-024: `{"version", "notes"}` of the update being put in place, for
  /// the «Что нового» note after the next start into that version.
  static const whatsNew = 'updates_whats_new';
}

/// The short note after an update (PKG-024).
class WhatsNew {
  const WhatsNew(this.version, this.notes);

  final AppVersion version;

  /// The changelog of the release (without the downloads table); may be
  /// empty.
  final String notes;
}

/// A verified download waiting for the user's tap (Android).
class _Staged {
  _Staged(this.tag, this.file, this.assetName, this.sums, this.signature);

  final String tag;
  final File file;
  final String assetName;
  final List<int> sums;
  final String? signature;
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
    this.playback,
    this.tv = false,
    this.idleApply = UpdateSchedule.idleApply,
    this.meteredNetwork,
    this.requireForeground = true,
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

  /// True while something plays or a video is open (pa_player's
  /// `PlaybackActivity.busy`): a silent update neither downloads nor
  /// restarts then, and no strip covers the player (PKG-024).
  final ValueListenable<bool>? playback;

  /// A TV: never restarts by itself; the update waits for the next start.
  final bool tv;

  /// How long the app must be idle before a desktop restarts into a staged
  /// update; seconds in the CI update test ([kUpdateSmokeEnv]).
  final Duration idleApply;

  /// Whether the network is metered (Android): no silent download then.
  final Future<bool> Function()? meteredNetwork;

  /// A silent restart waits for the window to be in front. Off only in the
  /// CI update test: its runners have no window manager to focus the
  /// window (the rule itself is tested on its own).
  final bool requireForeground;

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

  /// A release not downloaded silently because the network is metered:
  /// offered like in «Только сообщать».
  String? _meteredSkipped;

  /// A silent download waits for the playback to end.
  bool _deferred = false;
  _Staged? _staged;
  WhatsNew? _whatsNew;
  late DateTime _lastInput = clock();
  bool _foreground = true;
  Timer? _idleTimer;
  final List<Timer> _timers = [];
  Timer? _ticker;

  /// The app installs updates itself here (and the remote config allows
  /// it).
  bool get canInstall => installer != null && !installKilled;

  /// A staged desktop update goes in by itself (PKG-024): at the next
  /// start, or by a restart once the app is idle.
  bool get restartsQuietly =>
      _mode == UpdateMode.auto && target.appliesQuietly && canInstall;

  /// Something plays or a video is open.
  bool get playing => playback?.value ?? false;

  /// The note after an update into this version, until closed.
  WhatsNew? get whatsNew => _whatsNew;

  /// Show the «Что нового» strip: after an update, not over the player.
  bool get whatsNewVisible => _whatsNew != null && !playing;

  /// Updates of [r] go without a question here (PKG-024): the download is
  /// silent, and on desktops so is the restart.
  bool _silentFor(AppRelease r) =>
      _mode == UpdateMode.auto &&
      target.downloadsInBackground &&
      canInstall &&
      r.tag != _backgroundFailed &&
      r.tag != _meteredSkipped;

  /// Where a check nobody asked for must not interfere.
  UpdateActivity get activity => switch (_phase) {
    UpdatePhase.checking => UpdateActivity.checking,
    UpdatePhase.downloading ||
    UpdatePhase.verifying ||
    UpdatePhase.installing => UpdateActivity.downloading,
    UpdatePhase.restartNeeded ||
    UpdatePhase.readyToInstall ||
    UpdatePhase.installed ||
    UpdatePhase.handedOver => UpdateActivity.readyToRestart,
    _ => _busy ? UpdateActivity.checking : UpdateActivity.idle,
  };

  /// Show the banner across the app: a release the user has not closed it
  /// for, or a download that runs. Never over the player, and without a
  /// question while the update is silent (PKG-024): then only a failure
  /// somebody asked for, or Android's «Установить», shows.
  bool get bannerVisible {
    final r = _release;
    if (r == null || r.tag == _dismissed || playing) return false;
    if (_silentFor(r)) {
      return switch (_phase) {
        UpdatePhase.readyToInstall || UpdatePhase.failed => true,
        UpdatePhase.restartNeeded => !target.appliesQuietly,
        _ => false,
      };
    }
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
    _lastInput = clock();
    playback?.addListener(_onPlayback);
    log(
      'info',
      'Running $current ($target), mode ${_mode.name}, '
          'checks ${_autoCheck ? 'on' : 'off'}',
    );
    await _readWhatsNew();
    notifyListeners();
  }

  /// The note of the update this start runs (PKG-024). A note for an
  /// older version is dropped; one for a newer version (staged, not yet
  /// in place) waits.
  Future<void> _readWhatsNew() async {
    final raw = await preferences.read(UpdatePrefs.whatsNew);
    if (raw == null) return;
    AppVersion? version;
    var notes = '';
    try {
      final m = jsonDecode(raw) as Map<String, Object?>;
      version = AppVersion.tryParse('${m['version']}');
      notes = '${m['notes'] ?? ''}';
    } on FormatException {
      version = null;
    } on TypeError {
      version = null;
    }
    if (version != null && version == current) {
      _whatsNew = WhatsNew(version, notes);
      log('info', 'Updated to $current'); // l10n-ignore
    } else if (version == null || !(version > current)) {
      await preferences.write(UpdatePrefs.whatsNew, null);
    }
  }

  /// Closes the «Что нового» note for good.
  Future<void> dismissWhatsNew() async {
    if (_whatsNew == null) return;
    _whatsNew = null;
    await preferences.write(UpdatePrefs.whatsNew, null);
    notifyListeners();
  }

  /// Remembers [r]'s changelog before it is put in place: the next start
  /// into that version shows it.
  Future<void> _rememberWhatsNew(AppRelease r) async {
    final notes = r.changelog;
    await preferences.write(
      UpdatePrefs.whatsNew,
      jsonEncode({
        'version': r.version.text,
        'notes': notes.length > 4000 ? notes.substring(0, 4000) : notes,
      }),
    );
  }

  void _onPlayback() {
    if (!playing) {
      // The end of the playback counts as activity: no restart right
      // after the credits.
      _lastInput = clock();
      if (_deferred) {
        _deferred = false;
        log('info', 'Playback ended; downloading the update'); // l10n-ignore
        unawaited(_quietDownload());
      }
    }
    notifyListeners();
  }

  /// A key press, a click or a touch anywhere in the app.
  void userActive() => _lastInput = clock();

  /// The window is in front (or went behind other windows, or hid).
  bool get foreground => _foreground;
  set foreground(bool value) {
    if (value == _foreground) return;
    _foreground = value;
    if (value) _lastInput = clock();
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
    await _quietDownload();
  }

  /// The silent download (PKG-024) of a found release: never while
  /// something plays (it starts when the playback ends) and on Android not
  /// over a metered network (then the release is offered instead).
  Future<void> _quietDownload() async {
    final r = _release;
    if (r == null || _phase != UpdatePhase.available || !_silentFor(r)) {
      return;
    }
    final metered = !playing && (await meteredNetwork?.call() ?? false);
    if (!mayDownloadQuietly(playing: playing, metered: metered)) {
      if (playing && !_deferred) {
        log('info', 'Update ${r.version} waits for the playback to end'); // l10n-ignore
      }
      _deferred = playing;
      if (metered) {
        log('info', 'Metered network: offering ${r.version} instead'); // l10n-ignore
        _meteredSkipped = r.tag;
        notifyListeners();
      }
      return;
    }
    await update(quiet: true);
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
        final why = e.message;
        log('warn', 'Scheduled update check failed: $why'); // l10n-ignore
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
    if (_staged case final staged?
        when staged.tag == r.tag && _phase == UpdatePhase.readyToInstall) {
      return _installStaged(inst, r, staged);
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
      if (quiet && inst.asksUser) {
        // Android: the system dialog is the user's to open, when nothing
        // plays. The verified APK waits in the app's own cache.
        _staged = _Staged(r.tag, file, asset.name, sums, signature);
        file = null;
        _set(UpdatePhase.readyToInstall);
        return;
      }
      _set(UpdatePhase.installing);
      await _rememberWhatsNew(r);
      if (_apply(inst, await inst.install(file))) file = null;
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

  /// Moves to the state [outcome] leads to; true when the downloaded file
  /// is now in use (a staged installer, a DMG opened in Finder).
  bool _apply(UpdateInstaller inst, InstallOutcome outcome) {
    switch (outcome) {
      case InstallOutcome.installed:
        _set(UpdatePhase.installed);
      case InstallOutcome.restartNeeded:
        // The AppImage now, or the staged Windows installer.
        _set(UpdatePhase.restartNeeded);
        _watchIdle();
        return true;
      case InstallOutcome.openedDmg:
        _set(UpdatePhase.handedOver);
        return true; // Open in Finder.
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
    return false;
  }

  /// «Установить» on a silently downloaded APK: checked once more (it
  /// waited on disk), then the system dialog.
  Future<void> _installStaged(
    UpdateInstaller inst,
    AppRelease r,
    _Staged staged,
  ) async {
    _busy = true;
    _staged = null;
    final file = staged.file;
    try {
      _set(UpdatePhase.verifying);
      if (!file.existsSync()) {
        throw const UpdateVerifyFailure('the download is gone'); // l10n-ignore
      }
      await verify(
        file: file,
        assetName: staged.assetName,
        sums: staged.sums,
        signature: staged.signature,
      );
      _set(UpdatePhase.installing);
      await _rememberWhatsNew(r);
      _apply(inst, await inst.install(file));
    } on UpdateVerifyFailure catch (e) {
      log('warn', 'Update rejected: ${e.message}'); // l10n-ignore
      _fail(
        e.signature ? UpdateProblem.signature : UpdateProblem.checksum,
        e.message,
      );
    } finally {
      // The APK stays until the next start while PackageInstaller may
      // read it; otherwise it goes.
      if (_phase != UpdatePhase.installed &&
          _phase != UpdatePhase.installing &&
          file.existsSync()) {
        try {
          file.deleteSync();
        } on FileSystemException catch (e) {
          log('warn', 'Update download not removed: $e'); // l10n-ignore
        }
      }
      _busy = false;
    }
  }

  /// A staged desktop update in «Обновлять автоматически»: restart into it
  /// once the app is idle (PKG-024). Otherwise it is in place at the next
  /// start anyway (Windows: when the app quits).
  void _watchIdle() {
    if (_mode != UpdateMode.auto || !target.appliesQuietly || tv) return;
    final seconds = (idleApply.inSeconds ~/ 20).clamp(
      1,
      UpdateSchedule.idlePoll.inSeconds,
    );
    _idleTimer?.cancel();
    _idleTimer = Timer.periodic(
      Duration(seconds: seconds),
      (_) => unawaited(applyIfIdle()),
    );
  }

  /// Restarts into a staged update when [mayApplyWhenIdle] allows; true
  /// when it did.
  Future<bool> applyIfIdle() async {
    if (_phase != UpdatePhase.restartNeeded ||
        _mode != UpdateMode.auto ||
        !target.appliesQuietly) {
      return false;
    }
    if (!mayApplyWhenIdle(
      now: clock(),
      lastInput: _lastInput,
      playing: playing,
      foreground: _foreground || !requireForeground,
      tv: tv,
      idle: idleApply,
    )) {
      return false;
    }
    _idleTimer?.cancel();
    _idleTimer = null;
    log('info', 'Idle with nothing playing: applying the update'); // l10n-ignore
    await restart();
    return true;
  }

  /// Stops a running download.
  void cancel() => _cancelled = true;

  /// Runs the new version: restarts into the replaced AppImage or
  /// Kinolune.app, or runs the staged Windows installer.
  Future<void> restart() async {
    if (_phase != UpdatePhase.restartNeeded) return;
    _idleTimer?.cancel();
    _idleTimer = null;
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
    if (_phase == UpdatePhase.restartNeeded) {
      if (m == UpdateMode.auto) {
        _watchIdle();
      } else {
        _idleTimer?.cancel();
        _idleTimer = null;
      }
    }
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
    _idleTimer?.cancel();
    playback?.removeListener(_onPlayback);
    super.dispose();
  }
}
