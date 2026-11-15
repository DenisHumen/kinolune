// SPDX-License-Identifier: MPL-2.0
//
// The core half of diagnostics (CORE-008): pa_core's `diagnostics*`.

import 'package:flutter/foundation.dart';
import 'package:pa_core/pa_core.dart' as core;

import '../integration/library.dart' show RustLibrary;
import 'diagnostics.dart';

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
  bool recordCrash(String kind, String message, String stack) => core
      .diagnosticsRecordCrash(kind: kind, message: message, stack: stack);

  @override
  Future<String> export(
    String outPath,
    Map<String, String> versions,
    Map<String, String> config,
  ) async {
    final r = await core.diagnosticsExportReport(
      outPath: outPath,
      versions: _entries(versions),
      config: _entries(config),
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
    return d;
  } catch (e) {
    debugPrint('Diagnostics unavailable: $e');
    return Diagnostics.instance;
  }
}
