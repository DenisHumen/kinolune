// SPDX-License-Identifier: MPL-2.0
//
// PLAY-AND-004: «Звук: авто / всегда декодировать» — kept, shown on
// Android (where the player can pass Dolby and DTS to a receiver) only.

import 'package:flutter/foundation.dart';
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

const _tile = ValueKey('settings-audio-output');

void main() {
  test('auto by default, «всегда декодировать» is kept', () async {
    final prefs = MemoryPreferences();
    final s = await _services(prefs);
    expect(s.audioOutputMode.value, AudioOutputMode.auto);
    expect(prefs.values[Preferences.audioOutputMode], isNull);

    await s.setAudioOutputMode(AudioOutputMode.decode);
    expect(prefs.values[Preferences.audioOutputMode], 'decode');
    expect(
      (await _services(prefs)).audioOutputMode.value,
      AudioOutputMode.decode,
    );

    // Back to the default: nothing stored.
    await s.setAudioOutputMode(AudioOutputMode.auto);
    expect(prefs.values[Preferences.audioOutputMode], isNull);
  });

  test('an unknown stored value is auto', () async {
    final prefs = MemoryPreferences();
    await prefs.write(Preferences.audioOutputMode, 'bitstream');
    final s = await _services(prefs);
    expect(s.audioOutputMode.value, AudioOutputMode.auto);
  });

  for (final tv in [false, true]) {
    testWidgets('on Android ${tv ? 'TV' : 'phone'}: the tile switches to '
        '«всегда декодировать»', variant: TargetPlatformVariant.only(
      TargetPlatform.android,
    ), (t) async {
      final prefs = MemoryPreferences();
      final s = await _services(prefs);
      setSurface(t, tv ? const Size(1920, 1080) : const Size(412, 915));
      await t.pumpWidget(
        testApp(
          s,
          section: Section.settings,
          formFactor: tv ? FormFactor.tv : FormFactor.phone,
        ),
      );
      await settle(t, 8);
      expect(await _shows(t, _tile), isTrue);
      expect(find.text('Auto (passthrough)'), findsOneWidget);
      await t.ensureVisible(find.byKey(_tile));
      await settle(t, 3);
      await t.tap(find.byKey(_tile));
      await settle(t, 4);
      await t.tap(find.byKey(const ValueKey('settings-audio-output-1')));
      await settle(t, 6);
      expect(s.audioOutputMode.value, AudioOutputMode.decode);
      expect(prefs.values[Preferences.audioOutputMode], 'decode');
      expect(find.text('Always decode'), findsOneWidget);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }

  testWidgets('not on a desktop', variant: TargetPlatformVariant.only(
    TargetPlatform.linux,
  ), (t) async {
    final s = await _services(MemoryPreferences());
    setSurface(t, const Size(1280, 800));
    await t.pumpWidget(testApp(s, section: Section.settings));
    await settle(t, 8);
    expect(await _shows(t, _tile), isFalse);
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });
}
