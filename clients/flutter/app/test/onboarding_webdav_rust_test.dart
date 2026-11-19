// SPDX-License-Identifier: MPL-2.0
//
// UI-004/UI-042 end to end on the real Rust core: the first-run setup finds a
// WebDAV share announced over mDNS (NET-020, loopback), the UI-005 form
// adds it with a login to the core's source manager, the core scans it
// (LIB-018) and the library shows its titles once the setup is done.
//
// Needs the nginx WebDAV of core/crates/pa-catalog/tests/servers in Docker;
// tool/onboarding_webdav_test.sh starts it, seeds a small library and runs
// this file. Without PA_ONBOARDING_WEBDAV the test is skipped.

import 'dart:async';
import 'dart:io';
import 'dart:math';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/screens/onboarding_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/core_discovery.dart';
import 'package:kinolune/sources/core_source_manager.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_core/pa_core.dart';
import 'package:pa_player/testing.dart';

import 'support/native_library.dart';
import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final _env = Platform.environment;

/// `http://127.0.0.1:18581/dav/` of the Docker stand.
final _webdav = _env['PA_ONBOARDING_WEBDAV'];
const _instance = 'Kinolune WebDAV stand';

/// Pumps frames with real time in between until [done] (the core works on
/// its own threads), at most [seconds].
Future<void> _waitFor(
  WidgetTester tester,
  bool Function() done, {
  int seconds = 60,
  String? what,
}) async {
  for (var i = 0; i < seconds * 5 && !done(); i++) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 200)),
    );
    await tester.pump();
  }
  if (!done()) {
    // What the screen says (an error of the form, say) explains a timeout.
    final texts = [
      for (final e in find.byType(Text).evaluate())
        (e.widget as Text).data ?? '',
    ].where((t) => t.isNotEmpty);
    fail(
      'timed out waiting for ${what ?? 'it'}; on screen: ${texts.join(' | ')}',
    );
  }
}

final _vault = <String, String>{};

/// Platform secret store of the core, in memory.
PlatformSecretReply _memoryKeystore(PlatformSecretCall call) =>
    switch (call.op) {
      'load' => PlatformSecretReply(value: _vault[call.key], existed: false),
      'save' => () {
        _vault[call.key] = call.value ?? '';
        return const PlatformSecretReply(existed: false);
      }(),
      'remove' => PlatformSecretReply(existed: _vault.remove(call.key) != null),
      _ => const PlatformSecretReply(existed: false, errorCode: 'failed'),
    };

Future<void> _next(WidgetTester tester) async {
  await tester.tap(find.byKey(const ValueKey('onboarding-next')));
  await settle(tester, 6);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late Directory tmp;
  late RustLibrary library;
  late CoreSourceManager manager;
  late CoreSourceDiscovery discovery;
  CoreDiscovery? announcer;

  // The core is opened outside the widget test's fake-async zone.
  setUpAll(() async {
    if (_webdav == null) return;
    await loadAppFonts();
    await RustLibrary.ensureInitialized(
      externalLibrary: openBuiltCoreLibrary(),
    );
    tmp = Directory.systemTemp.createTempSync('kinolune_ui004_');
    final db = '${tmp.path}/$coreDbFileName';
    // The core's secret store as on Android (CORE-015): a platform store,
    // here in memory, so the test password never reaches the keychain of
    // the machine (which may be locked and wait for a prompt).
    await secretsOpenPlatform(dataDir: tmp.path, callback: _memoryKeystore);
    library = await RustLibrary.open(db);
    await library.connectSources(tmp.path);
    expect(library.networkSourcesError, isNull);
    manager = await CoreSourceManager.open(dbPath: db, dataDir: tmp.path);
    // The stand's address announced as `_webdav._tcp` on loopback and a
    // private port: the app finds it as on a home network.
    final port = 30000 + Random().nextInt(20000);
    discovery = await CoreSourceDiscovery.open(loopbackPort: port);
    announcer = await CoreDiscovery.open(
      config: CoreDiscoveryConfig(loopbackPort: port, ownIds: const []),
      multicastLock: (_) {},
    );
    final uri = Uri.parse(_webdav!);
    await announcer!.announce(
      kind: CoreServiceKind.webDav,
      instance: _instance,
      port: uri.port,
      txt: [CoreTxtEntry(key: 'path', value: uri.path)],
    );
  });

  tearDownAll(() async {
    if (_webdav == null) return;
    await discovery.close();
    await announcer?.close();
    await library.close();
    secretsClose();
    tmp.deleteSync(recursive: true);
  });

  testWidgets(
    'first run: WebDAV found on the network, added, scanned, library filled',
    (tester) async {
      setSurface(tester, const Size(1280, 900));
      final prefs = MemoryPreferences();
      final services = AppServices(
        library: library,
        secureStore: MemorySecureStore(),
        preferences: prefs,
        playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
        sourceManager: manager,
        discovery: discovery,
      );
      await tester.runAsync(services.init);
      expect(services.onboardingPending, isTrue);
      await tester.pumpWidget(testApp(services));
      await settle(tester);

      // UI-042: welcome → what was found on the network.
      expect(find.byType(OnboardingScreen), findsOneWidget);
      await _next(tester);
      expect(find.text(en.onboardingFindTitle), findsOneWidget);

      // The announced share shows up in "On this network".
      await _waitFor(
        tester,
        () => find.text(_instance).evaluate().isNotEmpty,
        what: 'the announced WebDAV share',
      );
      await capture(tester, 'ui004-onboarding-source-discovered');
      // «Добавить»: a WebDAV share needs a login, so its form opens.
      await tester.tap(
        find.byWidgetPredicate(
          (w) =>
              w.key is ValueKey<String> &&
              (w.key! as ValueKey<String>).value.startsWith(
                'onboarding-add-webdav-',
              ),
        ),
      );
      await settle(tester, 4);
      final url = tester
          .widget<TextField>(find.byKey(const ValueKey('source-url')))
          .controller!
          .text;
      expect(Uri.parse(url).port, Uri.parse(_webdav!).port);
      expect(Uri.parse(url).path, Uri.parse(_webdav!).path);
      await tester.enterText(
        find.byKey(const ValueKey('source-user')),
        _env['PA_SERVERS_USER']!,
      );
      await tester.enterText(
        find.byKey(const ValueKey('source-password')),
        _env['PA_SERVERS_PASS']!,
      );
      await capture(tester, 'ui004-onboarding-webdav-form');
      await tester.tap(find.byKey(const ValueKey('source-submit')));

      // Saved by the core (password in its secret store), then scanned.
      await _waitFor(
        tester,
        () => services.managedSources.isNotEmpty,
        what: 'the source to be saved',
      );
      final source = services.managedSources.single;
      expect(source.type, SourceType.webdav);
      expect(source.hasPassword, isTrue);
      await _waitFor(
        tester,
        () =>
            !services.scanning &&
            (services.scanProgress?.isDone ?? false) &&
            (services.managedSource(source.id)?.status.lastScanFinishedAt !=
                null),
        seconds: 180,
        what: 'the scan',
      );
      expect(services.scanProgress!.error, isNull);
      expect(services.scanProgress!.filesFound, greaterThanOrEqualTo(5));
      await settle(tester, 4);
      expect(
        find.byKey(ValueKey('onboarding-added-${source.id}')),
        findsOneWidget,
      );
      await capture(tester, 'ui004-onboarding-source-scanned');

      final movies = await tester.runAsync(() => library.movies().first);
      final shows = await tester.runAsync(() => library.shows().first);
      expect(movies!.length, 3);
      expect([for (final s in shows!) s.title], contains('Dark'));

      // Pairing is optional; «Открыть главную» opens the library.
      await _next(tester);
      expect(find.text(en.onboardingDevicesTitle('Kinolune')), findsOneWidget);
      await _next(tester);
      await settle(tester, 4);
      expect(find.byType(AppShell), findsOneWidget);
      expect(prefs.values[Preferences.onboardingDone], 'true');
      // Home fills from the core: the titles of the share, no empty state.
      await _waitFor(
        tester,
        () => find.text(en.homeEmptyTitle).evaluate().isEmpty,
        what: 'Home to show the library',
      );
      await settle(tester, 4);
      await capture(tester, 'ui004-home-after-onboarding');

      // The watcher and scan queue were started in this test's zone: close
      // the library here, with real time, not in tearDownAll.
      var closed = false;
      unawaited(library.close().then((_) => closed = true));
      await _waitFor(tester, () => closed, what: 'the library to close');

      await tester.pumpWidget(const SizedBox());
      await settle(tester, 2);
    },
    skip: _webdav == null,
  );
}
