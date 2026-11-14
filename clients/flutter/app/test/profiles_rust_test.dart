// SPDX-License-Identifier: MPL-2.0
//
// UI-015 "done when": on the Rust core (LIB-012: Argon2id PINs, the active
// profile of the library), a kid profile cannot open Settings without the
// parent PIN; Streams are hidden for it.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/screens/settings_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/image_cache_control.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/native_library.dart';
import 'support/test_app.dart';

/// Pumps with real time in between (the core hashes PINs on its threads).
Future<void> _waitFor(WidgetTester t, bool Function() done) async {
  for (var i = 0; i < 100 && !done(); i++) {
    await t.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
    await t.pump();
  }
  await settle(t, 2);
}

Future<void> _enter(WidgetTester t, String pin) async {
  for (final d in pin.split('')) {
    await t.tap(find.byKey(ValueKey('pin-key-$d')).last);
    await t.pump();
  }
  await t.tap(find.byKey(const ValueKey('pin-key-ok')).last);
  await t.pump();
}

void main() {
  late Directory tmp;
  late RustLibrary lib;

  setUpAll(() async {
    await RustLibrary.ensureInitialized(
      externalLibrary: openBuiltCoreLibrary(),
    );
    await loadAppFonts();
  });

  setUp(() async {
    tmp = Directory.systemTemp.createTempSync('kinolune_ui015_');
    lib = await RustLibrary.open('${tmp.path}/db/core.db');
  });

  tearDown(() async {
    await lib.close();
    tmp.deleteSync(recursive: true);
  });

  testWidgets('a kid profile cannot open Settings without the PIN', (t) async {
    setSurface(t, const Size(1280, 800));
    final services = await t.runAsync(() async {
      final parent = await lib.activeProfile();
      await lib.updateProfile(
        parent.id,
        parent.settings.copyWith(name: 'Parent'),
      );
      await lib.setProfilePin(parent.id, '2468');
      final kid = await lib.createProfile(
        const ProfileSettings.kid(name: 'Mia'),
      );
      expect(await lib.switchProfile(kid.id), isNotNull);
      final prefs = MemoryPreferences()
        ..values[Preferences.onboardingDone] = 'true'
        ..values[Preferences.profilesPickerOnStart] = 'false'
        ..values[Preferences.showTwitch] = 'true';
      final s = AppServices(
        library: lib,
        secureStore: MemorySecureStore(),
        preferences: prefs,
        playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
        sourceManager: FakeSourceManager(lib),
        sync: FakeDeviceSync.demo(),
        clock: () => demoNow,
        imageCache: demoImageCache(),
      );
      await s.init();
      return s;
    });
    final s = services!;
    expect(s.profileControl.active!.name, 'Mia');
    expect(s.profileControl.restricted, isTrue);

    await t.pumpWidget(testApp(s));
    await settle(t);
    final shell = t.state<AppShellState>(find.byType(AppShell));
    // Streams are hidden for the kid profile although turned on.
    expect(shell.menuSections, isNot(contains(Section.twitch)));

    shell.select(Section.settings);
    await settle(t, 6);
    final gate = find.byKey(const ValueKey('profiles-gate-settings'));
    expect(gate, findsOneWidget);
    expect(find.byType(SettingsScreen), findsNothing);

    // The kid's own guess is refused by the core's Argon2id check.
    await _enter(t, '1111');
    await _waitFor(
      t,
      () => find.byKey(const ValueKey('pin-error')).evaluate().isNotEmpty,
    );
    expect(find.byKey(const ValueKey('pin-error')), findsOneWidget);
    expect(find.byType(SettingsScreen), findsNothing);
    expect(s.profileControl.limiter.failures, 1);

    await _enter(t, '2468');
    await _waitFor(t, () => gate.evaluate().isEmpty);
    expect(find.byType(SettingsScreen), findsOneWidget);
    expect(s.profileControl.limiter.failures, 0);

    await t.pumpWidget(const SizedBox());
  });
}
