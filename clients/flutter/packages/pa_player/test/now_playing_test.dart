// SPDX-License-Identifier: MPL-2.0
//
// MUS-005: the Now Playing format line («FLAC · 24/96 · bit-perfect»), its
// transport and output picker, and the mini player on a phone, the desktop
// and TV (focus with the remote).

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

const _hiRes = AudioFormat(codec: 'flac', sampleRateHz: 96000, bitDepth: 24);

List<PlayRequest> _tracks(int n, {AudioFormat? format = _hiRes}) => [
  for (var i = 0; i < n; i++)
    PlayRequest(
      uri: '/music/$i.flac',
      title: 'Track $i',
      artist: 'Artist',
      album: 'Album',
      audioFormat: format,
    ),
];

/// An engine whose output is bit-perfect (what MUS-009 brings).
class _BitPerfectEngine extends FakePlayerEngine implements BitPerfectEngine {
  @override
  bool bitPerfect = true;
}

void _surface(WidgetTester t, Size size) {
  t.view.physicalSize = size;
  t.view.devicePixelRatio = 1;
  addTearDown(t.view.reset);
}

void main() {
  group('format line', () {
    test('lossless: codec and bit depth / sample rate', () {
      expect(audioFormatLine(_hiRes), 'FLAC · 24/96');
      expect(
        audioFormatLine(
          const AudioFormat(codec: 'alac', sampleRateHz: 44100, bitDepth: 16),
        ),
        'ALAC · 16/44.1',
      );
      expect(
        audioFormatLine(const AudioFormat(codec: 'flac', sampleRateHz: 88200)),
        'FLAC · 88.2 kHz',
      );
      expect(
        audioFormatLine(_hiRes, bitPerfect: true),
        'FLAC · 24/96 · bit-perfect',
      );
    });

    test('lossy, DSD and unknown', () {
      expect(
        audioFormatLine(
          const AudioFormat(
            codec: 'mp3',
            sampleRateHz: 44100,
            bitrateKbps: 320,
          ),
        ),
        'MP3 · 320 kbps',
      );
      expect(
        audioFormatLine(const AudioFormat(codec: 'dsd', sampleRateHz: 2822400)),
        'DSD64',
      );
      expect(audioFormatLine(const AudioFormat(codec: 'opus')), 'OPUS');
      expect(audioFormatLine(const AudioFormat(sampleRateHz: 48000)), isNull);
      expect(audioFormatLine(null), isNull);
    });

    test('libmpv codec names map to the library ones', () {
      expect(engineCodec('pcm_s24le'), 'pcm');
      expect(engineCodec('dsd_lsbf_planar'), 'dsd');
      expect(engineCodec('FLAC'), 'flac');
      expect(engineCodec(''), isNull);
    });

    test('survives the saved queue', () {
      final json = musicRequestToJson(_tracks(1).single);
      expect(musicRequestFromJson(json)!.audioFormat, _hiRes);
    });
  });

  group('controller', () {
    test('the library format, else the codec the engine decodes', () async {
      final engine = FakePlayerEngine()
        ..openAudioTracks = const [MediaTrack(id: '1', codec: 'flac')];
      final c = MusicPlayerController(
        engineFactory: ({bool video = true}) => engine,
      );
      addTearDown(c.dispose);
      await c.play(_tracks(1));
      expect(c.formatLine, 'FLAC · 24/96');
      expect(c.bitPerfect, isFalse, reason: 'shared libmpv output');

      await c.play(_tracks(1, format: null));
      expect(c.formatLine, 'FLAC');
    });

    test(
      'bit-perfect only when the engine says so and no gain applies',
      () async {
        final engine = _BitPerfectEngine();
        final c = MusicPlayerController(
          engineFactory: ({bool video = true}) => engine,
        );
        addTearDown(c.dispose);
        await c.play(_tracks(2));
        expect(c.bitPerfect, isFalse, reason: 'ReplayGain changes the samples');

        await c.setAudioProfile(
          const AudioProfile(replayGain: ReplayGainMode.off),
        );
        expect(c.bitPerfect, isTrue);
        expect(c.formatLine, 'FLAC · 24/96 · bit-perfect');

        await c.setDucked(true);
        expect(c.bitPerfect, isFalse, reason: 'ducked for another app');
        await c.setDucked(false);
        engine.bitPerfect = false;
        expect(c.formatLine, 'FLAC · 24/96');
      },
    );
  });

  group('Now Playing', () {
    late FakePlayerEngine engine;
    late MusicPlayerController c;
    late List<MethodCall> pickerCalls;
    const channel = MethodChannel('test/audio_output');

    setUp(() {
      engine = FakePlayerEngine(loadedDuration: const Duration(minutes: 4));
      c = MusicPlayerController(engineFactory: ({bool video = true}) => engine);
      pickerCalls = [];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, (call) async {
            pickerCalls.add(call);
            return true;
          });
    });

    tearDown(() {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, null);
    });

    for (final size in [const Size(400, 900), const Size(1600, 900)]) {
      testWidgets('${size.width}px: transport, times, format, output', (
        t,
      ) async {
        _surface(t, size);
        await c.play(_tracks(3));
        engine.advance(const Duration(seconds: 65));
        await t.pumpWidget(
          MaterialApp(
            home: NowPlayingScreen(
              controller: c,
              audioOutput: SystemAudioOutput(
                channel: channel,
                platform: TargetPlatform.android,
              ),
            ),
          ),
        );
        await t.pump(const Duration(milliseconds: 500));

        expect(find.text('FLAC · 24/96'), findsOneWidget);
        expect(find.byKey(const ValueKey('np-format')), findsOneWidget);
        // The seek bar shows the elapsed time.
        expect(find.text('1:05'), findsOneWidget);

        await t.tap(find.byKey(const ValueKey('np-play')));
        await t.pump();
        expect(engine.calls.last, 'pause');
        await t.tap(find.byKey(const ValueKey('np-play')));
        await t.pump();
        expect(engine.calls.last, 'play');

        await t.tap(find.byKey(const ValueKey('np-next')));
        await t.pump();
        expect(engine.calls.last, 'jump:1');
        expect(find.byKey(const ValueKey('np-title')), findsOneWidget);
        expect(
          t.widget<Text>(find.byKey(const ValueKey('np-title'))).data,
          'Track 1',
        );
        // Past the restart threshold «previous» restarts the track.
        engine.advance(const Duration(seconds: 10));
        await t.tap(find.byKey(const ValueKey('np-prev')));
        await t.pump();
        expect(engine.calls.last, 'seek:0');

        await t.tap(find.byKey(const ValueKey('np-shuffle')));
        await t.pump();
        expect(c.shuffle, isTrue);
        await t.tap(find.byKey(const ValueKey('np-repeat')));
        await t.pump();
        expect(c.repeat, QueueRepeat.all);

        await t.tap(find.byKey(const ValueKey('np-output')));
        await t.pump();
        expect(pickerCalls.single.method, 'showPicker');

        // The queue is on screen (beside on the desktop, below on a phone
        // after the queue button).
        if (size.width < 900) {
          await t.tap(find.byKey(const ValueKey('np-show-queue')));
          await t.pump(const Duration(milliseconds: 500));
        } else {
          expect(find.byKey(const ValueKey('np-show-queue')), findsNothing);
        }
        expect(find.byType(NowPlayingQueue), findsOneWidget);
        await t.pump(const Duration(milliseconds: 500));
      });
    }

    testWidgets('no output button where the platform has no picker', (t) async {
      _surface(t, const Size(400, 900));
      await c.play(_tracks(1, format: null));
      await t.pumpWidget(
        MaterialApp(
          home: NowPlayingScreen(
            controller: c,
            audioOutput: SystemAudioOutput(channel: channel, platform: null),
          ),
        ),
      );
      await t.pump(const Duration(milliseconds: 500));
      expect(find.byKey(const ValueKey('np-output')), findsNothing);
      expect(
        find.byKey(const ValueKey('np-format')),
        findsNothing,
        reason: 'nothing known about the stream',
      );
      await t.pump(const Duration(milliseconds: 500));
    });
  });

  group('mini player', () {
    late FakePlayerEngine engine;
    late MusicPlayerController c;

    setUp(() {
      engine = FakePlayerEngine();
      c = MusicPlayerController(engineFactory: ({bool video = true}) => engine);
    });

    Widget host(MiniPlayerStyle style) => MaterialApp(
      home: Scaffold(
        body: Align(
          alignment: Alignment.bottomCenter,
          child: NowPlayingBar(controller: c, style: style),
        ),
      ),
    );

    testWidgets('phone: a capsule that opens Now Playing', (t) async {
      _surface(t, const Size(400, 800));
      await t.pumpWidget(host(MiniPlayerStyle.capsule));
      expect(find.byKey(const ValueKey('now-playing-bar')), findsNothing);
      await c.play(_tracks(2));
      await t.pump();
      expect(find.byKey(const ValueKey('mini-player-capsule')), findsOneWidget);
      await t.tap(find.byKey(const ValueKey('bar-title')));
      await t.pumpAndSettle();
      expect(find.byType(NowPlayingScreen), findsOneWidget);
    });

    testWidgets('desktop: a bar with transport, seek and format', (t) async {
      _surface(t, const Size(1280, 800));
      await t.pumpWidget(host(MiniPlayerStyle.bar));
      await c.play(_tracks(3));
      await t.pump();
      expect(find.byKey(const ValueKey('mini-player-bar')), findsOneWidget);
      expect(find.byKey(const ValueKey('bar-seek')), findsOneWidget);
      expect(find.text('FLAC · 24/96'), findsOneWidget);
      await t.tap(find.byKey(const ValueKey('bar-next')));
      await t.pump();
      expect(engine.calls.last, 'jump:1');
      await t.tap(find.byKey(const ValueKey('bar-prev')));
      await t.pump();
      expect(engine.calls.last, 'jump:0');
      await t.tap(find.byKey(const ValueKey('bar-play')));
      await t.pump();
      expect(engine.calls.last, 'pause');
      await t.tap(find.byKey(const ValueKey('bar-open')));
      await t.pumpAndSettle();
      expect(find.byType(NowPlayingScreen), findsOneWidget);
    });

    testWidgets('TV: the remote focuses the plate; OK opens Now Playing', (
      t,
    ) async {
      _surface(t, const Size(1920, 1080));
      await t.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Column(
              children: [
                const Focus(autofocus: true, child: Text('content')),
                const Spacer(),
                Align(
                  alignment: Alignment.bottomRight,
                  child: NowPlayingBar(
                    controller: c,
                    style: MiniPlayerStyle.tv,
                  ),
                ),
              ],
            ),
          ),
        ),
      );
      await c.play(_tracks(2));
      await t.pump();
      expect(find.byKey(const ValueKey('mini-player-tv')), findsOneWidget);

      // D-pad down from the content lands on the plate.
      await t.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await t.pump(const Duration(milliseconds: 200));
      final plate = find.byKey(const ValueKey('now-playing-bar'));
      final focused = FocusManager.instance.primaryFocus!.context!;
      expect(
        find.descendant(of: plate, matching: find.byWidget(focused.widget)),
        findsOneWidget,
      );
      final box = t.widget<AnimatedContainer>(
        find.byKey(const ValueKey('mini-player-tv')),
      );
      expect(
        (box.decoration! as BoxDecoration).color,
        PaTokens.focusRing,
        reason: 'the focused plate is drawn white',
      );

      // Play/pause on the remote.
      await t.sendKeyEvent(LogicalKeyboardKey.mediaPlayPause);
      await t.pump();
      expect(engine.calls.last, 'pause');

      // OK opens Now Playing.
      await t.sendKeyEvent(LogicalKeyboardKey.enter);
      await t.pumpAndSettle();
      expect(find.byType(NowPlayingScreen), findsOneWidget);
    });
  });
}
