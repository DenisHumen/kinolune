// SPDX-License-Identifier: MPL-2.0
// PLAY-011: the library's skip segments reach the player.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/screens/playback_skip.dart';

class _Library implements LibrarySkipSegments {
  final analyzed = <String>[];
  List<MediaSkipSegment> segments = const [];

  @override
  Future<void> analyzeSkipSegments(LibraryItem item) async =>
      analyzed.add(item.id);

  @override
  Future<List<MediaSkipSegment>> skipSegments(LibraryItem item) async =>
      segments;
}

/// A [Library] that also offers skip segments: everything else is unused.
class _SkipLibrary extends _Library implements Library {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

LibraryItem _item(ItemKind kind) =>
    LibraryItem(id: 'e1', kind: kind, title: 'E1', filePath: '/m/e1.mkv');

void main() {
  test('kinds map; previews and empty segments are left out', () {
    final out = playerSegments(const [
      MediaSkipSegment(kind: MediaSkipKind.intro, startMs: 1000, endMs: 41000),
      MediaSkipSegment(
        kind: MediaSkipKind.credits,
        startMs: 1300000,
        endMs: 1380000,
      ),
      MediaSkipSegment(
        kind: MediaSkipKind.preview,
        startMs: 1380000,
        endMs: 1400000,
      ),
      MediaSkipSegment(kind: MediaSkipKind.recap, startMs: 5000, endMs: 5000),
    ]);
    expect(out, const [
      SkipSegment(
        kind: SkipKind.intro,
        start: Duration(seconds: 1),
        end: Duration(seconds: 41),
      ),
      SkipSegment(
        kind: SkipKind.credits,
        start: Duration(seconds: 1300),
        end: Duration(seconds: 1380),
      ),
    ]);
  });

  test('episodes queue their season; movies do not', () async {
    final lib = _SkipLibrary()
      ..segments = const [
        MediaSkipSegment(kind: MediaSkipKind.intro, startMs: 0, endMs: 30000),
      ];
    final segs = await skipSegmentsOf(lib, _item(ItemKind.episode));
    expect(segs, hasLength(1));
    expect(lib.analyzed, ['e1']);
    await skipSegmentsOf(lib, _item(ItemKind.movie));
    await skipSegmentsOf(lib, _item(ItemKind.episode), analyze: false);
    expect(lib.analyzed, ['e1']);
  });
}
