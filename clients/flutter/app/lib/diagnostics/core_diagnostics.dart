// SPDX-License-Identifier: MPL-2.0
//
// The core half of diagnostics (CORE-008): pa_core's `diagnostics*`.

import 'package:flutter/foundation.dart';
import 'package:pa_core/pa_core.dart' as core;
import 'package:pa_player/pa_player.dart' show MediaKitEngine, PlayerScreen;

import '../integration/library.dart' show RustLibrary;
import 'diagnostics.dart';
import 'diagnostics_section.dart' show exportPlaybackReport;

DiagnosticsState _state(core.DiagnosticsStatus s) => DiagnosticsState(
  enabled: s.enabled,
  logFiles: s.logFiles,
  logBytes: s.logBytes.toInt(),
  crashReports: s.crashReports,
);

List<core.DiagnosticsEntry> _entries(Map<String, String> m) => [
  for (final e in m.entries) core.DiagnosticsEntry(key: e.key, value: e.value),
];

class CoreDiagnosticsBackend implements DiagnosticsBackend {
  @override
  DiagnosticsState status() {
    final s = core.diagnosticsStatus();
    return s == null ? const DiagnosticsState() : _state(s);
  }

  @override
  DiagnosticsState setEnabled(bool on) =>
      _state(core.diagnosticsSetEnabled(enabled: on));

  @override
  DiagnosticsState clear() => _state(core.diagnosticsClear());

  @override
  void log(String level, String target, String message) =>
      core.diagnosticsLog(level: level, target: target, message: message);

  @override
  bool recordCrash(String kind, String message, String stack) =>
      core.diagnosticsRecordCrash(kind: kind, message: message, stack: stack);

  @override
  Future<String> export(
    String outPath,
    Map<String, String> versions,
    Map<String, String> config, {
    Map<String, String> attachments = const {},
  }) async {
    final r = await core.diagnosticsExportReport(
      outPath: outPath,
      versions: _entries(versions),
      config: _entries(config),
      attachments: _entries(attachments),
    );
    return r.path;
  }

  @override
  String issueUrl(String repoUrl, Map<String, String> versions, String name) =>
      core.diagnosticsIssueUrl(
        repoUrl: repoUrl,
        versions: _entries(versions),
        reportName: name,
      );

  @override
  Future<List<String>> recentLog(int maxLines) =>
      core.diagnosticsRecentLog(maxLines: maxLines);

  @override
  List<CrashInfo> crashes() => [
    for (final c in core.diagnosticsCrashes())
      CrashInfo(
        name: c.name,
        kind: c.kind,
        headline: c.headline,
        time: DateTime.tryParse(c.time),
      ),
  ];

  @override
  String crashText(String name) => core.diagnosticsCrashText(name: name);

  @override
  Map<String, String> engineVersions() => {
    for (final e in core.diagnosticsEngineVersions()) e.key: e.value,
  };
}

/// Opens the core's diagnostics in `<data>/diagnostics` and routes Flutter
/// and Dart errors and `debugPrint` into them. Writes nothing until the
/// user opts in (Settings → Privacy).
Future<Diagnostics> startDiagnostics(String dataDir) async {
  try {
    await RustLibrary.ensureInitialized();
    // The core cuts `$HOME` / `%USERPROFILE%` itself, plus any
    // `/home/<user>`, `/Users/<user>`, `C:\Users\<user>` and the iOS
    // container id; the app's data folder under them goes with them.
    core.diagnosticsOpen(dir: '$dataDir/diagnostics', homeDirs: const []);
    final d = Diagnostics(CoreDiagnosticsBackend());
    installDiagnosticsHandlers(d);
    Diagnostics.instance = d;
    // PLAY-010: the player's «Сохранить отчёт» writes this report.
    PlayerScreen.statsExporter = exportPlaybackReport;
    // UI-017: libmpv and FFmpeg versions on the «Состояние» screen.
    Diagnostics.playerVersions = MediaKitEngine.engineVersions;
    return d;
  } catch (e) {
    debugPrint('Diagnostics unavailable: $e');
    return Diagnostics.instance;
  }
}
