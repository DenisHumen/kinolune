// SPDX-License-Identifier: MPL-2.0
//
// CORE-008: Settings → Diagnostics — the opt-in, "Export diagnostic report"
// with the "Report saved" dialog and the prefilled GitHub issue, deleting
// what is kept; hidden without the core. With KINOLUNE_SCREENSHOTS_DIR set
// the frames are written to docs/screenshots/progress/core-008:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/core-008 \
//     flutter test test/diagnostics_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/diagnostics/diagnostics.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/services/external_links.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

class FakeDiagnosticsBackend implements DiagnosticsBackend {
  var state = const DiagnosticsState();
  final logs = <String>[];
  Map<String, String>? exportedConfig;
  Map<String, String>? exportedAttachments;
  Map<String, String>? exportedVersions;
  String? exportedPath;

  @override
  DiagnosticsState status() => state;

  @override
  DiagnosticsState setEnabled(bool on) => state = on
      ? const DiagnosticsState(
          enabled: true,
          logFiles: 2,
          logBytes: 340 * 1024,
          crashReports: 1,
        )
      : const DiagnosticsState();

  @override
  DiagnosticsState clear() => state = DiagnosticsState(enabled: state.enabled);

  @override
  void log(String level, String target, String message) {
    if (state.enabled) logs.add(message);
  }

  @override
  bool recordCrash(String kind, String message, String stack) => state.enabled;

  @override
  Future<String> export(
    String outPath,
    Map<String, String> versions,
    Map<String, String> config, {
    Map<String, String> attachments = const {},
  }) async {
    exportedPath = outPath;
    exportedVersions = versions;
    exportedConfig = config;
    exportedAttachments = attachments;
    return outPath;
  }

  @override
  String issueUrl(String repoUrl, Map<String, String> versions, String name) =>
      '$repoUrl/issues/new?template=bug_report.yml&logs=$name';

  @override
  Future<List<String>> recentLog(int maxLines) async => const [];

  @override
  List<CrashInfo> crashes() => const [];

  @override
  String crashText(String name) => '';

  @override
  Map<String, String> engineVersions() => const {};
}

Finder _page() => find
    .descendant(
      of: find.byType(Scaffold).first,
      matching: find.byType(Scrollable),
    )
    .first;

Future<void> _reveal(WidgetTester t, Finder f) async {
  if (f.evaluate().isEmpty) {
    await t.scrollUntilVisible(f, 200, scrollable: _page());
  }
  await t.ensureVisible(f);
  await settle(t, 4);
}

void main() {
  setUpAll(loadAppFonts);
  late FakeDiagnosticsBackend backend;
  setUp(() {
    backend = FakeDiagnosticsBackend();
    Diagnostics.instance = Diagnostics(backend);
    Diagnostics.reportDir = () async => '~/Downloads';
  });
  tearDown(() => Diagnostics.instance = Diagnostics.unavailable());

  testWidgets('hidden without the core', (t) async {
    Diagnostics.instance = Diagnostics.unavailable();
    setSurface(t, const Size(1280, 800));
    final services = await demoServices(advancedSettings: true);
    await t.pumpWidget(testApp(services, section: Section.settings));
    await settle(t, 8);
    await t.scrollUntilVisible(
      find.byKey(const ValueKey('settings-section-about')),
      300,
      scrollable: _page(),
    );
    expect(
      find.byKey(const ValueKey('settings-section-diagnostics')),
      findsNothing,
    );
  });

  testWidgets('opt in, export, open the issue, delete', (t) async {
    setSurface(t, const Size(1280, 800));
    final prefs = MemoryPreferences()
      ..values[Preferences.locale] = 'en'
      ..values[Preferences.searchHistory] = '["private query"]';
    final services = await demoServices(
      preferences: prefs,
      advancedSettings: true,
    );
    await t.pumpWidget(testApp(services, section: Section.settings));
    await settle(t, 8);
    final keep = find.byKey(const ValueKey('settings-diagnostics-keep'));
    await _reveal(t, keep);
    expect(find.text(en.settingsDiagnosticsExportOff), findsOneWidget);
    expect(
      find.byKey(const ValueKey('settings-diagnostics-clear')),
      findsNothing,
    );

    await t.tap(keep);
    await settle(t, 4);
    expect(backend.state.enabled, isTrue);
    expect(
      find.text(en.settingsDiagnosticsExportKept(en.detailSizeMb('0.3'), 1)),
      findsOneWidget,
    );

    final opened = <Uri>[];
    final prevOpen = openExternalLink;
    openExternalLink = (u) async {
      opened.add(u);
      return true;
    };
    addTearDown(() => openExternalLink = prevOpen);
    await t.tap(find.byKey(const ValueKey('settings-diagnostics-export')));
    await settle(t, 6);
    expect(find.byKey(const ValueKey('diagnostics-saved')), findsOneWidget);
    expect(
      backend.exportedPath,
      startsWith('~/Downloads/kinolune-diagnostics-'),
    );
    expect(backend.exportedPath, endsWith('.zip'));
    // Settings go in, private data does not.
    expect(backend.exportedConfig, containsPair(Preferences.locale, 'en'));
    expect(
      backend.exportedConfig!.keys,
      isNot(contains(Preferences.searchHistory)),
    );
    expect(backend.exportedVersions!['app'], isNotEmpty);
    await t.tap(find.byKey(const ValueKey('diagnostics-open-issue')));
    await settle(t, 4);
    expect(opened.single.path, '/DenisHumen/kinolune/issues/new');
    expect(find.byKey(const ValueKey('diagnostics-saved')), findsNothing);

    final clear = find.byKey(const ValueKey('settings-diagnostics-clear'));
    await _reveal(t, clear);
    await t.tap(clear);
    await settle(t, 4);
    expect(backend.state.hasData, isFalse);
    expect(find.text(en.settingsDiagnosticsCleared), findsOneWidget);

    // Off: nothing kept.
    await _reveal(t, keep);
    await t.tap(keep);
    await settle(t, 4);
    expect(backend.state.enabled, isFalse);
  });

  test('app log lines and crashes are dropped without the opt-in', () {
    final d = Diagnostics.instance;
    d.log('info', 'app', 'before');
    d.recordCrash('dart', StateError('x'), StackTrace.empty);
    expect(backend.logs, isEmpty);
  });

  test('report name', () {
    expect(
      diagnosticsReportName(DateTime.utc(2026, 11, 15, 9, 5, 7)),
      'kinolune-diagnostics-20261115-090507.zip',
    );
  });

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('phone', FormFactor.phone, const Size(390, 844)),
  ]) {
    testWidgets('screenshots on $name', (t) async {
      setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
      await Diagnostics.instance.setEnabled(true);
      final services = await demoServices(advancedSettings: true);
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t, 8);
      final section = find.byKey(
        const ValueKey('settings-section-diagnostics'),
      );
      await _reveal(t, section);
      await _reveal(
        t,
        find.byKey(const ValueKey('settings-diagnostics-export')),
      );
      expect(find.text(ru.settingsDiagnosticsKeep), findsOneWidget);
      await capture(t, '0${name == 'desktop' ? 1 : 3}-section-$name');
      await t.tap(find.byKey(const ValueKey('settings-diagnostics-export')));
      await settle(t, 6);
      expect(find.text(ru.settingsDiagnosticsSavedTitle), findsOneWidget);
      await capture(t, '0${name == 'desktop' ? 2 : 4}-saved-$name');
    });
  }
}
