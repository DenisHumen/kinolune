// SPDX-License-Identifier: MPL-2.0
//
// UI-014 in the app: the player's track panels (Audio · Subtitles · Video)
// open across the top on a TV and as a sheet on a phone; the quick
// subtitle style in the subtitles tab is stored like Settings → Subtitle
// style; and a track chosen in the tabs is kept for the show in the
// profile and applied to the next episode (ROADMAP «done when»).
//
// With KINOLUNE_SCREENSHOTS_DIR set the frames are written there:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-014 \
//     flutter test test/track_panels_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/playback.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart'
    show
        MediaTrack,
        OsdPanel,
        OsdPanelPlacement,
        PlayerScreen,
        PlayerScreenState;
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

const _show = 'show-moonlit-bureau-s2e4';

const _audio = [
  MediaTrack(
    id: '1',
    language: 'eng',
    title: 'English',
    codec: 'eac3',
    channels: 6,
    isDefault: true,
  ),
  MediaTrack(
    id: '2',
    language: 'rus',
    title: 'Дубляж',
    codec: 'ac3',
    channels: 6,
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

const _subtitles = [
  MediaTrack(id: '4', language: 'rus', title: 'Надписи', forced: true),
  MediaTrack(id: '5', language: 'rus', title: 'Полные'),
  MediaTrack(
    id: '6',
    language: 'eng',
    title: 'English SDH',
    hearingImpaired: true,
  ),
];

const _video = [
  MediaTrack(id: '1', codec: 'hevc', width: 3840, height: 2160, fps: 23.976),
  MediaTrack(id: '2', codec: 'h264', width: 1280, height: 720, fps: 23.976),
];

/// A fake engine that draws a dim scene instead of black, so the
/// screenshots show the panel over a picture.
class _SceneEngine extends FakePlayerEngine {
  _SceneEngine() : super(loadedDuration: const Duration(minutes: 52)) {
    openAudioTracks = _audio;
    embeddedSubtitles = _subtitles;
    openVideoTracks = _video;
  }

  @override
  Widget buildVideo(BuildContext context, {BoxFit fit = BoxFit.contain}) =>
      const DecoratedBox(
        decoration: BoxDecoration(
          gradient: LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [Color(0xFF1E2A44), Color(0xFF3B2F4A), Color(0xFF6A4B3A)],
          ),
        ),
        child: SizedBox.expand(),
      );
}

Future<(AppServices, _SceneEngine)> _services() async {
  final engine = _SceneEngine();
  final services = await demoServices(
    engineFactory: ({bool video = true}) => engine,
  );
  return (services, engine);
}

Future<void> _play(WidgetTester t, AppServices services, String id) async {
  final item = (await services.library.item(id))!;
  final context = t.element(find.byType(Scaffold).first);
  playVideo(context, item);
  await settle(t, 30);
  expect(find.byType(PlayerScreen), findsOneWidget);
}

PlayerScreenState _player(WidgetTester t) =>
    t.state<PlayerScreenState>(find.byType(PlayerScreen));

void main() {
  // ROADMAP UI-014 «done when».
  testWidgets('a track chosen for a show is applied to the next episode', (
    t,
  ) async {
    setSurface(t, const Size(1280, 800));
    final (services, engine) = await _services();
    await t.pumpWidget(testApp(services));
    await settle(t);
    await _play(t, services, _show);
    // By the default policy the English track plays first.
    expect(engine.state.value.audioTrack!.id, '1');

    // The dub in the audio tab, then full Russian subtitles in the
    // subtitles tab of the same panel.
    await t.tap(find.byKey(const ValueKey('osd-audio')));
    await settle(t, 6);
    await t.tap(find.byKey(const ValueKey('audio-2')));
    await settle(t, 6);
    await t.tap(find.byKey(const ValueKey('osd-audio')));
    await settle(t, 6);
    await t.tap(find.byKey(const ValueKey('tab-subtitles')));
    await settle(t, 6);
    await t.tap(find.byKey(const ValueKey('sub-5')));
    await settle(t, 6);
    expect(engine.state.value.audioTrack!.id, '2');
    expect(engine.state.value.subtitleTrack!.id, '5');

    // Stored for the show in the profile.
    final item = (await services.library.item(_show))!;
    final memory = await services.subtitleMemory(item.showId!);
    expect(memory!.audio!.language, 'rus');
    expect(memory.choice!.language, 'rus');

    // The player is closed; the next episode is opened from the library,
    // its file lists the tracks in another order.
    _player(t).exit();
    await settle(t, 10);
    expect(find.byType(PlayerScreen), findsNothing);
    engine
      ..openAudioTracks = const [
        MediaTrack(id: '11', language: 'rus', title: 'Дубляж'),
        MediaTrack(id: '12', language: 'eng', isDefault: true),
      ]
      ..embeddedSubtitles = const [
        MediaTrack(id: '13', language: 'eng'),
        MediaTrack(id: '14', language: 'rus'),
      ];
    final next = await services.library.nextEpisode(_show);
    expect(next, isNotNull);
    expect(next!.showId, item.showId);
    await _play(t, services, next.id);
    expect(engine.state.value.audioTrack!.id, '11');
    expect(engine.state.value.subtitleTrack!.id, '14');

    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  for (final (ff, size, placement) in [
    (FormFactor.tv, const Size(1920, 1080), OsdPanelPlacement.top),
    (FormFactor.phone, const Size(390, 844), OsdPanelPlacement.bottom),
    (FormFactor.desktop, const Size(1280, 800), OsdPanelPlacement.side),
  ]) {
    testWidgets('the panels on ${ff.name} open at the ${placement.name}', (
      t,
    ) async {
      setSurface(t, size);
      final (services, _) = await _services();
      await t.pumpWidget(testApp(services, formFactor: ff));
      await settle(t);
      await _play(t, services, _show);
      final player = t.widget<PlayerScreen>(find.byType(PlayerScreen));
      expect(player.tvLayout, ff == FormFactor.tv);
      expect(_player(t).panelPlacement(size), placement);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }

  testWidgets('the quick subtitle style is stored like the settings', (
    t,
  ) async {
    setSurface(t, const Size(1280, 800));
    final (services, _) = await _services();
    await t.pumpWidget(testApp(services));
    await settle(t);
    await _play(t, services, _show);
    _player(t).osd.openPanel(OsdPanel.subtitles);
    await settle(t, 6);
    final more = find.byKey(const ValueKey('sub-size-more'));
    await t.ensureVisible(more);
    await settle(t, 4);
    await t.tap(more);
    await settle(t, 4);
    await t.tap(more);
    await settle(t, 4);
    expect(services.subtitleSettings.value.useSystemStyle, isFalse);
    expect(services.subtitleSettings.value.scale, 1.25);
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  group('screenshots', () {
    setUpAll(loadAppFonts);

    for (final (name, ff, size, dpr) in [
      ('tv', FormFactor.tv, const Size(1920, 1080), 1.0),
      ('phone', FormFactor.phone, const Size(390, 844), 2.0),
      ('desktop', FormFactor.desktop, const Size(1280, 800), 1.0),
    ]) {
      testWidgets('track panels on $name', (t) async {
        setSurface(t, size, dpr: dpr);
        final (services, engine) = await _services();
        services.locale.value = const Locale('ru');
        await t.pumpWidget(testApp(services, formFactor: ff));
        await settle(t);
        await _play(t, services, _show);
        final player = _player(t);
        engine.showSubtitle(const ['— Кажется, дело снова открыто.']);
        for (final (i, panel) in [
          OsdPanel.audio,
          OsdPanel.subtitles,
          OsdPanel.video,
        ].indexed) {
          player.osd.openPanel(panel);
          await settle(t, 8);
          expect(t.takeException(), isNull);
          await capture(t, '0${i + 1}-$name-${panel.name}');
        }
        await t.pumpWidget(const SizedBox());
        await settle(t, 2);
      });
    }
  });
}
