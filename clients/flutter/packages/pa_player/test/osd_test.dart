// SPDX-License-Identifier: MPL-2.0
//
// UI-013: the player OSD on the mock engine — ends-at, configurable skip,
// chapters, Skip Intro, Up Next in the credits, zoom, stats, double-tap,
// swipes, pinch and the 2× hold.
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

const _chapters = [
  PlaybackChapter(start: Duration.zero, title: 'Opening'),
  PlaybackChapter(start: Duration(minutes: 5), title: 'The heist'),
  PlaybackChapter(start: Duration(minutes: 20)),
];

class _Harness {
  _Harness({this.seekStep = const Duration(seconds: 10)});

  final Duration seekStep;
  final engine = FakePlayerEngine(loadedDuration: const Duration(minutes: 30));
  final brightness = _FakeBrightness();
  Future<UpNextItem?> Function()? loadNext;

  Widget screen(PlayRequest request) => MaterialApp(
    home: PlayerScreen(
      request: request,
      onProgress: (_, _) {},
      engineFactory: ({bool video = true}) => engine,
      seekStep: seekStep,
      loadNext: loadNext,
      screenBrightness: brightness,
      clock: () => DateTime(2026, 11, 14, 20, 0),
    ),
  );

  PlayerScreenState state(WidgetTester t) =>
      t.state<PlayerScreenState>(find.byType(PlayerScreen));
}

class _FakeBrightness implements ScreenBrightness {
  double? written;
  bool wasReset = false;

  @override
  bool get dimsPicture => true;

  @override
  Future<double> read() async => 1.0;

  @override
  Future<void> write(double value) async => written = value;

  @override
  Future<void> reset() async => wasReset = true;
}

/// The chapter ticks on the scrubber.
final _markers = find.byWidgetPredicate(
  (w) =>
      w.key is ValueKey<String> &&
      (w.key! as ValueKey<String>).value.startsWith('chapter-marker-'),
);

const _movie = PlayRequest(uri: '/media/movie.mkv', title: 'Movie');

Future<_Harness> _open(
  WidgetTester t, {
  PlayRequest request = _movie,
  _Harness? harness,
  List<PlaybackChapter> chapters = const [],
}) async {
  final h = harness ?? _Harness();
  h.engine.openChapters = chapters;
  await t.pumpWidget(h.screen(request));
  await t.pump(const Duration(milliseconds: 300));
  return h;
}

Offset _videoAt(WidgetTester t, double fx, double fy) {
  final r = t.getRect(find.byKey(const ValueKey('osd-video')));
  return Offset(r.left + r.width * fx, r.top + r.height * fy);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await t.pump(const Duration(seconds: 1));
}

void main() {
  group('osd model', () {
    test('chapter lookup and navigation', () {
      expect(chapterIndexAt(_chapters, Duration.zero), 0);
      expect(chapterIndexAt(_chapters, const Duration(minutes: 7)), 1);
      expect(chapterIndexAt(_chapters, const Duration(hours: 1)), 2);
      expect(chapterIndexAt(const [], Duration.zero), -1);
      expect(
        nextChapterStart(_chapters, const Duration(minutes: 7)),
        const Duration(minutes: 20),
      );
      expect(nextChapterStart(_chapters, const Duration(minutes: 25)), isNull);
      // Well into a chapter: back to its start; just after it: the one
      // before.
      expect(
        previousChapterStart(_chapters, const Duration(minutes: 7)),
        const Duration(minutes: 5),
      );
      expect(
        previousChapterStart(_chapters, const Duration(minutes: 5, seconds: 1)),
        Duration.zero,
      );
    });

    test('skip segment at the playhead', () {
      const intro = SkipSegment(
        kind: SkipKind.intro,
        start: Duration(seconds: 30),
        end: Duration(seconds: 90),
      );
      expect(skipSegmentAt(const [intro], const Duration(seconds: 10)), isNull);
      expect(skipSegmentAt(const [intro], const Duration(seconds: 30)), intro);
      // Nothing to skip in the last second.
      expect(
        skipSegmentAt(const [intro], const Duration(milliseconds: 89500)),
        isNull,
      );
    });

    test('zoom modes cycle and map to fits', () {
      expect(VideoZoom.fit.next, VideoZoom.fill);
      expect(VideoZoom.ratio235.next, VideoZoom.fit);
      expect(VideoZoom.fit.boxFit, BoxFit.contain);
      expect(VideoZoom.fill.boxFit, BoxFit.cover);
      expect(VideoZoom.ratio4x3.aspect, closeTo(4 / 3, 1e-9));
      expect(clampPinchScale(9), kMaxPinchScale);
      expect(clampPinchScale(0.2), kMinPinchScale);
    });

    test('ends at follows the speed', () {
      const s = EngineState(
        position: Duration(minutes: 10),
        duration: Duration(minutes: 70),
      );
      final now = DateTime(2026, 11, 14, 20, 0);
      expect(endsAtLabel(now, s), '21:00');
      expect(endsAtLabel(now, s.copyWith(rate: 2)), '20:30');
      expect(
        remainingOf(s.copyWith(position: const Duration(hours: 2))),
        Duration.zero,
      );
    });
  });

  testWidgets('remaining and ends at under the scrubber', (t) async {
    final h = await _open(t);
    await h.engine.seek(const Duration(minutes: 10));
    await t.pump();
    // 20 min left at 1× from 20:00.
    expect(find.text('-20:00 · ends at 20:20'), findsOneWidget);
    await h.engine.setRate(2);
    await t.pump();
    expect(find.text('-20:00 · ends at 20:10'), findsOneWidget);
    await _close(t);
  });

  testWidgets('skip step is configurable', (t) async {
    final h = await _open(
      t,
      harness: _Harness(seekStep: const Duration(seconds: 30)),
    );
    expect(find.byTooltip('Back 30 s'), findsOneWidget);
    expect(find.byTooltip('Forward 30 s'), findsOneWidget);
    await t.tap(find.byKey(const ValueKey('osd-fwd10')));
    await t.pump();
    expect(h.engine.calls, contains('seek:30000'));
    await t.sendKeyEvent(LogicalKeyboardKey.arrowRight);
    await t.pump();
    expect(h.engine.calls, contains('seek:60000'));
    await _close(t);
  });

  testWidgets('chapters: markers, current chapter, keys and panel', (t) async {
    final h = await _open(t, chapters: _chapters);
    // A marker for every chapter after the start.
    expect(_markers, findsNWidgets(2));
    expect(find.text('Opening'), findsOneWidget);

    await t.sendKeyEvent(LogicalKeyboardKey.pageDown);
    await t.pump();
    expect(h.engine.calls, contains('seek:300000'));
    expect(find.text('The heist'), findsWidgets);

    await t.sendKeyEvent(LogicalKeyboardKey.pageDown);
    await t.pump();
    expect(h.engine.calls, contains('seek:1200000'));
    // Untitled chapter.
    expect(find.text('Chapter 3'), findsWidgets);

    await t.tap(find.byKey(const ValueKey('osd-chapters')));
    await t.pumpAndSettle();
    expect(find.byKey(const ValueKey('panel-chapters')), findsOneWidget);
    await t.tap(find.byKey(const ValueKey('chapter-1')));
    await t.pumpAndSettle();
    expect(h.engine.calls.last, 'seek:300000');
    expect(find.byKey(const ValueKey('panel-chapters')), findsNothing);
    await _close(t);
  });

  testWidgets('app chapters win over the file ones', (t) async {
    await _open(
      t,
      chapters: _chapters,
      request: const PlayRequest(
        uri: '/m/vod.mp4',
        title: 'VOD',
        chapters: [
          PlaybackChapter(start: Duration.zero, title: 'Just Chatting'),
          PlaybackChapter(start: Duration(minutes: 12), title: 'Elden Ring'),
        ],
      ),
    );
    expect(_markers, findsOneWidget);
    expect(find.text('Just Chatting'), findsOneWidget);
    await _close(t);
  });

  testWidgets('no chapters: no markers and no chapters button', (t) async {
    await _open(t);
    expect(_markers, findsNothing);
    expect(find.byKey(const ValueKey('osd-chapters')), findsNothing);
    await _close(t);
  });

  testWidgets('Skip Intro: button and Select while the OSD is hidden', (
    t,
  ) async {
    final h = await _open(
      t,
      request: const PlayRequest(
        uri: '/m/e1.mkv',
        title: 'E1',
        startPositionMs: 40000,
        skipSegments: [
          SkipSegment(
            kind: SkipKind.intro,
            start: Duration(seconds: 30),
            end: Duration(seconds: 90),
          ),
        ],
      ),
    );
    expect(find.text('Skip Intro'), findsOneWidget);
    await t.tap(find.byKey(const ValueKey('osd-skip-segment')));
    await t.pump();
    expect(h.engine.calls.last, 'seek:90000');
    expect(find.text('Skip Intro'), findsNothing);

    // Back into the intro, OSD hidden: Select skips instead of pausing.
    await h.engine.seek(const Duration(seconds: 45));
    await t.pump(const Duration(seconds: 5));
    await t.pumpAndSettle();
    expect(h.state(t).osd.visible, isFalse);
    expect(find.text('Skip Intro'), findsOneWidget);
    await t.sendKeyEvent(LogicalKeyboardKey.select);
    await t.pump();
    expect(h.engine.calls.last, 'seek:90000');
    expect(h.engine.state.value.playing, isTrue);
    await _close(t);
  });

  testWidgets('Up Next comes up during the credits', (t) async {
    final h = _Harness()
      ..loadNext = () async => UpNextItem(
        request: const PlayRequest(uri: '/m/e2.mkv', title: 'Episode 2'),
        onProgress: (_, _) {},
      );
    await _open(
      t,
      harness: h,
      request: const PlayRequest(
        uri: '/m/e1.mkv',
        title: 'E1',
        skipSegments: [
          SkipSegment(
            kind: SkipKind.credits,
            start: Duration(minutes: 28),
            end: Duration(minutes: 30),
          ),
        ],
      ),
    );
    expect(find.byKey(const ValueKey('up-next')), findsNothing);
    await h.engine.seek(const Duration(minutes: 28, seconds: 5));
    await t.pump();
    expect(find.byKey(const ValueKey('up-next')), findsOneWidget);
    expect(find.text('Episode 2'), findsOneWidget);
    // The card replaces "Skip Credits".
    expect(find.text('Skip Credits'), findsNothing);
    // Back out of the credits: the card goes away.
    await h.engine.seek(const Duration(minutes: 10));
    await t.pump();
    expect(find.byKey(const ValueKey('up-next')), findsNothing);
    await _close(t);
  });

  testWidgets('zoom: Z cycles, the panel forces an aspect', (t) async {
    final h = await _open(t);
    await t.sendKeyEvent(LogicalKeyboardKey.keyZ);
    await t.pump();
    expect(h.state(t).zoom.value, VideoZoom.fill);
    expect(find.text('Fill'), findsOneWidget); // feedback pill

    await t.tap(find.byKey(const ValueKey('osd-zoom')));
    await t.pumpAndSettle();
    await t.tap(find.byKey(const ValueKey('zoom-ratio4x3')));
    await t.pumpAndSettle();
    expect(h.state(t).zoom.value, VideoZoom.ratio4x3);
    final box = t.getSize(find.byKey(const ValueKey('osd-video-box')));
    expect(box.width / box.height, closeTo(4 / 3, 0.01));
    expect(box.height, 600); // 800x600 test window: full height
    await _close(t);
  });

  testWidgets('stats overlay: S and the info panel switch', (t) async {
    final h = await _open(t);
    await t.sendKeyEvent(LogicalKeyboardKey.keyS);
    await t.pump();
    expect(find.byKey(const ValueKey('osd-stats')), findsOneWidget);
    expect(h.engine.statsActive, isTrue);
    expect(find.textContaining('8.0 Mbps'), findsOneWidget);

    await t.tap(find.byKey(const ValueKey('osd-info')));
    await t.pumpAndSettle();
    await t.tap(find.byKey(const ValueKey('info-stats')));
    await t.pumpAndSettle();
    expect(find.byKey(const ValueKey('osd-stats')), findsNothing);
    expect(h.engine.statsActive, isFalse);
    await _close(t);
  });

  // Found on the emulator: the stats overflowed a phone in landscape.
  testWidgets('stats fit a phone in landscape', (t) async {
    t.view.physicalSize = const Size(1440, 720);
    t.view.devicePixelRatio = 2;
    addTearDown(t.view.reset);
    final h = await _open(t);
    h.engine.emit(
      (s) => s.copyWith(
        info: {for (var i = 0; i < 14; i++) 'Key $i': 'value $i'},
      ),
    );
    await t.sendKeyEvent(LogicalKeyboardKey.keyS);
    await t.pump();
    expect(t.takeException(), isNull);
    final r = t.getRect(find.byKey(const ValueKey('osd-stats')));
    expect(r.bottom, lessThanOrEqualTo(360));
    await _close(t);
  });

  testWidgets('speed: [ and ] step through the speeds', (t) async {
    final h = await _open(t);
    await t.sendKeyEvent(LogicalKeyboardKey.bracketRight);
    await t.pump();
    expect(h.engine.state.value.rate, 1.25);
    await t.sendKeyEvent(LogicalKeyboardKey.bracketLeft);
    await t.sendKeyEvent(LogicalKeyboardKey.bracketLeft);
    await t.pump();
    expect(h.engine.state.value.rate, 0.75);
    await _close(t);
  });

  testWidgets('double tap on the sides seeks and adds up', (t) async {
    final h = await _open(t);
    final right = _videoAt(t, 0.85, 0.45);
    for (var i = 0; i < 2; i++) {
      await t.tapAt(right);
      await t.pump(const Duration(milliseconds: 50));
      await t.tapAt(right);
      await t.pump(const Duration(milliseconds: 100));
    }
    expect(h.engine.calls.where((c) => c.startsWith('seek:')), [
      'seek:10000',
      'seek:20000',
    ]);
    expect(find.text('+20 s'), findsOneWidget);

    final left = _videoAt(t, 0.15, 0.45);
    await t.tapAt(left);
    await t.pump(const Duration(milliseconds: 50));
    await t.tapAt(left);
    await t.pump(const Duration(milliseconds: 100));
    expect(h.engine.calls.last, 'seek:10000');
    expect(find.text('−10 s'), findsOneWidget);
    await _close(t);
  });

  testWidgets('vertical swipes: right half volume, left half brightness', (
    t,
  ) async {
    final h = await _open(t);
    await t.timedDragFrom(
      _videoAt(t, 0.8, 0.3),
      const Offset(0, 280),
      const Duration(milliseconds: 400),
    );
    await t.pump();
    final volume = h.engine.state.value.volume;
    expect(volume, lessThan(60));
    expect(volume, greaterThan(20));

    await t.timedDragFrom(
      _videoAt(t, 0.2, 0.3),
      const Offset(0, 200),
      const Duration(milliseconds: 400),
    );
    await t.pump();
    expect(h.brightness.written, isNotNull);
    expect(h.brightness.written!, lessThan(0.8));
    expect(find.byKey(const ValueKey('osd-dim')), findsOneWidget);
    // Horizontal drags are not swipes.
    final before = h.engine.state.value.volume;
    await t.timedDragFrom(
      _videoAt(t, 0.6, 0.4),
      const Offset(200, 10),
      const Duration(milliseconds: 300),
    );
    await t.pump();
    expect(h.engine.state.value.volume, before);

    await _close(t);
    expect(h.brightness.wasReset, isTrue);
  });

  // Found on the emulator: the brightness pill overflowed a 360 px phone.
  for (final locale in const [Locale('en'), Locale('ru')]) {
    testWidgets('swipe feedback fits a 360 px phone ($locale)', (t) async {
      t.view.physicalSize = const Size(720, 1480);
      t.view.devicePixelRatio = 2;
      addTearDown(t.view.reset);
      final h = _Harness();
      await t.pumpWidget(
        MaterialApp(
          locale: locale,
          supportedLocales: PaPlayerStrings.supportedLocales,
          localizationsDelegates: const [
            PaPlayerStrings.delegate,
            ...GlobalMaterialLocalizations.delegates,
          ],
          home: PlayerScreen(
            request: _movie,
            onProgress: (_, _) {},
            engineFactory: ({bool video = true}) => h.engine,
            screenBrightness: h.brightness,
          ),
        ),
      );
      await t.pump(const Duration(milliseconds: 300));
      for (final x in const [0.2, 0.8]) {
        await t.timedDragFrom(
          _videoAt(t, x, 0.3),
          const Offset(0, 150),
          const Duration(milliseconds: 300),
        );
        await t.pump();
        expect(t.takeException(), isNull);
        expect(find.byKey(const ValueKey('osd-feedback')), findsOneWidget);
        final r = t.getRect(find.byKey(const ValueKey('osd-feedback')));
        expect(r.left, greaterThanOrEqualTo(0));
        expect(r.right, lessThanOrEqualTo(360));
      }
      await _close(t);
    });
  }

  testWidgets('a mouse drag does not change the volume', (t) async {
    final h = await _open(t);
    final g = await t.startGesture(
      _videoAt(t, 0.8, 0.3),
      kind: PointerDeviceKind.mouse,
    );
    for (var i = 0; i < 10; i++) {
      await g.moveBy(const Offset(0, 25));
      await t.pump(const Duration(milliseconds: 20));
    }
    await g.up();
    await t.pump();
    expect(h.engine.state.value.volume, 100);
    await _close(t);
  });

  testWidgets('pinch zooms the picture', (t) async {
    final h = await _open(t);
    final c = _videoAt(t, 0.5, 0.45);
    final a = await t.startGesture(c - const Offset(40, 0), pointer: 1);
    final b = await t.startGesture(c + const Offset(40, 0), pointer: 2);
    await t.pump();
    for (var i = 0; i < 4; i++) {
      await a.moveBy(const Offset(-20, 0));
      await b.moveBy(const Offset(20, 0));
      await t.pump(const Duration(milliseconds: 20));
    }
    await a.up();
    await b.up();
    await t.pump();
    expect(h.state(t).pinchScale.value, greaterThan(1.5));
    final scale = t.widget<Transform>(
      find.byKey(const ValueKey('osd-video-scale')),
    );
    expect(scale.transform.getMaxScaleOnAxis(), greaterThan(1.5));
    // Choosing a zoom mode resets the pinch.
    h.state(t).setZoom(VideoZoom.fit);
    await t.pump();
    expect(h.state(t).pinchScale.value, 1.0);
    await _close(t);
  });

  testWidgets('hold plays at 2× until released', (t) async {
    final h = await _open(t);
    await h.engine.setRate(1.25);
    final g = await t.startGesture(_videoAt(t, 0.5, 0.45));
    await t.pump(kLongPressTimeout + const Duration(milliseconds: 50));
    expect(h.engine.state.value.rate, 2.0);
    expect(find.byKey(const ValueKey('osd-hold-speed')), findsOneWidget);
    await g.up();
    await t.pump();
    expect(h.engine.state.value.rate, 1.25);
    expect(find.byKey(const ValueKey('osd-hold-speed')), findsNothing);
    await _close(t);
  });

  testWidgets('OSD auto-hides after 4 s of no input while playing', (t) async {
    final h = await _open(t);
    await t.pump(const Duration(seconds: 3));
    expect(h.state(t).osd.visible, isTrue);
    await t.pump(const Duration(seconds: 2));
    expect(h.state(t).osd.visible, isFalse);
    await _close(t);
  });
}
