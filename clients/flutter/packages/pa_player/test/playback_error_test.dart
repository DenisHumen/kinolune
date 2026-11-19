// SPDX-License-Identifier: MPL-2.0
//
// BUG-003: harmless mpv messages (the hardware decoder probe without a
// GPU, a broken frame, an external subtitle) never set the player's
// error; a fatal one does, and the player screen shows it by its stable
// code with "Try again" and "Back". A new open or playback going on
// clears it.
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

void main() {
  group('PlaybackErrorTracker', () {
    test('the hwdec probe and per-frame errors are not fatal', () {
      final t = PlaybackErrorTracker();
      for (final (prefix, text) in [
        ('vd', 'Could not open codec.'),
        ('ffmpeg/video', 'h264: Invalid NAL unit size'),
        ('vd', 'Error while decoding frame!'),
        ('ad', 'Error decoding audio.'),
        ('vo/gpu', 'Failed to create a hwdec interop'),
        ('ffmpeg', 'tcp: Connection reset by peer'),
      ]) {
        expect(
          t.onLog(
            prefix: prefix,
            level: 'error',
            text: text,
            loaded: true,
            hasVideo: true,
            hasAudio: true,
          ),
          isNull,
          reason: '$prefix: $text',
        );
        // Not even before the file has loaded.
        expect(
          t.onLog(prefix: prefix, level: 'error', text: text),
          isNull,
          reason: '$prefix: $text (opening)',
        );
      }
      expect(
        t.onLog(prefix: 'cplayer', level: 'warn', text: 'Failed to open x'),
        isNull,
      );
    });

    test('a file that cannot be opened is fatal while opening', () {
      final t = PlaybackErrorTracker();
      expect(
        t
            .onLog(
              prefix: 'file',
              level: 'error',
              text: "Cannot open file '/media/gone.mkv': No such file",
            )
            ?.code,
        PlaybackErrorCode.openFailed,
      );
      expect(
        t
            .onLog(
              prefix: 'cplayer',
              level: 'error',
              text: 'Failed to open file:///media/gone.mkv.',
            )
            ?.code,
        PlaybackErrorCode.openFailed,
      );
      expect(
        t
            .onLog(
              prefix: 'cplayer',
              level: 'error',
              text: 'Failed to recognize file format.',
            )
            ?.code,
        PlaybackErrorCode.unsupportedFormat,
      );
    });

    test('an open error once the file plays is not fatal', () {
      final t = PlaybackErrorTracker();
      expect(
        t.onLog(
          prefix: 'cplayer',
          level: 'error',
          text: 'Can not open external file /media/movie.srt.',
          loaded: true,
        ),
        isNull,
      );
      expect(
        t.onLog(
          prefix: 'file',
          level: 'error',
          text: "Cannot open file '/media/movie.srt'",
          loaded: true,
        ),
        isNull,
      );
    });

    test('nothing to play: fatal message or every decoder failed', () {
      final t = PlaybackErrorTracker();
      expect(
        t
            .onLog(
              prefix: 'cplayer',
              level: 'fatal',
              text: 'No video or audio streams selected.',
              loaded: true,
            )
            ?.code,
        PlaybackErrorCode.nothingToPlay,
      );
      const msg = "Failed to initialize a decoder for codec 'xyz'.";
      // The video decoder fails, the sound still plays.
      expect(
        t.onLog(
          prefix: 'vd',
          level: 'error',
          text: msg,
          loaded: true,
          hasVideo: true,
          hasAudio: true,
        ),
        isNull,
      );
      // Then the audio one too: nothing left.
      expect(
        t
            .onLog(
              prefix: 'ad',
              level: 'error',
              text: msg,
              loaded: true,
              hasVideo: true,
              hasAudio: true,
            )
            ?.code,
        PlaybackErrorCode.nothingToPlay,
      );
      // A new file starts clean.
      t.reset();
      expect(
        t.onLog(
          prefix: 'ad',
          level: 'error',
          text: msg,
          loaded: true,
          hasVideo: true,
          hasAudio: true,
        ),
        isNull,
      );
      // A video without sound: its only decoder failing is fatal.
      t.reset();
      expect(
        t
            .onLog(
              prefix: 'vd',
              level: 'error',
              text: msg,
              loaded: true,
              hasVideo: true,
            )
            ?.code,
        PlaybackErrorCode.nothingToPlay,
      );
    });

    test('every code has a stable snake_case string', () {
      final codes = PlaybackErrorCode.values.map((c) => c.code).toList();
      expect(codes.toSet(), hasLength(codes.length));
      for (final c in codes) {
        expect(c, matches(RegExp(r'^[a-z][a-z0-9_]*$')));
      }
    });
  });

  test('EngineState.copyWith keeps or clears the error', () {
    const e = PlaybackError(PlaybackErrorCode.openFailed, 'x');
    final s = const EngineState().copyWith(error: e);
    expect(s.copyWith(position: Duration.zero).error, e);
    expect(s.copyWith(clearError: true).error, isNull);
  });

  group('player screen', () {
    Future<FakePlayerEngine> open(WidgetTester t, {Locale? locale}) async {
      final engine = FakePlayerEngine();
      await t.pumpWidget(
        MaterialApp(
          locale: locale,
          supportedLocales: const [Locale('en'), Locale('ru')],
          localizationsDelegates: const [
            PaPlayerStrings.delegate,
            DefaultMaterialLocalizations.delegate,
            DefaultWidgetsLocalizations.delegate,
          ],
          home: Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => PlayerScreen(
                      request: const PlayRequest(
                        uri: '/media/gone.mkv',
                        title: 'Gone',
                      ),
                      onProgress: (_, _) {},
                      engineFactory: ({bool video = true}) => engine,
                    ),
                  ),
                ),
                child: const Text('open'),
              ),
            ),
          ),
        ),
      );
      await t.tap(find.text('open'));
      await t.pumpAndSettle();
      return engine;
    }

    testWidgets('no error, no error screen', (t) async {
      await open(t);
      expect(find.byKey(const ValueKey('player-error')), findsNothing);
    });

    testWidgets('a fatal error shows text, code and actions; retry '
        'reopens and clears it', (t) async {
      final engine = await open(t);
      engine.emit(
        (s) => s.copyWith(
          playing: false,
          error: const PlaybackError(
            PlaybackErrorCode.openFailed,
            "Cannot open file '/media/gone.mkv'",
          ),
        ),
      );
      await t.pump();
      expect(find.byKey(const ValueKey('player-error')), findsOneWidget);
      expect(find.text("Couldn't open the file"), findsOneWidget);
      expect(
        find.textContaining('network folder or server is reachable'),
        findsOneWidget,
      );
      expect(find.text('Error code: open_failed'), findsOneWidget);
      // mpv's own text is never the message.
      expect(find.textContaining('Cannot open file'), findsNothing);
      expect(find.text('Try again'), findsOneWidget);
      expect(find.text('Back'), findsWidgets);

      final opens = engine.calls.where((c) => c.startsWith('open:')).length;
      await t.tap(find.byKey(const ValueKey('player-error-retry')));
      await t.pumpAndSettle();
      expect(
        engine.calls.where((c) => c.startsWith('open:')).length,
        opens + 1,
      );
      expect(engine.state.value.error, isNull);
      expect(find.byKey(const ValueKey('player-error')), findsNothing);
    });

    testWidgets('Back on the error screen leaves the player', (t) async {
      final engine = await open(t);
      engine.emit(
        (s) => s.copyWith(
          error: const PlaybackError(PlaybackErrorCode.unsupportedFormat),
        ),
      );
      await t.pump();
      await t.tap(find.byKey(const ValueKey('player-error-back')));
      await t.pumpAndSettle();
      expect(find.byType(PlayerScreen), findsNothing);
      expect(find.text('open'), findsOneWidget);
    });

    testWidgets('remote Back over the error leaves the player', (t) async {
      final engine = await open(t);
      engine.emit(
        (s) => s.copyWith(
          error: const PlaybackError(PlaybackErrorCode.nothingToPlay),
        ),
      );
      await t.pump();
      await t.sendKeyEvent(LogicalKeyboardKey.escape);
      await t.pumpAndSettle();
      expect(find.byType(PlayerScreen), findsNothing);
    });

    testWidgets('Russian texts', (t) async {
      final engine = await open(t, locale: const Locale('ru'));
      engine.emit(
        (s) => s.copyWith(
          error: const PlaybackError(PlaybackErrorCode.unsupportedFormat),
        ),
      );
      await t.pump();
      expect(find.text('Этот файл не воспроизводится'), findsOneWidget);
      expect(find.text('Повторить'), findsOneWidget);
      expect(find.text('Код ошибки: unsupported_format'), findsOneWidget);
    });
  });
}
