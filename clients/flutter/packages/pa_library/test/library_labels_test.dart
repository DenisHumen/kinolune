// SPDX-License-Identifier: MPL-2.0
// LIB-022 through RustLibrary on the real native core: categories, tags
// with autocomplete and 1–5 star ratings, filtered and sorted in queries.

import 'dart:io';

import 'package:pa_library/pa_library.dart';
import 'package:path/path.dart' as p;
import 'package:test/test.dart';

import 'native_library.dart';

/// A fake media file with content unique to [seed] (fingerprints differ).
void media(String root, String rel, int seed) {
  final f = File(p.join(root, rel));
  f.parent.createSync(recursive: true);
  f.writeAsBytesSync(List.generate(150000, (i) => (i * 31 + seed) & 0xff));
}

void main() {
  late Directory tmp;
  late RustLibrary lib;

  setUpAll(() async {
    await RustLibrary.ensureInitialized(
      externalLibrary: openBuiltCoreLibrary(),
    );
  });

  setUp(() async {
    tmp = Directory.systemTemp.createTempSync('pa_labels_');
    final root = p.join(tmp.path, 'media');
    media(root, 'Movies/Arrival (2016)/Arrival.2016.mkv', 1);
    media(root, 'Movies/Dune (2021)/Dune.2021.mkv', 2);
    media(root, 'Movies/Heat (1995)/Heat.1995.mkv', 3);
    lib = await RustLibrary.open(p.join(tmp.path, 'db', 'core.db'));
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
    await lib.close();
    tmp.deleteSync(recursive: true);
  });

  Future<String> id(String title) async =>
      (await lib.movies().first).firstWhere((m) => m.title == title).id;

  Future<List<String>> titles(LibraryFilter filter, [LibrarySort? sort]) async {
    final page = await lib.query(
      LibraryQuery(
        ItemKind.movie,
        filter: filter,
        sort: sort ?? const LibrarySort(),
      ),
    );
    return [for (final i in page.items) i.title];
  }

  test('categories, tags and ratings are saved and filter queries', () async {
    final arrival = await id('Arrival');
    final dune = await id('Dune');
    final heat = await id('Heat');

    final scifi = await lib.createCategory('Sci-fi');
    final crime = await lib.createCategory('Crime');
    await expectLater(lib.createCategory('sci-FI'), throwsA(anything));
    await lib.renameCategory(crime.id, 'Crime films');
    expect(await lib.setInCategory(scifi.id, arrival, true), isTrue);
    await lib.setInCategory(scifi.id, dune, true);
    await lib.setInCategory(crime.id, heat, true);
    await lib.setInCategory(crime.id, dune, true);
    expect([for (final c in await lib.categories()) c.name], [
      'Sci-fi',
      'Crime films',
    ]);
    expect([
      for (final c in await lib.itemCategories(dune)) c.name,
    ], ['Sci-fi', 'Crime films']);
    expect((await lib.categoryItems(scifi.id)).length, 2);

    await lib.setItemTags(dune, ['Desert', 'Epic']);
    await lib.setItemTags(arrival, ['epic', 'Aliens']);
    expect(await lib.itemTags(dune), ['Desert', 'Epic']);
    expect([for (final t in await lib.suggestTags('e')) t.name], [
      'Epic',
      'Aliens',
      'Desert',
    ]);
    expect([for (final t in await lib.suggestTags('ali')) t.name], [
      'Aliens',
    ]);
    expect(await lib.tags(), [
      const MediaTag('Aliens', 1),
      const MediaTag('Desert', 1),
      const MediaTag('Epic', 2),
    ]);
    expect((await lib.tagItems('EPIC')).length, 2);

    await lib.setItemRating(dune, 5);
    await lib.setItemRating(heat, 3);
    expect(await lib.itemRating(dune), 5);
    expect(await lib.itemRating(arrival), isNull);
    expect(() => lib.setItemRating(dune, 6), throwsArgumentError);

    expect(await titles(LibraryFilter(categories: [crime.id])), [
      'Dune',
      'Heat',
    ]);
    expect(await titles(const LibraryFilter(tags: ['Epic'])), [
      'Arrival',
      'Dune',
    ]);
    expect(await titles(const LibraryFilter(ratings: {0})), ['Arrival']);
    expect(
      await titles(
        const LibraryFilter(),
        const LibrarySort(field: SortField.userRating, descending: true),
      ),
      ['Dune', 'Heat', 'Arrival'],
    );
    final facets = await lib.facets(ItemKind.movie);
    expect(facets.tags, [
      const FacetCount('Aliens', 1),
      const FacetCount('Desert', 1),
      const FacetCount('Epic', 2),
    ]);
    expect(facets.ratings, [const FacetCount(3, 1), const FacetCount(5, 1)]);
    expect(facets.categories.length, 2);

    await lib.setItemRating(dune, null);
    expect(await lib.itemRating(dune), isNull);
    await lib.deleteCategory(scifi.id);
    expect((await lib.categories()).length, 1);
  });
}
