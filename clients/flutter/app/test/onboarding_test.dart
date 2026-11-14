// SPDX-License-Identifier: MPL-2.0
//
// UI-004: the first-run setup — shown only on the first start, skippable,
// language, modes (our server and Plex/Jellyfin/Emby announced as "soon"),
// the local profile, the first source from "On this network" (NET-020) with
// the UI-005 form, the optional pairing step (UI-019) and Settings opening
// it again.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/screens/devices_screen.dart';
import 'package:kinolune/screens/onboarding_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/image_cache_control.dart';
import 'package:kinolune/services/local_profile.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

/// Empty services over [prefs], LAN discovery from [demoDiscovery].
Future<AppServices> _services({
  Preferences? prefs,
  FakeLibrary? library,
}) async {
  final lib = library ?? FakeLibrary(scanStepDelay: Duration.zero);
  final s = AppServices(
    library: lib,
    secureStore: MemorySecureStore(),
    preferences: prefs ?? MemoryPreferences(),
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    sourceManager: FakeSourceManager(lib),
    discovery: demoDiscovery,
    sync: FakeDeviceSync.demo(),
    clock: () => demoNow,
    imageCache: demoImageCache(),
  );
  await s.init();
  return s;
}

Future<void> _next(WidgetTester tester) async {
  await tester.tap(find.byKey(const ValueKey('onboarding-next')));
  await settle(tester, 6);
}

Future<void> _start(WidgetTester tester, AppServices services) async {
  setSurface(tester, const Size(1280, 900));
  await tester.pumpWidget(testApp(services));
  await settle(tester);
}

void main() {
  setUpAll(loadAppFonts);

  testWidgets('first start: the setup, not the shell', (tester) async {
    final services = await _services();
    expect(services.onboardingPending, isTrue);
    await _start(tester, services);
    expect(find.byType(OnboardingScreen), findsOneWidget);
    expect(find.byType(AppShell), findsNothing);
    expect(find.text(en.onboardingStep(1, 5)), findsOneWidget);
    expect(find.text(en.onboardingTryDemo), findsOneWidget);
  });

  testWidgets('Skip ends the setup and it is not shown again', (tester) async {
    final prefs = MemoryPreferences();
    final services = await _services(prefs: prefs);
    await _start(tester, services);
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
    expect(find.text(ru.onboardingStep(1, 5)), findsOneWidget);
  });

  testWidgets('modes: our server lists servers found and says "soon"; '
      'Plex, Jellyfin and Emby are not selectable', (tester) async {
    final services = await _services();
    await _start(tester, services);
    await _next(tester);
    expect(find.text(en.onboardingModeTitle), findsOneWidget);
    expect(find.text(en.onboardingSoon), findsNWidgets(2));

    await tester.tap(
      find.byKey(const ValueKey('onboarding-mode-media-server')),
    );
    await settle(tester, 4);
    await tester.tap(find.byKey(const ValueKey('onboarding-mode-server')));
    await settle(tester, 4);
    expect(find.textContaining('Home server'), findsOneWidget);
    expect(find.textContaining('media.local:8487'), findsOneWidget);
    expect(find.text(en.onboardingServerSoonNote), findsOneWidget);

    await _next(tester);
    expect(services.appMode.value, AppMode.server);
  });

  testWidgets('the profile is saved with its avatar', (tester) async {
    final prefs = MemoryPreferences();
    final services = await _services(prefs: prefs);
    await _start(tester, services);
    await _next(tester);
    await _next(tester);
    expect(find.text(en.onboardingProfileTitle), findsOneWidget);
    await tester.enterText(
      find.byKey(const ValueKey('onboarding-profile-name')),
      'Alex',
    );
    await tester.tap(find.byKey(const ValueKey('onboarding-avatar-3')));
    await settle(tester, 2);
    expect(find.text('A'), findsOneWidget); // the avatar's initial
    await _next(tester);
    expect(services.profile.value, const LocalProfile(name: 'Alex', avatar: 3));
    final reopened = await _services(prefs: prefs);
    expect(reopened.profile.value, const LocalProfile(name: 'Alex', avatar: 3));
  });

  testWidgets('first source from "On this network": WebDAV, prefilled, '
      'added and scanned; then pairing and Done', (tester) async {
    final services = await _services();
    await _start(tester, services);
    for (var i = 0; i < 3; i++) {
      await _next(tester);
    }
    expect(find.text(en.onboardingSourceTitle), findsOneWidget);
    expect(find.text(en.sourcesAddDiscovered), findsOneWidget);
    // Manual kinds are there too, the unavailable ones as "soon".
    expect(find.text(en.sourcesTypeLocal), findsOneWidget);

    await tester.tap(find.text('Living-room NAS'));
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
    expect(added.address, 'http://192.168.1.20:8080/dav/');
    expect(added.hasPassword, isTrue);
    expect(services.scanProgress?.isDone, isTrue);
    // Still in the setup, with the source and the scan result listed.
    expect(
      find.byKey(ValueKey('onboarding-added-${added.id}')),
      findsOneWidget,
    );
    expect(
      find.byKey(const ValueKey('onboarding-scan-status')),
      findsOneWidget,
    );

    await _next(tester);
    expect(find.text(en.onboardingDevicesTitle('Kinolune')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('onboarding-pair')));
    await settle(tester, 6);
    expect(find.byType(DevicesScreen), findsOneWidget);
    await tester.pageBack();
    await settle(tester, 6);

    await tester.tap(find.text(en.onboardingFinish));
    await settle(tester);
    expect(find.byType(AppShell), findsOneWidget);
    expect(services.onboardingPending, isFalse);
  });

  testWidgets('Back returns to the previous step', (tester) async {
    final services = await _services();
    await _start(tester, services);
    await _next(tester);
    await _next(tester);
    await tester.tap(find.byKey(const ValueKey('onboarding-back')));
    await settle(tester, 4);
    expect(find.text(en.onboardingModeTitle), findsOneWidget);
  });

  testWidgets('Settings opens the setup again; Done returns to Settings', (
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
}
