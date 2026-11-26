// SPDX-License-Identifier: MPL-2.0
//
// LIB-028 live check: two real apps (Rust core, real LAN sync) side by
// side under sway. The `source` drops a big file into the library on an
// SMB share; the `watcher` (another «computer» of the family) shows the
// Movies page, where the grey card appears and its bar moves. Screenshots
// of both windows are taken from outside (grim); this driver logs
// machine-readable `LIVE` lines with timestamps for the report.
//
// Not for CI. Environment:
//   PA_LIVE_ROLE      source | watcher
//   PA_LIVE_DIR       a folder both instances share (the pairing code and
//                     the steps go through it)
//   PA_LIVE_FILE      source: the local file to drop
//   PA_LIVE_SMB       source: smb://host/share of the library
//   PA_LIVE_SMB_USER, PA_LIVE_SMB_PASS
// The app's data dirs (XDG_*, HOME) must be empty and differ per instance.

import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:kinolune/files/library_upload.dart';
import 'package:kinolune/library/live_uploads.dart';
import 'package:kinolune/main.dart' as app;
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';

final _env = Platform.environment;
final _role = _env['PA_LIVE_ROLE'] ?? 'source';
final _dir = Directory(_env['PA_LIVE_DIR'] ?? '/tmp/live');

int _now() => DateTime.now().millisecondsSinceEpoch;

void _log(String line) {
  final text = 'LIVE ${_now()} $_role $line';
  // ignore: avoid_print
  print(text);
  File(
    '${_dir.path}/$_role.log',
  ).writeAsStringSync('$text\n', mode: FileMode.append);
}

File _step(String name) => File('${_dir.path}/$name');

Future<String> _waitFor(String name, {Duration max = const Duration(minutes: 10)}) async {
  final end = DateTime.now().add(max);
  while (DateTime.now().isBefore(end)) {
    final f = _step(name);
    if (f.existsSync()) return f.readAsStringSync();
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
  throw StateError('no $name');
}

/// CPU time of this process (user + system), ms.
int _cpuMs() {
  final stat = File('/proc/self/stat').readAsStringSync();
  final f = stat.substring(stat.lastIndexOf(')') + 2).split(' ');
  return (int.parse(f[11]) + int.parse(f[12])) * 10;
}

Future<void> _pump(WidgetTester t, Duration d) async {
  final end = DateTime.now().add(d);
  while (DateTime.now().isBefore(end)) {
    await t.pump(const Duration(milliseconds: 50));
    await Future<void>.delayed(const Duration(milliseconds: 50));
  }
}

Future<AppServices> _start(WidgetTester t) async {
  _dir.createSync(recursive: true);
  unawaited(app.main(const []));
  AppServices? s;
  for (var i = 0; i < 600 && s == null; i++) {
    await _pump(t, const Duration(milliseconds: 100));
    final apps = find.byType(MaterialApp);
    if (apps.evaluate().isNotEmpty) {
      final found = Services.read(t.element(apps.first));
      if (found.loaded) s = found;
    }
  }
  if (s == null) throw StateError('the app did not start');
  if (s.onboardingPending) await s.completeOnboarding();
  await _pump(t, const Duration(seconds: 2));
  return s;
}

Future<void> _show(WidgetTester t, Section section) async {
  final shell = find.byType(AppShell);
  for (var i = 0; i < 100 && shell.evaluate().isEmpty; i++) {
    await _pump(t, const Duration(milliseconds: 100));
  }
  t.state<AppShellState>(shell).select(section);
  await _pump(t, const Duration(seconds: 1));
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('LIB-028 live: $_role', (t) async {
    final s = await _start(t);
    _log('started device=${s.liveUploads.link.deviceId}');
    if (_role == 'source') {
      await _source(t, s);
    } else {
      await _watcher(t, s);
    }
  }, timeout: const Timeout(Duration(minutes: 30)));
}

Future<void> _source(WidgetTester t, AppServices s) async {
  final made = await s.createSource(
    SourceDraft(
      type: SourceType.smb,
      name: 'NAS',
      address: _env['PA_LIVE_SMB']!,
      username: _env['PA_LIVE_SMB_USER'],
      password: _env['PA_LIVE_SMB_PASS'],
    ),
  );
  _log('source ${made.id}');
  final invite = await s.sync.invite();
  _step('invite').writeAsStringSync(invite.qr);
  await _waitFor('joined');
  await _pump(t, const Duration(seconds: 3));
  _log('paired devices=${s.sync.devices().length}');
  await _show(t, Section.movies);
  await _waitFor('watching');
  await _pump(t, const Duration(seconds: 2));

  final file = _env['PA_LIVE_FILE']!;
  _step('drop-ms').writeAsStringSync('${_now()}');
  _log('drop $file ${File(file).lengthSync()}');
  final zone = t.state<LibraryDropZoneState>(find.byType(LibraryDropZone));
  unawaited(zone.drop([file]));
  // Copying: keep the frames coming (the watcher takes the screenshots).
  LiveUpload? card;
  for (var i = 0; i < 20 * 60 * 5; i++) {
    await _pump(t, const Duration(milliseconds: 200));
    card ??= s.liveUploads.uploads.where((u) => u.local).firstOrNull;
    final jobs = s.filesControl?.libraryUploads ?? const [];
    if (jobs.isNotEmpty && jobs.every((j) => j.resultPath != null)) break;
  }
  _step('done-ms').writeAsStringSync('${_now()}');
  _log('done ${card?.id}');
  await _waitFor('quit', max: const Duration(minutes: 5));
}

Future<void> _watcher(WidgetTester t, AppServices s) async {
  final code = await _waitFor('invite');
  await s.sync.join(code);
  _step('joined').writeAsStringSync('${_now()}');
  await _pump(t, const Duration(seconds: 3));
  _log('joined devices=${s.sync.devices().length}');
  await _show(t, Section.movies);
  _step('watching').writeAsStringSync('${_now()}');

  final drop = int.parse(await _waitFor('drop-ms'));
  LiveUpload? card;
  while (card == null) {
    await t.pump(const Duration(milliseconds: 10));
    await Future<void>.delayed(const Duration(milliseconds: 10));
    card = s.liveUploads.uploads.where((u) => !u.local).firstOrNull;
  }
  final seen = _now();
  _log('card ${card.id} after_ms=${seen - drop} title=${card.info.value.guess.title}');
  _step('card-ms').writeAsStringSync('$seen');
  var ticks = 0;
  var last = card.progress.value;
  final tickTimes = <int>[];
  card.progress.addListener(() {
    final p = card!.progress.value;
    if (p.done != last.done) {
      ticks++;
      tickTimes.add(_now());
    }
    if (p.state != last.state) _log('state ${p.state.name} at ${p.percent}%');
    last = p;
  });
  final cpu0 = _cpuMs(), t0 = _now();
  var reported = t0;
  while (!_step('done-ms').existsSync()) {
    await _pump(t, const Duration(milliseconds: 200));
    if (_now() - reported >= 5000) {
      reported = _now();
      final p = card.progress.value;
      _log(
        'progress ${p.percent}% done=${p.done} rate=${p.rate} ticks=$ticks '
        'cpu_ms=${_cpuMs() - cpu0} wall_ms=${_now() - t0}',
      );
      _step('shot-now').writeAsStringSync('${p.percent}');
    }
  }
  final doneAt = int.parse(_step('done-ms').readAsStringSync());
  for (var i = 0; i < 100 && card.progress.value.state != LiveUploadState.done; i++) {
    await _pump(t, const Duration(milliseconds: 50));
  }
  _log('done_seen after_ms=${_now() - doneAt} state=${card.progress.value.state.name}');
  final span = tickTimes.isEmpty ? 0 : tickTimes.last - tickTimes.first;
  _log(
    'summary ticks=$ticks span_ms=$span hz=${span == 0 ? 0 : ((ticks - 1) * 1000 / span).toStringAsFixed(2)} '
    'cpu_ms=${_cpuMs() - cpu0} wall_ms=${_now() - t0}',
  );
  await _pump(t, const Duration(seconds: 3));
  _step('quit').writeAsStringSync('${_now()}');
}
