// SPDX-License-Identifier: MPL-2.0
//
// PLAY-007 in the app: subtitle style settings are stored and edited in
// Settings → Subtitle style (with PLAY-006's ASS override and HDR
// brightness); playVideo hands the library's sidecars to the
// player and keeps the subtitle choice and delay per show.

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/screens/playback.dart';
import 'package:kinolune/screens/subtitle_settings_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart' show PlayerScreen, PlayerScreenState;
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

/// The demo library with sidecar subtitles next to every file.
class _SubtitledLibrary extends FakeLibrary implements LibrarySubtitles {
  _SubtitledLibrary() : super.demo(scanStepDelay: Duration.zero);

  final asked = <String>[];

  @override
  Future<List<SidecarSubtitle>> sidecarSubtitles(LibraryItem item) async {
    asked.add(item.playbackUri);
    final base = item.playbackUri.replaceFirst(RegExp(r'\.\w+$'), '');
    return [
      SidecarSubtitle(
        uri: '$base.ru.forced.srt',
        name: '${base.split('/').last}.ru.forced.srt',
        language: 'rus',
        forced: true,
      ),
      SidecarSubtitle(
        uri: '$base.en.srt',
        name: '${base.split('/').last}.en.srt',
        language: 'eng',
      ),
    ];
  }
}

/// A library whose movie has subtitles for two parts (LIB-021).
class _PartsLibrary extends FakeLibrary implements LibrarySubtitles {
  _PartsLibrary() : super.demo(scanStepDelay: Duration.zero);

  @override
  Future<List<SidecarSubtitle>> sidecarSubtitles(LibraryItem item) async => [
    const SidecarSubtitle(
      uri: '/m/Heat.cd1.srt',
      name: 'Heat.cd1.srt',
      parts: ['/m/Heat.cd1.srt', '/m/Heat.cd2.srt'],
    ),
  ];
}

void main() {
  test('subtitle settings are stored and read back', () async {
    final prefs = MemoryPreferences();
    final a = AppServices(
      library: FakeLibrary(),
      secureStore: MemorySecureStore(),
      preferences: prefs,
    );
    await a.init();
    expect(a.subtitleSettings.value, const SubtitleSettings());
    const custom = SubtitleSettings(
      useSystemStyle: false,
      scale: 1.5,
      font: SubtitleFont.serif,
      position: 0.2,
    );
    await a.setSubtitleSettings(custom);
    expect(
      jsonDecode(prefs.values[Preferences.subtitleSettings]!),
      custom.toJson(),
    );

    final b = AppServices(
      library: FakeLibrary(),
      secureStore: MemorySecureStore(),
      preferences: prefs,
    );
    await b.init();
    expect(b.subtitleSettings.value, custom);

    // Broken JSON falls back to the defaults.
    prefs.values[Preferences.subtitleSettings] = '{oops';
    final c = AppServices(
      library: FakeLibrary(),
      secureStore: MemorySecureStore(),
      preferences: prefs,
    );
    await c.init();
    expect(c.subtitleSettings.value, const SubtitleSettings());
  });

  test(
    'subtitles of the parts reach the player as one track (LIB-021)',
    () async {
      final lib = _PartsLibrary();
      final item = (await lib.item('show-moonlit-bureau-s2e4'))!;
      final subs = (await sidecarSubtitles(lib, item))!;
      expect(subs.single.title, 'Heat.cd1.srt');
      expect(subs.single.uri, '/m/Heat.cd1.srt');
      expect(subs.single.parts, ['/m/Heat.cd1.srt', '/m/Heat.cd2.srt']);
    },
  );

  test('series memory is stored per show', () async {
    final prefs = MemoryPreferences();
    final s = AppServices(
      library: FakeLibrary(),
      secureStore: MemorySecureStore(),
      preferences: prefs,
    );
    expect(await s.subtitleMemory('show-1'), isNull);
    const m = SubtitleMemory(delay: Duration(milliseconds: -500));
    await s.saveSubtitleMemory('show-1', m);
    expect(await s.subtitleMemory('show-1'), m);
    expect(await s.subtitleMemory('show-2'), isNull);
  });

  testWidgets('Settings → Subtitle style edits the settings', (t) async {
    setSurface(t, const Size(1280, 800));
    final services = AppServices(
      library: FakeLibrary.demo(scanStepDelay: Duration.zero),
      secureStore: MemorySecureStore(),
    );
    await services.init();
    await services.setShowAdvancedSettings(true);
    await t.pumpWidget(testApp(services, section: Section.settings));
    await settle(t);
    await t.ensureVisible(find.text('Subtitle style').last);
    await settle(t, 4);
    await t.tap(find.text('Subtitle style').last);
    await settle(t);
    expect(find.byType(SubtitleSettingsScreen), findsOneWidget);

    // No system style on the test host: the standard look, no custom rows.
    expect(find.byKey(const ValueKey('subtitles-preview')), findsOneWidget);
    expect(find.byKey(const ValueKey('subtitles-size-0')), findsNothing);

    await t.tap(find.byKey(const ValueKey('subtitles-system')));
    await settle(t);
    expect(services.subtitleSettings.value.useSystemStyle, isFalse);

    await t.tap(find.byKey(const ValueKey('subtitles-size-4'))); // 150 %
    await settle(t);
    await t.ensureVisible(find.byKey(const ValueKey('subtitles-font-2')));
    await settle(t, 4);
    await t.tap(find.byKey(const ValueKey('subtitles-font-2'))); // monospace
    await settle(t);
    await t.ensureVisible(find.byKey(const ValueKey('subtitles-background-2')));
    await settle(t, 4);
    await t.tap(find.byKey(const ValueKey('subtitles-background-2')));
    await settle(t);
    await t.ensureVisible(find.byKey(const ValueKey('subtitles-position-up')));
    await settle(t, 4);
    await t.tap(find.byKey(const ValueKey('subtitles-position-up')));
    await settle(t);

    final s = services.subtitleSettings.value;
    expect(s.scale, 1.5);
    expect(s.font, SubtitleFont.monospace);
    expect(s.background, SubtitleBackground.opaque);
    expect(s.position, closeTo(0.10, 1e-9));

    // The preview draws with the new look.
    final text = t.widget<Text>(
      find.descendant(
        of: find.byKey(const ValueKey('subtitles-preview')),
        matching: find.byKey(const ValueKey('subtitle-text')),
      ),
    );
    expect(text.style!.fontFamily, 'monospace');
  });

  testWidgets('Subtitle style: ASS override and HDR brightness (PLAY-006)', (
    t,
  ) async {
    setSurface(t, const Size(1280, 800));
    final prefs = MemoryPreferences();
    final services = AppServices(
      library: FakeLibrary.demo(scanStepDelay: Duration.zero),
      secureStore: MemorySecureStore(),
      preferences: prefs,
    );
    await services.init();
    await services.setShowAdvancedSettings(true);
    await t.pumpWidget(testApp(services, section: Section.settings));
    await settle(t);
    await t.ensureVisible(find.text('Subtitle style').last);
    await settle(t, 4);
    await t.tap(find.text('Subtitle style').last);
    await settle(t);

    // Off by default: ASS keeps its own look; the hint says so.
    expect(services.subtitleSettings.value.overrideAssStyle, isFalse);
    final override = find.byKey(const ValueKey('subtitles-override-ass'));
    await t.ensureVisible(override);
    await settle(t, 4);
    expect(find.textContaining('keep their own fonts'), findsOneWidget);
    await t.tap(override);
    await settle(t);
    expect(services.subtitleSettings.value.overrideAssStyle, isTrue);
    expect(find.textContaining('as plain text'), findsOneWidget);

    final nits300 = find.byKey(const ValueKey('subtitles-hdr-2'));
    await t.ensureVisible(nits300);
    await settle(t, 4);
    expect(find.text('203 nits'), findsOneWidget);
    await t.tap(nits300);
    await settle(t);
    final s = services.subtitleSettings.value;
    expect(s.hdrPeakNits, 300);
    expect(
      s.renderOptions,
      const SubtitleRenderOptions(overrideAssStyle: true, hdrPeakNits: 300),
    );
    // Stored with the rest of the style.
    final stored = jsonDecode(prefs.values[Preferences.subtitleSettings]!);
    expect(stored['assOverride'], isTrue);
    expect(stored['hdrPeak'], 300);
  });

  testWidgets('playVideo passes sidecars and keeps the choice per show', (
    t,
  ) async {
    setSurface(t, const Size(1280, 800));
    final engine = FakePlayerEngine(
      loadedDuration: const Duration(minutes: 40),
    );
    final library = _SubtitledLibrary();
    final services = AppServices(
      library: library,
      secureStore: MemorySecureStore(),
      playerEngineFactory: ({bool video = true}) => engine,
    );
    await services.init();
    await t.pumpWidget(testApp(services));
    await settle(t);

    const id = 'show-moonlit-bureau-s2e4';
    final item = (await library.item(id))!;
    final context = t.element(find.byType(Scaffold).first);
    playVideo(context, item);
    await settle(t);
    expect(find.byType(PlayerScreen), findsOneWidget);
    // The next episode's are fetched too, for Up Next.
    expect(library.asked.first, item.playbackUri);
    expect(engine.autoLoadSubtitles, isFalse);
    expect([for (final s in engine.addedSubtitles) s.language], ['rus', 'eng']);

    // The user switches to English and shifts them 0.5 s later.
    final player = t.state<PlayerScreenState>(find.byType(PlayerScreen));
    final english = engine.state.value.subtitleTracks.firstWhere(
      (s) => s.language == 'eng',
    );
    await player.selectSubtitle(english);
    await player.setSubtitleDelay(const Duration(milliseconds: 500));
    await t.pump();

    final memory = await services.subtitleMemory(item.showId!);
    expect(memory!.choice!.language, 'eng');
    expect(memory.delay, const Duration(milliseconds: 500));

    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });
}
