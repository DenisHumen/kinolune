// SPDX-License-Identifier: MPL-2.0
// UI-025 through RustPersonalMedia on the real native core: a personal
// source is never recognized, a locked library answers nothing, and a
// movie moves into a personal library and back.

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
  late RustPersonalMedia personal;
  late int family;

  setUpAll(() async {
    await RustLibrary.ensureInitialized(
      externalLibrary: openBuiltCoreLibrary(),
    );
  });

  setUp(() async {
    tmp = Directory.systemTemp.createTempSync('pa_personal_');
    final root = p.join(tmp.path, 'media');
    final home = p.join(tmp.path, 'home');
    media(root, 'Movies/Arrival (2016)/Arrival.2016.mkv', 1);
    media(root, 'Movies/Dune (2021)/Dune.2021.mkv', 2);
    media(home, 'Heat.1995.1080p.mkv', 3);
    media(home, 'Birthday/clip_0042.mp4', 4);
    lib = await RustLibrary.open(p.join(tmp.path, 'db', 'core.db'));
    personal = RustPersonalMedia(lib);
    await lib.addSource(
      MediaSource(
        id: '',
        kind: SourceKind.localFolder,
        name: 'Media',
        path: root,
      ),
    );
    family = (await personal.createLibrary(
      'Семья',
      lock: PersonalLock.pin,
      secret: '2468',
    )).id;
    final added = await lib.addSource(
      MediaSource(
        id: '',
        kind: SourceKind.localFolder,
        name: 'Home',
        path: home,
      ),
    );
    await personal.setSourceContent(
      int.parse(added.id),
      SourceContent.personal,
      libraryId: family,
    );
    await lib.scan().toList();
  });

  tearDown(() async {
    await lib.close();
    tmp.deleteSync(recursive: true);
  });

  Future<List<String>> movies() async =>
      [for (final m in await lib.movies().first) m.title]..sort();

  test(
    'a personal source is not recognized and stays out of the library',
    () async {
      expect(await movies(), ['Arrival', 'Dune']);
      expect(await lib.search('Heat'), isEmpty);
      final videos = await personal.videos(family, sort: PersonalSort.title);
      expect(
        [for (final v in videos) v.title],
        ['clip_0042', 'Heat.1995.1080p'],
      );
      expect(videos.every((v) => !v.moved), isTrue);
      final uris = await personal.playUris(family, videos.last);
      expect(uris.single, endsWith('Heat.1995.1080p.mkv'));
    },
  );

  test('a locked library answers nothing without its PIN', () async {
    await personal.lockAll();
    final listed = (await personal.libraries()).single;
    expect(listed.name, 'Семья');
    expect(listed.locked, isTrue);
    expect(listed.itemCount, isNull);
    await expectLater(personal.videos(family), throwsA(isA<PersonalLocked>()));
    await expectLater(personal.tags(family), throwsA(isA<PersonalLocked>()));
    final wrong = await personal.unlock(family, '0000');
    expect(wrong.ok, isFalse);
    expect(wrong.attempts.failures, 1);
    expect((await personal.unlock(family, '2468')).ok, isTrue);
    expect(await personal.videos(family), hasLength(2));
  });

  test('a movie moves into personal media and back, labels stay', () async {
    final dune = (await lib.movies().first).firstWhere(
      (m) => m.title == 'Dune',
    );
    await personal.moveToPersonal(dune.id, family);
    expect(await movies(), ['Arrival']);
    expect(await lib.item(dune.id), isNull);
    final moved = (await personal.videos(family)).firstWhere((v) => v.moved);
    expect(moved.title, 'Dune');
    await personal.setRating(family, moved.labelId!, 4);
    await personal.setTags(family, moved.labelId!, ['лето']);
    final cat = await personal.createCategory(family, 'Отпуск');
    await personal.setInCategory(
      family,
      int.parse(cat.id),
      moved.labelId!,
      true,
    );
    final rated = await personal.videos(
      family,
      filter: const PersonalFilter(ratings: [4], tags: ['ЛЕТО']),
    );
    expect([for (final v in rated) v.title], ['Dune']);
    expect(rated.single.categories, [int.parse(cat.id)]);
    // The library's tags stay in the library.
    expect(await lib.tags(), isEmpty);

    await personal.moveToMain(family, moved.fileId);
    expect(await movies(), ['Arrival', 'Dune']);
    expect((await lib.item(dune.id))?.title, 'Dune');
  });
}
