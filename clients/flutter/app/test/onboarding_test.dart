// SPDX-License-Identifier: MPL-2.0
//
// UI-042 (was UI-004): the first run in three steps — hello and language;
// what was found on the network with a one-tap «Добавить» (NET-020), a
// folder on this device and an address; done, with optional pairing
// (UI-019). Shown only on the first start, skippable, opened again from
// the empty library, Sources and Settings. A found DLNA server or a guest
// SMB folder is added without a single question and fills home; SMB with
// a password still asks for the login.

import 'package:flutter/services.dart' show LogicalKeyboardKey;
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_smb_server.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/add_source_dialog.dart';
import 'package:kinolune/screens/devices_screen.dart';
import 'package:kinolune/screens/home_screen.dart';
import 'package:kinolune/screens/onboarding_screen.dart';
import 'package:kinolune/screens/sources_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/image_cache_control.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/discovery.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

const _dlna = DiscoveredSource(
  type: SourceType.dlna,
  name: 'Living-room TV box',
  host: '192.168.1.40',
  port: 8200,
  url: 'http://192.168.1.40:8200/rootDesc.xml',
  stableName: 'uuid:4d696e69-444c-164e-9d41-b827eb000001',
);

const _smb = DiscoveredSource(
  type: SourceType.smb,
  name: 'Study PC',
  host: '192.168.1.31',
  url: 'smb://192.168.1.31/',
  stableName: 'STUDY-PC',
);

/// Empty services over [prefs]; LAN discovery from [discovery] (the demo
/// WebDAV share and SMB server by default).
Future<AppServices> _services({
  Preferences? prefs,
  SourceDiscovery discovery = demoDiscovery,
  List<FakeSmbServer>? smb,
}) async {
  final lib = FakeLibrary(scanStepDelay: Duration.zero);
  final manager = FakeSourceManager(lib);
  if (smb != null) manager.smbServers = smb;
  final s = AppServices(
    library: lib,
    secureStore: MemorySecureStore(),
    preferences: prefs ?? MemoryPreferences(),
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    sourceManager: manager,
    discovery: discovery,
    sync: FakeDeviceSync.demo(),
    clock: () => demoNow,
    imageCache: demoImageCache(),
  );
  await s.init();
  return s;
}

/// [text] inside the «Нашли» row of source [id] (`type-host`).
Finder _rowSays(String id, String text) => find.descendant(
  of: find.byKey(ValueKey('onboarding-found-$id')),
  matching: find.text(text),
);

FakeSourceManager _manager(AppServices s) =>
    s.sourceManager as FakeSourceManager;

Future<void> _next(WidgetTester tester) async {
  await tester.tap(find.byKey(const ValueKey('onboarding-next')));
  await settle(tester, 6);
}

Future<void> _start(
  WidgetTester tester,
  AppServices services, {
  FormFactor formFactor = FormFactor.desktop,
  Size size = const Size(1280, 900),
}) async {
  setSurface(tester, size);
  await tester.pumpWidget(testApp(services, formFactor: formFactor));
  await settle(tester);
}

void main() {
  setUpAll(loadAppFonts);

  testWidgets('first start: the setup in three steps, not the shell', (
    tester,
  ) async {
    final services = await _services();
    expect(services.onboardingPending, isTrue);
    await _start(tester, services);
    expect(find.byType(OnboardingScreen), findsOneWidget);
    expect(find.byType(AppShell), findsNothing);
    expect(find.text(en.onboardingStep(1, 3)), findsOneWidget);
    expect(find.text(en.onboardingTryDemo), findsOneWidget);
    expect(find.byKey(const ValueKey('onboarding-back')), findsNothing);
  });

  testWidgets('Skip ends the setup and it is not shown again', (tester) async {
    final prefs = MemoryPreferences();
    final services = await _services(prefs: prefs);
    await _start(tester, services);
    await _next(tester);
    await tester.tap(find.byKey(const ValueKey('onboarding-skip')));
    await settle(tester);
    expect(find.byType(AppShell), findsOneWidget);
    expect(prefs.values[Preferences.onboardingDone], 'true');
    // Next start, still without sources: straight to the (empty) library.
    final again = await _services(prefs: prefs);
    expect(again.onboardingPending, isFalse);
  });

  test('an install that already has sources skips the setup', () async {
    final services = await demoServices();
    expect(services.sources, isNotEmpty);
    expect(services.onboardingPending, isFalse);
  });

  testWidgets('the language applies at once and is kept', (tester) async {
    final prefs = MemoryPreferences();
    final services = await _services(prefs: prefs);
    await _start(tester, services);
    await tester.tap(find.byKey(const ValueKey('onboarding-language-ru')));
    await settle(tester, 4);
    expect(services.locale.value, const Locale('ru'));
    expect(prefs.values[Preferences.locale], 'ru');
    expect(find.text(ru.onboardingStart), findsOneWidget);
    expect(find.text(ru.onboardingStep(1, 3)), findsOneWidget);
  });

  testWidgets('done when: a fresh install finds a server, one tap on Add, '
      'and home fills without a single question', (tester) async {
    final prefs = MemoryPreferences();
    final services = await _services(
      prefs: prefs,
      discovery: const StaticSourceDiscovery([_dlna]),
    );
    await _start(tester, services);
    await _next(tester);
    expect(find.text(en.onboardingFindTitle), findsOneWidget);
    expect(find.text(en.onboardingFindFound), findsOneWidget);
    expect(find.text('Living-room TV box'), findsOneWidget);
    // Never a question about what the folder holds (ADR-0099).
    expect(find.textContaining('content'), findsNothing);

    await tester.tap(
      find.byKey(const ValueKey('onboarding-add-dlna-192.168.1.40')),
    );
    await settle(tester, 2);
    // No form, no dialog: added as it was found.
    expect(find.byType(Dialog), findsNothing);
    expect(find.byType(AlertDialog), findsNothing);
    final added = services.managedSources.single;
    expect(added.type, SourceType.dlna);
    expect(added.address, 'http://192.168.1.40:8200/rootDesc.xml');
    expect(added.lanName, 'uuid:4d696e69-444c-164e-9d41-b827eb000001');
    expect(_manager(services).drafts.single.password, isNull);
    await settle(tester, 20);
    // The row says so; the scan runs at once.
    expect(_rowSays('dlna-192.168.1.40', en.onboardingFindAdded), findsOneWidget);
    expect(
      find.byKey(ValueKey('onboarding-added-${added.id}')),
      findsOneWidget,
    );
    expect(services.scanProgress?.isDone, isTrue);

    await _next(tester);
    expect(find.text(en.onboardingStep(3, 3)), findsOneWidget);
    expect(find.text(en.onboardingDoneTitle), findsOneWidget);
    expect(
      find.byKey(const ValueKey('onboarding-scan-status')),
      findsOneWidget,
    );
    await tester.tap(find.text(en.onboardingOpenHome));
    await settle(tester, 10);
    expect(find.byType(AppShell), findsOneWidget);
    expect(prefs.values[Preferences.onboardingDone], 'true');
    expect(find.text(en.homeEmptyTitle), findsNothing);
    await scrollHomeTo(tester, find.text('Recently Added Movies'));
    expect(find.text('Recently Added Movies'), findsOneWidget);
  });

  testWidgets('SMB with a guest folder is added in one tap', (tester) async {
    final services = await _services(
      discovery: const StaticSourceDiscovery([_smb]),
      smb: [FakeSmbServer(address: '192.168.1.31')],
    );
    await _start(tester, services);
    await _next(tester);
    await tester.tap(
      find.byKey(const ValueKey('onboarding-add-smb-192.168.1.31')),
    );
    await settle(tester, 6);
    expect(find.byType(Dialog), findsNothing);
    final added = services.managedSources.single;
    expect(added.type, SourceType.smb);
    expect(added.address, 'smb://192.168.1.31/public');
    expect(added.username, isNull);
    expect(added.lanName, 'STUDY-PC');
  });

  testWidgets('SMB with a password still asks for the login', (tester) async {
    final services = await _services(
      discovery: const StaticSourceDiscovery([_smb]),
      smb: [FakeSmbServer(address: '192.168.1.31', guestShares: const {})],
    );
    await _start(tester, services);
    await _next(tester);
    await tester.tap(
      find.byKey(const ValueKey('onboarding-add-smb-192.168.1.31')),
    );
    await settle(tester, 6);
    // The UI-028 flow at its sign-in step, the server already filled in.
    expect(find.byKey(const ValueKey('smb-user')), findsOneWidget);
    expect(services.managedSources, isEmpty);
    await tester.enterText(find.byKey(const ValueKey('smb-user')), 'alex');
    await tester.enterText(
      find.byKey(const ValueKey('smb-password')),
      'secret',
    );
    await tester.tap(find.byKey(const ValueKey('smb-connect')));
    await settle(tester, 6);
    await tester.tap(find.byKey(const ValueKey('smb-share-Media')));
    await settle(tester, 6);
    await tester.tap(find.byKey(const ValueKey('smb-use-folder')));
    await settle(tester, 10);
    final added = services.managedSources.single;
    expect(added.address, 'smb://192.168.1.31/Media');
    expect(added.username, 'alex');
    expect(find.byType(OnboardingScreen), findsOneWidget);
    expect(_rowSays('smb-192.168.1.31', en.onboardingFindAdded), findsOneWidget);
  });

  testWidgets('a WebDAV share opens its form prefilled; the login is the '
      'only question', (tester) async {
    final services = await _services();
    await _start(tester, services);
    await _next(tester);
    await tester.tap(
      find.byKey(const ValueKey('onboarding-add-webdav-192.168.1.20')),
    );
    await settle(tester, 4);
    final url = tester
        .widget<TextField>(find.byKey(const ValueKey('source-url')))
        .controller!
        .text;
    expect(url, 'http://192.168.1.20:8080/dav/');
    await tester.enterText(find.byKey(const ValueKey('source-user')), 'viewer');
    await tester.enterText(
      find.byKey(const ValueKey('source-password')),
      'secret',
    );
    await tester.tap(find.byKey(const ValueKey('source-submit')));
    await settle(tester, 20);
    final added = services.managedSources.single;
    expect(added.type, SourceType.webdav);
    expect(added.hasPassword, isTrue);
    expect(services.scanProgress?.isDone, isTrue);
    expect(
      find.byKey(ValueKey('onboarding-added-${added.id}')),
      findsOneWidget,
    );
  });

  testWidgets('a folder on this device and an address open the add flow', (
    tester,
  ) async {
    final services = await _services();
    await _start(tester, services);
    await _next(tester);
    await tester.tap(find.byKey(const ValueKey('onboarding-local')));
    await settle(tester, 4);
    expect(find.text(en.addSourceFolder), findsOneWidget);
    await tester.tap(find.text(en.commonCancel).last);
    await settle(tester, 4);
    await tester.tap(find.byKey(const ValueKey('onboarding-address')));
    await settle(tester, 4);
    // The kind chooser of «Добавить источник» (UI-005).
    expect(find.text(en.addSourceTitle), findsOneWidget);
    expect(find.text(en.sourcesAddManual), findsOneWidget);
  });

  testWidgets('nothing found: a plain line, the other two ways stay', (
    tester,
  ) async {
    final services = await _services(
      discovery: const StaticSourceDiscovery([]),
    );
    await _start(tester, services);
    await _next(tester);
    expect(find.text(en.onboardingFindNone), findsOneWidget);
    expect(find.byKey(const ValueKey('onboarding-local')), findsOneWidget);
    expect(find.byKey(const ValueKey('onboarding-address')), findsOneWidget);
    await _next(tester);
    // Nothing added: the last step says it can wait.
    expect(find.text(en.onboardingDoneLaterTitle), findsOneWidget);
  });

  testWidgets('pairing is offered on the last step', (tester) async {
    final services = await _services();
    await _start(tester, services);
    await _next(tester);
    await _next(tester);
    expect(find.text(en.onboardingDevicesTitle('Kinolune')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('onboarding-pair')));
    await settle(tester, 6);
    expect(find.byType(DevicesScreen), findsOneWidget);
    await tester.pageBack();
    await settle(tester, 6);
    expect(find.byType(OnboardingScreen), findsOneWidget);
  });

  testWidgets('Back returns to the previous step', (tester) async {
    final services = await _services();
    await _start(tester, services);
    await _next(tester);
    await _next(tester);
    await tester.tap(find.byKey(const ValueKey('onboarding-back')));
    await settle(tester, 4);
    expect(find.text(en.onboardingFindTitle), findsOneWidget);
  });

  testWidgets('TV: «Добавить» has the remote, OK adds, then on to «Далее»', (
    tester,
  ) async {
    final services = await _services(
      discovery: const StaticSourceDiscovery([_dlna]),
    );
    await _start(
      tester,
      services,
      formFactor: FormFactor.tv,
      size: const Size(960, 540),
    );
    await pressKey(tester, LogicalKeyboardKey.select);
    await settle(tester, 6);
    expect(find.text(en.onboardingFindTitle), findsOneWidget);
    final add = find.byKey(const ValueKey('onboarding-add-dlna-192.168.1.40'));
    final next = find.byKey(const ValueKey('onboarding-next'));
    bool focused(Finder f) {
      final ctx = FocusManager.instance.primaryFocus?.context;
      if (ctx == null || f.evaluate().isEmpty) return false;
      final el = f.evaluate().single;
      if (ctx == el) return true;
      var hit = false;
      ctx.visitAncestorElements((e) {
        hit = e == el;
        return !hit;
      });
      return hit;
    }

    // The first «Добавить» takes the remote's focus: one OK adds.
    expect(focused(add), isTrue, reason: 'focus is not on «Добавить»');
    await pressKey(tester, LogicalKeyboardKey.select);
    await settle(tester, 10);
    expect(services.managedSources.single.type, SourceType.dlna);
    // The button stays («Добавлено») and keeps the focus: no lost focus.
    expect(_rowSays('dlna-192.168.1.40', en.onboardingFindAdded), findsOneWidget);
    expect(focused(add), isTrue);
    // Down to the bottom row («Назад» · «Далее»), right to «Далее».
    final back = find.byKey(const ValueKey('onboarding-back'));
    for (var i = 0; i < 6 && !focused(next) && !focused(back); i++) {
      await pressKey(tester, LogicalKeyboardKey.arrowDown);
      await settle(tester, 2);
    }
    if (focused(back)) {
      await pressKey(tester, LogicalKeyboardKey.arrowRight);
      await settle(tester, 2);
    }
    expect(focused(next), isTrue, reason: 'the remote never reached «Далее»');
    await pressKey(tester, LogicalKeyboardKey.select);
    await settle(tester, 6);
    expect(find.text(en.onboardingDoneTitle), findsOneWidget);
  });

  testWidgets('the empty home opens the setup at what was found', (
    tester,
  ) async {
    setSurface(tester, const Size(1280, 900));
    final prefs = MemoryPreferences()
      ..values[Preferences.onboardingDone] = 'true';
    final services = await _services(prefs: prefs);
    await tester.pumpWidget(testApp(services));
    await settle(tester);
    expect(find.byType(HomeScreen), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('home-find-sources')));
    await settle(tester, 6);
    expect(find.byType(OnboardingScreen), findsOneWidget);
    expect(find.text(en.onboardingStep(2, 3)), findsOneWidget);
    expect(find.text('Living-room NAS'), findsOneWidget);
    // No demo from here; Skip returns to the library.
    await tester.tap(find.byKey(const ValueKey('onboarding-skip')));
    await settle(tester, 6);
    expect(find.byType(OnboardingScreen), findsNothing);
    expect(find.byType(HomeScreen), findsOneWidget);
  });

  testWidgets('Sources: «Найти в сети» opens the setup too', (tester) async {
    setSurface(tester, const Size(1280, 900));
    final services = await demoServices(discovery: demoDiscovery);
    await tester.pumpWidget(testApp(services, section: Section.settings));
    await settle(tester);
    await tester.ensureVisible(find.text(en.sourcesManage));
    await settle(tester, 2);
    await tester.tap(find.text(en.sourcesManage));
    await settle(tester, 8);
    expect(find.byType(SourcesScreen), findsOneWidget);
    final button = find.byKey(const ValueKey('sources-find'));
    await tester.ensureVisible(button);
    await settle(tester, 2);
    await tester.tap(button);
    await settle(tester, 6);
    expect(find.text(en.onboardingFindTitle), findsOneWidget);
    expect(find.text('Living-room NAS'), findsOneWidget);
  });

  testWidgets('Settings opens the setup again; Skip returns to Settings', (
    tester,
  ) async {
    setSurface(tester, const Size(1280, 900));
    final services = await demoServices();
    await tester.pumpWidget(testApp(services, section: Section.settings));
    await settle(tester);
    final tile = find.byKey(const ValueKey('settings-onboarding'));
    await tester.ensureVisible(tile);
    await settle(tester, 4);
    await tester.tap(tile);
    await settle(tester, 6);
    expect(find.byType(OnboardingScreen), findsOneWidget);
    // No demo from here: the user has a library.
    expect(find.text(en.onboardingTryDemo), findsNothing);
    await tester.tap(find.byKey(const ValueKey('onboarding-skip')));
    await settle(tester, 6);
    expect(find.byType(OnboardingScreen), findsNothing);
    expect(find.text(en.settingsOnboarding), findsOneWidget);
  });

  // With KINOLUNE_SCREENSHOTS_DIR set the frames are written there:
  //   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-042 \
  //     flutter test test/onboarding_test.dart --plain-name screenshots
  for (final (name, ff, size, dpr) in [
    ('phone', FormFactor.phone, const Size(390, 844), 2.0),
    ('desktop', FormFactor.desktop, const Size(1280, 800), 1.0),
    ('tv', FormFactor.tv, const Size(960, 540), 2.0),
  ]) {
    testWidgets('screenshots $name: found, added, done, home', (t) async {
      setSurface(t, size, dpr: dpr);
      final services = await _services(
        discovery: const StaticSourceDiscovery([
          _dlna,
          _smb,
          DiscoveredSource(
            type: SourceType.jellyfin,
            name: 'Jellyfin',
            host: '192.168.1.20',
            port: 8096,
            url: 'http://192.168.1.20:8096',
          ),
        ]),
        smb: [FakeSmbServer(address: '192.168.1.31', guestShares: const {})],
      );
      await services.setLocale(const Locale('ru'));
      await t.pumpWidget(testApp(services, formFactor: ff));
      await settle(t);
      await capture(t, '01-welcome-$name');
      await _next(t);
      await capture(t, '02-found-$name');
      final add = find.byKey(const ValueKey('onboarding-add-dlna-192.168.1.40'));
      await t.ensureVisible(add);
      await settle(t, 2);
      await t.tap(add);
      await settle(t, 20);
      expect(services.managedSources.single.type, SourceType.dlna);
      await capture(t, '03-added-$name');
      await _next(t);
      await capture(t, '04-done-$name');
      await t.tap(find.byKey(const ValueKey('onboarding-next')));
      await settle(t, 10);
      expect(find.text(ru.homeEmptyTitle), findsNothing);
      await capture(t, '05-home-$name');
    });
  }

  group('quickSourceDraft', () {
    Future<SourceDraft?> draft(FakeSmbServer server, DiscoveredSource d) {
      final m = FakeSourceManager(FakeLibrary(scanStepDelay: Duration.zero))
        ..smbServers = [server];
      return quickSourceDraft(m, d);
    }

    test('several guest folders: the user picks, nothing is guessed', () async {
      final server = FakeSmbServer(
        address: '192.168.1.31',
        guestShares: const {'public', 'Media'},
      );
      expect(await draft(server, _smb), isNull);
    });

    test('hidden shares like IPC\$ do not count', () async {
      final server = FakeSmbServer(
        address: '192.168.1.31',
        shares: const {
          'IPC\$': [],
          'films': ['a.mkv'],
        },
        guestShares: const {'IPC\$', 'films'},
      );
      final d = await draft(server, _smb);
      expect(d?.address, 'smb://192.168.1.31/films');
    });

    test('Plex, Jellyfin, Emby and WebDAV always open their form', () async {
      final m = FakeSourceManager(FakeLibrary(scanStepDelay: Duration.zero));
      for (final t in [
        SourceType.plex,
        SourceType.jellyfin,
        SourceType.emby,
        SourceType.webdav,
      ]) {
        final d = DiscoveredSource(type: t, name: 'x', host: '192.168.1.50');
        expect(await quickSourceDraft(m, d), isNull, reason: t.name);
      }
    });
  });
}
