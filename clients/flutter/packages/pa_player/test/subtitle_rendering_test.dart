// SPDX-License-Identifier: MPL-2.0
//
// PLAY-006 in the player: styled ASS and bitmap subtitles (PGS, VobSub) are
// drawn by libmpv, plain text by the overlay in the user's style; "override
// ASS style" sends ASS back to the overlay; the settings carry the option and
// the HDR subtitle brightness; Android gets a fallback font per language.
// The frames themselves are checked by tools/mpv/tests/subtitles (golden).
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

const _own = SubtitleRenderOptions();
const _override = SubtitleRenderOptions(overrideAssStyle: true);

MediaTrack _track(String codec, {String? language}) =>
    MediaTrack(id: '3', codec: codec, language: language);

void main() {
  group('renderer per track', () {
    test('bitmaps are always drawn by the engine', () {
      for (final codec in [
        'hdmv_pgs_subtitle',
        'dvd_subtitle',
        'dvb_subtitle',
        'PGSSUB',
      ]) {
        expect(
          subtitleRendererFor(_track(codec), _own),
          SubtitleRenderer.engine,
          reason: codec,
        );
        expect(
          subtitleRendererFor(_track(codec), _override),
          SubtitleRenderer.engine,
          reason: 'a picture cannot take the text style: $codec',
        );
      }
    });

    test('ASS keeps its own style unless overridden', () {
      expect(subtitleRendererFor(_track('ass'), _own), SubtitleRenderer.engine);
      expect(subtitleRendererFor(_track('ssa'), _own), SubtitleRenderer.engine);
      expect(
        subtitleRendererFor(_track('ass'), _override),
        SubtitleRenderer.overlay,
      );
    });

    test('plain text and "off" stay with the overlay', () {
      for (final codec in ['subrip', 'webvtt', 'mov_text', 'text']) {
        expect(
          subtitleRendererFor(_track(codec), _own),
          SubtitleRenderer.overlay,
          reason: codec,
        );
      }
      expect(subtitleRendererFor(null, _own), SubtitleRenderer.overlay);
      expect(
        subtitleRendererFor(MediaTrack.off, _own),
        SubtitleRenderer.overlay,
      );
      expect(
        subtitleRendererFor(const MediaTrack(id: '2'), _own),
        SubtitleRenderer.overlay,
        reason: 'unknown codec: the overlay at least shows its text',
      );
    });
  });

  test('mpv properties for each renderer', () {
    expect(mpvSubtitleProperties(SubtitleRenderer.engine, _own), {
      'sub-visibility': 'yes',
      'sub-ass': 'yes',
      'sub-hdr-peak': '203',
      'image-subs-hdr-peak': '203',
    });
    expect(
      mpvSubtitleProperties(
        SubtitleRenderer.overlay,
        const SubtitleRenderOptions(hdrPeakNits: 300),
      ),
      {
        'sub-visibility': 'no',
        'sub-ass': 'no',
        'sub-hdr-peak': '300',
        'image-subs-hdr-peak': '300',
      },
    );
  });

  group('settings', () {
    test('defaults: own ASS style, 203 nits', () {
      const s = SubtitleSettings();
      expect(s.overrideAssStyle, isFalse);
      expect(s.hdrPeakNits, 203);
      expect(s.renderOptions, _own);
    });

    test('JSON round trip and old JSON', () {
      final s = const SubtitleSettings().copyWith(
        overrideAssStyle: true,
        hdrPeakNits: 250,
      );
      final back = SubtitleSettings.fromJson(s.toJson());
      expect(back, s);
      expect(
        back.renderOptions,
        const SubtitleRenderOptions(overrideAssStyle: true, hdrPeakNits: 250),
      );
      // Settings saved by PLAY-007 have neither key.
      final old = SubtitleSettings.fromJson({'system': false, 'scale': 1.5});
      expect(old.overrideAssStyle, isFalse);
      expect(old.hdrPeakNits, 203);
      expect(old.scale, 1.5);
    });

    test('HDR brightness is clamped', () {
      expect(
        const SubtitleSettings().copyWith(hdrPeakNits: 5).hdrPeakNits,
        SubtitleRenderOptions.minHdrPeakNits,
      );
      expect(
        SubtitleSettings.fromJson({'hdrPeak': 99999}).hdrPeakNits,
        SubtitleRenderOptions.maxHdrPeakNits,
      );
      expect(SubtitleSettings.fromJson({'hdrPeak': 'x'}).hdrPeakNits, 203);
    });

    test('the text style does not change with the new options', () {
      const a = SubtitleSettings(useSystemStyle: false, scale: 1.25);
      final b = a.copyWith(overrideAssStyle: true, hdrPeakNits: 300);
      expect(b.resolve(null), a.resolve(null));
      expect(b == a, isFalse);
    });
  });

  group('Android fallback font', () {
    const all = SubtitleFontDir('/data/subfonts', {
      'Roboto-Regular.ttf',
      SubtitleFontDir.cjk,
      SubtitleFontDir.arabic,
      SubtitleFontDir.hebrew,
      SubtitleFontDir.thai,
    });

    test('Noto Sans CJK by default, in the variant of the language', () {
      expect(all.fallbackFamily(null), 'Noto Sans CJK JP');
      expect(all.fallbackFamily('rus'), 'Noto Sans CJK JP');
      expect(all.fallbackFamily('jpn'), 'Noto Sans CJK JP');
      expect(all.fallbackFamily('chi'), 'Noto Sans CJK SC');
      expect(all.fallbackFamily('zh-Hant'), 'Noto Sans CJK TC');
      expect(all.fallbackFamily('zh_TW'), 'Noto Sans CJK TC');
      expect(all.fallbackFamily('kor'), 'Noto Sans CJK KR');
    });

    test('scripts Noto Sans CJK lacks', () {
      expect(all.fallbackFamily('ara'), 'Noto Naskh Arabic');
      expect(all.fallbackFamily('fa'), 'Noto Naskh Arabic');
      expect(all.fallbackFamily('heb'), 'Noto Sans Hebrew');
      expect(all.fallbackFamily('th'), 'Noto Sans Thai');
    });

    test('devices without the fonts', () {
      const roboto = SubtitleFontDir('/d', {'Roboto-Regular.ttf'});
      expect(roboto.fallbackFamily('jpn'), 'Roboto');
      expect(roboto.fallbackFamily('ara'), 'Roboto');
      const cjkOnly = SubtitleFontDir('/d', {SubtitleFontDir.cjk});
      expect(cjkOnly.fallbackFamily('heb'), 'Noto Sans CJK JP');
    });

    test('plugin answer', () {
      final d = SubtitleFontDir.fromMap({
        'dir': '/data/user/0/app/no_backup/subfonts',
        'files': [SubtitleFontDir.cjk, 7, 'Roboto-Regular.ttf'],
      })!;
      expect(d.path, '/data/user/0/app/no_backup/subfonts');
      expect(d.files, {SubtitleFontDir.cjk, 'Roboto-Regular.ttf'});
      expect(SubtitleFontDir.fromMap({'files': []}), isNull);
    });
  });

  group('player screen', () {
    late FakePlayerEngine engine;
    late ValueNotifier<SubtitleSettings> settings;

    Future<void> open(WidgetTester t) async {
      engine = FakePlayerEngine()
        ..embeddedSubtitles = const [
          MediaTrack(id: '1', codec: 'subrip', language: 'rus'),
          MediaTrack(id: '2', codec: 'ass', language: 'rus'),
          MediaTrack(id: '3', codec: 'hdmv_pgs_subtitle', language: 'eng'),
        ];
      settings = ValueNotifier(const SubtitleSettings());
      await t.pumpWidget(
        MaterialApp(
          home: PlayerScreen(
            request: const PlayRequest(uri: '/m/a.mkv', title: 'A'),
            onProgress: (_, _) {},
            engineFactory: ({bool video = true}) => engine,
            subtitleSettings: settings,
            captionStyleLoader: () async => null,
          ),
        ),
      );
      await t.pumpAndSettle();
    }

    Future<void> select(WidgetTester t, String id) async {
      await engine.setSubtitleTrack(
        engine.state.value.subtitleTracks.firstWhere((x) => x.id == id),
      );
      engine.showSubtitle(const ['Line of the cue']);
      await t.pump();
    }

    final cue = find.text('Line of the cue');

    // Disposes the screen (its clock and progress timers).
    Future<void> close(WidgetTester t) => t.pumpWidget(const SizedBox());

    testWidgets('the engine gets the settings on start and on change', (
      t,
    ) async {
      await open(t);
      expect(engine.renderOptions, _own);
      settings.value = settings.value.copyWith(
        overrideAssStyle: true,
        hdrPeakNits: 300,
      );
      await t.pump();
      expect(
        engine.renderOptions,
        const SubtitleRenderOptions(overrideAssStyle: true, hdrPeakNits: 300),
      );
      await close(t);
    });

    testWidgets('text: the overlay draws the cue', (t) async {
      await open(t);
      await select(t, '1');
      expect(engine.state.value.subtitleRenderer, SubtitleRenderer.overlay);
      expect(cue, findsOneWidget);
      await close(t);
    });

    testWidgets('PGS: drawn by the engine, the overlay stays empty', (
      t,
    ) async {
      await open(t);
      await select(t, '3');
      expect(engine.state.value.subtitleRenderer, SubtitleRenderer.engine);
      expect(cue, findsNothing);
      await close(t);
    });

    testWidgets('ASS: own style by the engine, overlay with the override', (
      t,
    ) async {
      await open(t);
      await select(t, '2');
      expect(engine.state.value.subtitleRenderer, SubtitleRenderer.engine);
      expect(cue, findsNothing);

      settings.value = settings.value.copyWith(overrideAssStyle: true);
      await t.pump();
      expect(engine.state.value.subtitleRenderer, SubtitleRenderer.overlay);
      expect(cue, findsOneWidget);
      await close(t);
    });
  });
}
