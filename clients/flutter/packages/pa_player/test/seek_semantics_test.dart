// SPDX-License-Identifier: MPL-2.0
// BUG-015: seeking in the player broke the semantics tree (`Null check
// operator` in `_updateSemanticsNodeGeometry`). On Linux the GTK
// accessibility bridge keeps semantics on all the time, so every seek of
// the live QA run threw twice. The test seeks the way that run did — the
// remote's fast-forward key, then a jump to 3:00 — with semantics on.
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

/// The video as media_kit draws it: a texture of the decoder's size in a
/// FittedBox, 1×1 until the first frame.
class _TextureEngine extends FakePlayerEngine {
  _TextureEngine() : super(loadedDuration: const Duration(minutes: 30));
  final rect = ValueNotifier<Size?>(const Size(1, 1));

  @override
  Widget buildVideo(BuildContext context, {BoxFit fit = BoxFit.contain}) =>
      ColoredBox(
        color: const Color(0xFF000000),
        child: ClipRect(
          child: FittedBox(
            fit: fit,
            child: ValueListenableBuilder<Size?>(
              valueListenable: rect,
              builder: (context, r, _) => r == null
                  ? const SizedBox.shrink()
                  : SizedBox(
                      width: r.width,
                      height: r.height,
                      child: Stack(
                        children: [
                          const SizedBox(),
                          const Positioned.fill(child: Texture(textureId: 1)),
                          if (r.width <= 1 && r.height <= 1)
                            Positioned.fill(
                              child: Container(color: const Color(0xFF000000)),
                            ),
                        ],
                      ),
                    ),
            ),
          ),
        ),
      );
}

/// The engine the next [_open] uses (default: a plain fake).
FakePlayerEngine? nextEngine;

Future<FakePlayerEngine> _open(WidgetTester t) async {
  final engine =
      nextEngine ??
      FakePlayerEngine(loadedDuration: const Duration(minutes: 30));
  nextEngine = null;
  // As in the app: the film page stays under the player and shows the
  // saved position, which every seek updates.
  final saved = ValueNotifier<int>(0);
  addTearDown(saved.dispose);
  await t.pumpWidget(
    MaterialApp(
      home: Builder(
        builder: (context) => Scaffold(
          body: ListView(
            children: [
              ValueListenableBuilder<int>(
                valueListenable: saved,
                builder: (context, ms, _) => Column(
                  children: [
                    Text('Continue from ${ms ~/ 1000} s'),
                    LinearProgressIndicator(value: ms / 1800000),
                    if (ms > 0) const Chip(label: Text('Started')),
                  ],
                ),
              ),
              TextButton(
                onPressed: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => PlayerScreen(
                      request: const PlayRequest(
                        uri: '/media/movie.mkv',
                        title: 'Big Buck Bunny',
                        startPositionMs: 10000,
                      ),
                      onProgress: (p, _) => saved.value = p,
                      engineFactory: ({bool video = true}) => engine,
                    ),
                  ),
                ),
                child: const Text('play'),
              ),
            ],
          ),
        ),
      ),
    ),
  );
  await t.tap(find.text('play'));
  await t.pumpAndSettle();
  return engine;
}

/// Pumps frame by frame for [d], so every frame of the OSD's fade and of
/// the seek is built, laid out and has its semantics flushed.
Future<void> _frames(
  WidgetTester t,
  Duration d, {
  FakePlayerEngine? playing,
}) async {
  const step = Duration(milliseconds: 16);
  for (var at = Duration.zero; at < d; at += step) {
    playing?.advance(step);
    await t.pump(step);
    expect(t.takeException(), isNull);
  }
}

/// A real engine's seek: buffering for a moment, then the new position.
Future<void> _engineSeek(
  WidgetTester t,
  FakePlayerEngine engine,
  Duration to,
) async {
  engine.emit((s) => s.copyWith(buffering: true));
  await _frames(t, const Duration(milliseconds: 200));
  await engine.seek(to);
  engine.emit((s) => s.copyWith(buffering: false));
}

void main() {
  for (final platform in [TargetPlatform.linux, TargetPlatform.android]) {
    testWidgets('seek by key and to 3:00 with semantics on ($platform)', (
      t,
    ) async {
      debugDefaultTargetPlatformOverride = platform;
      t.view.physicalSize = const Size(1280, 720);
      t.view.devicePixelRatio = 1;
      addTearDown(t.view.reset);
      final semantics = t.ensureSemantics();
      try {
        final engine = await _open(t);
        await _frames(t, const Duration(seconds: 10), playing: engine);
        expect(
          t.widget<AnimatedOpacity>(find.byKey(const ValueKey('osd'))).opacity,
          0,
          reason: 'the OSD hid itself while playing',
        );
        // The fast-forward key brings the OSD back mid-seek.
        await t.sendKeyEvent(
          LogicalKeyboardKey.mediaFastForward,
          platform: 'android',
        );
        engine.emit((s) => s.copyWith(buffering: true));
        await _frames(t, const Duration(milliseconds: 300));
        engine.emit((s) => s.copyWith(buffering: false));
        await _frames(t, const Duration(milliseconds: 1200), playing: engine);
        expect(engine.calls.where((c) => c.startsWith('seek:')), hasLength(1));

        // The jump to 3:00 straight on the engine (as the live run does).
        await _engineSeek(t, engine, const Duration(minutes: 3));
        await _frames(t, const Duration(seconds: 5), playing: engine);
        expect(
          engine.state.value.position,
          greaterThanOrEqualTo(const Duration(minutes: 3)),
        );

        // Seeks with the OSD up and the seek bar focused (arrow keys).
        await t.sendKeyEvent(LogicalKeyboardKey.arrowRight);
        await _frames(t, const Duration(milliseconds: 500), playing: engine);
        await t.sendKeyEvent(LogicalKeyboardKey.arrowRight);
        await _frames(t, const Duration(seconds: 5), playing: engine);
      } finally {
        semantics.dispose();
        debugDefaultTargetPlatformOverride = null;
      }
    });
  }

  for (final sizes in [
    [const Size(853, 480), const Size(1, 1), const Size(853, 480)],
    [const Size(853, 480), null, const Size(853, 480)],
    [const Size(853, 480), Size.zero, const Size(853, 480)],
  ]) {
    testWidgets('a media_kit-like texture resized around a seek: $sizes', (
      t,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      t.view.physicalSize = const Size(1280, 720);
      t.view.devicePixelRatio = 1;
      addTearDown(t.view.reset);
      final semantics = t.ensureSemantics();
      final video = _TextureEngine();
      nextEngine = video;
      try {
        final engine = await _open(t);
        await _frames(t, const Duration(seconds: 1), playing: engine);
        video.rect.value = sizes[0];
        await _frames(t, const Duration(seconds: 6), playing: engine);
        await t.sendKeyEvent(
          LogicalKeyboardKey.mediaFastForward,
          platform: 'android',
        );
        video.rect.value = sizes[1];
        await _frames(t, const Duration(milliseconds: 300), playing: engine);
        video.rect.value = sizes[2];
        await _frames(t, const Duration(seconds: 2), playing: engine);
        video.rect.value = sizes[1];
        await _engineSeek(t, engine, const Duration(minutes: 3));
        video.rect.value = sizes[2];
        await _frames(t, const Duration(seconds: 5), playing: engine);
      } finally {
        semantics.dispose();
        debugDefaultTargetPlatformOverride = null;
      }
    });
  }
}
