// SPDX-License-Identifier: MPL-2.0
//
// PLAY-004 in the OSD audio panel: track tags (channels, commentary), the
// audio delay with a clear sign, the quick toggles of the sound processing,
// and the series memory of the audio choice and delay, which the next
// episode starts with.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

const _en = MediaTrack(id: '1', language: 'eng', channels: 6, isDefault: true);
const _ru = MediaTrack(id: '2', language: 'rus', title: 'Дубляж', channels: 2);
const _comm = MediaTrack(
  id: '3',
  language: 'eng',
  title: 'Commentary',
  commentary: true,
);

class _Harness {
  _Harness({this.loadNext});

  final engine = FakePlayerEngine();
  final Future<UpNextItem?> Function()? loadNext;
  final processing = ValueNotifier(const AudioProcessing());
  final memories = <(String, TrackMemory)>[];

  Widget app() => MaterialApp(
    home: PlayerScreen(
      request: const PlayRequest(
        uri: '/tv/e1.mkv',
        title: 'E1',
        subtitles: [],
        seriesKey: 's1',
        originalLanguage: 'en',
      ),
      onProgress: (_, _) {},
      engineFactory: ({bool video = true}) => engine,
      loadNext: loadNext,
      subtitleLanguages: const ['ru'],
      captionStyleLoader: () async => null,
      onSubtitleMemory: (k, m) => memories.add((k, m)),
      audioProcessing: processing,
      onAudioProcessing: (a) => processing.value = a,
    ),
  );

  PlayerScreenState state(WidgetTester t) =>
      t.state<PlayerScreenState>(find.byType(PlayerScreen));
}

Future<_Harness> _open(WidgetTester t, _Harness h) async {
  h.engine.openAudioTracks = const [_en, _ru, _comm];
  await t.pumpWidget(h.app());
  await t.pump();
  await t.pump(const Duration(seconds: 2));
  return h;
}

Future<void> _openPanel(WidgetTester t, _Harness h) async {
  h.state(t).osd.openPanel(OsdPanel.audio);
  await t.pumpAndSettle();
}

void main() {
  testWidgets('the dub plays; tags show channels and commentary', (t) async {
    final h = await _open(t, _Harness());
    expect(h.engine.state.value.audioTrack, _ru);
    expect(h.memories, isEmpty, reason: 'an automatic pick is not a choice');
    await _openPanel(t, h);
    expect(find.text('5.1'), findsOneWidget);
    expect(find.text('Stereo'), findsOneWidget);
    expect(find.text('Commentary'), findsWidgets);
  });

  testWidgets('the sound processing reaches the engine and the toggles', (
    t,
  ) async {
    final h = await _open(t, _Harness());
    expect(h.engine.audioProcessing, const AudioProcessing());
    await _openPanel(t, h);
    await t.tap(find.byKey(const ValueKey('audio-downmix')));
    await t.pumpAndSettle();
    expect(h.processing.value.downmix, DownmixMode.stereo);
    expect(h.engine.audioProcessing!.downmix, DownmixMode.stereo);

    await t.ensureVisible(find.byKey(const ValueKey('audio-lfe')));
    await t.tap(find.byKey(const ValueKey('audio-lfe')));
    await t.pumpAndSettle();
    expect(h.engine.audioProcessing!.lfeToStereo, isTrue);

    final more = find.byKey(const ValueKey('audio-boost-more'));
    await t.ensureVisible(more);
    for (var i = 0; i < 7; i++) {
      await t.tap(more);
      await t.pumpAndSettle();
    }
    expect(h.engine.audioProcessing!.boost, 300, reason: 'stops at 300 %');
    expect(find.text('300%'), findsOneWidget);
    expect(
      h.engine.audioProcessing!.mpvFilter,
      contains('alimiter=limit=0.891'),
    );
    final less = find.byKey(const ValueKey('audio-boost-less'));
    await t.tap(less);
    await t.pumpAndSettle();
    expect(h.engine.audioProcessing!.boost, 250);
  });

  testWidgets('audio delay says which way and is remembered', (t) async {
    final h = await _open(t, _Harness());
    await _openPanel(t, h);
    final later = find.byKey(const ValueKey('audio-delay-later'));
    await t.ensureVisible(later);
    await t.tap(later);
    await t.pumpAndSettle();
    expect(h.engine.calls, contains('audiodelay:50'));
    expect(find.text('Sound 0.05 s later'), findsOneWidget);
    for (var i = 0; i < 2; i++) {
      await t.tap(find.byKey(const ValueKey('audio-delay-earlier')));
      await t.pumpAndSettle();
    }
    expect(find.text('Sound 0.05 s earlier'), findsOneWidget);
    expect(h.memories.last.$2.audioDelay, const Duration(milliseconds: -50));
  });

  testWidgets('a picked audio track is remembered for the series', (t) async {
    final h = await _open(t, _Harness());
    await _openPanel(t, h);
    await t.tap(find.byKey(const ValueKey('audio-1')));
    await t.pumpAndSettle();
    expect(h.engine.state.value.audioTrack, _en);
    expect(h.memories.single, (
      's1',
      const TrackMemory(audio: AudioChoice(language: 'eng')),
    ));
  });

  testWidgets('the next episode keeps the audio choice and delay', (t) async {
    final h = _Harness(
      loadNext: () async => UpNextItem(
        request: const PlayRequest(
          uri: '/tv/e2.mkv',
          title: 'E2',
          subtitles: [],
          seriesKey: 's1',
        ),
        onProgress: (_, _) {},
      ),
    );
    await _open(t, h);
    final s = h.state(t);
    await s.selectAudio(_en);
    await s.setAudioDelay(const Duration(milliseconds: 200));
    await t.pump();
    // The next file lists its tracks in another order.
    h.engine.openAudioTracks = const [
      MediaTrack(id: '7', language: 'rus'),
      MediaTrack(id: '8', language: 'eng'),
    ];
    h.engine.finish();
    await t.pump();
    s.upNext.playNow();
    await t.pump();
    await t.pump(const Duration(seconds: 2));
    expect(s.request.title, 'E2');
    expect(h.engine.state.value.audioTrack!.id, '8');
    expect(h.engine.state.value.audioDelay.inMilliseconds, 200);
  });

  testWidgets('the delay row starts in sync', (t) async {
    final h = await _open(t, _Harness());
    await _openPanel(t, h);
    expect(find.byKey(const ValueKey('audio-delay-value')), findsOneWidget);
    expect(find.text('In sync'), findsOneWidget);
  });
}
