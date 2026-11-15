// SPDX-License-Identifier: MPL-2.0
//
// Diagnostics (CORE-008, ADR-NNNN): opt-in logs and crash reports kept on
// the device by the core (`pa-log`), and the diagnostic report zip the user
// attaches to a GitHub issue by hand. Nothing is written before the opt-in
// and nothing is ever sent by the app.

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';

import '../branding.dart';
import '../build_info.dart';
import '../services/preferences.dart';

/// Opt-in and what is kept.
@immutable
class DiagnosticsState {
  const DiagnosticsState({
    this.enabled = false,
    this.logFiles = 0,
    this.logBytes = 0,
    this.crashReports = 0,
  });

  final bool enabled;
  final int logFiles;
  final int logBytes;
  final int crashReports;

  bool get hasData => logFiles > 0 || crashReports > 0;
}

/// The written report.
@immutable
class DiagnosticsExport {
  const DiagnosticsExport({
    required this.path,
    required this.shownPath,
    required this.name,
    required this.issueUrl,
  });

  /// The zip on disk.
  final String path;

  /// Where the user finds it (`Download/…` on Android).
  final String shownPath;
  final String name;

  /// The prefilled GitHub issue; the app opens it only when asked.
  final Uri issueUrl;
}

/// The core side; [CoreDiagnosticsBackend] in the app, fakes in tests.
abstract class DiagnosticsBackend {
  DiagnosticsState status();
  DiagnosticsState setEnabled(bool on);
  DiagnosticsState clear();
  void log(String level, String target, String message);
  bool recordCrash(String kind, String message, String stack);
  Future<String> export(
    String outPath,
    Map<String, String> versions,
    Map<String, String> config,
  );
  String issueUrl(String repoUrl, Map<String, String> versions, String name);
}

/// Settings that go into the report (values are redacted by the core, keys
/// that look like secrets lose their value). Search history, favourites,
/// profiles and PINs are left out on purpose.
const diagnosticsConfigKeys = [
  Preferences.locale,
  Preferences.appMode,
  Preferences.showTwitch,
  Preferences.hideSpoilers,
  Preferences.continuousPlayback,
  Preferences.skipSeconds,
  Preferences.skipMode,
  Preferences.trickplayGenerate,
  Preferences.defaultEpisodeOrder,
  Preferences.metadataLanguage,
  Preferences.musicGapless,
  Preferences.musicReplayGain,
  Preferences.imageCacheMaxBytes,
  Preferences.offlineWifiOnly,
  Preferences.twitchQuality,
  Preferences.twitchLatencyMode,
  Preferences.twitchLiveNotifications,
];

/// Diagnostics of the app; `available` is false without the native core
/// (demo data, widget tests).
class Diagnostics extends ChangeNotifier {
  Diagnostics(this._backend) {
    _refresh();
  }

  /// No core: the Settings section is hidden, every call is a no-op.
  Diagnostics.unavailable() : _backend = null;

  /// The app's diagnostics, set by `startDiagnostics`.
  static Diagnostics instance = Diagnostics.unavailable();

  /// Where reports go by default: Downloads on desktops, the app's
  /// documents elsewhere (Android then copies into the shared Downloads).
  /// Tests replace it.
  static Future<String> Function() reportDir = _reportDir;

  final DiagnosticsBackend? _backend;
  DiagnosticsState _state = const DiagnosticsState();
  bool _busy = false;

  bool get available => _backend != null;
  DiagnosticsState get state => _state;
  bool get busy => _busy;

  void _refresh() {
    try {
      _state = _backend?.status() ?? const DiagnosticsState();
    } catch (e) {
      _state = const DiagnosticsState();
    }
  }

  Future<void> setEnabled(bool on) async {
    final b = _backend;
    if (b == null) return;
    _state = b.setEnabled(on);
    notifyListeners();
  }

  Future<void> clear() async {
    final b = _backend;
    if (b == null) return;
    _state = b.clear();
    notifyListeners();
  }

  /// An app log line; dropped without the opt-in.
  void log(String level, String target, String message) {
    try {
      _backend?.log(level, target, message);
    } catch (_) {}
  }

  /// A crash; saved only with the opt-in.
  void recordCrash(String kind, Object error, StackTrace? stack) {
    try {
      if (_backend?.recordCrash(kind, '$error', '${stack ?? ''}') ?? false) {
        _refresh();
        notifyListeners();
      }
    } catch (_) {}
  }

  /// Writes the report into [outDir] (default: the Downloads folder or
  /// the app's documents) and returns where it is and the issue link.
  Future<DiagnosticsExport?> exportReport({
    required BuildInfo build,
    required Preferences preferences,
    String? outDir,
    DateTime? now,
  }) async {
    final b = _backend;
    if (b == null) return null;
    _busy = true;
    notifyListeners();
    try {
      final versions = diagnosticsVersions(build);
      final config = <String, String>{};
      for (final k in diagnosticsConfigKeys) {
        final v = await preferences.read(k);
        if (v != null) config[k] = v;
      }
      final name = diagnosticsReportName(now ?? DateTime.now());
      final dir = outDir ?? (await reportDir());
      var path = await b.export('$dir/$name', versions, config);
      var shown = path;
      if (Platform.isAndroid && outDir == null) {
        final saved = await _androidSaveToDownloads(path, name);
        if (saved != null) {
          shown = saved;
          // The copy in Downloads is the one to attach; ours goes.
          try {
            await File(path).delete();
          } catch (_) {}
          path = saved;
        }
      }
      final url = b.issueUrl(kRepoUrl, versions, name);
      _refresh();
      return DiagnosticsExport(
        path: path,
        shownPath: shown,
        name: name,
        issueUrl: Uri.parse(url),
      );
    } finally {
      _busy = false;
      notifyListeners();
    }
  }
}

/// `kinolune-diagnostics-20261115-123456.zip`.
String diagnosticsReportName(DateTime t) {
  String two(int v) => v.toString().padLeft(2, '0');
  final u = t.toUtc();
  return 'kinolune-diagnostics-${u.year}${two(u.month)}${two(u.day)}'
      '-${two(u.hour)}${two(u.minute)}${two(u.second)}.zip';
}

/// Versions for the report and the issue form.
Map<String, String> diagnosticsVersions(BuildInfo b) => {
  'app': '${b.appVersion} (${b.flavor})',
  if (b.capabilities.isNotEmpty) 'app_capabilities': b.capabilities.join(' '),
  if (b.coreVersion != null)
    'core': '${b.coreVersion} (${b.coreFlavor ?? '?'})',
  if (b.coreCapabilities?.isNotEmpty ?? false)
    'core_capabilities': b.coreCapabilities!.join(' '),
  'platform': Platform.operatingSystem,
  'platform_version': Platform.operatingSystemVersion,
  'dart': Platform.version.split(' ').first,
  'locale': Platform.localeName,
};

Future<String> _reportDir() async {
  Directory? dir;
  try {
    if (!Platform.isAndroid && !Platform.isIOS) {
      dir = await getDownloadsDirectory();
    }
  } catch (_) {}
  dir ??= await getApplicationDocumentsDirectory();
  return dir.path;
}

const _channel = MethodChannel('app.kinolune/diagnostics');

/// Copies the zip into the shared Downloads (Android 10+); the name the
/// user sees there, or null (older Android: the app's own folder stays).
Future<String?> _androidSaveToDownloads(String path, String name) async {
  try {
    return await _channel.invokeMethod<String>('saveToDownloads', {
      'path': path,
      'name': name,
    });
  } catch (e) {
    debugPrint('Diagnostics: not saved to Downloads: $e');
    return null;
  }
}

/// Android: crashes and ANRs of earlier runs the system remembers
/// (`ApplicationExitInfo`, Android 11+), after [sinceMs]. Native crashes of
/// libmpv or the core land here, not in a Dart handler.
Future<void> collectAndroidExitReasons(
  Diagnostics d,
  Preferences preferences,
) async {
  if (!Platform.isAndroid || !d.available || !d.state.enabled) return;
  const key = 'diagnostics_exit_seen';
  try {
    final since = int.tryParse(await preferences.read(key) ?? '') ?? 0;
    final raw = await _channel.invokeListMethod<Map<Object?, Object?>>(
      'exitReasons',
      {'sinceMs': since},
    );
    var newest = since;
    for (final e in raw ?? const <Map<Object?, Object?>>[]) {
      final time = (e['time'] as num?)?.toInt() ?? 0;
      if (time <= since) continue;
      if (time > newest) newest = time;
      d.recordCrash(
        'android-${e['reason']}',
        'Earlier run ended: ${e['reason']} (status ${e['status']}) at ' // l10n-ignore
            '${DateTime.fromMillisecondsSinceEpoch(time, isUtc: true).toIso8601String()}'
            ': ${e['description'] ?? ''}',
        StackTrace.fromString('${e['trace'] ?? ''}'),
      );
    }
    if (newest > since) await preferences.write(key, '$newest');
  } catch (e) {
    debugPrint('Diagnostics: exit reasons not read: $e');
  }
}

/// Routes Flutter and uncaught Dart errors and `debugPrint` into [d]. The
/// previous handlers keep running.
void installDiagnosticsHandlers(Diagnostics d) {
  final prevFlutter = FlutterError.onError;
  FlutterError.onError = (details) {
    d.log('error', 'flutter', details.exceptionAsString());
    if (!details.silent) {
      d.recordCrash('flutter', details.exceptionAsString(), details.stack);
    }
    if (prevFlutter != null) {
      prevFlutter(details);
    } else {
      FlutterError.presentError(details);
    }
  };
  final dispatcher = PlatformDispatcher.instance;
  final prevPlatform = dispatcher.onError;
  dispatcher.onError = (error, stack) {
    d.recordCrash('dart', error, stack);
    return prevPlatform?.call(error, stack) ?? false;
  };
  final prevPrint = debugPrint;
  debugPrint = (String? message, {int? wrapWidth}) {
    prevPrint(message, wrapWidth: wrapWidth);
    if (message != null) d.log('info', 'app', message);
  };
}
