// SPDX-License-Identifier: MPL-2.0
//
// CORE-008: Settings → Diagnostics — the opt-in, "Export diagnostic report"
// with the "Report saved" dialog and the prefilled GitHub issue, deleting
// what is kept; hidden without the core. With KINOLUNE_SCREENSHOTS_DIR set
// the frames are written to docs/screenshots/progress/core-008:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/core-008 \
//     flutter test test/diagnostics_test.dart

import 'dart:async' show unawaited;
import 'dart:io' show Platform;

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/build_info.dart';
import 'package:kinolune/diagnostics/diagnostics.dart';
import 'package:kinolune/diagnostics/diagnostics_section.dart';
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
      ..values[Preferences.searchHistory] = '["private query"]'
      ..values[Preferences.localProfile] = '{"name":"Alice"}'
      ..values[Preferences.twitchFavorites] = '["somestreamer"]'
      ..values['twitch_access_token'] = 'q8xzb3twitchoauthtoken5k2m9'
      ..values['profiles_pin_alice'] = '1234';
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
    final revealed = <DiagnosticsExport>[];
    final prevReveal = Diagnostics.revealReport;
    Diagnostics.revealReport = (r) async {
      revealed.add(r);
      return true;
    };
    addTearDown(() => Diagnostics.revealReport = prevReveal);
    await t.tap(find.byKey(const ValueKey('settings-diagnostics-export')));
    await settle(t, 6);
    expect(find.byKey(const ValueKey('diagnostics-saved')), findsOneWidget);
    expect(
      backend.exportedPath,
      startsWith('~/Downloads/kinolune-diagnostics-'),
    );
    expect(backend.exportedPath, endsWith('.zip'));
    // BUG-024: every setting goes in (unchanged ones as "(default)"),
    // private data and secrets do not.
    final config = backend.exportedConfig!;
    expect(config, containsPair(Preferences.locale, 'en'));
    expect(
      config,
      containsPair(Preferences.hideSpoilers, diagnosticsConfigDefault),
    );
    expect(config.keys, unorderedEquals(diagnosticsConfigKeys));
    for (final k in [
      Preferences.searchHistory,
      Preferences.localProfile,
      Preferences.twitchFavorites,
      'twitch_access_token',
      'profiles_pin_alice',
    ]) {
      expect(config.keys, isNot(contains(k)));
    }
    final text = config.entries.map((e) => '${e.key}: ${e.value}').join('\n');
    for (final secret in [
      'q8xzb3twitchoauthtoken5k2m9',
      '1234',
      'Alice',
      'private query',
      'somestreamer',
    ]) {
      expect(text, isNot(contains(secret)));
    }
    // BUG-024: the full build version, the same as About.
    expect(
      backend.exportedVersions!['app'],
      '${services.buildInfo.appVersion} (${services.buildInfo.flavor})',
    );
    // BUG-024: the dialog says where the file is and opens the folder.
    final name = backend.exportedPath!.split('/').last;
    expect(
      t
          .widget<SelectableText>(
            find.byKey(const ValueKey('diagnostics-saved-folder')),
          )
          .data,
      '~/Downloads',
    );
    expect(
      t
          .widget<SelectableText>(
            find.byKey(const ValueKey('diagnostics-saved-file')),
          )
          .data,
      name,
    );
    expect(find.text(en.settingsDiagnosticsOpenFolder), findsOneWidget);
    await t.tap(find.byKey(const ValueKey('diagnostics-open-folder')));
    await settle(t, 4);
    expect(revealed.single.name, name);
    expect(revealed.single.path, backend.exportedPath);
    expect(find.byKey(const ValueKey('diagnostics-saved')), findsOneWidget);
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

  test('report name is in local time (BUG-024)', () {
    expect(
      diagnosticsReportName(DateTime(2026, 11, 21, 12, 38, 37)),
      'kinolune-diagnostics-20261121-123837.zip',
    );
    // A UTC moment is named by the device's clock, not by UTC.
    final utc = DateTime.utc(2026, 11, 21, 10, 38, 37);
    final local = utc.toLocal();
    String two(int v) => v.toString().padLeft(2, '0');
    expect(
      diagnosticsReportName(utc),
      'kinolune-diagnostics-${local.year}${two(local.month)}'
      '${two(local.day)}-${two(local.hour)}${two(local.minute)}'
      '${two(local.second)}.zip',
    );
  });

  group('OS name (BUG-024)', () {
    test('Windows 11 from the build number', () {
      expect(
        friendlyOsVersion('windows', '"Windows 10 Pro" 10.0 (Build 26200)'),
        'Windows 11 Pro 10.0 (Build 26200)',
      );
      expect(
        friendlyOsVersion('windows', '"Windows 10 Home" 10.0 (Build 22000)'),
        'Windows 11 Home 10.0 (Build 22000)',
      );
      expect(
        friendlyOsVersion('windows', '"Windows 11 Pro" 10.0 (Build 22631)'),
        'Windows 11 Pro 10.0 (Build 22631)',
      );
      expect(
        friendlyOsVersion(
          'windows',
          '"Windows Server 2022 Datacenter" 10.0 (Build 20348)',
        ),
        'Windows Server 2022 Datacenter 10.0 (Build 20348)',
      );
      expect(
        friendlyOsVersion(
          'windows',
          '"Windows Server 2025 Standard" 10.0 (Build 26100)',
        ),
        'Windows Server 2025 Standard 10.0 (Build 26100)',
      );
    });

    test('Windows 10 stays Windows 10', () {
      expect(
        friendlyOsVersion('windows', '"Windows 10 Pro" 10.0 (Build 19045)'),
        'Windows 10 Pro 10.0 (Build 19045)',
      );
      expect(
        friendlyOsVersion('windows', '"Windows 10 Pro" 10.0 (Build 21999)'),
        'Windows 10 Pro 10.0 (Build 21999)',
      );
      expect(
        friendlyOsVersion('windows', '10.0 (Build 26100)'),
        'Windows 11 10.0 (Build 26100)',
      );
      expect(friendlyOsVersion('windows', ''), 'Windows');
    });

    test('macOS and Linux', () {
      expect(
        friendlyOsVersion('macos', 'Version 15.1 (Build 24B83)'),
        'macOS 15.1 (Build 24B83)',
      );
      expect(
        friendlyOsVersion(
          'linux',
          '6.8.0-49-generic #49-Ubuntu SMP PREEMPT_DYNAMIC',
          linuxOsRelease:
              'NAME="Ubuntu"\nVERSION_ID="24.04"\n'
              'PRETTY_NAME="Ubuntu 24.04.1 LTS"\nID=ubuntu\n',
        ),
        'Ubuntu 24.04.1 LTS (kernel 6.8.0-49-generic)',
      );
      expect(
        friendlyOsVersion(
          'linux',
          '6.11.4-301.fc41.x86_64 #1 SMP',
          linuxOsRelease: "NAME='Fedora Linux'\nVERSION_ID=41\n",
        ),
        'Fedora Linux 41 (kernel 6.11.4-301.fc41.x86_64)',
      );
      expect(
        friendlyOsVersion('linux', '6.1.0 #1'),
        'Linux (kernel 6.1.0)',
      );
      expect(friendlyOsVersion('android', '5.15.0'), '5.15.0');
    });

    test('versions carry the friendly name, the raw one and the zone', () {
      final v = diagnosticsVersions(const BuildInfo());
      expect(v['app'], '$kBuildVersion (${const BuildInfo().flavor})');
      expect(v['platform_version'], currentOsVersion());
      if (v['platform_version'] != Platform.operatingSystemVersion) {
        expect(v['platform_version_raw'], Platform.operatingSystemVersion);
      }
      expect(v['time_zone'], contains('UTC'));
    });
  });

  test('no setting of the report looks like a secret', () {
    for (final k in diagnosticsConfigKeys) {
      expect(diagnosticsSecretKey(k), isFalse, reason: k);
    }
    for (final k in [
      'twitch_access_token',
      'tmdb_api_key',
      'smb_password',
      'profiles_pin',
      'client_secret',
      'session_id',
      'oauth_cookie',
      'Authorization',
    ]) {
      expect(diagnosticsSecretKey(k), isTrue, reason: k);
    }
    expect(diagnosticsSecretKey(Preferences.imageCacheMaxBytes), isFalse);
  });

  test('desktop reveal commands', () {
    expect(
      revealCommand('windows', r'C:\Users\A\Downloads\r.zip', 'x'),
      ['explorer.exe', r'/select,C:\Users\A\Downloads\r.zip'],
    );
    expect(revealCommand('macos', '/Users/a/Downloads/r.zip', 'x'), [
      'open',
      '-R',
      '/Users/a/Downloads/r.zip',
    ]);
    expect(revealCommand('linux', '/home/a/Downloads/r.zip', '/home/a/D'), [
      'xdg-open',
      '/home/a/D',
    ]);
    expect(revealCommand('android', 'a', 'b'), isNull);
  });

  test('folder of a report', () {
    final url = Uri.parse('https://example.org');
    expect(
      DiagnosticsExport(
        path: r'C:\Users\A\Downloads\r.zip',
        shownPath: r'C:\Users\A\Downloads\r.zip',
        name: 'r.zip',
        issueUrl: url,
      ).folder,
      r'C:\Users\A\Downloads',
    );
    expect(
      DiagnosticsExport(
        path: 'Download/r.zip',
        shownPath: 'Download/r.zip',
        name: 'r.zip',
        issueUrl: url,
        shareUri: 'content://media/external/downloads/42',
      ).folder,
      'Download',
    );
  });

  testWidgets('phone: the dialog shares the report (BUG-024)', (t) async {
    setSurface(t, const Size(390, 844), dpr: 2);
    final shared = <String?>[];
    final prevReveal = Diagnostics.revealReport;
    Diagnostics.revealReport = (r) async {
      shared.add(r.shareUri);
      return false;
    };
    addTearDown(() => Diagnostics.revealReport = prevReveal);
    final services = await demoServices();
    await t.pumpWidget(
      testApp(services, formFactor: FormFactor.phone, section: Section.home),
    );
    await settle(t, 6);
    final context = t.element(find.byType(Scaffold).first);
    unawaited(
      showDiagnosticsSaved(
        context,
        DiagnosticsExport(
          path: 'Download/kinolune-diagnostics-20261121-123837.zip',
          shownPath: 'Download/kinolune-diagnostics-20261121-123837.zip',
          name: 'kinolune-diagnostics-20261121-123837.zip',
          issueUrl: Uri.parse('https://github.com/DenisHumen/kinolune'),
          shareUri: 'content://media/external/downloads/42',
        ),
      ),
    );
    await settle(t, 6);
    expect(
      t
          .widget<SelectableText>(
            find.byKey(const ValueKey('diagnostics-saved-folder')),
          )
          .data,
      'Download',
    );
    expect(find.text(en.settingsDiagnosticsShare), findsOneWidget);
    expect(find.text(en.settingsDiagnosticsOpenFolder), findsNothing);
    await t.tap(find.byKey(const ValueKey('diagnostics-open-folder')));
    await settle(t, 4);
    expect(shared, ['content://media/external/downloads/42']);
    // It did not open: the user is told so.
    expect(find.text(en.settingsDiagnosticsShareFailed), findsOneWidget);
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
