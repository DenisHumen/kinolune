// SPDX-License-Identifier: MPL-2.0
//
// Diagnostics (CORE-008, ADR-0082): opt-in logs and crash reports kept on
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
    this.savedFolder,
    this.shareUri,
  });

  /// The zip on disk.
  final String path;

  /// Where the user finds it (`Download/…` on Android).
  final String shownPath;
  final String name;

  /// The folder the report was written to; null on Android (Downloads).
  final String? savedFolder;

  /// The folder the report is in, as the user sees it (BUG-024): with the
  /// system's separators (`C:\Users\…\Downloads` on Windows), `Download`
  /// on Android.
  String get folder {
    final f = savedFolder;
    if (f != null) return f;
    final cut = shownPath.lastIndexOf(RegExp(r'[/\\]'));
    return cut <= 0 ? shownPath : shownPath.substring(0, cut);
  }

  /// Android: the `content://` address of the copy in Downloads, for the
  /// share sheet; null elsewhere.
  final String? shareUri;

  /// The prefilled GitHub issue; the app opens it only when asked.
  final Uri issueUrl;
}

/// One crash report kept on the device (the «Состояние» screen, UI-017).
@immutable
class CrashInfo {
  const CrashInfo({
    required this.name,
    required this.kind,
    required this.headline,
    this.time,
  });

  /// File name; the id for [Diagnostics.crashText].
  final String name;

  /// `rust-panic`, `dart`, `flutter`, `android-crash`, …
  final String kind;
  final String headline;
  final DateTime? time;
}

/// The core side; [CoreDiagnosticsBackend] in the app, fakes in tests.
abstract class DiagnosticsBackend {
  DiagnosticsState status();
  DiagnosticsState setEnabled(bool on);
  DiagnosticsState clear();
  void log(String level, String target, String message);
  bool recordCrash(String kind, String message, String stack);

  /// [attachments]: extra text files (name → text) for `extra/` of the
  /// report, e.g. the player's stats (PLAY-010).
  Future<String> export(
    String outPath,
    Map<String, String> versions,
    Map<String, String> config, {
    Map<String, String> attachments = const {},
  });
  String issueUrl(String repoUrl, Map<String, String> versions, String name);

  /// The last [maxLines] log lines, oldest first, redacted (UI-017).
  Future<List<String>> recentLog(int maxLines);

  /// Crash reports kept on the device, newest first.
  List<CrashInfo> crashes();

  /// The redacted text of one crash report.
  String crashText(String name);

  /// Engines inside the core: `sqlite`, `core_target`.
  Map<String, String> engineVersions();
}

/// Settings that go into the report (values are redacted by the core, keys
/// that look like secrets lose their value). Search history, favourites,
/// profiles, names and PINs are left out on purpose.
const diagnosticsConfigKeys = [
  Preferences.locale,
  Preferences.appMode,
  Preferences.settingsAdvanced,
  Preferences.onboardingDone,
  Preferences.showTwitch,
  Preferences.hideSpoilers,
  Preferences.browserShowFileNames,
  Preferences.continuousPlayback,
  Preferences.skipSeconds,
  Preferences.skipMode,
  Preferences.trickplayGenerate,
  Preferences.displayMatch,
  Preferences.audioOutputMode,
  Preferences.subtitleSettings,
  Preferences.defaultEpisodeOrder,
  Preferences.metadataLanguage,
  Preferences.animeDataUpdates,
  Preferences.animeFrameSearch,
  Preferences.musicGapless,
  Preferences.musicReplayGain,
  Preferences.lyricsOnline,
  Preferences.imageCacheMaxBytes,
  Preferences.takeoverPause,
  Preferences.profilesPickerOnStart,
  Preferences.profilesAppLock,
  Preferences.offlineWifiOnly,
  Preferences.offlineCleanupDefault,
  Preferences.twitchQuality,
  Preferences.twitchLatencyMode,
  Preferences.twitchLiveNotifications,
  Preferences.twitchChatPauseAnimations,
];

/// What `config.txt` says of a setting the user never changed.
const diagnosticsConfigDefault = '(default)'; // l10n-ignore

/// Keys whose value never leaves the app, whatever the list above says:
/// the words of the core's `Redactor::is_secret_key` (the core drops such
/// values too; this keeps them out of the call altogether).
bool diagnosticsSecretKey(String key) {
  final k = key.toLowerCase();
  const words = [
    'pass',
    'secret',
    'token',
    'key',
    'auth',
    'cookie',
    'pin',
    'session',
    'credential',
  ];
  return words.any(k.contains) && !k.endsWith('_bytes');
}

/// The settings for `config.txt` (BUG-024): every key of
/// [diagnosticsConfigKeys] with the stored value or
/// [diagnosticsConfigDefault]. Before, settings the user never changed
/// were left out and the file came out empty.
Future<Map<String, String>> diagnosticsConfig(Preferences preferences) async {
  final config = <String, String>{};
  for (final k in diagnosticsConfigKeys) {
    if (diagnosticsSecretKey(k)) continue;
    String? v;
    try {
      v = await preferences.read(k);
    } catch (_) {}
    config[k] = v ?? diagnosticsConfigDefault;
  }
  return config;
}

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

  /// Versions of the player's engines (`mpv`, `ffmpeg`), read once from a
  /// short-lived libmpv instance; set by `startDiagnostics`, empty in
  /// tests.
  static Future<Map<String, String>> Function() playerVersions = () async =>
      const {};

  /// Shows the saved report (BUG-024): its folder with the file selected on
  /// desktops, the share sheet on Android. False when nothing opened.
  /// Tests replace it.
  static Future<bool> Function(DiagnosticsExport report) revealReport =
      _revealReport;

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

  Map<String, String>? _engines;

  /// Versions of the core's and the player's engines (UI-017), read once
  /// per run: they do not change while the app runs.
  Future<Map<String, String>> engineVersions() async {
    final cached = _engines;
    if (cached != null) return cached;
    final out = <String, String>{};
    try {
      out.addAll(await playerVersions());
    } catch (_) {}
    try {
      out.addAll(_backend?.engineVersions() ?? const {});
    } catch (_) {}
    return _engines = Map.unmodifiable(out);
  }

  /// The last [maxLines] lines of the log, oldest first (redacted by the
  /// core). Empty without the opt-in.
  Future<List<String>> recentLog({int maxLines = 300}) async {
    try {
      return await _backend?.recentLog(maxLines) ?? const [];
    } catch (_) {
      return const [];
    }
  }

  /// Crash reports kept on the device, newest first.
  List<CrashInfo> crashes() {
    try {
      return _backend?.crashes() ?? const [];
    } catch (_) {
      return const [];
    }
  }

  /// The text of one crash report; null when it is gone.
  String? crashText(String name) {
    try {
      return _backend?.crashText(name);
    } catch (_) {
      return null;
    }
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
  /// [attachments] (file name → text) go into `extra/` of the zip: the
  /// player's stats report (PLAY-010).
  Future<DiagnosticsExport?> exportReport({
    required BuildInfo build,
    required Preferences preferences,
    String? outDir,
    DateTime? now,
    Map<String, String> attachments = const {},
  }) async {
    final b = _backend;
    if (b == null) return null;
    _busy = true;
    notifyListeners();
    try {
      final versions = {
        ...diagnosticsVersions(build),
        ...await engineVersions(),
      };
      final config = await diagnosticsConfig(preferences);
      final name = diagnosticsReportName(now ?? DateTime.now());
      final dir = outDir ?? (await reportDir());
      var path = await b.export(
        '$dir${Platform.pathSeparator}$name',
        versions,
        config,
        attachments: attachments,
      );
      var shown = path;
      String? folder = dir;
      String? shareUri;
      if (Platform.isAndroid && outDir == null) {
        final saved = await _androidSaveToDownloads(path, name);
        if (saved != null) {
          shown = saved.shown;
          folder = null;
          shareUri = saved.uri;
          // The copy in Downloads is the one to attach; ours goes.
          try {
            await File(path).delete();
          } catch (_) {}
          path = saved.shown;
        }
      }
      final url = b.issueUrl(kRepoUrl, versions, name);
      _refresh();
      return DiagnosticsExport(
        path: path,
        shownPath: shown,
        name: name,
        issueUrl: Uri.parse(url),
        savedFolder: folder,
        shareUri: shareUri,
      );
    } finally {
      _busy = false;
      notifyListeners();
    }
  }
}

/// `kinolune-diagnostics-20261115-123456.zip`, in the device's local time
/// (BUG-024: it was UTC, hours off the clock); the zone is in
/// `versions.txt` (`time_zone`).
String diagnosticsReportName(DateTime t) {
  String two(int v) => v.toString().padLeft(2, '0');
  final u = t.toLocal();
  return 'kinolune-diagnostics-${u.year}${two(u.month)}${two(u.day)}'
      '-${two(u.hour)}${two(u.minute)}${two(u.second)}.zip';
}

/// Versions for the report and the issue form.
Map<String, String> diagnosticsVersions(BuildInfo b) {
  final raw = Platform.operatingSystemVersion;
  final os = currentOsVersion();
  return {
    'app': '${b.appVersion} (${b.flavor})',
    if (b.capabilities.isNotEmpty)
      'app_capabilities': b.capabilities.join(' '),
    if (b.coreVersion != null)
      'core': '${b.coreVersion} (${b.coreFlavor ?? '?'})',
    if (b.coreCapabilities?.isNotEmpty ?? false)
      'core_capabilities': b.coreCapabilities!.join(' '),
    'platform': Platform.operatingSystem,
    'platform_version': os,
    if (os != raw) 'platform_version_raw': raw,
    'dart': Platform.version.split(' ').first,
    'locale': Platform.localeName,
    'time_zone': diagnosticsTimeZone(DateTime.now()),
  };
}

/// `EET (UTC+02:00)`.
String diagnosticsTimeZone(DateTime t) {
  final o = t.timeZoneOffset;
  final sign = o.isNegative ? '-' : '+';
  final m = o.inMinutes.abs();
  String two(int v) => v.toString().padLeft(2, '0');
  return '${t.timeZoneName} (UTC$sign${two(m ~/ 60)}:${two(m % 60)})';
}

/// The system's name and version as people know it (BUG-024), from
/// [Platform.operatingSystemVersion]:
///
/// * Windows says `"Windows 10 Pro" 10.0 (Build 26200)` on Windows 11 too
///   (the product name in the registry never changed); build 22000 and
///   later is Windows 11: `Windows 11 Pro 10.0 (Build 26200)`.
/// * macOS: `Version 15.1 (Build 24B83)` → `macOS 15.1 (Build 24B83)`.
/// * Linux: the distribution from `/etc/os-release` ([linuxOsRelease]) and
///   the kernel: `Ubuntu 24.04.1 LTS (kernel 6.8.0-49-generic)`.
String friendlyOsVersion(
  String os,
  String version, {
  String? linuxOsRelease,
}) {
  final v = version.trim();
  switch (os) {
    case 'windows':
      final build = int.tryParse(
        RegExp(r'Build (\d+)').firstMatch(v)?.group(1) ?? '',
      );
      final quoted = RegExp(r'^"([^"]*)"\s*(.*)$').firstMatch(v);
      var name = quoted?.group(1)?.trim() ?? '';
      final rest = quoted == null ? v : quoted.group(2)!.trim();
      if (name.isEmpty) name = 'Windows';
      if (build != null && build >= 22000) {
        // Windows Server keeps its own name (2025 is build 26100 too).
        if (name.contains(RegExp(r'Windows 10\b'))) {
          name = name.replaceFirst(RegExp(r'Windows 10\b'), 'Windows 11');
        } else if (name == 'Windows') {
          name = 'Windows 11';
        }
      }
      return rest.isEmpty ? name : '$name $rest';
    case 'macos':
      final m = RegExp(r'^Version\s+(.*)$').firstMatch(v);
      return 'macOS ${m?.group(1) ?? v}';
    case 'linux':
      final kernel = v.split(RegExp(r'\s+')).first;
      final distro = _osReleaseName(linuxOsRelease) ?? 'Linux';
      return kernel.isEmpty ? distro : '$distro (kernel $kernel)';
    default:
      return v;
  }
}

/// `PRETTY_NAME` (or `NAME VERSION_ID`) of an `os-release` file.
String? _osReleaseName(String? text) {
  if (text == null) return null;
  final values = <String, String>{};
  for (final line in text.split('\n')) {
    final eq = line.indexOf('=');
    if (eq <= 0) continue;
    var value = line.substring(eq + 1).trim();
    if (value.length >= 2 &&
        (value.startsWith('"') && value.endsWith('"') ||
            value.startsWith("'") && value.endsWith("'"))) {
      value = value.substring(1, value.length - 1);
    }
    values[line.substring(0, eq).trim()] = value;
  }
  final pretty = values['PRETTY_NAME'];
  if (pretty != null && pretty.isNotEmpty) return pretty;
  final name = values['NAME'];
  if (name == null || name.isEmpty) return null;
  final id = values['VERSION_ID'];
  return id == null || id.isEmpty ? name : '$name $id';
}

String? _osVersionCache;

/// [friendlyOsVersion] of this device, read once.
String currentOsVersion() => _osVersionCache ??= friendlyOsVersion(
  Platform.operatingSystem,
  Platform.operatingSystemVersion,
  linuxOsRelease: Platform.isLinux ? _readOsRelease() : null,
);

String? _readOsRelease() {
  for (final p in const ['/etc/os-release', '/usr/lib/os-release']) {
    try {
      final f = File(p);
      if (f.existsSync()) return f.readAsStringSync();
    } catch (_) {}
  }
  return null;
}

Future<String> _reportDir() async {
  Directory? dir;
  try {
    if (!Platform.isAndroid && !Platform.isIOS) {
      dir = await getDownloadsDirectory();
    }
  } catch (_) {}
  try {
    dir ??= await getApplicationDocumentsDirectory();
  } catch (_) {}
  // QA-016: Linux without xdg-user-dirs has neither folder; the report then
  // goes to the home folder, or to the app's own folder.
  final home = Platform.environment['HOME'];
  if (dir == null &&
      !Platform.isAndroid &&
      !Platform.isIOS &&
      home != null &&
      Directory(home).existsSync()) {
    dir = Directory(home);
  }
  dir ??= await getApplicationSupportDirectory();
  return dir.path;
}

const _channel = MethodChannel('app.kinolune/diagnostics');

/// Copies the zip into the shared Downloads (Android 10+): the name the
/// user sees there and its `content://` address, or null (older Android:
/// the app's own folder stays).
Future<({String shown, String? uri})?> _androidSaveToDownloads(
  String path,
  String name,
) async {
  try {
    final r = await _channel.invokeMapMethod<String, Object?>(
      'saveToDownloads',
      {'path': path, 'name': name},
    );
    final shown = r?['shown'];
    if (shown is! String) return null;
    final uri = r?['uri'];
    return (shown: shown, uri: uri is String ? uri : null);
  } catch (e) {
    debugPrint('Diagnostics: not saved to Downloads: $e');
    return null;
  }
}

/// How each desktop shows a file in its folder (BUG-024): Explorer and
/// Finder select it; on Linux the file manager gets the folder
/// (`xdg-open`; the FileManager1 D-Bus call that selects a file is not
/// everywhere). Null elsewhere.
List<String>? revealCommand(String os, String file, String folder) =>
    switch (os) {
      'windows' => ['explorer.exe', '/select,$file'],
      'macos' => ['open', '-R', file],
      'linux' => ['xdg-open', folder],
      _ => null,
    };

Future<bool> _revealReport(DiagnosticsExport report) async {
  if (Platform.isAndroid) {
    final uri = report.shareUri;
    if (uri == null) return false;
    try {
      return await _channel.invokeMethod<bool>('share', {
            'uri': uri,
            'name': report.name,
          }) ??
          false;
    } catch (e) {
      debugPrint('Diagnostics: share sheet not opened: $e');
      return false;
    }
  }
  final cmd = revealCommand(
    Platform.operatingSystem,
    report.path,
    report.folder,
  );
  if (cmd == null) return false;
  try {
    // Explorer exits with 1 even when its window opened; only a failure to
    // start counts.
    await Process.start(
      cmd.first,
      cmd.sublist(1),
      mode: ProcessStartMode.detached,
    );
    return true;
  } catch (e) {
    debugPrint('Diagnostics: folder not opened: $e');
    return false;
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
