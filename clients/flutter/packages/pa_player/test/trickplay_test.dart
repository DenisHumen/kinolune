// SPDX-License-Identifier: MPL-2.0
import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/src/ui/scrubber.dart';
import 'package:pa_player/testing.dart';

/// A 2x1 PNG: enough for the image codec in tests.
final _png = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAADUlEQVR4nGP4zwAE/wEHAAH/4iOeWQAAAABJRU5ErkJggg==',
);

class _FakeTrickplay implements TrickplaySource {
  final asked = <Duration>[];
  bool disposed = false;

  @override
  Future<TrickplayFrame?> frameAt(Duration position) async {
    asked.add(position);
    return TrickplayFrame(
      image: Uint8List.fromList(_png),
      imageId: position.inSeconds ~/ 10,
      position: Duration(seconds: position.inSeconds ~/ 10 * 10),
    );
  }

  @override
  void dispose() => disposed = true;
}

Widget _scrubber(TrickplaySource? source) => MaterialApp(
  home: Scaffold(
    backgroundColor: Colors.black,
    body: Align(
      alignment: Alignment.bottomCenter,
      child: SizedBox(
        width: 800,
        child: Scrubber(
          position: Duration.zero,
          duration: const Duration(minutes: 100),
          onSeek: (_) {},
          trickplay: source,
        ),
      ),
    ),
  ),
);

void main() {
  testWidgets('dragging shows the preview of the position under the finger', (
    t,
  ) async {
    final source = _FakeTrickplay();
    await t.pumpWidget(_scrubber(source));
    expect(find.byKey(const ValueKey('trickplay-preview')), findsNothing);

    final bar = t.getRect(find.byType(Scrubber));
    final gesture = await t.startGesture(
      Offset(bar.left + 200, bar.top + 14),
    );
    await gesture.moveBy(const Offset(200, 0));
    await t.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
    await t.pump();

    // Half of the 800 px bar: 50 of 100 minutes.
    expect(source.asked, isNotEmpty);
    expect(source.asked.last.inMinutes, 50);
    expect(find.byKey(const ValueKey('trickplay-preview')), findsOneWidget);
    expect(find.text('50:00'), findsWidgets);
    expect(find.byType(TrickplayImage), findsOneWidget);

    await gesture.up();
    await t.pump();
    expect(find.byKey(const ValueKey('trickplay-preview')), findsNothing);
  });

  testWidgets('no preview without a source', (t) async {
    await t.pumpWidget(_scrubber(null));
    final bar = t.getRect(find.byType(Scrubber));
    final gesture = await t.startGesture(Offset(bar.left + 100, bar.top + 14));
    await gesture.moveBy(const Offset(100, 0));
    await t.pump();
    expect(find.byKey(const ValueKey('trickplay-preview')), findsNothing);
    await gesture.up();
  });

  testWidgets('the player disposes the source when it closes', (t) async {
    final source = _FakeTrickplay();
    final engine = FakePlayerEngine(loadedDuration: const Duration(minutes: 30));
    await t.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (context) => TextButton(
            onPressed: () => Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) => PlayerScreen(
                  request: PlayRequest(
                    uri: '/media/movie.mkv',
                    title: 'Movie',
                    trickplay: source,
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
    );
    await t.tap(find.text('open'));
    await t.pumpAndSettle();
    expect(source.disposed, isFalse);
    await t.sendKeyEvent(LogicalKeyboardKey.escape);
    await t.pumpAndSettle();
    expect(find.byType(PlayerScreen), findsNothing);
    expect(source.disposed, isTrue);
  });
}
