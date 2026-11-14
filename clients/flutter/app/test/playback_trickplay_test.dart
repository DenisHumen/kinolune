// SPDX-License-Identifier: MPL-2.0

import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/screens/playback_trickplay.dart';
import 'package:material_ui/material_ui.dart';

class _Previews implements TrickplayPreviews {
  final asked = <int>[];
  bool disposed = false;

  @override
  Future<TrickplayPreview?> frameAt(int positionMs) async {
    asked.add(positionMs);
    if (positionMs < 0) return null;
    return TrickplayPreview(
      jpeg: Uint8List.fromList([0xFF, 0xD8]),
      imageId: 3,
      positionMs: positionMs ~/ 10000 * 10000,
      cropX: 320,
      cropY: 180,
      cropWidth: 320,
      cropHeight: 180,
    );
  }

  @override
  void dispose() => disposed = true;
}

void main() {
  test('maps library previews to player frames', () async {
    final previews = _Previews();
    var opened = 0;
    final source = LibraryTrickplaySource(() async {
      opened++;
      return previews;
    });
    final frame = await source.frameAt(const Duration(seconds: 95));
    expect(opened, 1);
    expect(previews.asked, [95000]);
    expect(frame!.position, const Duration(seconds: 90));
    expect(frame.imageId, 3);
    expect(frame.crop, const Rect.fromLTWH(320, 180, 320, 180));
    await source.frameAt(const Duration(seconds: 5));
    expect(opened, 1);
    source.dispose();
    await Future<void>.delayed(Duration.zero);
    expect(previews.disposed, isTrue);
    expect(await source.frameAt(const Duration(seconds: 5)), isNull);
  });

  test('a lazy source opens nothing until asked', () async {
    var opened = 0;
    final previews = _Previews();
    final source = LibraryTrickplaySource(() async {
      opened++;
      return previews;
    });
    source.dispose();
    expect(opened, 0);
    expect(await source.frameAt(Duration.zero), isNull);
    expect(opened, 0);
  });

  test('previews opened after dispose are released at once', () async {
    final previews = _Previews();
    final source = LibraryTrickplaySource(() async {
      await Future<void>.delayed(const Duration(milliseconds: 10));
      return previews;
    })..start();
    source.dispose();
    await Future<void>.delayed(const Duration(milliseconds: 30));
    expect(previews.disposed, isTrue);
  });

  test('no previews from a library without them or a failing one', () async {
    final failing = LibraryTrickplaySource(() async => throw StateError('x'))
      ..start();
    expect(await failing.frameAt(Duration.zero), isNull);
  });
}
