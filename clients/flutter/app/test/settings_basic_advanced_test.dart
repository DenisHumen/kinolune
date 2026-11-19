// SPDX-License-Identifier: MPL-2.0
//
// UI-041: Settings show only the basic sections on a fresh install; the
// «Показать расширенные настройки» switch (remembered) adds the advanced
// ones; the search finds advanced sections with the switch off; the TV
// reaches the switch with the remote. With KINOLUNE_SCREENSHOTS_DIR set the
// frames are written to docs/screenshots/progress/ui-041:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-041 \
//     flutter test test/settings_basic_advanced_test.dart

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/settings_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final _en = lookupAppLocalizations(const Locale('en'));
final _ru = lookupAppLocalizations(const Locale('ru'));

const _basic = {
  'profiles',
  'library',
  'language',
  'devices',
  'downloads',
  'about',
};

const _advanced = [
  'playback',
  'shows',
  'trailers',
  'music',
  'metadata',
  'network',
  'twitch',
  'storage',
  'privacy',
  'diagnostics',
];

/// Every `settings-section-*` on the page, built or not yet scrolled to.
Set<String> _sections() => {
  for (final e in find
      .byWidgetPredicate(
        (w) =>
            w.key is ValueKey<String> &&
            (w.key! as ValueKey<String>).value.startsWith('settings-section-'),
        skipOffstage: false,
      )
      .evaluate())
    (e.widget.key! as ValueKey<String>).value.substring(
      'settings-section-'.length,
    ),
};

Finder _section(String name) =>
    find.byKey(ValueKey('settings-section-$name'), skipOffstage: false);

Finder get _switch =>
    find.byKey(const ValueKey('settings-advanced'), skipOffstage: false);

Future<void> _open(
  WidgetTester t,
  AppServices s, {
  FormFactor ff = FormFactor.desktop,
  Size size = const Size(1280, 800),
  double dpr = 1,
}) async {
  setSurface(t, size, dpr: dpr);
  await t.pumpWidget(testApp(s, formFactor: ff, section: Section.settings));
  await settle(t, 8);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

Future<void> _tapSwitch(WidgetTester t) async {
  await t.ensureVisible(_switch);
  await settle(t, 3);
  await t.tap(_switch);
  await settle(t, 6);
}

void main() {
  setUpAll(loadAppFonts);

  testWidgets('fresh install: at most 7 basic sections, advanced hidden', (
    t,
  ) async {
    final prefs = MemoryPreferences();
    final s = await demoServices(preferences: prefs);
    await _open(t, s);
    final shown = _sections();
    expect(shown.length, lessThanOrEqualTo(7), reason: '$shown');
    expect(shown.difference(_basic), isEmpty, reason: '$shown');
    for (final name in ['library', 'language', 'devices', 'about']) {
      expect(shown, contains(name));
    }
    for (final name in _advanced) {
      expect(_section(name), findsNothing, reason: name);
    }
    expect(_switch, findsOneWidget);
    expect(s.showAdvancedSettings.value, isFalse);
    // Audio and subtitle languages are basic, next to the UI language.
    expect(
      find.descendant(
        of: _section('language'),
        matching: find.byKey(
          const ValueKey('settings-languages-sound'),
          skipOffstage: false,
        ),
      ),
      findsOneWidget,
    );
    await _close(t);
  });

  testWidgets('the switch shows the advanced sections and is remembered', (
    t,
  ) async {
    final prefs = MemoryPreferences();
    final s = await demoServices(preferences: prefs);
    await _open(t, s);
    await _tapSwitch(t);
    expect(s.showAdvancedSettings.value, isTrue);
    expect(await prefs.read(Preferences.settingsAdvanced), 'true');
    for (final name in _advanced) {
      expect(_section(name), findsOneWidget, reason: name);
    }
    // The advanced sections come after the switch.
    final switchY = t.getTopLeft(_switch).dy;
    expect(t.getTopLeft(_section('playback')).dy, greaterThan(switchY));
    expect(t.getTopLeft(_section('about')).dy, lessThan(switchY));
    await _close(t);

    // A new start with the same preferences keeps them open.
    final again = await demoServices(preferences: prefs);
    expect(again.showAdvancedSettings.value, isTrue);
    await _open(t, again);
    expect(_section('diagnostics'), findsOneWidget);
    await _tapSwitch(t);
    expect(_section('diagnostics'), findsNothing);
    expect(await prefs.read(Preferences.settingsAdvanced), isNull);
    await _close(t);
  });

  testWidgets('search finds an advanced setting with the switch off', (
    t,
  ) async {
    final s = await demoServices(preferences: MemoryPreferences());
    await _open(t, s);
    final field = find.byKey(const ValueKey('settings-search'));
    await t.enterText(field, _en.settingsImageCache.toLowerCase());
    await settle(t, 4);
    expect(_section('storage'), findsOneWidget);
    expect(_section('library'), findsNothing);
    expect(_switch, findsNothing);
    expect(s.showAdvancedSettings.value, isFalse);

    // Another advanced row, by a word of its title.
    await t.enterText(field, 'RSA');
    await settle(t, 4);
    expect(_section('network'), findsOneWidget);
    expect(find.byKey(const ValueKey('settings-ssh-rsa-keys')), findsOneWidget);

    // Basic sections are found too.
    await t.enterText(field, _en.settingsLanguagesSound);
    await settle(t, 4);
    expect(_section('language'), findsOneWidget);

    await t.enterText(field, 'zzzz-no-such-setting');
    await settle(t, 4);
    expect(find.byKey(const ValueKey('settings-search-empty')), findsOneWidget);
    expect(_sections(), isEmpty);

    // Clearing the field brings the basic page back.
    await t.tap(find.byKey(const ValueKey('settings-search-clear')));
    await settle(t, 4);
    expect(_section('library'), findsOneWidget);
    expect(_section('storage'), findsNothing);
    expect(_switch, findsOneWidget);
    await _close(t);
  });

  testWidgets('TV: the remote reaches the switch and turns it on', (t) async {
    final s = await demoServices(preferences: MemoryPreferences());
    await _open(t, s, ff: FormFactor.tv, size: const Size(1920, 1080));
    expect(_sections().length, lessThanOrEqualTo(7));
    bool onSwitch() {
      final ctx = FocusManager.instance.primaryFocus?.context;
      if (ctx == null) return false;
      return find
          .descendant(of: _switch, matching: find.byWidget(ctx.widget))
          .evaluate()
          .isNotEmpty;
    }

    // From the search field (↓ leaves it) down to the switch.
    await t.tap(find.byKey(const ValueKey('settings-search')));
    await settle(t, 3);
    for (var i = 0; i < 60 && !onSwitch(); i++) {
      await pressKey(t, LogicalKeyboardKey.arrowDown);
    }
    expect(onSwitch(), isTrue, reason: 'focus never reached the switch');
    await pressKey(t, LogicalKeyboardKey.select);
    await settle(t, 6);
    expect(s.showAdvancedSettings.value, isTrue);
    expect(_section('playback'), findsOneWidget);
    await _close(t);
  });

  for (final (name, ff, size, dpr) in <(String, FormFactor, Size, double)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800), 1),
    ('phone', FormFactor.phone, const Size(390, 844), 2),
    ('tv', FormFactor.tv, const Size(1920, 1080), 1),
  ]) {
    testWidgets('screenshots on $name', (t) async {
      final s = await demoServices(preferences: MemoryPreferences());
      s.locale.value = const Locale('ru');
      await _open(t, s, ff: ff, size: size, dpr: dpr);
      expect(find.text(_ru.settingsAdvancedShow), findsOneWidget);
      await capture(t, '01-basic-$name');
      await t.ensureVisible(_switch);
      await settle(t, 4);
      await capture(t, '02-basic-bottom-$name');
      await _tapSwitch(t);
      await t.ensureVisible(find.byKey(const ValueKey('settings-advanced-header')));
      await settle(t, 4);
      await capture(t, '03-advanced-$name');
      // Off again: the search still finds the advanced image cache.
      await _tapSwitch(t);
      final scroll = find.descendant(
        of: find.byType(SettingsScreen),
        matching: find.byType(Scrollable),
      );
      t.state<ScrollableState>(scroll.first).position.jumpTo(0);
      await settle(t, 4);
      await t.enterText(
        find.byKey(const ValueKey('settings-search')),
        _ru.settingsImageCache.toLowerCase(),
      );
      await settle(t, 6);
      expect(_section('storage'), findsOneWidget);
      await capture(t, '04-search-$name');
      await _close(t);
    });
  }
}
