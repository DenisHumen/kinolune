// SPDX-License-Identifier: MPL-2.0
//
// UI-015: profiles and parental control in the app, on in-memory profiles
// (FakeProfiles: a parent with PIN 1234, a second adult, a kid profile).
// The same flow on the real core: profiles_rust_test.dart.

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_profiles.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/profiles/pin_limiter.dart';
import 'package:kinolune/profiles/profile_controller.dart';
import 'package:kinolune/screens/search_screen.dart';
import 'package:kinolune/screens/settings_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

/// Preferences without the start picker (tests open the app directly).
MemoryPreferences _prefs({bool picker = false, bool empty = false}) {
  final p = MemoryPreferences();
  if (!picker) p.values[Preferences.profilesPickerOnStart] = 'false';
  if (empty) p.values[Preferences.onboardingDone] = 'true';
  return p;
}

Future<AppServices> _services({
  bool kid = false,
  bool picker = false,
  bool empty = false,
  FakeProfiles? profiles,
  MemoryPreferences? prefs,
}) => demoServices(
  empty: empty,
  preferences: prefs ?? _prefs(picker: picker, empty: empty),
  profiles: profiles ?? FakeProfiles.demo(kidActive: kid),
);

AppShellState _shell(WidgetTester t) => t.state(find.byType(AppShell));

Future<void> _open(WidgetTester t, AppServices s, {FormFactor? ff}) async {
  setSurface(t, const Size(1280, 800));
  await t.pumpWidget(testApp(s, formFactor: ff ?? FormFactor.desktop));
  await settle(t);
}

Future<void> _select(WidgetTester t, Section s) async {
  _shell(t).select(s);
  await settle(t, 6);
}

/// Taps the digits of [pin] and OK on the visible pad.
Future<void> _tapPin(WidgetTester t, String pin) async {
  for (final d in pin.split('')) {
    await t.tap(find.byKey(ValueKey('pin-key-$d')).last);
    await t.pump();
  }
  await t.tap(find.byKey(const ValueKey('pin-key-ok')).last);
  await settle(t, 4);
}

final _settingsGate = find.byKey(const ValueKey('profiles-gate-settings'));

void main() {
  setUpAll(loadAppFonts);

  group('PinAttemptLimiter', () {
    test('five free tries, then a doubling wait up to the cap', () async {
      var now = DateTime(2026, 11, 14, 12);
      final prefs = MemoryPreferences();
      final l = PinAttemptLimiter(preferences: prefs, clock: () => now);
      for (var i = 0; i < 4; i++) {
        await l.fail();
        expect(l.locked, isFalse);
      }
      expect(l.attemptsBeforeWait, 1);
      await l.fail();
      expect(l.locked, isTrue);
      expect(l.remaining, const Duration(seconds: 30));
      now = now.add(const Duration(seconds: 31));
      expect(l.locked, isFalse);
      await l.fail();
      expect(l.remaining, const Duration(minutes: 1));
      expect(l.delayAfter(7), const Duration(minutes: 2));
      expect(l.delayAfter(40), const Duration(hours: 1));

      // Kept across restarts.
      final again = PinAttemptLimiter(preferences: prefs, clock: () => now);
      await again.load();
      expect(again.failures, 6);
      expect(again.locked, isTrue);

      await again.succeed();
      expect(again.failures, 0);
      expect(again.locked, isFalse);
      expect(prefs.values[Preferences.profilesPinAttempts], isNull);
    });

    test('a stored wait longer than the cap is cut to it', () async {
      final now = DateTime(2026, 11, 14, 12);
      final prefs = MemoryPreferences()
        ..values[Preferences.profilesPinAttempts] =
            '{"failures": 9, "until": ${now.add(const Duration(days: 30)).millisecondsSinceEpoch}}';
      final l = PinAttemptLimiter(preferences: prefs, clock: () => now);
      await l.load();
      expect(l.remaining, const Duration(hours: 1));
    });
  });

  group('ProfileController', () {
    Future<ProfileController> ctl(FakeProfiles store) async {
      final c = ProfileController(store: store, preferences: _prefs());
      await c.init();
      return c;
    }

    test('leaving a kid profile needs the parent PIN', () async {
      final c = await ctl(FakeProfiles.demo(kidActive: true));
      final sam = c.all.firstWhere((p) => p.name == 'Sam');
      final alex = c.all.firstWhere((p) => p.name == 'Alex');
      expect(c.needsFor(sam).parentPin, isTrue);
      expect(c.needsFor(sam).profilePin, isFalse);
      // The parent's own PIN is a parent PIN: asked once.
      expect(c.needsFor(alex).parentPin, isFalse);
      expect(c.needsFor(alex).profilePin, isTrue);
      expect(c.needsFor(c.active!).any, isFalse);

      expect(await c.verifyParent('0000'), PinCheck.wrong);
      expect(await c.verifyParent('1234'), PinCheck.ok);
      expect(await c.switchTo(alex, pin: '1234'), PinCheck.ok);
      expect(c.active!.name, 'Alex');
      expect(c.restricted, isFalse);
    });

    test('a kid profile needs a parent PIN, which then stays', () async {
      final store = FakeProfiles();
      final c = await ctl(store);
      expect(
        () => c.create(const ProfileSettings.kid(name: 'Mia')),
        throwsA(isA<ProfileChangeException>()),
      );
      expect(
        () => c.setAppLock(true),
        throwsA(isA<ProfileChangeException>()),
      );
      await c.setPin(c.active!.id, '2468');
      await c.create(const ProfileSettings.kid(name: 'Mia'));
      expect(c.all.length, 2);
      // The last parent PIN is not removed while a kid profile needs it.
      expect(
        () => c.setPin(c.active!.id, null),
        throwsA(isA<ProfileChangeException>()),
      );
      expect(
        () => c.delete(c.active!.id),
        throwsA(isA<ProfileChangeException>()),
      );
    });

    test('wrong PINs are rate limited across prompts', () async {
      final c = await ctl(FakeProfiles.demo(kidActive: true));
      for (var i = 0; i < 5; i++) {
        expect(await c.unlockSettings('9999'), PinCheck.wrong);
      }
      expect(c.limiter.locked, isTrue);
      // Locked out: even the right PIN is not checked, by any prompt.
      expect(await c.unlockSettings('1234'), PinCheck.locked);
      expect(await c.unlockSearch('1234'), PinCheck.locked);
      final alex = c.all.firstWhere((p) => p.name == 'Alex');
      expect(await c.switchTo(alex, pin: '1234'), PinCheck.locked);
      expect(c.settingsLocked, isTrue);
      expect(c.active!.name, 'Mia');
    });
  });

  testWidgets('a kid profile cannot open Settings without the parent PIN', (
    t,
  ) async {
    final s = await _services(kid: true);
    await _open(t, s);
    await _select(t, Section.settings);
    expect(_settingsGate, findsOneWidget);
    expect(find.byType(SettingsScreen), findsNothing);

    await _tapPin(t, '0000');
    expect(find.byKey(const ValueKey('pin-error')), findsOneWidget);
    expect(find.byType(SettingsScreen), findsNothing);

    await _tapPin(t, '1234');
    expect(_settingsGate, findsNothing);
    expect(find.byType(SettingsScreen), findsOneWidget);
    await capture(t, 'ui015-settings-unlocked');

    // Leaving Settings locks them again.
    await _select(t, Section.home);
    await _select(t, Section.settings);
    expect(_settingsGate, findsOneWidget);
    expect(find.byType(SettingsScreen), findsNothing);
  });

  testWidgets('an adult profile opens Settings without a PIN', (t) async {
    final s = await _services();
    await _open(t, s);
    await _select(t, Section.settings);
    expect(_settingsGate, findsNothing);
    expect(find.byType(SettingsScreen), findsOneWidget);
    expect(
      find.byKey(const ValueKey('settings-section-profiles')),
      findsOneWidget,
    );
  });

  testWidgets('wrong PINs lock the pad for a while', (t) async {
    final s = await _services(kid: true);
    await _open(t, s);
    await _select(t, Section.settings);
    for (var i = 0; i < 5; i++) {
      await _tapPin(t, '9999');
    }
    expect(find.byKey(const ValueKey('pin-locked')), findsOneWidget);
    expect(s.profileControl.limiter.locked, isTrue);
    await capture(t, 'ui015-pin-locked');
    // Keys do nothing while locked; the right PIN does not open Settings.
    await _tapPin(t, '1234');
    expect(find.byType(SettingsScreen), findsNothing);
    expect(s.profileControl.limiter.failures, 5);
  });

  testWidgets('the PIN pad works with the D-pad and the remote', (t) async {
    final s = await _services(kid: true);
    await _open(t, s);
    await _select(t, Section.settings);
    // The pad focuses "1" first; walk 1 → 2 → 3 → 6 → 5 → 4 → 7 → erase
    // → 0 → OK.
    Future<void> press(LogicalKeyboardKey k) => pressKey(t, k);
    await press(LogicalKeyboardKey.select); // 1
    await press(LogicalKeyboardKey.arrowRight);
    await press(LogicalKeyboardKey.select); // 2
    await press(LogicalKeyboardKey.arrowRight);
    await press(LogicalKeyboardKey.select); // 3
    await press(LogicalKeyboardKey.arrowDown);
    await press(LogicalKeyboardKey.arrowLeft);
    await press(LogicalKeyboardKey.arrowLeft);
    await press(LogicalKeyboardKey.select); // 4
    await press(LogicalKeyboardKey.arrowDown);
    await press(LogicalKeyboardKey.arrowDown);
    await press(LogicalKeyboardKey.arrowRight);
    await press(LogicalKeyboardKey.arrowRight);
    await press(LogicalKeyboardKey.select); // OK
    await settle(t, 4);
    expect(find.byType(SettingsScreen), findsOneWidget);
  });

  testWidgets('number keys, Backspace and Enter type a PIN', (t) async {
    final s = await _services(kid: true);
    await _open(t, s);
    await _select(t, Section.settings);
    for (final k in [
      LogicalKeyboardKey.digit1,
      LogicalKeyboardKey.digit9,
      LogicalKeyboardKey.backspace,
      LogicalKeyboardKey.numpad2,
      LogicalKeyboardKey.digit3,
      LogicalKeyboardKey.digit4,
      LogicalKeyboardKey.enter,
    ]) {
      await pressKey(t, k);
    }
    expect(find.byType(SettingsScreen), findsOneWidget);
  });

  testWidgets('Streams are hidden for a kid profile by default', (t) async {
    final s = await _services(kid: true);
    await s.setShowTwitch(true);
    await _open(t, s);
    expect(_shell(t).menuSections, isNot(contains(Section.twitch)));
    expect(s.twitchVisible, isFalse);
    await _select(t, Section.twitch);
    expect(_shell(t).section, Section.home);

    // The parent allows them for this profile.
    await s.profileControl.setHideTwitch(s.profileControl.active!, false);
    await settle(t, 4);
    expect(_shell(t).menuSections, contains(Section.twitch));
  });

  testWidgets('Streams stay for an adult profile', (t) async {
    final s = await _services();
    await s.setShowTwitch(true);
    await _open(t, s);
    expect(_shell(t).menuSections, contains(Section.twitch));
  });

  testWidgets('switching to the kid profile hides Streams at once', (t) async {
    final s = await _services();
    await s.setShowTwitch(true);
    await _open(t, s);
    expect(_shell(t).menuSections, contains(Section.twitch));
    final mia = s.profileControl.all.firstWhere((p) => p.isKid);
    expect(await s.profileControl.switchTo(mia), PinCheck.ok);
    await settle(t, 6);
    expect(_shell(t).menuSections, isNot(contains(Section.twitch)));
    expect(s.profile.value?.name, 'Mia');
  });

  testWidgets('Music is hidden when the white list leaves no tracks', (
    t,
  ) async {
    // The empty library has no tracks; the kid profile white-lists folders.
    final s = await _services(kid: true, empty: true);
    await _open(t, s);
    expect(s.profileControl.musicHidden, isTrue);
    expect(_shell(t).menuSections, isNot(contains(Section.music)));
    await _select(t, Section.music);
    expect(_shell(t).section, Section.home);
  });

  testWidgets('Music stays when the chosen folders have music', (t) async {
    final s = await _services(kid: true);
    await _open(t, s);
    expect(s.profileControl.musicHidden, isFalse);
    expect(_shell(t).menuSections, contains(Section.music));
  });

  testWidgets('who is watching at start; leaving the kid profile asks the '
      'parent PIN', (t) async {
    final s = await _services(kid: true, picker: true);
    await _open(t, s);
    expect(
      find.byKey(const ValueKey('profile-picker-start')),
      findsOneWidget,
    );
    expect(find.byType(AppShell), findsNothing);
    await capture(t, 'ui015-picker');

    await t.tap(find.byKey(const ValueKey('profile-pick-2'))); // Sam
    await settle(t, 6);
    expect(find.byKey(const ValueKey('pin-dialog')), findsOneWidget);
    await _tapPin(t, '1234');
    expect(find.byType(AppShell), findsOneWidget);
    expect(s.profileControl.active!.name, 'Sam');
  });

  testWidgets('the kid picks the kid profile without a PIN', (t) async {
    final s = await _services(kid: true, picker: true);
    await _open(t, s);
    await t.tap(find.byKey(const ValueKey('profile-pick-3')));
    await settle(t, 6);
    expect(find.byKey(const ValueKey('pin-dialog')), findsNothing);
    expect(find.byType(AppShell), findsOneWidget);
  });

  testWidgets('App Lock asks for a PIN at start and after the background', (
    t,
  ) async {
    final prefs = _prefs()..values[Preferences.profilesAppLock] = 'true';
    final s = await _services(prefs: prefs);
    await _open(t, s);
    expect(find.byKey(const ValueKey('profiles-app-lock')), findsOneWidget);
    await capture(t, 'ui015-app-lock');
    await _tapPin(t, '1234');
    expect(find.byKey(const ValueKey('profiles-app-lock')), findsNothing);

    s.profileControl.onBackground();
    await settle(t, 4);
    expect(find.byKey(const ValueKey('profiles-app-lock')), findsOneWidget);
  });

  testWidgets('Search Lock asks for the parent PIN before Search', (t) async {
    final s = await _services();
    await s.profileControl.setSearchLock(true);
    s.profileControl.onBackground();
    await _open(t, s);
    await _select(t, Section.search);
    expect(
      find.byKey(const ValueKey('profiles-gate-search')),
      findsOneWidget,
    );
    expect(find.byType(SearchScreen), findsNothing);
    await _tapPin(t, '1234');
    expect(find.byType(SearchScreen), findsOneWidget);
  });

  testWidgets('a parent adds a kid profile in Settings', (t) async {
    final store = FakeProfiles.demo();
    final s = await _services(profiles: store);
    await _open(t, s);
    await _select(t, Section.settings);
    await t.tap(find.byKey(const ValueKey('settings-profiles-manage')));
    await settle(t, 6);
    // The parent PIN first.
    expect(find.byKey(const ValueKey('pin-dialog')), findsOneWidget);
    await _tapPin(t, '1234');
    expect(find.byKey(const ValueKey('profiles-screen')), findsOneWidget);
    await capture(t, 'ui015-profiles');

    await t.tap(find.byKey(const ValueKey('profiles-add-kid')));
    await settle(t, 6);
    await t.enterText(find.byKey(const ValueKey('profile-name')), 'Leo');
    await t.pump();
    await capture(t, 'ui015-kid-editor');
    final save = find.byKey(const ValueKey('profile-save'));
    await t.ensureVisible(save);
    await settle(t, 2);
    await t.tap(save);
    await settle(t, 6);
    final leo = (await store.profiles()).firstWhere((p) => p.name == 'Leo');
    expect(leo.isKid, isTrue);
    expect(leo.maxAge, 12);
    expect(leo.hideUnrated, isTrue);
    expect(leo.restrictFolders, isTrue);
    expect(s.profileControl.hidesTwitch(leo), isTrue);
  });

  testWidgets('each profile has its own search history', (t) async {
    final s = await _services();
    final adultKey = s.profileControl.searchHistoryKey;
    expect(adultKey, Preferences.searchHistory);
    final mia = s.profileControl.all.firstWhere((p) => p.isKid);
    await s.profileControl.switchTo(mia);
    expect(s.profileControl.searchHistoryKey, isNot(adultKey));
  });
}
