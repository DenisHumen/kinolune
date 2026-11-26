// SPDX-License-Identifier: MPL-2.0
//
// PLAY-022: the sleep timer and «Вы ещё смотрите?» on fake clocks — the
// controller alone, then in the video player with Up Next.
import 'package:fake_async/fake_async.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

class _Host implements SleepTimerHost {
  final List<String> calls = [];
  double factor = 1.0;

  @override
  void sleepPause() => calls.add('pause');

  @override
  void sleepFade(double f) {
    factor = f;
    calls.add('fade:${f.toStringAsFixed(2)}');
  }
}

/// A controller whose clock is the fake time of [async].
SleepTimerController _timer(FakeAsync async, {int after = 3}) {
  final start = DateTime(2026, 11, 26, 23, 0);
  return SleepTimerController(
    clock: () => start.add(async.elapsed),
    stillWatchingAfter: after,
  );
}

/// A clock for widget tests, moved by [_Clock.pump].
class _Clock {
  DateTime now = DateTime(2026, 11, 26, 23, 0);

  /// Pumps [d] in 250 ms steps, moving the clock with it.
  Future<void> pump(
    WidgetTester t,
    Duration d, {
    FakePlayerEngine? play,
  }) async {
    const step = Duration(milliseconds: 250);
    var left = d;
    while (left > Duration.zero) {
      now = now.add(step);
      if (play != null && play.state.value.playing) play.advance(step);
      await t.pump(step);
      left -= step;
    }
  }
}

void main() {
  group('SleepTimerController', () {
    test('15 min: fades over the last 30 s, then pauses', () {
      fakeAsync((async) {
        final host = _Host();
        final c = _timer(async)..attach(host);
        c.set(SleepTimerOption.minutes15);
        expect(c.active, isTrue);
        expect(c.remaining, const Duration(minutes: 15));
        async.elapse(const Duration(minutes: 14, seconds: 29));
        expect(c.warning, isFalse);
        expect(host.calls, isEmpty);
        async.elapse(const Duration(seconds: 2));
        expect(c.warning, isTrue);
        expect(host.factor, lessThan(1.0));
        expect(host.factor, greaterThan(0.9));
        async.elapse(const Duration(seconds: 15));
        expect(host.factor, closeTo(0.45, 0.06));
        expect(host.calls, isNot(contains('pause')));
        async.elapse(const Duration(seconds: 14));
        // Paused, then the volume is back for the next play.
        expect(host.calls.where((c) => c == 'pause'), hasLength(1));
        expect(host.calls.last, 'fade:1.00');
        expect(c.active, isFalse);
        expect(c.warning, isFalse);
        // Nothing more after the pause.
        async.elapse(const Duration(minutes: 30));
        expect(host.calls.where((c) => c == 'pause'), hasLength(1));
      });
    });

    test('«Продлить» adds 15 min and brings the volume back', () {
      fakeAsync((async) {
        final host = _Host();
        final c = _timer(async)..attach(host);
        c.set(SleepTimerOption.minutes15);
        async.elapse(const Duration(minutes: 14, seconds: 50));
        expect(c.warning, isTrue);
        expect(host.factor, lessThan(0.4));
        c.extend();
        expect(host.factor, 1.0);
        expect(c.warning, isFalse);
        expect(c.remaining, const Duration(minutes: 15, seconds: 10));
        async.elapse(const Duration(minutes: 15));
        expect(host.calls, isNot(contains('pause')));
        async.elapse(const Duration(seconds: 11));
        expect(host.calls, contains('pause'));
      });
    });

    test('«в конце серии» follows the item and pauses at its end', () {
      fakeAsync((async) {
        final host = _Host();
        final c = _timer(async)..attach(host);
        c.set(SleepTimerOption.endOfItem);
        expect(c.holdsUpNext, isTrue);
        c.itemProgress(const Duration(minutes: 5), playing: true);
        expect(c.remaining, const Duration(minutes: 5));
        expect(c.warning, isFalse);
        c.itemProgress(const Duration(seconds: 15), playing: true);
        expect(c.warning, isTrue);
        expect(host.factor, closeTo(0.5, 0.01));
        // Paused by the user: no fade while nothing plays.
        c.itemProgress(const Duration(seconds: 15), playing: false);
        expect(host.factor, 1.0);
        c.itemProgress(const Duration(seconds: 3), playing: true);
        expect(host.factor, closeTo(0.1, 0.01));
        expect(c.itemEnded(), isTrue);
        expect(host.calls, contains('pause'));
        expect(host.factor, 1.0);
        expect(c.active, isFalse);
        // Off: the next end is not taken.
        expect(c.itemEnded(), isFalse);
      });
    });

    test('«Продлить» at the end of an episode is 15 min from now', () {
      fakeAsync((async) {
        final c = _timer(async)..attach(_Host());
        c.set(SleepTimerOption.endOfItem);
        c.itemProgress(const Duration(seconds: 10), playing: true);
        c.extend();
        expect(c.holdsUpNext, isFalse);
        expect(c.remaining, const Duration(minutes: 15));
      });
    });

    test('off cancels and restores the volume', () {
      fakeAsync((async) {
        final host = _Host();
        final c = _timer(async)..attach(host);
        c.set(SleepTimerOption.minutes15);
        async.elapse(const Duration(minutes: 14, seconds: 45));
        expect(host.factor, lessThan(1.0));
        c.cancel();
        expect(host.factor, 1.0);
        async.elapse(const Duration(hours: 1));
        expect(host.calls, isNot(contains('pause')));
      });
    });

    test('3 episodes without input: asks, then pauses after 60 s', () {
      fakeAsync((async) {
        final host = _Host();
        final c = _timer(async)..attach(host);
        c.autoAdvanced();
        c.autoAdvanced();
        expect(c.asking, isFalse);
        c.autoAdvanced();
        expect(c.asking, isTrue);
        expect(c.askRemaining, const Duration(seconds: 60));
        async.elapse(const Duration(seconds: 59));
        expect(host.calls, isEmpty);
        async.elapse(const Duration(seconds: 2));
        expect(host.calls, ['pause']);
        expect(c.asking, isFalse);
        expect(c.autoplaysInRow, 0);
      });
    });

    test('any press resets the count and answers the question', () {
      fakeAsync((async) {
        final host = _Host();
        final c = _timer(async)..attach(host);
        c.autoAdvanced();
        c.autoAdvanced();
        c.userInput();
        expect(c.autoplaysInRow, 0);
        c.autoAdvanced();
        c.autoAdvanced();
        expect(c.asking, isFalse);
        c.autoAdvanced();
        expect(c.asking, isTrue);
        async.elapse(const Duration(seconds: 30));
        c.userInput();
        expect(c.asking, isFalse);
        async.elapse(const Duration(minutes: 2));
        expect(host.calls, isEmpty);
      });
    });

    test('the number of episodes changes, is stored, 0 turns it off', () {
      fakeAsync((async) {
        final stored = <int>[];
        final start = DateTime(2026, 11, 26);
        final c = SleepTimerController(
          clock: () => start.add(async.elapsed),
          onStillWatchingAfterChanged: stored.add,
        );
        expect(c.stillWatchingAfter, 3);
        c.stillWatchingAfter = 2;
        c.autoAdvanced();
        c.autoAdvanced();
        expect(c.asking, isTrue);
        c.stillWatchingAfter = 0;
        expect(c.asking, isFalse);
        for (var i = 0; i < 10; i++) {
          c.autoAdvanced();
        }
        expect(c.asking, isFalse);
        expect(stored, [2, 0]);
        // Read back from the settings: not stored again.
        c.restoreStillWatchingAfter(5);
        expect(c.stillWatchingAfter, 5);
        expect(stored, [2, 0]);
      });
    });

    test(
      'a new host takes over a running timer (Twitch switching streams)',
      () {
        fakeAsync((async) {
          final a = _Host(), b = _Host();
          final c = _timer(async)..attach(a);
          c.set(SleepTimerOption.minutes15);
          c.detach(a);
          c.attach(b);
          async.elapse(const Duration(minutes: 15, seconds: 1));
          expect(a.calls, isNot(contains('pause')));
          expect(b.calls, contains('pause'));
        });
      },
    );
  });

  group('in the video player', () {
    late FakePlayerEngine engine;
    late _Clock clock;
    late SleepTimerController sleep;
    late List<(String, int)> saves;

    UpNextItem item(String id) => UpNextItem(
      request: PlayRequest(uri: '/tv/$id.mkv', title: id.toUpperCase()),
      onProgress: (p, _) => saves.add((id, p)),
      loadNext: () async => item('${id}n'),
    );

    Future<PlayerScreenState> open(WidgetTester t) async {
      clock = _Clock();
      saves = [];
      engine = FakePlayerEngine(loadedDuration: const Duration(minutes: 20));
      sleep = SleepTimerController(clock: () => clock.now);
      addTearDown(sleep.dispose);
      await t.pumpWidget(
        MaterialApp(
          home: PlayerScreen(
            request: const PlayRequest(uri: '/tv/e1.mkv', title: 'E1'),
            onProgress: (p, _) => saves.add(('e1', p)),
            engineFactory: ({bool video = true}) => engine,
            loadNext: () async => item('e2'),
            sleepTimer: sleep,
            upNextCountdown: const Duration(seconds: 3),
          ),
        ),
      );
      await t.pump();
      await t.pump();
      return t.state<PlayerScreenState>(find.byType(PlayerScreen));
    }

    testWidgets('«в конце серии» pauses at the end, no Up Next', (t) async {
      await open(t);
      sleep.set(SleepTimerOption.endOfItem);
      engine.seek(const Duration(minutes: 19, seconds: 40));
      await t.pump();
      await clock.pump(t, const Duration(seconds: 5), play: engine);
      // The last 30 s: the warning and the fade.
      expect(find.byKey(const ValueKey('sleep-warning')), findsOneWidget);
      expect(engine.state.value.volume, lessThan(100));
      engine.finish();
      await t.pump();
      expect(engine.state.value.volume, 100);
      expect(engine.state.value.playing, isFalse);
      expect(engine.calls, contains('pause'));
      expect(find.byKey(const ValueKey('up-next')), findsNothing);
      // The end was saved.
      expect(saves.last, ('e1', 20 * 60 * 1000));
      // No countdown starts later either.
      await clock.pump(t, const Duration(seconds: 10));
      expect(find.byKey(const ValueKey('up-next')), findsNothing);
      expect(engine.uris, ['/tv/e1.mkv']);
    });

    testWidgets('15 min: the volume fades, then the video pauses', (t) async {
      await open(t);
      sleep.set(SleepTimerOption.minutes15);
      await clock.pump(t, const Duration(minutes: 14, seconds: 40));
      expect(engine.state.value.playing, isTrue);
      expect(find.byKey(const ValueKey('sleep-warning')), findsOneWidget);
      expect(
        find.textContaining(RegExp(r'^Pauses in 2[01] s$')),
        findsOneWidget,
      );
      expect(engine.state.value.volume, closeTo(66.7, 2));
      await clock.pump(t, const Duration(seconds: 21));
      expect(engine.state.value.playing, isFalse);
      expect(engine.state.value.volume, 100);
      expect(find.byKey(const ValueKey('sleep-warning')), findsNothing);
      expect(saves, isNotEmpty);
    });

    testWidgets('«Продлить» on the warning adds time', (t) async {
      await open(t);
      sleep.set(SleepTimerOption.minutes15);
      await clock.pump(t, const Duration(minutes: 14, seconds: 45));
      await t.tap(find.byKey(const ValueKey('sleep-extend')));
      await t.pump();
      expect(engine.state.value.volume, 100);
      expect(sleep.remaining!.inMinutes, 15);
      await clock.pump(t, const Duration(seconds: 30));
      expect(engine.state.value.playing, isTrue);
      sleep.cancel();
    });

    testWidgets('3 automatic episodes: «Вы ещё смотрите?», then pause', (
      t,
    ) async {
      await open(t);
      for (var i = 0; i < 3; i++) {
        engine.finish();
        await t.pump();
        await t.pump();
        expect(find.byKey(const ValueKey('up-next')), findsOneWidget);
        await clock.pump(t, const Duration(seconds: 4));
      }
      expect(engine.uris.single, '/tv/e2nn.mkv');
      expect(find.byKey(const ValueKey('still-watching')), findsOneWidget);
      expect(engine.state.value.playing, isTrue);
      await clock.pump(t, const Duration(seconds: 61));
      expect(engine.state.value.playing, isFalse);
      expect(find.byKey(const ValueKey('still-watching')), findsNothing);
    });

    testWidgets('a key press resets the count', (t) async {
      await open(t);
      for (var i = 0; i < 2; i++) {
        engine.finish();
        await t.pump();
        await t.pump();
        await clock.pump(t, const Duration(seconds: 4));
      }
      expect(sleep.autoplaysInRow, 2);
      await t.sendKeyEvent(LogicalKeyboardKey.keyI);
      await t.pump();
      expect(sleep.autoplaysInRow, 0);
      engine.finish();
      await t.pump();
      await t.pump();
      await clock.pump(t, const Duration(seconds: 4));
      expect(find.byKey(const ValueKey('still-watching')), findsNothing);
    });

    testWidgets('a key answers «Вы ещё смотрите?» and does nothing else', (
      t,
    ) async {
      await open(t);
      sleep.restoreStillWatchingAfter(1);
      engine.finish();
      await t.pump();
      await t.pump();
      await clock.pump(t, const Duration(seconds: 4));
      expect(find.byKey(const ValueKey('still-watching')), findsOneWidget);
      await t.sendKeyEvent(LogicalKeyboardKey.space);
      await t.pump();
      expect(find.byKey(const ValueKey('still-watching')), findsNothing);
      expect(engine.state.value.playing, isTrue);
    });

    testWidgets('leaving the player cancels the timer', (t) async {
      final s = await open(t);
      sleep.set(SleepTimerOption.minutes30);
      s.exit();
      await t.pump();
      expect(sleep.active, isFalse);
    });
  });
}
