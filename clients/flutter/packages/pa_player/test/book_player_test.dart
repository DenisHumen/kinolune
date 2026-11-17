// SPDX-License-Identifier: MPL-2.0
//
// MUS-018: the audiobook player — one time line over the parts, chapters,
// ±30 s, speed 0.5–3×, the sleep timer (minutes and «до конца главы») and
// the place saved per book.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

const _min = 60 * 1000;

/// Three parts of 30, 20 and 15 minutes; one chapter per part.
const _folder = AudiobookRequest(
  key: 'file:abc',
  title: 'Пикник на обочине',
  author: 'Стругацкие',
  parts: [
    BookPart(uri: '/b/01.mp3', durationMs: 30 * _min),
    BookPart(uri: '/b/02.mp3', durationMs: 20 * _min),
    BookPart(uri: '/b/10.mp3', durationMs: 15 * _min),
  ],
);

/// One m4b of 40 minutes with three chapters.
const _m4b = AudiobookRequest(
  key: 'file:def',
  title: 'Солярис',
  parts: [BookPart(uri: '/b/solaris.m4b', durationMs: 40 * _min)],
  chapters: [
    BookPlayerChapter(startMs: 0, title: 'Пришелец'),
    BookPlayerChapter(startMs: 10 * _min, title: 'Соляристы'),
    BookPlayerChapter(startMs: 25 * _min),
  ],
);

class _Timers {
  final List<(Duration, void Function())> made = [];
  Timer call(Duration d, void Function() f) {
    made.add((d, f));
    // The test fires it by hand.
    return Timer(Duration.zero, () {});
  }
}

typedef _Saved = (String, int, int, bool);

({BookPlayerController c, FakePlayerEngine e, List<_Saved> saved, _Timers t})
_player({SystemMediaSession? session}) {
  final e = FakePlayerEngine(loadedDuration: const Duration(minutes: 30));
  final saved = <_Saved>[];
  final timers = _Timers();
  final c = BookPlayerController(
    engineFactory: ({bool video = false}) => e,
    mediaSession: session ?? SystemMediaSession(enabled: false),
    timer: timers.call,
    onSave: (key, ms, total, {required finished}) async =>
        saved.add((key, ms, total, finished)),
  );
  addTearDown(c.dispose);
  return (c: c, e: e, saved: saved, t: timers);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('a folder of parts is one time line with a chapter per part', () async {
    final p = _player();
    await p.c.open(_folder, start: const Duration(minutes: 35));
    // 35 min = 5 min into the second part.
    expect(p.e.calls.first, 'open:3:1:${5 * _min}:true');
    expect(p.c.position, const Duration(minutes: 35));
    expect(p.c.duration, const Duration(minutes: 65));
    expect(p.c.chapters.map((c) => c.startMs), [0, 30 * _min, 50 * _min]);
    expect(p.c.chapterIndex, 1);
    expect(p.c.chapterBounds(1), (30 * _min, 50 * _min));

    // The engine plays on into the third part.
    p.e.emit(
      (s) => s.copyWith(playlistIndex: 2, position: const Duration(minutes: 1)),
    );
    expect(p.c.position, const Duration(minutes: 51));
    expect(p.c.chapterIndex, 2);
  });

  test('seeking across parts reopens there, within a part seeks', () async {
    final p = _player();
    await p.c.open(_folder);
    p.e.calls.clear();
    await p.c.seek(const Duration(minutes: 10));
    expect(p.e.calls, contains('seek:${10 * _min}'));
    p.e.calls.clear();
    await p.c.seek(const Duration(minutes: 55));
    expect(p.e.calls.first, 'open:3:2:${5 * _min}:true');
    expect(p.c.position, const Duration(minutes: 55));
    // Past the end stops at the end.
    await p.c.seek(const Duration(hours: 9));
    expect(p.c.position, const Duration(minutes: 65));
  });

  test('±30 s jumps, also over a part boundary', () async {
    final p = _player();
    await p.c.open(_folder, start: const Duration(minutes: 29, seconds: 50));
    await p.c.forward();
    expect(p.c.position, const Duration(minutes: 30, seconds: 20));
    expect(p.e.state.value.playlistIndex, 1);
    await p.c.back();
    expect(p.c.position, const Duration(minutes: 29, seconds: 50));
    expect(p.e.state.value.playlistIndex, 0);
    await p.c.seek(const Duration(seconds: 10));
    await p.c.back();
    expect(p.c.position, Duration.zero);
  });

  test('the chapters of an m4b; next and previous chapter', () async {
    final p = _player();
    await p.c.open(_m4b, start: const Duration(minutes: 12));
    expect(p.c.chapterIndex, 1);
    expect(p.c.chapters[1].title, 'Соляристы');
    await p.c.nextChapter();
    expect(p.c.position, const Duration(minutes: 25));
    expect(p.c.chapterIndex, 2);
    // Past the start of the chapter: back to its start.
    await p.c.seek(const Duration(minutes: 30));
    await p.c.previousChapter();
    expect(p.c.position, const Duration(minutes: 25));
    // Near its start: the chapter before.
    await p.c.previousChapter();
    expect(p.c.position, const Duration(minutes: 10));
    await p.c.jumpToChapter(0);
    expect(p.c.position, Duration.zero);
  });

  test('speed is 0.5–3×, kept across books', () async {
    final p = _player();
    await p.c.open(_folder);
    await p.c.setSpeed(1.5);
    expect(p.e.state.value.rate, 1.5);
    await p.c.setSpeed(5);
    expect(p.c.speed, BookPlayerController.maxSpeed);
    expect(p.e.state.value.rate, 3.0);
    await p.c.setSpeed(0.1);
    expect(p.c.speed, BookPlayerController.minSpeed);
    await p.c.setSpeed(2);
    await p.c.open(_m4b);
    expect(p.e.calls.last, 'rate:2.0');
    expect(p.e.state.value.rate, 2.0);
  });

  test('the sleep timer after minutes pauses', () async {
    final p = _player();
    await p.c.open(_folder);
    p.c.setSleepTimer(const SleepTimer.after(Duration(minutes: 15)));
    expect(p.c.sleepTimer, const SleepTimer.after(Duration(minutes: 15)));
    expect(p.c.sleepLeft, isNotNull);
    expect(p.t.made.single.$1, const Duration(minutes: 15));
    expect(p.c.playing, isTrue);
    p.t.made.single.$2(); // the time is up
    expect(p.c.playing, isFalse);
    expect(p.c.sleepTimer, isNull);
    // Off again.
    p.c.setSleepTimer(const SleepTimer.after(Duration(minutes: 5)));
    p.c.setSleepTimer(null);
    expect(p.c.sleepTimer, isNull);
  });

  test('«до конца главы» stops at the end of the chapter', () async {
    final p = _player();
    await p.c.open(_m4b, start: const Duration(minutes: 20));
    p.c.setSleepTimer(const SleepTimer.endOfChapter());
    p.e.advance(const Duration(minutes: 4));
    expect(p.c.playing, isTrue);
    // Past 25:00, the end of «Соляристы».
    p.e.advance(const Duration(minutes: 1, seconds: 1));
    await pumpEventQueue();
    expect(p.c.playing, isFalse);
    expect(p.c.sleepTimer, isNull);
    expect(p.c.position, const Duration(minutes: 25));

    // Of a folder: the end of the part.
    final q = _player();
    await q.c.open(_folder, start: const Duration(minutes: 29));
    q.c.setSleepTimer(const SleepTimer.endOfChapter());
    q.e.emit((s) => s.copyWith(playlistIndex: 1, position: Duration.zero));
    await pumpEventQueue();
    expect(q.c.playing, isFalse);
    expect(q.c.position, const Duration(minutes: 30));
  });

  test('the place is saved per book: on pause, seek and close', () async {
    final p = _player();
    await p.c.open(_folder, start: const Duration(minutes: 35));
    await p.c.seek(const Duration(minutes: 40));
    expect(p.saved.last, ('file:abc', 40 * _min, 65 * _min, false));
    p.e.advance(const Duration(minutes: 1));
    p.c.pause();
    await pumpEventQueue();
    expect(p.saved.last, ('file:abc', 41 * _min, 65 * _min, false));

    // Another book: the first is saved, then each has its own place.
    await p.c.open(_m4b, start: const Duration(minutes: 3));
    await p.c.seek(const Duration(minutes: 39, seconds: 30));
    expect(p.saved.last, ('file:def', 39 * _min + 30000, 40 * _min, true));
    await p.c.close();
    expect(p.c.book, isNull);
    expect(p.saved.where((s) => s.$1 == 'file:abc').last.$2, 41 * _min);
  });

  test('a book without a key is played but not saved', () async {
    final p = _player();
    await p.c.open(
      const AudiobookRequest(
        key: null,
        title: 'x',
        parts: [BookPart(uri: '/x.mp3')],
      ),
    );
    await p.c.seek(const Duration(minutes: 1));
    p.c.pause();
    expect(p.saved, isEmpty);
  });

  test(
    'the lock screen shows the chapter and its buttons move by chapters',
    () async {
      final calls = <MethodCall>[];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('test/book'), (
            c,
          ) async {
            calls.add(c);
            return null;
          });
      final session = SystemMediaSession(
        channel: const MethodChannel('test/book'),
        enabled: true,
      );
      final p = _player(session: session);
      await p.c.open(_m4b, start: const Duration(minutes: 12));
      await pumpEventQueue();
      final update =
          calls.lastWhere((c) => c.method == 'update').arguments as Map;
      expect(update['title'], 'Соляристы');
      expect(update['album'], 'Солярис');
      expect(update['hasNext'], isTrue);
      session.dispatch(MediaSessionCommand.next);
      await pumpEventQueue();
      expect(p.c.position, const Duration(minutes: 25));
      session.dispatch(MediaSessionCommand.pause);
      expect(p.c.playing, isFalse);
    },
  );
}
