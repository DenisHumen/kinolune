// SPDX-License-Identifier: MPL-2.0
// CORE-008 through the Dart bindings: nothing is written before the
// opt-in; with it, app lines and crashes reach the report zip without user
// paths or tokens.
import 'dart:io';

import 'package:pa_core/pa_core.dart';
import 'package:test/test.dart';

import 'native_library.dart';

List<File> _files(Directory d) => d.existsSync()
    ? d.listSync(recursive: true).whereType<File>().toList()
    : <File>[];

void main() {
  setUpAll(() async {
    await RustLib.init(externalLibrary: openBuiltCoreLibrary());
  });

  tearDownAll(RustLib.dispose);

  test('opt-in, redaction and the report', () async {
    final tmp = await Directory.systemTemp.createTemp('pa-diag-');
    addTearDown(() => tmp.delete(recursive: true));
    final dir = Directory('${tmp.path}/diagnostics');
    final status = diagnosticsOpen(dir: dir.path, homeDirs: ['/home/alice']);
    expect(status.enabled, isFalse);

    const secret =
        'open /home/alice/Videos/a.mkv?api_key=abc123secret '
        'Authorization: OAuth q8xzb3twitchtoken5k2m9';
    diagnosticsLog(level: 'info', target: 'test', message: secret);
    expect(
      diagnosticsRecordCrash(kind: 'dart', message: secret, stack: ''),
      isFalse,
    );
    expect(_files(tmp), isEmpty);

    final on = diagnosticsSetEnabled(enabled: true);
    expect(on.enabled, isTrue);
    diagnosticsLog(level: 'info', target: 'test', message: secret);
    expect(
      diagnosticsRecordCrash(kind: 'dart', message: secret, stack: ''),
      isTrue,
    );
    final kept = diagnosticsStatus()!;
    expect(kept.logFiles, 1);
    expect(kept.crashReports, 1);

    final out = '${tmp.path}/out/report.zip';
    final report = await diagnosticsExportReport(
      outPath: out,
      versions: [const DiagnosticsEntry(key: 'app', value: '0.1.0 (store)')],
      config: [const DiagnosticsEntry(key: 'tmdb_api_key', value: 'k3y')],
    );
    expect(report.path, out);
    final raw = String.fromCharCodes(File(out).readAsBytesSync());
    for (final s in [
      'alice',
      'abc123secret',
      'q8xzb3twitchtoken5k2m9',
      'k3y',
    ]) {
      expect(raw.contains(s), isFalse, reason: 'report leaks $s');
    }
    expect(raw, contains('open ~/Videos/a.mkv'));

    final url = diagnosticsIssueUrl(
      repoUrl: 'https://github.com/DenisHumen/project_alpha',
      versions: const [DiagnosticsEntry(key: 'app', value: '0.1.0')],
      reportName: 'report.zip',
    );
    expect(
      url,
      startsWith('https://github.com/DenisHumen/project_alpha/issues/new?'),
    );
    expect(url.contains('abc123secret'), isFalse);

    final off = diagnosticsSetEnabled(enabled: false);
    expect(off.enabled, isFalse);
    expect(dir.existsSync(), isFalse);
  });

  test('redact', () {
    expect(
      diagnosticsRedact(text: 'smb://bob:pw@nas/a'),
      'smb://<redacted>@nas/a',
    );
  });
}
