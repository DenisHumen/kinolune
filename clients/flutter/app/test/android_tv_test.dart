// SPDX-License-Identifier: MPL-2.0
//
// Android TV integration (PLAY-AND-009): launcher links, the Watch Next
// cards of the Continue and Up Next rows, their art for the launcher and the
// publisher (the channel is mocked; the TV provider side has JVM tests).

import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/platform/android_tv.dart';
import 'package:kinolune/screens/home_feed.dart';

import 'support/test_app.dart';

LibraryItem _item(
  String id, {
  ItemKind kind = ItemKind.movie,
  String? title,
  int? season,
  int? episode,
  String? backdrop,
}) => LibraryItem(
  id: id,
  kind: kind,
  title: title ?? id,
  filePath: '/media/$id.mkv',
  season: season,
  episode: episode,
  backdropUrl: backdrop,
);

Progress _p(String id, int pos, int at) => Progress(
  itemId: id,
  positionMs: pos,
  durationMs: 100000,
  updatedAt: DateTime.fromMillisecondsSinceEpoch(at),
);

class _PassArt extends TvArtFiles {
  _PassArt() : super(Directory.systemTemp);

  @override
  Future<String?> resolve(WatchNextCard card) async => card.artUrl;
}

void main() {
  group('TvLink', () {
    test('links round-trip, ids with reserved characters too', () {
      final play = TvLink.parse(TvLink.play('show:a b/c'));
      expect(play, isA<TvPlayLink>());
      expect((play! as TvPlayLink).id, 'show:a b/c');
      final item = TvLink.parse(TvLink.item('movie-1'));
      expect((item! as TvItemLink).id, 'movie-1');
      final vod = TvLink.parse(TvLink.twitchVod('123', 4500));
      expect((vod! as TvTwitchVodLink).videoId, '123');
      expect(vod.startMs, 4500);
    });

    test('search links come from the Android side with + for spaces', () {
      final s = TvLink.parse('kinolune://search?q=star+wars');
      expect((s! as TvSearchLink).query, 'star wars');
      final ru = TvLink.parse('kinolune://search?q=%D0%BA%D0%BE%D1%82');
      expect((ru! as TvSearchLink).query, 'кот');
    });

    test('anything else is not a link', () {
      for (final bad in [
        'https://example.org/play/1',
        'kinolune://play/',
        'kinolune://play',
        'kinolune://item/a/b',
        'kinolune://search?q=%20',
        'kinolune://other/1',
        'kinolune://twitch/live/x',
      ]) {
        expect(TvLink.parse(bad), isNull, reason: bad);
      }
    });
  });

  group('watchNextCards', () {
    final movie = _item('m1', backdrop: 'https://img/m1.jpg');
    final show = _item('show1', kind: ItemKind.show, title: 'The Show');
    final ep = _item(
      'e1',
      kind: ItemKind.episode,
      title: 'Pilot',
      season: 1,
      episode: 2,
    );
    final next = _item(
      'e2',
      kind: ItemKind.episode,
      title: 'Second',
      season: 1,
      episode: 3,
    );

    test('Continue entries keep their position, Up Next follows', () {
      final cards = watchNextCards(
        [
          ContinueEntry(
            kind: ContinueKind.movie,
            id: 'm1',
            title: 'm1',
            progress: _p('m1', 30000, 2000),
            item: movie,
            imageUrl: movie.backdropUrl,
          ),
          ContinueEntry(
            kind: ContinueKind.episode,
            id: 'e1',
            title: 'The Show',
            progress: _p('e1', 1000, 1000),
            item: ep,
            show: show,
          ),
          ContinueEntry(
            kind: ContinueKind.twitchVod,
            id: 'twitch:vod:42',
            title: 'Speedrun',
            subtitle: 'Streamer',
            progress: _p('twitch:vod:42', 9000, 500),
          ),
        ],
        [
          UpNextEntry(
            episode: next,
            show: show,
            after: DateTime.fromMillisecondsSinceEpoch(300),
          ),
          // Already in the Continue row: not twice.
          UpNextEntry(
            episode: ep,
            show: show,
            after: DateTime.fromMillisecondsSinceEpoch(200),
          ),
        ],
      );
      expect(cards.map((c) => c.key), ['m1', 'e1', 'twitch:vod:42', 'e2']);
      final m = cards[0].toMap(art: 'https://img/m1.jpg');
      expect(m['kind'], 'movie');
      expect(m['next'], 'continue');
      expect(m['positionMs'], 30000);
      expect(m['engagementMs'], 2000);
      expect(m['link'], 'kinolune://play/m1');
      final e = cards[1].toMap();
      expect(e['title'], 'The Show');
      expect(e['episodeTitle'], 'Pilot');
      expect(e['season'], 1);
      expect(e['episode'], 2);
      final v = cards[2].toMap();
      expect(v['kind'], 'clip');
      expect(v['link'], 'kinolune://twitch/vod/42?t=9000');
      final n = cards[3].toMap();
      expect(n['next'], 'next');
      expect(n['positionMs'], 0);
      expect(n['engagementMs'], 300);
    });
  });

  group('TvArtFiles', () {
    late Directory tmp;
    setUp(() => tmp = Directory.systemTemp.createTempSync('tv_art_test'));
    tearDown(() => tmp.deleteSync(recursive: true));

    WatchNextCard card(String? art) => WatchNextCard(
      key: 'k',
      kind: 'movie',
      next: false,
      title: 't',
      link: 'kinolune://play/k',
      engagement: DateTime(2030),
      artSeed: 'k',
      artUrl: art,
    );

    test('remote art stays remote; local art is copied into the dir', () async {
      final files = TvArtFiles(Directory('${tmp.path}/tv_art'));
      expect(
        await files.resolve(card('https://img/x.jpg')),
        'https://img/x.jpg',
      );
      final src = File('${tmp.path}/poster.jpg')..writeAsBytesSync([1, 2, 3]);
      final copied = await files.resolve(card(src.path));
      expect(copied, startsWith('${tmp.path}/tv_art/f'));
      expect(copied, endsWith('.jpg'));
      expect(File(copied!).readAsBytesSync(), [1, 2, 3]);
      // Not an image the launcher can show: the placeholder instead.
      final other = File('${tmp.path}/x.bin')..writeAsBytesSync([0]);
      TestWidgetsFlutterBinding.ensureInitialized();
      expect(
        await files.resolve(card(other.path)),
        '${tmp.path}/tv_art/placeholder.png',
      );
    });

    testWidgets('demo art is drawn into a PNG', (t) async {
      final png = await t.runAsync(() => TvArtFiles.renderArt('seed'));
      expect(png!.sublist(1, 4), utf8.encode('PNG'));
    });
  });

  testWidgets('the launcher publishes the Continue row and takes links', (
    t,
  ) async {
    final services = await demoServices();
    final library = services.library as FakeLibrary;
    const channel = MethodChannel('app.kinolune/tv');
    final published = <List<Object?>>[];
    final calls = <String>[];
    t.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
      call,
    ) async {
      calls.add(call.method);
      switch (call.method) {
        case 'takeLink':
          return 'kinolune://item/movie-the-quiet-orbit';
        case 'watchNextSupported':
          return true;
        case 'publishWatchNext':
          published.add(call.arguments as List<Object?>);
      }
      return null;
    });
    addTearDown(
      () => t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        null,
      ),
    );
    final links = <TvLink>[];
    final tv = AndroidTvLauncher(
      services: services,
      onLink: (l) async => links.add(l),
      art: () async => _PassArt(),
      debounce: const Duration(milliseconds: 10),
    );
    await t.runAsync(() async {
      await tv.start();
      await Future<void>.delayed(const Duration(milliseconds: 300));
    });
    expect(links.single, isA<TvItemLink>());
    expect(published, isNotEmpty);
    final keys = [for (final m in published.last) (m! as Map)['key']];
    expect(keys, contains('movie-the-quiet-orbit'));

    // A new position: the row follows.
    await t.runAsync(() async {
      await library.saveProgress(
        Progress(
          itemId: 'movie-harbor-lights',
          positionMs: 1800000,
          durationMs: 6000000,
          updatedAt: DateTime(2031),
        ),
      );
      await Future<void>.delayed(const Duration(milliseconds: 300));
    });
    final first = published.last.first! as Map;
    expect(first['key'], 'movie-harbor-lights');
    expect(first['positionMs'], 1800000);

    // A link while running; the same one again right away is dropped.
    await t.runAsync(() async {
      for (var i = 0; i < 2; i++) {
        await t.binding.defaultBinaryMessenger.handlePlatformMessage(
          'app.kinolune/tv',
          const StandardMethodCodec().encodeMethodCall(
            const MethodCall('link', 'kinolune://search?q=orbit'),
          ),
          (_) {},
        );
      }
    });
    expect(links.whereType<TvSearchLink>().single.query, 'orbit');
    tv.dispose();
  });
}
