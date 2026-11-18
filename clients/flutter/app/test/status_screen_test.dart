// SPDX-License-Identifier: MPL-2.0
//
// UI-017: the «Состояние» screen — the report in one click (with the
// sources' summary and the engine versions, no names or addresses), the
// versions of the app, core and engines, the speed test of a source, the
// crash reports (opt-in, copied or saved by hand), the log with the errors
// filter and the opt-in; phone, desktop and TV layouts (remote focus); the
// row in Settings → Diagnostics.
//
// With KINOLUNE_SCREENSHOTS_DIR set the frames are written there:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-017 \
//     flutter test test/status_screen_test.dart --plain-name screenshots

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/build_info.dart';
import 'package:kinolune/diagnostics/diagnostics.dart';
import 'package:kinolune/diagnostics/status_screen.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

class _Backend implements DiagnosticsBackend {
  var state = const DiagnosticsState();
  List<String> logLines = const [];
  List<CrashInfo> crashList = const [];
  final texts = <String, String>{};
  Map<String, String>? exportedVersions;
  Map<String, String>? exportedAttachments;
  var exports = 0;

  @override
  DiagnosticsState status() => state;

  @override
  DiagnosticsState setEnabled(bool on) => state = DiagnosticsState(
    enabled: on,
    logFiles: on ? 1 : 0,
    logBytes: on ? 4096 : 0,
    crashReports: on ? crashList.length : 0,
  );

  @override
  DiagnosticsState clear() {
    logLines = const [];
    crashList = const [];
    return state = DiagnosticsState(enabled: state.enabled);
  }

  @override
  void log(String level, String target, String message) {}

  @override
  bool recordCrash(String kind, String message, String stack) => false;

  @override
  Future<String> export(
    String outPath,
    Map<String, String> versions,
    Map<String, String> config, {
    Map<String, String> attachments = const {},
  }) async {
    exports++;
    exportedVersions = versions;
    exportedAttachments = attachments;
    return outPath;
  }

  @override
  String issueUrl(String repoUrl, Map<String, String> versions, String name) =>
      '$repoUrl/issues/new?logs=$name';

  @override
  Future<List<String>> recentLog(int maxLines) async =>
      state.enabled ? logLines : const [];

  @override
  List<CrashInfo> crashes() => state.enabled ? crashList : const [];

  @override
  String crashText(String name) =>
      texts[name] ?? (throw StateError('not_found'));

  @override
  Map<String, String> engineVersions() => const {
    'sqlite': '3.50.4',
    'core_target': 'linux-x86_64',
  };
}

const _log = [
  '2026-11-18T19:58:02Z  INFO app: Kinolune 0.1.0 started',
  '2026-11-18T19:58:03Z  INFO pa_sources: source 2 online (smb)',
  '2026-11-18T19:58:09Z  WARN pa_vfs: slow read from smb://<redacted>@nas/media',
  '2026-11-18T19:59:41Z ERROR pa_metadata: TMDB: 429 Too Many Requests',
  '2026-11-18T20:00:12Z  INFO pa_library: scan finished: 1 204 files',
];

final _crash = CrashInfo(
  name: 'crash-20261118-195512-123-dart.txt',
  kind: 'dart',
  headline: 'Null check operator used on a null value',
  time: DateTime.utc(2026, 11, 18, 19, 55, 12),
);

const _crashText =
    'Kinolune crash report\nkind: dart\ntime: 2026-11-18T19:55:12Z\n'
    'os: linux x86_64\n\nmessage:\nNull check operator used on a null value'
    '\n\nstack:\n#0 main (package:kinolune/main.dart:12)\n';

Future<AppServices> _services({MemoryPreferences? prefs}) async {
  final library = FakeLibrary.demo(scanStepDelay: Duration.zero);
  final s = AppServices(
    library: library,
    secureStore: MemorySecureStore(),
    preferences: prefs ?? MemoryPreferences(),
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    sourceManager: FakeSourceManager(library),
    clock: () => demoNow,
    buildInfo: const BuildInfo(
      appVersion: '0.1.0-alpha.12',
      flavor: 'full',
      coreVersion: '0.1.0',
      coreFlavor: 'full',
    ),
  );
  await s.init();
  return s;
}

Future<void> _open(
  WidgetTester t,
  AppServices s, {
  FormFactor formFactor = FormFactor.desktop,
}) async {
  await t.pumpWidget(testApp(s, formFactor: formFactor));
  await settle(t, 4);
  final nav = t.state<NavigatorState>(find.byType(Navigator).first);
  // Not awaited: the route stays open.
  // ignore: unawaited_futures
  nav.push(MaterialPageRoute<void>(builder: (_) => const StatusScreen()));
  await settle(t, 6);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

Finder _key(String k) => find.byKey(ValueKey(k));

/// Whether the focused node sits inside the widget keyed [k].
bool _focusedIn(String k) {
  final ctx = FocusManager.instance.primaryFocus?.context;
  if (ctx == null) return false;
  if (ctx.widget.key == ValueKey(k)) return true;
  var found = false;
  ctx.visitAncestorElements((e) {
    found = e.widget.key == ValueKey(k);
    return !found;
  });
  return found;
}

Future<void> _reveal(
  WidgetTester t,
  Finder f, {
  String list = 'status-main',
}) async {
  if (f.evaluate().isEmpty) {
    await t.dragUntilVisible(f, _key(list), const Offset(0, -200));
  }
  await t.ensureVisible(f);
  await settle(t, 3);
}

void main() {
  setUpAll(loadAppFonts);
  late _Backend backend;
  setUp(() {
    backend = _Backend();
    Diagnostics.instance = Diagnostics(backend);
    Diagnostics.reportDir = () async => '~/Downloads';
    Diagnostics.playerVersions = () async => const {
      'mpv': '0.40.0',
      'ffmpeg': 'n7.1.1',
    };
  });
  tearDown(() {
    Diagnostics.instance = Diagnostics.unavailable();
    Diagnostics.playerVersions = () async => const {};
  });

  test('the sources summary has no names, addresses or users', () {
    final s = ManagedSource(
      id: '2',
      type: SourceType.smb,
      name: 'Alice NAS',
      address: 'smb://nas.home/media',
      username: 'alice',
      hasPassword: true,
      status: SourceStatusInfo(
        online: true,
        speedBytesPerSec: 31000000,
        latencyMs: 4,
        speedMeasuredAt: DateTime.utc(2026, 11, 18, 20),
      ),
    );
    final text = statusSourcesSummary([s]);
    expect(
      text,
      'smb · online · 31000000 B/s · 4 ms · 2026-11-18T20:00:00.000Z',
    );
    for (final leak in ['Alice', 'alice', 'nas.home', 'media']) {
      expect(text.contains(leak), isFalse, reason: leak);
    }
  });

  test('errors filter keeps ERROR and WARN lines', () {
    expect(_log.where(statusIsError).length, 2);
  });

  testWidgets('report in one click with versions and sources', (t) async {
    setSurface(t, const Size(1280, 800));
    final s = await _services();
    s.locale.value = const Locale('en');
    await _open(t, s);
    expect(_key('status-wide'), findsOneWidget);
    expect(find.text(en.statusVersionMpv), findsOneWidget);
    expect(find.text('0.40.0'), findsOneWidget);
    expect(find.text('3.50.4'), findsOneWidget);
    await t.tap(_key('status-export-button'));
    await settle(t, 6);
    expect(backend.exports, 1);
    expect(_key('diagnostics-saved'), findsOneWidget);
    expect(backend.exportedVersions!['mpv'], '0.40.0');
    expect(backend.exportedVersions!['ffmpeg'], 'n7.1.1');
    expect(backend.exportedVersions!['sqlite'], '3.50.4');
    final sources = backend.exportedAttachments!['sources.txt']!;
    expect(sources, isNotEmpty);
    for (final m in s.managedSources) {
      expect(sources.contains(m.name), isFalse, reason: m.name);
    }
    await _close(t);
  });

  testWidgets('opt in shows the log, the filter and crash reports', (t) async {
    setSurface(t, const Size(1280, 800));
    backend
      ..logLines = _log
      ..crashList = [_crash]
      ..texts[_crash.name] = _crashText;
    final s = await _services();
    s.locale.value = const Locale('en');
    await _open(t, s);
    expect(find.text(en.statusCrashOff), findsOneWidget);
    expect(_key('status-log-view'), findsNothing);

    await _reveal(t, _key('status-log-keep'));
    await t.tap(_key('status-log-keep'));
    await settle(t, 6);
    expect(backend.state.enabled, isTrue);
    expect(_key('status-log-view'), findsOneWidget);
    expect(find.text(_log[0]), findsOneWidget);
    expect(find.text(en.statusLogLines(5)), findsOneWidget);

    await _reveal(t, _key('status-log-errors'));
    await t.tap(_key('status-log-errors'));
    await settle(t, 3);
    expect(find.text(_log[0]), findsNothing);
    expect(find.text(_log[3]), findsOneWidget);

    await _reveal(t, _key('status-log-copy'));
    String? copied;
    t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.setData') {
          copied = (call.arguments as Map)['text'] as String;
        }
        return null;
      },
    );
    await t.tap(_key('status-log-copy'));
    await settle(t, 3);
    expect(copied, '${_log[2]}\n${_log[3]}');

    final row = _key('status-crash-${_crash.name}');
    await _reveal(t, row);
    await t.tap(row);
    await settle(t, 4);
    expect(_key('status-crash-dialog'), findsOneWidget);
    expect(find.textContaining('#0 main'), findsOneWidget);
    await t.tap(_key('status-crash-copy'));
    await settle(t, 3);
    expect(copied, _crashText);
    await t.tap(_key('status-crash-report'));
    await settle(t, 6);
    expect(backend.exports, 1);
    expect(_key('diagnostics-saved'), findsOneWidget);
    t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      null,
    );
    await _close(t);
  });

  testWidgets('speed test of a source', (t) async {
    setSurface(t, const Size(1280, 800));
    final s = await _services();
    s.locale.value = const Locale('en');
    await _open(t, s);
    final first = s.managedSources.first;
    final row = _key('status-speed-${first.id}');
    await _reveal(t, row, list: 'status-side');
    expect(find.textContaining(en.statusSpeedNever), findsWidgets);
    await t.tap(row);
    await settle(t, 6);
    expect(find.text(en.sourcesSpeedResultTitle(first.name)), findsOneWidget);
    await t.tap(find.text(en.sourcesClose));
    await settle(t, 4);
    expect(
      s.managedSource(first.id)!.status.speedBytesPerSec,
      32 * 1000 * 1000,
    );
    await _close(t);
  });

  testWidgets('phone: one column, report first', (t) async {
    setSurface(t, const Size(390, 844), dpr: 2);
    final s = await _services();
    await _open(t, s, formFactor: FormFactor.phone);
    expect(_key('status-narrow'), findsOneWidget);
    expect(_key('status-wide'), findsNothing);
    expect(_key('status-export-button'), findsOneWidget);
    expect(t.takeException(), isNull);
    await _close(t);
  });

  testWidgets('TV: focus starts on the report and moves with the remote', (
    t,
  ) async {
    setSurface(t, const Size(960, 540), dpr: 2);
    backend
      ..logLines = _log
      ..state = const DiagnosticsState(enabled: true, logFiles: 1);
    final s = await _services();
    await _open(t, s, formFactor: FormFactor.tv);
    expect(_key('status-wide'), findsOneWidget);
    expect(_focusedIn('status-export-button'), isTrue);
    await pressKey(t, LogicalKeyboardKey.arrowDown);
    expect(FocusManager.instance.primaryFocus, isNot(isNull));
    expect(t.takeException(), isNull);
    await _close(t);
  });

  testWidgets('without the core the report button is off', (t) async {
    Diagnostics.instance = Diagnostics.unavailable();
    setSurface(t, const Size(1280, 800));
    final s = await _services();
    s.locale.value = const Locale('en');
    await _open(t, s);
    expect(find.text(en.statusUnavailable), findsOneWidget);
    await t.tap(_key('status-export-button'));
    await settle(t, 3);
    expect(_key('diagnostics-saved'), findsNothing);
    await _close(t);
  });

  testWidgets('Settings → Diagnostics opens the screen', (t) async {
    setSurface(t, const Size(1280, 800));
    final s = await _services();
    await t.pumpWidget(testApp(s, section: Section.settings));
    await settle(t, 8);
    final tile = _key('settings-diagnostics-status');
    await t.scrollUntilVisible(
      tile,
      300,
      scrollable: find
          .descendant(
            of: find.byType(Scaffold).first,
            matching: find.byType(Scrollable),
          )
          .first,
    );
    await settle(t, 3);
    await t.tap(tile);
    await settle(t, 6);
    expect(_key('status-page'), findsOneWidget);
    await _close(t);
  });

  // docs/screenshots/progress/ui-017 (KINOLUNE_SCREENSHOTS_DIR), in
  // Russian, with the log on: phone, desktop, TV and the crash report.
  group('screenshots', () {
    for (final (name, ff, size, dpr) in [
      ('phone', FormFactor.phone, const Size(390, 844), 2.0),
      ('desktop', FormFactor.desktop, const Size(1280, 800), 1.0),
      ('tv', FormFactor.tv, const Size(960, 540), 2.0),
    ]) {
      testWidgets('status on $name', (t) async {
        backend
          ..logLines = _log
          ..crashList = [_crash]
          ..texts[_crash.name] = _crashText
          ..state = const DiagnosticsState(
            enabled: true,
            logFiles: 1,
            logBytes: 4096,
            crashReports: 1,
          );
        final s = await _services();
        s.locale.value = const Locale('ru');
        await s.speedTest(s.managedSources.first.id);
        setSurface(t, size, dpr: dpr);
        await _open(t, s, formFactor: ff);
        expect(t.takeException(), isNull);
        await capture(t, '01-status-$name');
        if (name == 'desktop') {
          await t.tap(_key('status-crash-${_crash.name}'));
          await settle(t, 4);
          await capture(t, '02-crash-report-desktop');
        }
        if (name == 'phone') {
          await _reveal(t, _key('status-log-view'), list: 'status-narrow');
          await capture(t, '02-status-phone-log');
        }
        await _close(t);
      });
    }
  });
}
