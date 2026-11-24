// SPDX-License-Identifier: MPL-2.0
//
// MUS-006: LRC parsing (offsets, several time tags on a line, word timing,
// ID tags), the line sung at a position (±100 ms), and Now Playing's
// lyrics: highlight in time, tap a line to seek, «Текст не найден», plain
// text, the fullscreen mode and the remote/keyboard.

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

Duration _ms(int ms) => Duration(milliseconds: ms);

const _song = '''
[ar:Artist]
[ti:Song]
[al:Album]
[by:someone]
[length: 03:12]
[00:01.00]First line
[00:04.50]Second line
[00:08.250]Third line
[00:12.00][00:20.00]Chorus
[00:16.00]
[00:18:50]Colon fraction
''';

List<PlayRequest> _tracks(int n) => [
  for (var i = 0; i < n; i++)
    PlayRequest(uri: '/music/$i.flac', title: 'Track $i', artist: 'Artist'),
];

void _surface(WidgetTester t, Size size) {
  t.view.physicalSize = size;
  t.view.devicePixelRatio = 1;
  addTearDown(t.view.reset);
}

void main() {
  group('parseLrc', () {
    test('times, ID tags, several tags per line, gaps', () {
      final lines = parseLrc(_song);
      expect(lines, [
        LyricLine(_ms(1000), 'First line'),
        LyricLine(_ms(4500), 'Second line'),
        LyricLine(_ms(8250), 'Third line'),
        LyricLine(_ms(12000), 'Chorus'),
        LyricLine(_ms(16000), ''),
        LyricLine(_ms(18500), 'Colon fraction'),
        LyricLine(_ms(20000), 'Chorus'),
      ]);
    });

    test('fractions: tenths, hundredths, milliseconds, none', () {
      expect(parseLrc('[01:02.5]a').single.time, _ms(62500));
      expect(parseLrc('[01:02.05]a').single.time, _ms(62050));
      expect(parseLrc('[01:02.005]a').single.time, _ms(62005));
      expect(parseLrc('[01:02]a').single.time, _ms(62000));
      expect(parseLrc('[120:00.00]long mix').single.time, _ms(7200000));
    });

    test('offset: positive shows lines earlier, never before 0', () {
      final early = parseLrc('[offset:+500]\n[00:10.00]a\n[00:00.20]b');
      expect(early, [LyricLine(Duration.zero, 'b'), LyricLine(_ms(9500), 'a')]);
      final late = parseLrc('[offset:-250]\n[00:10.00]a');
      expect(late.single.time, _ms(10250));
      final bad = parseLrc('[offset:soon]\n[00:10.00]a');
      expect(bad.single.time, _ms(10000));
    });

    test('word timing is removed, CRLF and BOM are fine', () {
      final lines = parseLrc(
        '﻿[00:01.00]<00:01.00>Hello <00:01.50>world\r\n'
        '[00:02.00]  spaced   out  \r\n',
      );
      expect(lines.map((l) => l.text), ['Hello world', 'spaced out']);
    });

    test('same time keeps file order; unsorted input is sorted', () {
      final lines = parseLrc('[00:05.00]b\n[00:01.00]a\n[00:05.00]c');
      expect(lines.map((l) => l.text), ['a', 'b', 'c']);
    });

    test('text without time tags is plain lyrics', () {
      expect(parseLrc('Just words\nand more'), isEmpty);
      final plain = lyricsFromText(
        '[ar:Someone]\nJust words\nand more\n',
        LyricsSource.embedded,
      )!;
      expect(plain.synced, isFalse);
      expect(plain.plainText, 'Just words\nand more');
      final synced = lyricsFromText(_song, LyricsSource.lrcFile)!;
      expect(synced.synced, isTrue);
      expect(synced.source, LyricsSource.lrcFile);
      expect(lyricsFromText('  \n', LyricsSource.embedded), isNull);
      expect(lyricsFromText(null, LyricsSource.embedded), isNull);
    });
  });

  group('lyricLineAt (±100 ms)', () {
    final lines = parseLrc(_song);

    test('before the first line: none', () {
      expect(lyricLineAt(lines, Duration.zero), -1);
      expect(lyricLineAt(lines, _ms(999)), -1);
      expect(lyricLineAt(const [], _ms(5000)), -1);
    });

    test('a line starts exactly at its time, not 100 ms early or late', () {
      for (final (i, l) in lines.indexed) {
        expect(lyricLineAt(lines, l.time), i, reason: '$l at its time');
        expect(
          lyricLineAt(lines, l.time - _ms(100)),
          i - 1,
          reason: '$l 100 ms early',
        );
        // Still this line 100 ms in (all lines are longer than that).
        expect(lyricLineAt(lines, l.time + _ms(100)), i, reason: '$l +100');
      }
    });

    test('after the last line: the last one', () {
      expect(lyricLineAt(lines, const Duration(hours: 1)), lines.length - 1);
    });
  });

  group('Now Playing lyrics', () {
    late FakePlayerEngine engine;
    late MusicPlayerController c;

    setUp(() {
      engine = FakePlayerEngine(loadedDuration: const Duration(minutes: 4));
      c = MusicPlayerController(engineFactory: ({bool video = true}) => engine);
    });

    tearDown(() => c.dispose());

    Future<void> pumpScreen(
      WidgetTester t,
      LyricsLoader loader, {
      Size size = const Size(1600, 900),
    }) async {
      _surface(t, size);
      await t.pumpWidget(
        MaterialApp(
          home: NowPlayingScreen(controller: c, lyrics: loader),
        ),
      );
      await t.pump();
    }

    String? currentLine(WidgetTester t) {
      final f = find.byKey(const ValueKey('lyric-current'));
      if (f.evaluate().isEmpty) return null;
      return t.widget<Text>(f).data;
    }

    for (final size in [const Size(400, 900), const Size(1600, 900)]) {
      testWidgets('${size.width}px: highlight follows the music, tap seeks', (
        t,
      ) async {
        await c.play(_tracks(2));
        c.togglePlay(); // paused: the position is exactly the engine's
        await pumpScreen(
          t,
          (_) async => lyricsFromText(_song, LyricsSource.lrcFile),
          size: size,
        );
        // No lyrics button pressed yet: nothing looked up.
        expect(find.byKey(const ValueKey('np-lyrics-panel')), findsNothing);

        await t.tap(find.byKey(const ValueKey('np-lyrics')));
        await t.pump();
        await t.pump();
        expect(find.byKey(const ValueKey('np-lyrics-panel')), findsOneWidget);
        expect(find.text('First line'), findsOneWidget);
        expect(currentLine(t), isNull, reason: 'before the first line');

        engine.advance(_ms(4400));
        await t.pump();
        expect(currentLine(t), 'First line', reason: '100 ms before line 2');
        engine.advance(_ms(100));
        await t.pump();
        expect(currentLine(t), 'Second line', reason: 'at 4.5 s');
        engine.advance(_ms(3750 + 100));
        await t.pump();
        expect(currentLine(t), 'Third line', reason: '100 ms after 8.25 s');

        // Tap a line: seek to its time and highlight it.
        await t.ensureVisible(find.text('Colon fraction'));
        await t.pump(const Duration(milliseconds: 500));
        await t.tap(find.text('Colon fraction'));
        await t.pump();
        expect(engine.calls.last, 'seek:18500');
        expect(currentLine(t), 'Colon fraction');
        await t.pump(const Duration(seconds: 1));
      });
    }

    testWidgets('while playing, the highlight moves between engine reports', (
      t,
    ) async {
      await c.play(_tracks(1));
      await pumpScreen(
        t,
        (_) async => Lyrics.synced([
          LyricLine(_ms(500), 'one'),
          LyricLine(_ms(1000), 'two'),
        ], source: LyricsSource.lrclib),
      );
      await t.tap(find.byKey(const ValueKey('np-lyrics')));
      await t.pump();
      await t.pump();
      expect(find.text('Lyrics from LRCLIB'), findsOneWidget);
      // The engine says nothing more; the frame clock carries it on.
      await t.pump(_ms(450));
      await t.pump(_ms(100));
      expect(currentLine(t), 'one');
      await t.pump(_ms(500));
      expect(currentLine(t), 'two');
      c.togglePlay();
      await t.pump();
    });

    testWidgets('«not found», plain text, lookup per track', (t) async {
      final asked = <String>[];
      final plain = Completer<Lyrics?>();
      await c.play(_tracks(2));
      c.togglePlay();
      await pumpScreen(t, (r) {
        asked.add(r.title);
        return r.title == 'Track 0' ? Future.value() : plain.future;
      });
      await t.tap(find.byKey(const ValueKey('np-lyrics')));
      await t.pump();
      await t.pump();
      expect(find.text('Lyrics not found'), findsOneWidget);

      c.next();
      await t.pump();
      expect(find.text('Looking for lyrics…'), findsOneWidget);
      plain.complete(
        const Lyrics.plain(
          'Verse one\nVerse two',
          source: LyricsSource.embedded,
        ),
      );
      await t.pump();
      await t.pump();
      expect(find.text('Verse one\nVerse two'), findsOneWidget);
      expect(asked, ['Track 0', 'Track 1']);

      // Hiding the lyrics brings the queue back.
      await t.tap(find.byKey(const ValueKey('np-lyrics')));
      await t.pump();
      expect(find.byKey(const ValueKey('np-lyrics-panel')), findsNothing);
    });

    testWidgets('fullscreen lyrics open and close with Escape', (t) async {
      await c.play(_tracks(1));
      c.togglePlay();
      engine.advance(_ms(5000));
      await pumpScreen(
        t,
        (_) async => lyricsFromText(_song, LyricsSource.embeddedSynced),
      );
      await t.tap(find.byKey(const ValueKey('np-lyrics')));
      await t.pump();
      await t.pump();
      await t.tap(find.byKey(const ValueKey('np-lyrics-fullscreen')));
      await t.pump();
      await t.pump(const Duration(milliseconds: 300));
      expect(find.byKey(const ValueKey('lyrics-fullscreen')), findsOneWidget);
      expect(currentLine(t), 'Second line');

      await t.sendKeyEvent(LogicalKeyboardKey.escape);
      // maybePop() is asynchronous, so the 250 ms fade-out starts a frame
      // later and the route leaves the tree one frame after it ends.
      for (var i = 0; i < 3; i++) {
        await t.pump(const Duration(milliseconds: 200));
      }
      await t.pump();
      expect(find.byKey(const ValueKey('lyrics-fullscreen')), findsNothing);
    });

    testWidgets('keyboard / remote: L shows lyrics, Enter on a line seeks', (
      t,
    ) async {
      await c.play(_tracks(1));
      c.togglePlay();
      await pumpScreen(
        t,
        (_) async => lyricsFromText(_song, LyricsSource.lrcFile),
      );
      await t.sendKeyEvent(LogicalKeyboardKey.keyL);
      await t.pump();
      await t.pump();
      expect(find.byKey(const ValueKey('np-lyrics-panel')), findsOneWidget);

      // Focus the third line as the remote would and press select / Enter.
      final line = find.byKey(const ValueKey('lyric-line-2'));
      Focus.of(
        t.element(find.descendant(of: line, matching: find.byType(Text))),
      ).requestFocus();
      await t.pump();
      await t.sendKeyEvent(LogicalKeyboardKey.enter);
      await t.pump();
      expect(engine.calls.last, 'seek:8250');
      expect(currentLine(t), 'Third line');
      await t.pump(const Duration(seconds: 1));
    });

    testWidgets('no loader: no lyrics button', (t) async {
      _surface(t, const Size(1600, 900));
      await c.play(_tracks(1));
      await t.pumpWidget(MaterialApp(home: NowPlayingScreen(controller: c)));
      await t.pump();
      expect(find.byKey(const ValueKey('np-lyrics')), findsNothing);
    });
  });
}
