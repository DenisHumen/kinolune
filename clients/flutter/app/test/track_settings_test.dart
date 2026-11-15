// SPDX-License-Identifier: MPL-2.0
//
// PLAY-004 in the app: the track policy and sound processing are kept per
// profile and edited in Settings → Playback → Languages and sound;
// playVideo hands the player the policy, the sound processing and the
// title's original language, and the series memory (audio and subtitles)
// is kept per profile.
//
// With KINOLUNE_SCREENSHOTS_DIR set the frames are written there:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/play-004 \
//     flutter test test/track_settings_test.dart

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_profiles.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/playback.dart';
import 'package:kinolune/screens/track_settings_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart'
    show MediaTrack, OsdPanel, PlayerScreen, PlayerScreenState;
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

/// The demo library that knows the original language of its titles.
class _OriginalLibrary extends FakeLibrary implements LibraryOriginalLanguage {
  _OriginalLibrary() : super.demo(scanStepDelay: Duration.zero);

  final asked = <String>[];

  @override
  Future<String?> originalLanguage(String itemId) async {
    asked.add(itemId);
    return 'en';
  }
}

Future<void> _openTrackSettings(WidgetTester t) async {
  final tile = find.byKey(const ValueKey('settings-languages-sound'));
  await t.ensureVisible(tile);
  await settle(t, 4);
  await t.tap(tile);
  await settle(t);
  expect(find.byType(TrackSettingsScreen), findsOneWidget);
}

Future<void> _choose(WidgetTester t, String key, int option) async {
  final tile = find.byKey(ValueKey(key));
  await t.ensureVisible(tile);
  await settle(t, 4);
  await t.tap(tile);
  await settle(t);
  await t.tap(find.byKey(ValueKey('$key-$option')));
  await settle(t);
}

void main() {
  test('policy and sound are stored per profile', () async {
    final prefs = MemoryPreferences();
    final profiles = FakeProfiles.demo();
    final s = AppServices(
      library: FakeLibrary(),
      secureStore: MemorySecureStore(),
      preferences: prefs,
      profileStore: profiles,
    );
    await s.init();
    expect(s.trackPolicy.value, TrackPolicy());
    expect(s.audioProcessing.value, const AudioProcessing());

    final original = TrackPolicy(
      audio: LanguagePreference(mode: LanguageMode.original),
      subtitleMode: SubtitleMode.always,
    );
    await s.setTrackPolicy(original);
    await s.setAudioProcessing(const AudioProcessing(boost: 200));
    await s.saveSubtitleMemory(
      'show-1',
      const TrackMemory(audio: AudioChoice(language: 'eng')),
    );
    // The default profile keeps the PLAY-007 key of the series memory.
    expect(prefs.values.keys, contains('subtitle_series_show-1'));
    expect(
      jsonDecode(prefs.values[Preferences.trackPolicy('default')]!),
      original.toJson(),
    );

    // Another profile starts with its own (default) settings and memory.
    final sam = (await profiles.profiles()).firstWhere(
      (p) => p.syncId == 'p-sam',
    );
    await s.profileControl.switchTo(sam);
    await pumpEventQueue();
    expect(s.trackPolicy.value, TrackPolicy());
    expect(s.audioProcessing.value, const AudioProcessing());
    expect(await s.subtitleMemory('show-1'), isNull);
    await s.setAudioProcessing(const AudioProcessing(lfeToStereo: true));
    expect(prefs.values.keys, contains(Preferences.audioProcessing('p-sam')));

    // A restart reads the active profile's settings back.
    final again = AppServices(
      library: FakeLibrary(),
      secureStore: MemorySecureStore(),
      preferences: prefs,
      profileStore: profiles,
    );
    await again.init();
    expect(again.audioProcessing.value.lfeToStereo, isTrue);
  });

  testWidgets('Settings → Languages and sound edits the policy', (t) async {
    setSurface(t, const Size(1280, 800));
    final services = AppServices(
      library: FakeLibrary.demo(scanStepDelay: Duration.zero),
      secureStore: MemorySecureStore(),
    );
    await services.init();
    await t.pumpWidget(testApp(services, section: Section.settings));
    await settle(t);
    await _openTrackSettings(t);

    await _choose(t, 'tracks-audio-mode', 1); // Original
    expect(services.trackPolicy.value.audio.mode, LanguageMode.original);
    await _choose(t, 'tracks-audio-mode', 2); // Chosen languages
    final list = find.byKey(const ValueKey('tracks-audio-languages'));
    await t.ensureVisible(list);
    await settle(t, 4);
    expect(find.text('None chosen'), findsOneWidget);
    await t.tap(list);
    await settle(t);
    await t.tap(find.byKey(const ValueKey('tracks-language-jpn')));
    await t.pump();
    await t.tap(find.byKey(const ValueKey('tracks-language-eng')));
    await t.pump();
    await t.tap(find.byKey(const ValueKey('tracks-languages-done')));
    await settle(t);
    expect(services.trackPolicy.value.audio.languages, ['jpn', 'eng']);
    expect(find.text('日本語, English'), findsOneWidget);

    await _choose(t, 'tracks-subtitle-when', 2); // Forced only
    expect(services.trackPolicy.value.subtitleMode, SubtitleMode.forcedOnly);
    await _choose(t, 'tracks-sdh', 1); // Prefer
    expect(services.trackPolicy.value.sdh, SdhPreference.prefer);

    await _choose(t, 'tracks-downmix', 1); // Always stereo
    final lfe = find.byKey(const ValueKey('tracks-lfe'));
    await t.ensureVisible(lfe);
    await settle(t, 4);
    await t.tap(lfe);
    await settle(t);
    await _choose(t, 'tracks-boost', 5); // 300 %
    final a = services.audioProcessing.value;
    expect(a.downmix, DownmixMode.stereo);
    expect(a.lfeToStereo, isTrue);
    expect(a.boost, 300);
    expect(find.text('300%'), findsOneWidget);
  });

  testWidgets('playVideo hands over the policy, sound and original language', (
    t,
  ) async {
    setSurface(t, const Size(1280, 800));
    final engine = FakePlayerEngine(loadedDuration: const Duration(minutes: 40))
      ..openAudioTracks = const [
        MediaTrack(id: '1', language: 'rus', title: 'Дубляж'),
        MediaTrack(id: '2', language: 'eng', isDefault: true),
      ];
    final library = _OriginalLibrary();
    final services = AppServices(
      library: library,
      secureStore: MemorySecureStore(),
      playerEngineFactory: ({bool video = true}) => engine,
    );
    await services.init();
    await services.setTrackPolicy(
      TrackPolicy(audio: LanguagePreference(mode: LanguageMode.original)),
    );
    await services.setAudioProcessing(const AudioProcessing(boost: 150));
    await t.pumpWidget(testApp(services));
    await settle(t);

    const id = 'show-moonlit-bureau-s2e4';
    final item = (await library.item(id))!;
    final context = t.element(find.byType(Scaffold).first);
    playVideo(context, item);
    await settle(t, 30);
    final player = t.widget<PlayerScreen>(find.byType(PlayerScreen));
    expect(player.request.originalLanguage, 'en');
    expect(library.asked, contains(id));
    // Original: the English track, not the Russian dub.
    expect(engine.state.value.audioTrack!.id, '2');
    expect(engine.audioProcessing!.boost, 150);

    // The quick toggle in the OSD changes the profile's setting.
    final state = t.state<PlayerScreenState>(find.byType(PlayerScreen));
    state.changeAudioProcessing(
      services.audioProcessing.value.copyWith(downmix: DownmixMode.stereo),
    );
    await settle(t, 2);
    expect(services.audioProcessing.value.downmix, DownmixMode.stereo);
    expect(engine.audioProcessing!.downmix, DownmixMode.stereo);

    // A picked track is remembered for the show.
    await state.selectAudio(engine.state.value.audioTracks.first);
    await t.pump();
    final memory = await services.subtitleMemory(item.showId!);
    expect(memory!.audio!.language, 'rus');

    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  group('screenshots', () {
    setUpAll(loadAppFonts);

    for (final (name, ff, size) in <(String, FormFactor, Size)>[
      ('desktop', FormFactor.desktop, const Size(1280, 800)),
      ('phone', FormFactor.phone, const Size(390, 844)),
    ]) {
      testWidgets('languages and sound on $name', (t) async {
        setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
        final services = await demoServices();
        services.locale.value = const Locale('ru');
        await services.setTrackPolicy(
          TrackPolicy(
            audio: LanguagePreference(
              mode: LanguageMode.custom,
              languages: ['eng', 'rus'],
            ),
            subtitles: LanguagePreference(mode: LanguageMode.original),
          ),
        );
        await services.setAudioProcessing(
          const AudioProcessing(
            downmix: DownmixMode.stereo,
            lfeToStereo: true,
            boost: 200,
          ),
        );
        await t.pumpWidget(
          testApp(services, formFactor: ff, section: Section.settings),
        );
        await settle(t);
        await _openTrackSettings(t);
        await capture(t, '0${name == 'desktop' ? 1 : 3}-settings-$name');
        if (name == 'desktop') {
          await t.tap(find.byKey(const ValueKey('tracks-audio-languages')));
          await settle(t);
          expect(
            find.byKey(const ValueKey('tracks-languages-dialog')),
            findsOne,
          );
          await capture(t, '04-languages-dialog-desktop');
        }
        await t.pumpWidget(const SizedBox());
        await settle(t, 2);
      });
    }

    testWidgets('OSD audio panel', (t) async {
      setSurface(t, const Size(1280, 720));
      final engine =
          FakePlayerEngine(loadedDuration: const Duration(minutes: 52))
            ..openAudioTracks = const [
              MediaTrack(
                id: '1',
                language: 'rus',
                title: 'Дубляж',
                codec: 'ac3',
                channels: 6,
              ),
              MediaTrack(
                id: '2',
                language: 'eng',
                title: 'English',
                codec: 'eac3',
                channels: 6,
                isDefault: true,
              ),
              MediaTrack(
                id: '3',
                language: 'eng',
                title: 'Commentary',
                codec: 'aac',
                channels: 2,
                commentary: true,
              ),
            ];
      final services = await demoServices(
        engineFactory: ({bool video = true}) => engine,
      );
      services.locale.value = const Locale('ru');
      await services.setAudioProcessing(
        const AudioProcessing(downmix: DownmixMode.stereo, boost: 150),
      );
      final item = (await services.library.item('show-moonlit-bureau-s2e4'))!;
      await t.pumpWidget(testApp(services));
      await settle(t);
      final context = t.element(find.byType(Scaffold).first);
      playVideo(context, item);
      await settle(t, 30);
      final state = t.state<PlayerScreenState>(find.byType(PlayerScreen));
      await state.setAudioDelay(const Duration(milliseconds: 150));
      state.osd.openPanel(OsdPanel.audio);
      await settle(t, 10);
      expect(find.text('Звук позже на 0,15 с'), findsOneWidget);
      await capture(t, '02-osd-audio-panel');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  });
}
