// SPDX-License-Identifier: MPL-2.0
//
// TWI-026: saved streams grouped as a library — streamer → game → parts —
// and the search over it. Pure logic on made-up files, no core.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/twitch/twitch_archive.dart';
import 'package:kinolune/twitch/twitch_archive_library.dart';

const _game = TwitchSavedGame(
  id: '1159690097',
  name: 'Made-Up Quest',
  duration: Duration(hours: 3),
  start: Duration(minutes: 40),
);
const _talk = TwitchSavedGame(
  id: '509658',
  name: 'Just Chatting',
  duration: Duration(minutes: 40),
);
const _other = TwitchSavedGame(
  id: '512953',
  name: 'Tiny Racer',
  duration: Duration(hours: 2),
);

TwitchSavedFile _file(
  int id,
  DateTime at, {
  List<TwitchSavedGame> games = const [_game, _talk],
  String userId = '4040',
  String login = 'madeupstreamer',
  TwitchSavedKind kind = TwitchSavedKind.recording,
  String? videoId,
  String title = 'Evening stream',
}) => TwitchSavedFile(
  assetId: id,
  kind: kind,
  userId: userId,
  login: login,
  displayName: login,
  streamId: kind == TwitchSavedKind.recording ? '9$id' : null,
  videoId: videoId,
  title: '$title #$id',
  recordedAt: at,
  duration: const Duration(hours: 4),
  path: '/media/twitch/$login/$id.mkv',
  games: games,
);

void main() {
  // 12 streams of one game over five weeks: breaks of 1–6 days, two other
  // games and a talk-only stream in between, files in random order.
  final start = DateTime(2026, 10, 1, 19);
  final gaps = [0, 1, 3, 4, 8, 9, 15, 16, 20, 27, 30, 36];
  final quest = [
    for (final (i, d) in gaps.indexed)
      _file(100 + i, start.add(Duration(days: d))),
  ];
  final files = [
    ...quest.reversed.skip(6),
    _file(200, start.add(const Duration(days: 2)), games: [_other]),
    _file(201, start.add(const Duration(days: 10)), games: [_talk]),
    ...quest.reversed.take(6),
    _file(202, start.add(const Duration(days: 12)), games: [_other, _talk]),
    _file(
      300,
      start.add(const Duration(days: 5)),
      userId: '5050',
      login: 'otherstreamer',
      games: [_game],
      title: 'Speedrun',
    ),
  ];

  test('12 streams of one game with breaks are a playthrough of 12 parts', () {
    final lib = TwitchArchiveLibrary.build(files);
    final me = lib.streamers.firstWhere((s) => s.userId == '4040');
    final run = me.playthroughs.firstWhere((p) => p.gameId == _game.id);
    expect(run.gameName, 'Made-Up Quest');
    expect(run.parts, hasLength(12));
    expect([for (final p in run.parts) p.number], List.generate(12, (i) => i + 1));
    expect([for (final p in run.parts) p.file.assetId], [
      for (var i = 0; i < 12; i++) 100 + i,
    ]);
    final dates = [for (final p in run.parts) p.file.recordedAt!];
    for (var i = 1; i < dates.length; i++) {
      expect(dates[i].isAfter(dates[i - 1]), isTrue);
    }
    // A part opens where the game starts, not at the talk before it.
    expect(run.parts.first.gameStart, const Duration(minutes: 40));
    expect(run.gameTime, const Duration(hours: 36));
    // The other game is its own playthrough; the talk-only stream is
    // under «Другие эфиры»; the other streamer has their own shelf.
    final racer = me.playthroughs.firstWhere((p) => p.gameId == _other.id);
    expect([for (final p in racer.parts) p.file.assetId], [200, 202]);
    expect([for (final f in me.other) f.assetId], [201]);
    expect(lib.streamers.map((s) => s.userId), ['4040', '5050']);
    expect(lib.playthrough(run.id), same(run));
  });

  test('a long break starts a new playthrough of the game', () {
    final lib = TwitchArchiveLibrary.build([
      _file(1, DateTime(2025, 1, 10)),
      _file(2, DateTime(2025, 1, 12)),
      _file(3, DateTime(2025, 9, 1)),
    ]);
    final runs = lib.streamers.single.playthroughs;
    expect([for (final r in runs) (r.run, r.parts.length)], [(2, 1), (1, 2)]);
  });

  test('a second game played long enough makes the stream its part too', () {
    const short = TwitchSavedGame(
      id: '512953',
      name: 'Tiny Racer',
      duration: Duration(minutes: 5),
    );
    final lib = TwitchArchiveLibrary.build([
      _file(1, DateTime(2026, 3, 1), games: [_game, _other]),
      _file(2, DateTime(2026, 3, 2), games: [_game, short]),
    ]);
    final runs = {
      for (final r in lib.streamers.single.playthroughs) r.gameId: r,
    };
    expect(runs[_game.id]!.parts, hasLength(2));
    expect([for (final p in runs[_other.id]!.parts) p.file.assetId], [1]);
  });

  test('a recording and the downloaded VOD of a stream are one part', () {
    final rec = TwitchSavedFile(
      assetId: 1,
      kind: TwitchSavedKind.recording,
      userId: '4040',
      streamId: '91',
      videoId: '777',
      title: 'Live',
      recordedAt: DateTime(2026, 3, 1),
      duration: const Duration(hours: 3),
      path: '/a.mkv',
      games: const [_game],
    );
    final vod = TwitchSavedFile(
      assetId: 2,
      kind: TwitchSavedKind.vod,
      userId: '4040',
      videoId: '777',
      title: 'VOD',
      recordedAt: DateTime(2026, 3, 1),
      duration: const Duration(hours: 3),
      path: '/b.mp4',
      games: const [_game],
    );
    final run = TwitchArchiveLibrary.build([
      rec,
      vod,
    ]).streamers.single.playthroughs.single;
    expect(run.parts.single.file.assetId, 2);
  });

  group('search', () {
    final lib = TwitchArchiveLibrary.build(files);
    List<int> ids(TwitchArchiveFilter f) {
      final r = lib.search(f);
      return [
        for (final (_, p) in r.parts) p.file.assetId,
        for (final o in r.other) o.assetId,
      ]..sort();
    }

    test('by game keeps the part numbers', () {
      final r = lib.search(const TwitchArchiveFilter(query: 'tiny'));
      expect([for (final (_, p) in r.parts) (p.file.assetId, p.number)], [
        (202, 2),
        (200, 1),
      ]);
      expect(ids(const TwitchArchiveFilter(gameId: '512953')), [200, 202]);
    });

    test('by streamer', () {
      expect(ids(const TwitchArchiveFilter(query: 'otherstreamer')), [300]);
      expect(ids(const TwitchArchiveFilter(userId: '5050')), [300]);
      expect(ids(const TwitchArchiveFilter(userId: '4040')), hasLength(15));
    });

    test('by date', () {
      // Day 9 of the quest is 2026-10-10.
      expect(ids(const TwitchArchiveFilter(query: '2026-10-10')), [105]);
      expect(ids(const TwitchArchiveFilter(query: '10.10.2026')), [105]);
      expect(ids(TwitchArchiveFilter(day: DateTime(2026, 10, 11))), [201]);
      expect(lib.days, contains(DateTime(2026, 10, 6)));
      expect(lib.days, hasLength(16));
    });

    test('by title and several words', () {
      expect(ids(const TwitchArchiveFilter(query: 'speedrun')), [300]);
      expect(ids(const TwitchArchiveFilter(query: 'quest #111')), [111]);
      expect(ids(const TwitchArchiveFilter(query: 'quest nothing')), isEmpty);
    });
  });
}
