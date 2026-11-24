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

    // «Это не фильм»: final; the files are the user's own videos now
    // (LIB-025, ADR-0103), not a movie.
    final changed = inbox.changes.first;
    await inbox.notMedia(entry.workId);
    await changed;
    final after = await lib.movies().first;
    expect(after.map((m) => m.title), isNot(contains('Zzyzx Qwerty')));
    final own = await lib.ownVideos();
    expect(own.map((v) => v.relPath), contains(contains('Zzyzx')));
    expect((await inbox.count()).total, 0);
  });

  test('a pasted id or link through the bridge (UI-012)', () async {
    final movies = await lib.movies().first;
    final arrival = movies.firstWhere((m) => m.title == 'Arrival');
    final work = int.parse(arrival.id.substring(1));

    // Not an id: nothing understood, nothing asked.
    final none = await inbox.lookup(work, 'Arrival 2016');
    expect(none.understood, isFalse);
    expect(none.candidates, isEmpty);

    // An AniDB link is read; without the anime databases nobody names it
    // (and no request is made for it).
    final anidb = await inbox.lookup(work, 'https://anidb.net/anime/1');
    expect(anidb.source, 'anidb');
    expect(anidb.candidates, isEmpty);

    // A TMDB id needs TMDB: no key in test builds.
    await expectLater(inbox.lookup(work, 'tmdb 329865'), throwsA(anything));
  });

  test('folder rules through the bridge (META-035)', () async {
    final movies = await lib.movies().first;
    final arrival = movies.firstWhere((m) => m.title == 'Arrival');
    final work = arrival.id.substring(1);

    var rules = await inbox.folderRules(arrival.id);
    expect(rules!.kind, 'movie');
    expect(rules.rules, isEmpty);
    // Music has no folder rules.
    expect(await inbox.folderRules('t1'), isNull);

    // «Тип: сериал» teaches the folder.
    await inbox.setKind(arrival.id, 'show');
    rules = await inbox.folderRules('s$work');
    expect(rules!.kind, 'show');
    expect(rules.rules, hasLength(1));
    expect(rules.rules.single.folder, 'Movies/Arrival (2016)');
    expect(rules.rules.single.kind, 'show');

    // «Нумерация»: number in the name + 12 = episode of season 2.
    await inbox.setNumbering('s$work', season: 2, offset: 12);
    final n = (await inbox.folderRules('s$work'))!.numbering!;
    expect((n.absolute, n.season, n.offset), (false, 2, 12));

    // «Забыть правило папки».
    expect(await inbox.forgetFolderRules('s$work'), 1);
    expect((await inbox.folderRules('s$work'))!.rules, isEmpty);
    expect(await inbox.forgetFolderRules('s$work'), 0);
  });

  test('NFO export through the bridge (LIB-014)', () async {
    // No ids yet: nothing to write.
    var movies = await lib.movies().first;
    final arrival = movies.firstWhere((m) => m.title == 'Arrival');
    var plan = (await inbox.knowledgePlan(arrival.id))!;
    expect(plan.ids, isEmpty);
    expect(plan.canExport, isFalse);
    expect(await inbox.knowledgePlan('t1'), isNull);

    // An id in a folder name is read on the scan; the NFO goes there.
    final root = p.join(tmp.path, 'media');
    media(root, 'Movies/Up (2009) [tmdbid-14160]/Up.mkv', 3);
    await lib.scan().toList();
    movies = await lib.movies().first;
    final up = movies.firstWhere((m) => m.title == 'Up');
    plan = (await inbox.knowledgePlan(up.id))!;
    expect(plan.ids, ['tmdb 14160']);
    expect(plan.nfoPaths, ['/Movies/Up (2009) [tmdbid-14160]/movie.nfo']);
    expect(plan.existingNfo, isEmpty);
    expect(plan.renames, isEmpty);
    final r = await inbox.exportNfo(up.id);
    expect((r.written, r.replaced, r.skipped), (1, 0, 0));
    final nfo = File(p.join(root, 'Movies/Up (2009) [tmdbid-14160]/movie.nfo'));
    expect(nfo.readAsStringSync(), contains('<tmdbid>14160</tmdbid>'));
    expect((await inbox.knowledgePlan(up.id))!.existingNfo, hasLength(1));
    expect((await inbox.exportNfo(up.id)).skipped, 1);
    expect((await inbox.exportNfo(up.id, replace: true)).replaced, 1);
    expect(await inbox.renameWithIds(up.id), 0);
  });

  test('a scan or metadata run reaches the inbox listeners (BUG-012)', () async {
    // The home line and the Settings count reload on `changes`; a run that
    // puts works into the inbox decides nothing here, so the library's
    // own change signal must come through.
    var seen = 0;
    final sub = inbox.changes.listen((_) => seen++);
    await lib.scan().toList();
    await pumpEventQueue();
    expect(seen, greaterThan(0));
    final before = seen;
    lib.notifyExternalChange();
    await pumpEventQueue();
    expect(seen, greaterThan(before));
    await sub.cancel();
  });
}
