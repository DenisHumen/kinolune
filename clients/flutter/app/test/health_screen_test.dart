// SPDX-License-Identifier: MPL-2.0
//
// UI-044: «Всё ли в порядке» — sources, storage space, sync and updates,
// each green, yellow, red or grey; one «Исправить» per problem (a NAS that
// does not let us in → its form); codes only on «Состояние». The rules of
// each line, the screen on desktop, phone and TV (remote focus), and the
// row in Settings → Diagnostics.
//
// With KINOLUNE_SCREENSHOTS_DIR set the frames are written there:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-044 \
//     flutter test test/health_screen_test.dart test/friendly_errors_test.dart

import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/build_info.dart';
import 'package:kinolune/diagnostics/diagnostics.dart';
import 'package:kinolune/diagnostics/health_screen.dart';
import 'package:kinolune/errors/friendly_errors.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_offline.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/features/updates/app_version.dart';
import 'package:kinolune/features/updates/releases.dart';
import 'package:kinolune/features/updates/update_controller.dart';
import 'package:kinolune/features/updates/update_platform.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/offline/offline_downloads.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/add_source_dialog.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:kinolune/sync/device_sync.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

const _gb = 1000 * 1000 * 1000;

/// The demo offline folder with [free] bytes free of 128 GB.
class _Space extends FakeOfflineBackend {
  _Space(this.free);

  final int free;

  @override
  Future<OfflineSettings> settings() async =>
      OfflineSettings(availableBytes: free, totalBytes: 128 * _gb);
}

class _Releases implements ReleaseSource {
  _Releases(this.list, {this.fail = false});

  final List<AppRelease> list;
  final bool fail;

  @override
  Future<List<AppRelease>> releases() async {
    if (fail) throw const UpdateNetworkException('network: offline');
    return list;
  }
}

AppRelease _release(String v) => AppRelease(
  tag: 'app-v$v',
  version: AppVersion.tryParse(v)!,
  prerelease: true,
  notes: '',
  page: Uri.parse('https://example.org/r/$v'),
  assets: const [],
);

UpdateController _updates(ReleaseSource source) => UpdateController(
  current: AppVersion.tryParse('0.1.0-alpha.12')!,
  target: const UpdateTarget(UpdateKind.androidApk, abis: ['x86_64']),
  source: source,
  verify: ({
    required File file,
    required String assetName,
    required List<int> sums,
    required String? signature,
  }) async {},
  cacheDir: Directory('${Directory.systemTemp.path}/ui044-updates'),
  preferences: MemoryPreferences(),
  clock: () => demoNow.toUtc(),
);

class _Diag implements DiagnosticsBackend {
  @override
  DiagnosticsState status() => const DiagnosticsState();
  @override
  DiagnosticsState setEnabled(bool on) => DiagnosticsState(enabled: on);
  @override
  DiagnosticsState clear() => const DiagnosticsState();
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
  }) async => outPath;
  @override
  String issueUrl(String repoUrl, Map<String, String> versions, String name) =>
      repoUrl;
  @override
  Future<List<String>> recentLog(int maxLines) async => const [];
  @override
  List<CrashInfo> crashes() => const [];
  @override
  String crashText(String name) => '';
  @override
  Map<String, String> engineVersions() => const {};
}

ManagedSource _source(String id, SourceStatusInfo status) => ManagedSource(
  id: id,
  type: SourceType.smb,
  name: 'NAS $id',
  address: 'smb://nas/media',
  status: status,
);

Future<(AppServices, FakeSourceManager, FakeDeviceSync)> _services({
  int free = 48 * _gb,
  UpdateController? updates,
}) async {
  final library = FakeLibrary.demo(scanStepDelay: Duration.zero);
  final manager = FakeSourceManager(library);
  final sync = FakeDeviceSync.demo();
  final s = AppServices(
    library: library,
    secureStore: MemorySecureStore(),
    preferences: MemoryPreferences(),
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    sourceManager: manager,
    sync: sync,
    offline: OfflineController(_Space(free)),
    updates: updates,
    clock: () => demoNow,
    buildInfo: const BuildInfo(appVersion: '0.1.0-alpha.12', flavor: 'full'),
  );
  await s.init();
  return (s, manager, sync);
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
  nav.push(MaterialPageRoute<void>(builder: (_) => const HealthScreen()));
  await settle(t, 10);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 3);
}

Finder _key(String k) => find.byKey(ValueKey(k));

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

void main() {
  setUpAll(loadAppFonts);
  setUp(() => Diagnostics.instance = Diagnostics(_Diag()));
  tearDown(() => Diagnostics.instance = Diagnostics.unavailable());

  group('rules', () {
    test('sources: none, all online, unchecked, a failing one', () {
      expect(sourcesHealth(ru, const []).level, HealthLevel.warning);
      expect(sourcesHealth(ru, const []).fix, ErrorFix.openSources);
      final ok = _source('1', const SourceStatusInfo(online: true));
      expect(sourcesHealth(ru, [ok]).level, HealthLevel.ok);
      expect(sourcesHealth(ru, [ok]).fix, isNull);
      final unchecked = _source('2', const SourceStatusInfo());
      expect(sourcesHealth(ru, [ok, unchecked]).level, HealthLevel.warning);
      final at = DateTime(2026, 11, 19, 10);
      final refused = _source(
        '3',
        SourceStatusInfo(
          online: false,
          checkedAt: at,
          lastError: 'authentication failed: 0xC000006D',
          lastErrorAt: at,
        ),
      );
      final h = sourcesHealth(ru, [ok, refused]);
      expect(h.level, HealthLevel.problem);
      expect(h.status, ru.healthSourcesFailing(1, 2));
      expect(h.text, ru.healthSourceError('NAS 3', ru.sourcesErrorAuth));
      expect(h.text!.contains('0xC'), isFalse);
      expect(h.fix, ErrorFix.editSource);
      expect(h.source?.id, '3');
    });

    test('storage: plenty, low, critical, unknown', () {
      const l = Locale('ru');
      HealthLevel at(int free) => storageHealth(
        ru,
        l,
        OfflineSettings(availableBytes: free, totalBytes: 128 * _gb),
      ).level;
      expect(at(48 * _gb), HealthLevel.ok);
      expect(at(5 * _gb), HealthLevel.warning);
      expect(at(1 * _gb), HealthLevel.problem);
      expect(at(300 * 1000 * 1000), HealthLevel.problem);
      expect(storageHealth(ru, l, null).level, HealthLevel.off);
      expect(
        storageHealth(
          ru,
          l,
          const OfflineSettings(availableBytes: 1, totalBytes: 2),
        ).fix,
        ErrorFix.freeSpace,
      );
    });

    test('sync: off, paired, fresh, stale, failed', () {
      final now = DateTime(2026, 11, 19, 12);
      HealthItem h({
        bool available = true,
        bool paired = false,
        FolderSyncState? folder,
      }) => syncHealth(
        ru,
        available: available,
        paired: paired,
        folder: folder,
        now: now,
      );
      expect(h(available: false).level, HealthLevel.off);
      expect(h().level, HealthLevel.off);
      expect(h().fix, ErrorFix.openSync);
      expect(h(paired: true).level, HealthLevel.ok);
      final fresh = FolderSyncState(
        sourceId: '1',
        lastSync: now.subtract(const Duration(hours: 1)),
      );
      expect(h(folder: fresh).level, HealthLevel.ok);
      final stale = FolderSyncState(
        sourceId: '1',
        lastSync: now.subtract(const Duration(days: 5)),
      );
      expect(h(folder: stale).level, HealthLevel.warning);
      final failed = FolderSyncState(
        sourceId: '1',
        lastSync: now,
        error: FolderSyncError.denied,
      );
      expect(h(folder: failed).level, HealthLevel.problem);
      expect(h(folder: failed).text, ru.devicesFolderErrorDenied);
    });

    test('updates: store build, never checked, new version, up to date, '
        'failed', () async {
      String problem() => 'x';
      expect(
        updatesHealth(ru, null, problemText: problem).level,
        HealthLevel.off,
      );
      final never = _updates(_Releases(const []));
      expect(
        updatesHealth(ru, never, problemText: problem).level,
        HealthLevel.off,
      );
      final fresh = _updates(_Releases([_release('0.1.0-alpha.13')]));
      await fresh.check();
      final a = updatesHealth(ru, fresh, problemText: problem);
      expect(a.level, HealthLevel.warning);
      expect(a.status, ru.healthUpdatesAvailable('0.1.0-alpha.13'));
      expect(a.fix, ErrorFix.openUpdates);
      final same = _updates(_Releases([_release('0.1.0-alpha.12')]));
      await same.check();
      expect(
        updatesHealth(ru, same, problemText: problem).level,
        HealthLevel.ok,
      );
      final broken = _updates(_Releases(const [], fail: true));
      await broken.check();
      final f = updatesHealth(ru, broken, problemText: problem);
      expect(f.level, HealthLevel.warning);
      expect(f.text, 'x');
    });

    test('the summary is the worst line; grey does not count', () {
      HealthItem i(HealthLevel l) => HealthItem(id: 'x', level: l, status: '');
      expect(
        overallHealth([i(HealthLevel.ok), i(HealthLevel.off)]),
        HealthLevel.ok,
      );
      expect(
        overallHealth([i(HealthLevel.warning), i(HealthLevel.ok)]),
        HealthLevel.warning,
      );
      expect(
        overallHealth([i(HealthLevel.warning), i(HealthLevel.problem)]),
        HealthLevel.problem,
      );
    });
  });

  testWidgets('all green: sources online, space, paired devices, latest '
      'version', (t) async {
    setSurface(t, const Size(1280, 900));
    final up = _updates(_Releases([_release('0.1.0-alpha.12')]));
    await up.check();
    final (s, _, _) = await _services(updates: up);
    s.locale.value = const Locale('en');
    await _open(t, s);
    expect(_key('health-summary-ok'), findsOneWidget);
    expect(find.text(en.healthSummaryOk), findsOneWidget);
    expect(_key('health-sources-ok'), findsOneWidget);
    expect(_key('health-storage-ok'), findsOneWidget);
    expect(_key('health-sync-ok'), findsOneWidget);
    expect(_key('health-updates-ok'), findsOneWidget);
    expect(find.text(en.healthLevelOk), findsNWidgets(4));
    expect(_key('health-sources-fix'), findsNothing);
    await _close(t);
  });

  testWidgets('a NAS that does not let us in: red, «Исправить» opens its '
      'form; low space is yellow', (t) async {
    setSurface(t, const Size(1280, 900));
    final (s, manager, _) = await _services(free: 3 * _gb);
    s.locale.value = const Locale('ru');
    manager.nextError = const SourceException(
      SourceErrorCode.authFailed,
      'authentication failed: NT_STATUS_LOGON_FAILURE 0xC000006D',
    );
    await _open(t, s);
    expect(_key('health-summary-problem'), findsOneWidget);
    expect(_key('health-sources-problem'), findsOneWidget);
    expect(_key('health-storage-warning'), findsOneWidget);
    expect(find.textContaining(ru.sourcesErrorAuth), findsOneWidget);
    // No codes on this screen.
    expect(find.textContaining('0xC000006D'), findsNothing);
    final fix = _key('health-sources-fix');
    expect(
      find.descendant(of: fix, matching: find.text(ru.errorFix)),
      findsOneWidget,
    );
    await t.tap(fix);
    await settle(t, 8);
    expect(find.byType(SourceForm), findsOneWidget);
    await _close(t);
  });

  testWidgets('«Проверить снова» checks the sources again', (t) async {
    setSurface(t, const Size(1280, 900));
    final (s, manager, _) = await _services();
    s.locale.value = const Locale('en');
    manager.nextError = const SourceException(
      SourceErrorCode.unreachable,
      'source unavailable: host down',
    );
    await _open(t, s);
    expect(_key('health-sources-problem'), findsOneWidget);
    // The NAS is back: the check finds every source online.
    await t.tap(_key('health-check'));
    await settle(t, 10);
    expect(_key('health-sources-ok'), findsOneWidget);
    await _close(t);
  });

  testWidgets('sync failure and «Подробно» to the Status screen', (t) async {
    setSurface(t, const Size(1280, 900));
    final (s, _, sync) = await _services();
    sync.folder = FolderSyncState(
      sourceId: s.managedSources.first.id,
      lastSync: demoNow,
      error: FolderSyncError.unavailable,
    );
    s.locale.value = const Locale('en');
    await _open(t, s);
    expect(_key('health-sync-problem'), findsOneWidget);
    expect(find.text(en.devicesFolderErrorUnavailable), findsOneWidget);
    await t.ensureVisible(_key('health-details'));
    await t.tap(_key('health-details'));
    await settle(t, 8);
    expect(_key('status-page'), findsOneWidget);
    await _close(t);
  });

  testWidgets('phone: one column without overflow', (t) async {
    setSurface(t, const Size(390, 844), dpr: 2);
    final (s, manager, _) = await _services(free: 300 * 1000 * 1000);
    s.locale.value = const Locale('ru');
    manager.nextError = const SourceException(
      SourceErrorCode.authFailed,
      'authentication failed',
    );
    await _open(t, s, formFactor: FormFactor.phone);
    expect(_key('health-storage-problem'), findsOneWidget);
    expect(t.takeException(), isNull);
    await _close(t);
  });

  testWidgets('TV: focus starts on «Проверить снова» and reaches «Исправить»', (
    t,
  ) async {
    setSurface(t, const Size(960, 540), dpr: 2);
    final (s, manager, _) = await _services();
    s.locale.value = const Locale('ru');
    manager.nextError = const SourceException(
      SourceErrorCode.authFailed,
      'authentication failed',
    );
    await _open(t, s, formFactor: FormFactor.tv);
    expect(_focusedIn('health-check'), isTrue);
    // Down from the summary reaches the red line's «Исправить».
    for (var i = 0; i < 4 && !_focusedIn('health-sources-fix'); i++) {
      await pressKey(t, LogicalKeyboardKey.arrowDown);
    }
    expect(_focusedIn('health-sources-fix'), isTrue);
    expect(t.takeException(), isNull);
    await _close(t);
  });

  testWidgets('Settings → Diagnostics opens the screen', (t) async {
    setSurface(t, const Size(1280, 800));
    final (s, _, _) = await _services();
    await t.pumpWidget(testApp(s, section: Section.settings));
    await settle(t, 8);
    final tile = _key('settings-diagnostics-health');
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
    await settle(t, 10);
    expect(_key('health-page'), findsOneWidget);
    await _close(t);
  });

  // docs/screenshots/progress/ui-044 (KINOLUNE_SCREENSHOTS_DIR), in
  // Russian: a refused NAS password, little space, a new version.
  group('screenshots', () {
    for (final (name, ff, size, dpr) in [
      ('phone', FormFactor.phone, const Size(390, 844), 2.0),
      ('desktop', FormFactor.desktop, const Size(1280, 800), 1.0),
      ('tv', FormFactor.tv, const Size(960, 540), 2.0),
    ]) {
      testWidgets('health on $name', (t) async {
        final up = _updates(_Releases([_release('0.1.0-alpha.13')]));
        await up.check();
        final (s, manager, _) = await _services(free: 3 * _gb, updates: up);
        s.locale.value = const Locale('ru');
        manager.nextError = const SourceException(
          SourceErrorCode.authFailed,
          'authentication failed',
        );
        setSurface(t, size, dpr: dpr);
        await _open(t, s, formFactor: ff);
        expect(t.takeException(), isNull);
        await capture(t, '01-health-problem-$name');
        await t.tap(_key('health-check'));
        await settle(t, 10);
        await capture(t, '02-health-after-check-$name');
        await _close(t);
      });
    }
  });
}
