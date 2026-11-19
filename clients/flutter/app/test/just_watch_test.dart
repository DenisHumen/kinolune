// SPDX-License-Identifier: MPL-2.0
//
// UI-047: the "just watch" mode of a profile. A profile in the mode sees
// one screen of large cards (Continue, One more episode, favourites) with
// no navigation, settings, search or menus; leaving it needs the PIN of an
// adult profile. In-memory profiles (FakeProfiles.demo: Alex with PIN 1234,
// Sam, kid Mia). With KINOLUNE_SCREENSHOTS_DIR set the frames are written
// to docs/screenshots/progress/ui-047:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-047 \
//     flutter test test/just_watch_test.dart

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_profiles.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/profiles/just_watch_screen.dart';
import 'package:kinolune/profiles/profile_controller.dart';
import 'package:kinolune/profiles/profiles_screen.dart';
import 'package:kinolune/screens/search_screen.dart';
import 'package:kinolune/screens/settings_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

Finder _key(String k) => find.byKey(ValueKey(k));

/// Preferences without the start picker; Sam in the mode when [samMode].
MemoryPreferences _prefs({bool samMode = true}) {
  final p = MemoryPreferences();
  p.values[Preferences.profilesPickerOnStart] = 'false';
  if (samMode) p.values[Preferences.profileJustWatch('p-sam')] = 'true';
  return p;
}

LibraryProfile _named(ProfileController c, String name) =>
    c.all.firstWhere((p) => p.name == name);

/// Demo services with two favourites, Sam active and in the mode.
Future<AppServices> _services({bool samMode = true}) async {
  final library = FakeLibrary.demo(scanStepDelay: Duration.zero);
  final s = await demoServices(
    library: library,
    preferences: _prefs(samMode: samMode),
    profiles: FakeProfiles.demo(),
  );
  final movies = await library.movies().first;
  final shows = await library.shows().first;
  await library.setFavorite(movies.first.id, true);
  if (shows.isNotEmpty) await library.setFavorite(shows.first.id, true);
  final c = s.profileControl;
  expect(await c.switchTo(_named(c, 'Sam')), PinCheck.ok);
  return s;
}

Future<void> _open(
  WidgetTester t,
  AppServices s, {
  FormFactor ff = FormFactor.phone,
}) async {
  if (ff == FormFactor.tv) {
    setSurface(t, const Size(960, 540), dpr: 2);
  } else if (ff == FormFactor.phone) {
    setSurface(t, const Size(390, 844), dpr: 2);
  } else {
    setSurface(t, const Size(1280, 800));
  }
  await t.pumpWidget(testApp(s, formFactor: ff));
  await settle(t);
}

Future<void> _tapPin(WidgetTester t, String pin) async {
  for (final d in pin.split('')) {
    await t.tap(find.byKey(ValueKey('pin-key-$d')).last);
    await t.pump();
  }
  await t.tap(find.byKey(const ValueKey('pin-key-ok')).last);
  await settle(t, 4);
}

/// Nothing of the full app is on screen.
void _expectNoChrome() {
  expect(find.byType(JustWatchScreen), findsOneWidget);
  expect(find.byType(AppShell), findsNothing);
  expect(find.byType(SettingsScreen), findsNothing);
  expect(find.byType(SearchScreen), findsNothing);
  expect(find.byType(NavigationBar), findsNothing);
  expect(find.byType(NavigationRail), findsNothing);
  expect(find.byType(Drawer), findsNothing);
  expect(find.byType(PopupMenuButton<Object>), findsNothing);
  expect(_key('nav-profile'), findsNothing);
}

void main() {
  setUpAll(loadAppFonts);

  group('ProfileController', () {
    Future<ProfileController> ctl({bool samMode = false}) async {
      final c = ProfileController(
        store: FakeProfiles.demo(),
        preferences: _prefs(samMode: samMode),
      );
      await c.init();
      return c;
    }

    test('the mode is kept per profile and leaving it needs the parent '
        'PIN', () async {
      final c = await ctl(samMode: true);
      final sam = _named(c, 'Sam');
      final alex = _named(c, 'Alex');
      final mia = _named(c, 'Mia');
      expect(c.justWatch(sam), isTrue);
      expect(c.justWatch(alex), isFalse);
      expect(await c.switchTo(sam), PinCheck.ok);
      expect(c.justWatchActive, isTrue);
      // Any other profile: the parent PIN first (Alex: his own PIN, which
      // is the parent PIN, once).
      expect(c.needsFor(mia).parentPin, isTrue);
      expect(c.needsFor(alex).parentPin, isFalse);
      expect(c.needsFor(alex).profilePin, isTrue);
      expect(await c.switchTo(alex, pin: '0000'), PinCheck.wrong);
      expect(c.justWatchActive, isTrue);
      expect(await c.switchTo(alex, pin: '1234'), PinCheck.ok);
      expect(c.justWatchActive, isFalse);
    });

    test('the mode needs a parent PIN on another profile and keeps none of '
        'its own', () async {
      final c = await ctl();
      final alex = _named(c, 'Alex');
      final sam = _named(c, 'Sam');
      // Alex's PIN is the only parent PIN: he cannot be in the mode.
      expect(c.needsParentPinForJustWatch(id: alex.id), isTrue);
      await expectLater(
        c.update(alex.id, alex.settings, justWatch: true),
        throwsA(
          isA<ProfileChangeException>().having(
            (e) => e.error,
            'error',
            ProfileChangeError.needsParentPin,
          ),
        ),
      );
      expect(c.justWatch(c.all.firstWhere((p) => p.id == alex.id)), isFalse);

      // Sam with a PIN: the mode removes it (the core would take it for a
      // parent PIN that opens the mode).
      await c.setPin(sam.id, '5555');
      expect(_named(c, 'Sam').hasPin, isTrue);
      await c.update(sam.id, sam.settings, justWatch: true);
      expect(c.justWatch(_named(c, 'Sam')), isTrue);
      expect(_named(c, 'Sam').hasPin, isFalse);
      await expectLater(
        c.setPin(sam.id, '5555'),
        throwsA(
          isA<ProfileChangeException>().having(
            (e) => e.error,
            'error',
            ProfileChangeError.justWatchPin,
          ),
        ),
      );
      // The last parent PIN stays while a profile is in the mode.
      await expectLater(
        c.setPin(alex.id, null),
        throwsA(isA<ProfileChangeException>()),
      );

      // Off again: nothing stays.
      await c.update(sam.id, sam.settings, justWatch: false);
      expect(c.justWatch(_named(c, 'Sam')), isFalse);
    });

    test('a new profile can start in the mode', () async {
      final c = await ctl();
      final p = await c.create(
        const ProfileSettings(name: 'Grandma'),
        justWatch: true,
      );
      expect(c.justWatch(p), isTrue);
      expect(p.hasPin, isFalse);
    });
  });

  for (final ff in [FormFactor.phone, FormFactor.tv, FormFactor.desktop]) {
    final name = ff.name;

    testWidgets('$name: only large cards, no settings or menus', (t) async {
      final s = await _services();
      await _open(t, s, ff: ff);
      _expectNoChrome();
      expect(_key('just-watch-continue'), findsOneWidget);
      expect(find.byKey(justWatchExitKey), findsOneWidget);
      if (ff != FormFactor.desktop) {
        await capture(t, '$name-just-watch');
        s.locale.value = const Locale('ru');
        await settle(t, 4);
        await capture(t, '$name-just-watch-ru');
        s.locale.value = null;
        await settle(t, 4);
      }
      await t.scrollUntilVisible(
        _key('just-watch-favorites'),
        200,
        scrollable: find
            .descendant(
              of: find.byType(JustWatchScreen),
              matching: find.byType(Scrollable),
            )
            .first,
      );
      await settle(t, 2);
      expect(_key('just-watch-favorites'), findsOneWidget);

      // A long press or the menu key opens no item menu.
      final card = find.byWidgetPredicate(
        (w) =>
            w.key is ValueKey<String> &&
            (w.key! as ValueKey<String>).value.startsWith('just-watch-fav-'),
      );
      await t.longPress(card.first);
      await settle(t, 4);
      expect(find.byType(PopupMenuItem<Object>), findsNothing);
      expect(find.byType(BottomSheet), findsNothing);
      _expectNoChrome();
    });

    testWidgets('$name: leaving needs the adult PIN', (t) async {
      final s = await _services();
      await _open(t, s, ff: ff);
      await t.tap(find.byKey(justWatchExitKey));
      await settle(t, 6);
      expect(_key('profile-picker'), findsOneWidget);

      // Kid Mia: the parent PIN; a wrong one keeps the mode.
      await t.tap(_key('profile-pick-3'));
      await settle(t, 6);
      expect(_key('pin-dialog'), findsOneWidget);
      await _tapPin(t, '0000');
      expect(_key('pin-error'), findsOneWidget);
      if (ff != FormFactor.desktop) await capture(t, '$name-just-watch-pin');
      await t.tap(_key('pin-cancel').last);
      await settle(t, 6);
      expect(s.profileControl.active!.name, 'Sam');

      // Back from the picker: still the simple screen.
      await t.pageBack();
      await settle(t, 6);
      _expectNoChrome();

      // Alex with his PIN: the full app again.
      await t.tap(find.byKey(justWatchExitKey));
      await settle(t, 6);
      await t.tap(_key('profile-pick-1'));
      await settle(t, 6);
      await _tapPin(t, '1234');
      await settle(t, 6);
      expect(s.profileControl.active!.name, 'Alex');
      expect(find.byType(JustWatchScreen), findsNothing);
      expect(find.byType(AppShell), findsOneWidget);
    });
  }

  testWidgets('the switch in the profile editor turns the mode on', (t) async {
    final s = await demoServices(
      preferences: _prefs(samMode: false),
      profiles: FakeProfiles.demo(),
    );
    final c = s.profileControl;
    final sam = _named(c, 'Sam');
    // The owner's language in the screenshot.
    s.locale.value = const Locale('ru');
    await _open(t, s);
    expect(find.byType(AppShell), findsOneWidget);
    final context = t.element(find.byType(Scaffold).first);
    unawaited(
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ProfileEditorScreen(profile: sam),
        ),
      ),
    );
    await settle(t, 8);
    expect(_key('profile-section-pin'), findsOneWidget);
    await t.ensureVisible(_key('profile-just-watch'));
    await settle(t, 2);
    await t.tap(_key('profile-just-watch'));
    await settle(t, 4);
    // Its own PIN is gone from the editor.
    expect(_key('profile-section-pin'), findsNothing);
    expect(_key('profile-just-watch-no-pin'), findsOneWidget);
    await t.ensureVisible(_key('profile-just-watch'));
    await settle(t, 2);
    await capture(t, 'phone-editor-ru');
    await t.ensureVisible(_key('profile-save'));
    await settle(t, 2);
    await t.tap(_key('profile-save'));
    await settle(t, 8);
    expect(c.justWatch(_named(c, 'Sam')), isTrue);
    // The list marks it.
    final en = lookupAppLocalizations(const Locale('en'));
    expect(
      profileSummary(en, _named(c, 'Sam'), justWatch: true),
      contains(en.profilesJustWatch),
    );
  });
}
