// SPDX-License-Identifier: MPL-2.0
// META-016 through RustReviewInbox on the real native core: without a
// TMDB key (test builds have no project key) no movie finds a result, so
// every movie is in the manual queue; «Это не фильм» takes one out for
// good, and nothing shown cannot be confirmed.

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

  test('movies without a result wait in the manual queue', () async {
    final count = await inbox.count();
    expect((count.check, count.manual), (0, 2));
    final entries = await inbox.entries();
    expect(entries.map((e) => e.localTitle), containsAll(['Arrival', 'Zzyzx Qwerty']));
    final zzyzx = entries.firstWhere((e) => e.localTitle == 'Zzyzx Qwerty');
    expect(zzyzx.state, ReviewState.manual);
    expect(zzyzx.itemId, 'm${zzyzx.workId}');
    expect(zzyzx.samplePath, 'Movies/Zzyzx Qwerty 2003.mkv');
    expect(zzyzx.candidates, isEmpty);
    expect((await inbox.entryFor(zzyzx.itemId))?.workId, zzyzx.workId);

    // Nothing is shown, so nothing can be confirmed.
    await expectLater(inbox.confirm(zzyzx.workId), throwsA(anything));

    final changed = inbox.changes.first;
    await inbox.notMedia(zzyzx.workId);
    await changed;
    expect((await inbox.count()).manual, 1);
    expect(
      (await inbox.entries()).map((e) => e.workId),
      isNot(contains(zzyzx.workId)),
    );
    // The item stays in the library under its own name.
    final movies = await lib.movies().first;
    expect(movies.map((m) => m.title), contains('Zzyzx Qwerty'));
    // A work outside the inbox still opens for «Исправить распознавание».
    final fix = await inbox.entryFor(zzyzx.itemId);
    expect(fix?.workId, zzyzx.workId);
    expect(fix?.state, ReviewState.manual);
  });
}
