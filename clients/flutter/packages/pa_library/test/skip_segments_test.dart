// SPDX-License-Identifier: MPL-2.0
// PLAY-011 through RustLibrary on the real native core: skip segments of a
// played file and queueing its folder for the background analysis (no
// FFmpeg needed: without it the job finds nothing and nothing breaks).

import 'dart:io';

import 'package:pa_library/pa_library.dart';
import 'package:path/path.dart' as p;
import 'package:test/test.dart';

import 'native_library.dart';

void main() {
  late Directory tmp;
  late String root;

  setUpAll(() async {
    await RustLibrary.ensureInitialized(
      externalLibrary: openBuiltCoreLibrary(),
    );
  });

  setUp(() {
    tmp = Directory.systemTemp.createTempSync('pa_skip_');
    root = p.join(tmp.path, 'media');
    final film = File(p.join(root, 'Movies/Heat (1995)/Heat.1995.1080p.mkv'));
    film.parent.createSync(recursive: true);
    film.writeAsBytesSync(List.generate(150000, (i) => (i * 7) & 0xff));
  });

  tearDown(() => tmp.deleteSync(recursive: true));

  test('a file without analysis has no segments; analysis is queued', () async {
    final lib = await RustLibrary.open(p.join(tmp.path, 'db', 'core.db'));
    addTearDown(lib.close);
    await lib.addSource(
      MediaSource(
        id: '',
        kind: SourceKind.localFolder,
        name: 'Media',
        path: root,
      ),
    );
    await lib.scan().toList();
    final movie = (await lib.movies().first).single;
    expect(await lib.skipSegments(movie), isEmpty);
    await lib.analyzeSkipSegments(movie);
    await lib.analyzeSkipSegments(movie); // queued once
    expect(await lib.skipSegments(movie), isEmpty);
  });

  test('files outside the library have none', () async {
    final lib = await RustLibrary.open(p.join(tmp.path, 'db', 'core.db'));
    addTearDown(lib.close);
    final outside = LibraryItem(
      id: 'x',
      kind: ItemKind.episode,
      title: 'Elsewhere',
      filePath: '/nowhere/else.mkv',
    );
    expect(await lib.skipSegments(outside), isEmpty);
    await lib.analyzeSkipSegments(outside);
  });

  test('segment values', () {
    const a = MediaSkipSegment(
      kind: MediaSkipKind.intro,
      startMs: 1000,
      endMs: 31000,
      origin: 'audio',
    );
    expect(
      a,
      const MediaSkipSegment(
        kind: MediaSkipKind.intro,
        startMs: 1000,
        endMs: 31000,
        origin: 'audio',
      ),
    );
    expect(a.toString(), contains('intro'));
  });
}
