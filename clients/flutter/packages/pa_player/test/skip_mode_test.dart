// SPDX-License-Identifier: MPL-2.0
//
// PLAY-011: what the player does in an intro or credits segment — the
// button, an automatic skip after 3 s with a countdown, at once, or nothing.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

const _intro = SkipSegment(
  kind: SkipKind.intro,
  start: Duration(seconds: 30),
  end: Duration(seconds: 90),
);
const _credits = SkipSegment(
  kind: SkipKind.credits,
  start: Duration(minutes: 28),
  end: Duration(minutes: 30),
);

class _Harness {
  _Harness(this.mode);

  final SkipMode mode;
  final engine = FakePlayerEngine(loadedDuration: const Duration(minutes: 30));
  Future<UpNextItem?> Function()? loadNext;

  Future<void> open(WidgetTester t, {int startMs = 0}) async {
    await t.pumpWidget(
      MaterialApp(
        home: PlayerScreen(
          request: PlayRequest(
            uri: '/m/e1.mkv',
            title: 'E1',
            startPositionMs: startMs,
            skipSegments: const [_intro, _credits],
          ),
          onProgress: (_, _) {},
          engineFactory: ({bool video = true}) => engine,
          loadNext: loadNext,
          skipMode: mode,
        ),
      ),
    );
    await t.pump(const Duration(milliseconds: 300));
  }

  Future<void> advance(WidgetTester t, Duration d) async {
    engine.advance(d);
    await t.pump();
  }

  List<String> get seeks =>
      engine.calls.where((c) => c.startsWith('seek:')).toList();
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await t.pump(const Duration(seconds: 1));
}

void main() {
  testWidgets('button: nothing happens by itself', (t) async {
    final h = _Harness(SkipMode.button);
    await h.open(t, startMs: 31000);
    await h.advance(t, const Duration(seconds: 10));
    expect(find.text('Skip Intro'), findsOneWidget);
    expect(h.seeks, isEmpty);
    await _close(t);
  });

  testWidgets('auto: counts down 3 s of playback, then skips once', (t) async {
    final h = _Harness(SkipMode.auto);
    await h.open(t, startMs: 29000);
    expect(find.textContaining('Skip Intro'), findsNothing);
    await h.advance(t, const Duration(seconds: 1)); // 30 s: enters
    expect(find.text('Skip Intro (3)'), findsOneWidget);
    await h.advance(t, const Duration(milliseconds: 1500));
    expect(find.text('Skip Intro (2)'), findsOneWidget);
    expect(h.seeks, isEmpty);
    // Paused: the count stops.
    await h.engine.pause();
    await t.pump();
    expect(h.seeks, isEmpty);
    await h.engine.play();
    await h.advance(t, const Duration(milliseconds: 1500));
    expect(h.seeks, ['seek:90000']);
    // Back into the intro: offered, not skipped again.
    await h.engine.seek(const Duration(seconds: 40));
    await h.advance(t, const Duration(seconds: 5));
    expect(h.seeks, ['seek:90000', 'seek:40000']);
    expect(find.text('Skip Intro'), findsOneWidget);
    await _close(t);
  });

  testWidgets('instant: skips on entry; credits to the end play Up Next', (
    t,
  ) async {
    final h = _Harness(SkipMode.instant)
      ..loadNext = () async => UpNextItem(
        request: const PlayRequest(uri: '/m/e2.mkv', title: 'Episode 2'),
        onProgress: (_, _) {},
      );
    await h.open(t, startMs: 29500);
    await h.advance(t, const Duration(seconds: 1));
    expect(h.seeks, ['seek:90000']);
    await h.engine.seek(const Duration(minutes: 27, seconds: 59));
    await h.advance(t, const Duration(seconds: 2));
    // Episode 2 opened instead of seeking to the end.
    expect(h.engine.uris, ['/m/e2.mkv']);
    await _close(t);
  });

  testWidgets('off: no button, no skip', (t) async {
    final h = _Harness(SkipMode.off);
    await h.open(t, startMs: 31000);
    await h.advance(t, const Duration(seconds: 10));
    expect(find.byKey(const ValueKey('osd-skip-segment')), findsNothing);
    expect(h.seeks, isEmpty);
    await _close(t);
  });
}
