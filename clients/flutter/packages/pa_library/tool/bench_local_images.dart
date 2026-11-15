// SPDX-License-Identifier: MPL-2.0
//
// QA-006: cost of finding local posters for N movies in ONE folder (a flat
// "Movies/" folder), per folder listing vs LocalImageIndex. The core's item
// lists (RustLibrary._item) look a poster up for every item on the UI
// isolate.
//
//   dart run tool/bench_local_images.dart [N=10000] [lookups=1000]

import 'dart:io';

import 'package:pa_library/src/music_tags.dart';
import 'package:path/path.dart' as p;

void main(List<String> args) {
  final n = args.isNotEmpty ? int.parse(args[0]) : 10000;
  final lookups = args.length > 1 ? int.parse(args[1]) : 1000;
  final tmp = Directory.systemTemp.createTempSync('kinolune_images_');
  try {
    final dir = Directory(p.join(tmp.path, 'Movies'))..createSync();
    for (var i = 0; i < n; i++) {
      File(p.join(dir.path, 'Movie $i (2000).mkv')).writeAsBytesSync(const []);
      if (i % 10 == 0) {
        File(p.join(dir.path, 'Movie $i (2000).jpg')).writeAsBytesSync(const []);
      }
    }
    String path(int i) => p.join(dir.path, 'Movie ${i % n} (2000).mkv');

    var sw = Stopwatch()..start();
    var found = 0;
    for (var i = 0; i < lookups; i++) {
      if (findLocalImage(path(i), video: true, allowGeneric: false) != null) {
        found++;
      }
    }
    final perCall = sw.elapsedMicroseconds / lookups;
    stdout.writeln(
      'findLocalImage: ${(perCall / 1000).toStringAsFixed(2)} ms per item '
      '($lookups items, $found posters) -> ${(perCall * n / 1e6).toStringAsFixed(1)} s '
      'for a list of $n',
    );

    final index = LocalImageIndex();
    sw = Stopwatch()..start();
    found = 0;
    for (var i = 0; i < n; i++) {
      if (index.find(path(i), video: true, allowGeneric: false) != null) {
        found++;
      }
    }
    stdout.writeln(
      'LocalImageIndex: ${sw.elapsedMilliseconds} ms for all $n items '
      '($found posters, one listing)',
    );
  } finally {
    tmp.deleteSync(recursive: true);
  }
}
