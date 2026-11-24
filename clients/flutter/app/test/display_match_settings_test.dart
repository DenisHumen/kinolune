// SPDX-License-Identifier: MPL-2.0
//
// PLAY-AND-005: the TV display settings of the player — «Частота кадров»
// and «Разрешение как у видео» — are kept and shown on TV only.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/settings_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

Future<AppServices> _services(MemoryPreferences prefs) async {
  final s = AppServices(
    library: FakeLibrary.demo(scanStepDelay: Duration.zero),
    secureStore: MemorySecureStore(),
    preferences: prefs,
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    clock: () => demoNow,
  );
  await s.init();
  await s.setShowAdvancedSettings(true);
  return s;
}

Finder _page() => find
    .descendant(
      of: find.byType(SettingsScreen),
      matching: find.byType(Scrollable),
    )
    .first;

Future<bool> _shows(WidgetTester t, Key key) async {
  final f = find.byKey(key);
  if (f.evaluate().isEmpty) {
    try {
      await t.scrollUntilVisible(f, 200, scrollable: _page(), maxScrolls: 80);
    } on StateError {
      return false;
    }
  }
  return f.evaluate().isNotEmpty;
}

void main() {
  test('defaults and persistence', () async {
    final prefs = MemoryPreferences();
    final s = await _services(prefs);
    expect(s.displayMatch.value, const DisplayMatchSettings());
    expect(prefs.values[Preferences.displayMatch], isNull);

    await s.setDisplayMatch(
      const DisplayMatchSettings(frameRate: FrameRateMatch.off),
    );
    expect(prefs.values[Preferences.displayMatch], isNotNull);
    final again = await _services(prefs);
    expect(again.displayMatch.value.frameRate, FrameRateMatch.off);

    // Back to the defaults: nothing stored.
    await again.setDisplayMatch(const DisplayMatchSettings());
    expect(prefs.values[Preferences.displayMatch], isNull);
  });

  test('a broken stored value falls back to the defaults', () async {
    final prefs = MemoryPreferences();
    await prefs.write(Preferences.displayMatch, '{not json');
    final s = await _services(prefs);
    expect(s.displayMatch.value, const DisplayMatchSettings());
  });

  testWidgets('on TV: frame rate and resolution tiles, switch is kept', (
    t,
  ) async {
    final prefs = MemoryPreferences();
    final s = await _services(prefs);
    setSurface(t, const Size(1920, 1080));
    await t.pumpWidget(
      testApp(s, section: Section.settings, formFactor: FormFactor.tv),
    );
    await settle(t, 8);
    expect(await _shows(t, const ValueKey('settings-frame-rate')), isTrue);
    final resolution = find.byKey(const ValueKey('settings-resolution-match'));
    expect(
      await _shows(t, const ValueKey('settings-resolution-match')),
      isTrue,
    );
    await t.ensureVisible(resolution);
    await settle(t, 3);
    await t.tap(resolution);
    await settle(t, 6);
    expect(s.displayMatch.value.resolution, isTrue);
    expect((await _services(prefs)).displayMatch.value.resolution, isTrue);
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  testWidgets('not on a desktop', (t) async {
    final s = await _services(MemoryPreferences());
    setSurface(t, const Size(1280, 800));
    await t.pumpWidget(testApp(s, section: Section.settings));
    await settle(t, 8);
    expect(await _shows(t, const ValueKey('settings-frame-rate')), isFalse);
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });
}
