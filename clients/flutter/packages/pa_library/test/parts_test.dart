// SPDX-License-Identifier: MPL-2.0
// LIB-020 through RustLibrary on the real native core: a split movie and a
// DVD title set give all their parts in order, a disc image is marked
// unplayable. LIB-021: the subtitles of the parts are joined across them.

import 'dart:io';

import 'package:pa_library/pa_library.dart';
import 'package:path/path.dart' as p;
import 'package:test/test.dart';

import 'native_library.dart';

void media(String root, String rel, int size, int seed) {
  final f = File(p.join(root, rel));
  f.parent.createSync(recursive: true);
  f.writeAsBytesSync(List.generate(size, (i) => (i * 31 + seed) & 0xff));
}

const cd1 = 'Movies/Heat (1995)/Heat.1995.DVDRip.cd1.avi';
const cd2 = 'Movies/Heat (1995)/Heat.1995.DVDRip.cd2.avi';
const dvd = 'Movies/Alien (1979)/VIDEO_TS';
const iso = 'Movies/Spirited Away (2001).iso';
const one = 'Movies/Arrival (2016)/Arrival.2016.1080p.mkv';

void main() {
  late Directory tmp;
  late String root;
  late RustLibrary lib;

  setUpAll(() async {
    await RustLibrary.ensureInitialized(
      externalLibrary: openBuiltCoreLibrary(),
    );
  });

  setUp(() async {
    tmp = Directory.systemTemp.createTempSync('pa_parts_');
    root = p.join(tmp.path, 'media');
    media(root, cd1, 120000, 1);
    media(root, cd2, 110000, 2);
    media(root, '$dvd/VIDEO_TS.VOB', 9000, 3);
    media(root, '$dvd/VTS_01_0.VOB', 5000, 4);
    media(root, '$dvd/VTS_01_1.VOB', 160000, 5);
    media(root, '$dvd/VTS_01_2.VOB', 90000, 6);
    media(root, iso, 70000, 7);
    media(root, one, 150000, 8);
    // LIB-021: subtitles of the parts and one of the whole folder.
    media(root, 'Movies/Heat (1995)/Heat.1995.DVDRip.cd1.srt', 100, 9);
    media(root, 'Movies/Heat (1995)/Heat.1995.DVDRip.cd2.srt', 100, 10);
    media(root, 'Movies/Heat (1995)/Heat.1995.DVDRip.cd2.en.srt', 100, 11);
    media(root, 'Movies/Heat (1995)/Subs/Russian.srt', 100, 12);
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

  Future<LibraryItem> movie(String title) async =>
      (await lib.movies().first).firstWhere((m) => m.title == title);

  test('a cd1/cd2 movie plays both parts in order', () async {
    final heat = await movie('Heat');
    expect(heat.unplayable, isNull);
    expect(await lib.playParts(heat), [p.join(root, cd1), p.join(root, cd2)]);
  });

  test('the subtitles of the parts are joined across them', () async {
    final heat = await movie('Heat');
    String at(String name) => p.join(root, 'Movies/Heat (1995)', name);
    expect(await lib.sidecarSubtitles(heat), [
      SidecarSubtitle(
        uri: at('Heat.1995.DVDRip.cd1.srt'),
        name: 'Heat.1995.DVDRip.cd1.srt',
        parts: [at('Heat.1995.DVDRip.cd1.srt'), at('Heat.1995.DVDRip.cd2.srt')],
      ),
      // The subtitle of the folder goes with the movie's first file (as
      // before LIB-021): it stays a plain subtitle of the whole timeline.
      SidecarSubtitle(
        uri: at('Subs/Russian.srt'),
        name: 'Russian.srt',
        language: 'rus',
      ),
      // Only part 2 has English subtitles.
      SidecarSubtitle(
        uri: at('Heat.1995.DVDRip.cd2.en.srt'),
        name: 'Heat.1995.DVDRip.cd2.en.srt',
        language: 'eng',
        parts: [null, at('Heat.1995.DVDRip.cd2.en.srt')],
      ),
    ]);
    // A single file keeps its plain list.
    expect(await lib.sidecarSubtitles(await movie('Arrival')), isEmpty);
  });

  test('a DVD folder plays the VOBs of its main title set', () async {
    final alien = await movie('Alien');
    expect(await lib.playParts(alien), [
      p.join(root, '$dvd/VTS_01_1.VOB'),
      p.join(root, '$dvd/VTS_01_2.VOB'),
    ]);
  });

  test('a single file plays alone; unknown files have no parts', () async {
    final arrival = await movie('Arrival');
    expect(await lib.playParts(arrival), [p.join(root, one)]);
    final outside = arrival.withFile(p.join(tmp.path, 'elsewhere.mkv'));
    expect(await lib.playParts(outside), isEmpty);
  });

  test('a disc image is unplayable, with its versions', () async {
    final image = await movie('Spirited Away');
    expect(image.unplayable, Unplayable.discImage);
    final versions = await lib.versions(image.id);
    expect(versions.single.unplayable, Unplayable.discImage);
  });

  test('unplayable codes', () {
    expect(Unplayable.fromCode(null), isNull);
    expect(Unplayable.fromCode(''), isNull);
    expect(Unplayable.fromCode('disc_image'), Unplayable.discImage);
    expect(Unplayable.fromCode('from_the_future'), Unplayable.other);
  });

  group('joinPartSubtitles', () {
    SidecarSubtitle sub(String name, {String? lang, bool forced = false}) =>
        SidecarSubtitle(
          uri: '/m/$name',
          name: name,
          language: lang,
          forced: forced,
        );

    test('one part or none: as given', () {
      expect(joinPartSubtitles([]), isEmpty);
      final only = [sub('a.srt')];
      expect(joinPartSubtitles([only]), only);
    });

    test('files of one kind join by their order in each part', () {
      final folder = sub('Subs/Russian.srt', lang: 'rus');
      final joined = joinPartSubtitles([
        [
          sub('a.cd1.en.srt', lang: 'eng'),
          sub('a.cd1.en.sdh.srt', lang: 'eng'),
          sub('a.cd1.en.forced.srt', lang: 'eng', forced: true),
          sub('a.cd1.ass'),
          folder,
        ],
        [
          sub('a.cd2.en.forced.srt', lang: 'eng', forced: true),
          sub('a.cd2.en.srt', lang: 'eng'),
          sub('a.cd2.en.sdh.srt', lang: 'eng'),
          sub('a.cd2.srt'),
          folder,
        ],
      ]);
      expect(joined.map((s) => (s.name, s.parts?.join(' + '))), [
        ('a.cd1.en.srt', '/m/a.cd1.en.srt + /m/a.cd2.en.srt'),
        ('a.cd1.en.sdh.srt', '/m/a.cd1.en.sdh.srt + /m/a.cd2.en.sdh.srt'),
        (
          'a.cd1.en.forced.srt',
          '/m/a.cd1.en.forced.srt + /m/a.cd2.en.forced.srt',
        ),
        // Another format does not join: two tracks.
        ('a.cd1.ass', null),
        ('a.cd2.srt', 'null + /m/a.cd2.srt'),
        ('Subs/Russian.srt', null),
      ]);
      expect(joined[1].uri, '/m/a.cd1.en.sdh.srt');
      expect(joined[4].uri, '/m/a.cd2.srt');
    });
  });
}
