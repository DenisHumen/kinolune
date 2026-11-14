// SPDX-License-Identifier: MPL-2.0
// PLAY-009 through RustLibrary on the real native core: seek previews of the
// played file from a BIF next to it (no FFmpeg needed).

import 'dart:io';
import 'dart:typed_data';

import 'package:pa_library/pa_library.dart';
import 'package:path/path.dart' as p;
import 'package:test/test.dart';

import 'native_library.dart';

/// A BIF (Roku/Plex/Emby) of [images] at [intervalMs].
Uint8List bif(int intervalMs, List<List<int>> images) {
  final out = BytesBuilder();
  void u32(int v) => out.add(
    (ByteData(4)..setUint32(0, v, Endian.little)).buffer.asUint8List(),
  );
  out.add([0x89, 0x42, 0x49, 0x46, 0x0D, 0x0A, 0x1A, 0x0A]);
  u32(0);
  u32(images.length);
  u32(intervalMs);
  out.add(List.filled(44, 0));
  var offset = 64 + (images.length + 1) * 8;
  for (var i = 0; i < images.length; i++) {
    u32(i);
    u32(offset);
    offset += images[i].length;
  }
  u32(0xFFFFFFFF);
  u32(offset);
  for (final image in images) {
    out.add(image);
  }
  return out.toBytes();
}

void main() {
  late Directory tmp;
  late String root;

  setUpAll(() async {
    await RustLibrary.ensureInitialized(
      externalLibrary: openBuiltCoreLibrary(),
    );
  });

  setUp(() {
    tmp = Directory.systemTemp.createTempSync('pa_trickplay_');
    root = p.join(tmp.path, 'media');
    final film = File(p.join(root, 'Movies/Heat (1995)/Heat.1995.1080p.mkv'));
    film.parent.createSync(recursive: true);
    film.writeAsBytesSync(List.generate(150000, (i) => (i * 7) & 0xff));
    File(p.join(root, 'Movies/Heat (1995)/Heat.1995.1080p.bif'))
        .writeAsBytesSync(
          bif(10000, [
            [0xFF, 0xD8, 1],
            [0xFF, 0xD8, 2, 2],
            [0xFF, 0xD8, 3, 3, 3],
          ]),
        );
  });

  tearDown(() => tmp.deleteSync(recursive: true));

  test('previews of the played file come from its BIF', () async {
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

    // Nothing to generate here: only the sidecar may answer.
    lib.trickplayGenerate = false;
    final previews = await lib.trickplay(movie);
    expect(previews, isNotNull);
    addTearDown(previews!.dispose);

    TrickplayPreview? frame;
    for (var i = 0; i < 100 && frame == null; i++) {
      frame = await previews.frameAt(15000);
      if (frame == null) {
        await Future<void>.delayed(const Duration(milliseconds: 20));
      }
    }
    expect(frame, isNotNull);
    expect(frame!.jpeg, [0xFF, 0xD8, 2, 2]);
    expect(frame.positionMs, 10000);
    expect(frame.isTile, isFalse);

    final last = await previews.frameAt(3600000);
    expect(last!.positionMs, 20000);
  });

  test('files outside the library have no previews', () async {
    final lib = await RustLibrary.open(p.join(tmp.path, 'db', 'core.db'));
    addTearDown(lib.close);
    final outside = LibraryItem(
      id: 'x',
      kind: ItemKind.movie,
      title: 'Elsewhere',
      filePath: '/nowhere/else.mkv',
    );
    expect(await lib.trickplay(outside), isNull);
  });
}
