// SPDX-License-Identifier: MPL-2.0
//
// In-app updates of the full flavor (PKG-018, ADR-NNNN): once a day and on
// "Check for updates" the app reads the GitHub releases of the project,
// offers a newer `app-v*` release with its changelog, downloads the asset
// of this installation, checks it against the release's SHA256SUMS (and
// the minisign signature once the owner publishes a release key in the
// signed remote config) and installs it.

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

import '../../services/preferences.dart';
import 'app_version.dart';
import 'installers.dart';
import 'releases.dart';
import 'update_platform.dart';

/// Checks a download against the release's `SHA256SUMS` and signature;
/// throws [UpdateVerifyFailure] when it does not match. The app uses the
/// core (`update_verify_artifact`, pa-update).
typedef ArtifactVerifier = Future<void> Function({
  required File file,
  required String assetName,
  required List<int> sums,
  required String? signature,
});

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

  /// The AppImage was replaced; a restart runs the new version.
  restartNeeded,

  /// PackageInstaller took the APK; Android replaces the app.
  installed,
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

  /// The release key is set but the signature is missing or wrong.
  signature,

  /// The APK is signed with another key (the CI debug key of alpha
  /// builds): uninstall and reinstall.
  signatureMismatch,

  /// "Install unknown apps" is off for Kinolune.
  needsPermission,

  /// The user cancelled the system dialog or the download.
  aborted,

  /// The AppImage's folder is read-only.
  notWritable,

  /// The remote config stopped in-app updates (`updates.install`).
  killed,
  installFailed,
}

/// Preference keys of in-app updates.
abstract final class UpdatePrefs {
  /// 'false' turns the daily check off; absent = on.
  static const autoCheck = 'updates_auto_check';

  /// `stable` or `beta`; absent = beta while the app is a pre-release.
  static const channel = 'updates_channel';

  /// Milliseconds since the epoch of the last successful check.
  static const lastCheck = 'updates_last_check';

  /// Tag of the release whose banner the user closed.
  static const dismissed = 'updates_dismissed';
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
    this.interval = const Duration(days: 1),
  }) : http = http ?? UpdateHttp(),
       clock = clock ?? DateTime.now;

  /// The running version.
  final AppVersion current;
  final UpdateTarget target;
  final ReleaseSource source;
  final ArtifactVerifier verify;

  /// Downloads go to `<cacheDir>` (`<cache>/updates`), except the
  /// AppImage's (next to it, [UpdateInstaller.downloadFile]).
  final Directory cacheDir;
  final Preferences preferences;

  /// Null where the app does not install updates itself (tar.gz,
  /// Windows, macOS): the UI shows instructions or the release page.
  final UpdateInstaller? installer;
  final UpdateHttp http;

  /// A kill-switch of the signed remote config (`updates.install`): the app
  /// still says a release exists but leaves installing to the user.
  final bool installKilled;
  final DateTime Function() clock;
  final Duration interval;

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
  bool get autoCheck => _autoCheck;

  UpdateChannel? _channel;
  UpdateChannel get channel =>
      _channel ??
      (current.isPrerelease ? UpdateChannel.beta : UpdateChannel.stable);

  DateTime? _lastCheck;
  DateTime? get lastCheck => _lastCheck;

  String? _dismissed;
  bool _cancelled = false;
  bool _busy = false;

  /// The app installs updates itself here (and the remote config allows
  /// it).
  bool get canInstall => installer != null && !installKilled;

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

  /// Loads the settings, removes leftovers of earlier downloads and runs
  /// the daily check when it is due.
  Future<void> start() async {
    _autoCheck = await preferences.read(UpdatePrefs.autoCheck) != 'false';
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
    notifyListeners();
    await maybeAutoCheck();
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
      debugPrint('Old update downloads not removed: $e');
    }
  }

  /// The daily check: when on and the last one is older than [interval].
  /// Never shows errors (the next start tries again).
  Future<void> maybeAutoCheck() async {
    if (!_autoCheck || _busy) return;
    final last = _lastCheck;
    if (last != null && clock().difference(last) < interval) return;
    await check(manual: false);
  }

  /// Looks for a newer release. A failed automatic check stays silent.
  Future<void> check({bool manual = true}) async {
    if (_busy) return;
    _busy = true;
    final before = _phase;
    _set(UpdatePhase.checking);
    try {
      final releases = await source.releases();
      _lastCheck = clock().toUtc();
      await preferences.write(
        UpdatePrefs.lastCheck,
        '${_lastCheck!.millisecondsSinceEpoch}',
      );
      _release = newestUpdate(releases, current, channel);
      _set(_release == null ? UpdatePhase.upToDate : UpdatePhase.available);
    } on UpdateNetworkException catch (e) {
      debugPrint('Update check failed: $e');
      if (manual) {
        _fail(
          e.rateLimited ? UpdateProblem.rateLimited : UpdateProblem.network,
          e.message,
        );
      } else {
        _set(_release == null ? before : UpdatePhase.available);
      }
    } finally {
      _busy = false;
    }
  }

  /// Downloads, verifies and installs [release].
  Future<void> update() async {
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
      _set(UpdatePhase.installing);
      final outcome = await inst.install(file);
      switch (outcome) {
        case InstallOutcome.installed:
          _set(UpdatePhase.installed);
        case InstallOutcome.restartNeeded:
          file = null; // It is the AppImage now.
          _set(UpdatePhase.restartNeeded);
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
      debugPrint('Update rejected: $e');
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
          debugPrint('Update download not removed: $e');
        }
      }
      _busy = false;
    }
  }

  /// Stops a running download.
  void cancel() => _cancelled = true;

  /// Restarts into the replaced AppImage.
  Future<void> restart() async {
    if (installer case final AppImageInstaller i) await i.restart();
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
    _phase = phase;
    if (phase != UpdatePhase.failed) {
      _problem = null;
      detail = null;
    }
    notifyListeners();
  }

  void _fail(UpdateProblem problem, [String? detail]) {
    _problem = problem;
    this.detail = detail;
    _phase = UpdatePhase.failed;
    notifyListeners();
  }
}
