// SPDX-License-Identifier: MPL-2.0
// META-016 through RustReviewInbox on the real native core (test builds
// have no TMDB key, so nothing reaches the network).

import 'dart:io';

import 'package:pa_library/pa_library.dart';
import 'package:path/path.dart' as p;
import 'package:test/test.dart';

import 'native_library.dart';

void media(String root, String rel, int seed) {
  final f = File(p.join(root, rel));
  f.parent.createSync(recursive: true);
  f.writeAsBytesSync(List.generate(150000, (i) => (i * 31 + seed) & 0xff));
}

void main() {
  late Directory tmp;
  late RustLibrary lib;
  late RustReviewInbox inbox;

  setUpAll(() async {
    await RustLibrary.ensureInitialized(
      externalLibrary: openBuiltCoreLibrary(),
    );
  });

  setUp(() async {
    tmp = Directory.systemTemp.createTempSync('pa_review_');
    final root = p.join(tmp.path, 'media');
    media(root, 'Movies/Arrival (2016)/Arrival.2016.mkv', 1);
    media(root, 'Movies/Zzyzx Qwerty 2003.mkv', 2);
    lib = await RustLibrary.open(p.join(tmp.path, 'db', 'core.db'));
    inbox = RustReviewInbox(lib);
    await lib.addSource(
      MediaSource(
        id: '',
        kind: SourceKind.localFolder,
        name: 'Media',
        path: root,
      ),
    );
    await lib.scan().toList();
  });

  tearDown(() async {
    await inbox.close();
    await lib.close();
    tmp.deleteSync(recursive: true);
  });

  test('the inbox API through the bridge, without a TMDB key', () async {
    // Without a key no movie is searched, so none waits in the inbox (the
    // states by threshold are covered by pa-catalog's tests/review_inbox.rs).
    final count = await inbox.count();
    expect((count.check, count.manual), (0, 0));
    expect(await inbox.entries(), isEmpty);

    // «Исправить распознавание» still opens any movie.
    final movies = await lib.movies().first;
    final zzyzx = movies.firstWhere((m) => m.title == 'Zzyzx Qwerty');
    final entry = await inbox.entryFor(zzyzx.id);
    expect(entry, isNotNull);
    expect(entry!.itemId, zzyzx.id);
    expect(entry.workId, int.parse(zzyzx.id.substring(1)));
    expect(entry.state, ReviewState.manual);
    expect(entry.candidates, isEmpty);

    // Nothing shown: nothing to confirm; no key: no search.
    await expectLater(inbox.confirm(entry.workId), throwsA(anything));
    await expectLater(
      inbox.search(entry.workId, 'Arrival', year: 2016),
      throwsA(anything),
    );

    // «Это не фильм»: final, and the item stays under its own name.
    final changed = inbox.changes.first;
    await inbox.notMedia(entry.workId);
    await changed;
    final after = await lib.movies().first;
    expect(after.map((m) => m.title), contains('Zzyzx Qwerty'));
    expect((await inbox.count()).total, 0);
  });
}
