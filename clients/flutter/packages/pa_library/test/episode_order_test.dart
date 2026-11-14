// SPDX-License-Identifier: MPL-2.0
// Episode orders in Dart (UI-010): what libraries without the core show.

import 'package:pa_library/pa_library.dart';
import 'package:test/test.dart';

LibraryItem ep(String id, int season, int episode) => LibraryItem(
  id: id,
  kind: ItemKind.episode,
  title: id,
  filePath: '/tv/$id.mkv',
  showId: 'show',
  season: season,
  episode: episode,
);

void main() {
  final episodes = [
    ep('s2e1', 2, 1),
    ep('s1e2', 1, 2),
    ep('sp1', 0, 1),
    ep('s1e1', 1, 1),
  ];

  test('TMDB keeps the numbers, specials first and as the last tab', () {
    final o = orderEpisodes(episodes);
    expect(o.order, EpisodeOrder.tmdb);
    expect(o.available, [EpisodeOrder.tmdb, EpisodeOrder.absolute]);
    expect(o.episodes.map((e) => e.item.id), ['sp1', 's1e1', 's1e2', 's2e1']);
    expect(o.seasons, [1, 2, 0]);
    expect(o.inSeason(1).map((e) => e.item.id), ['s1e1', 's1e2']);
  });

  test('absolute counts regular episodes across seasons', () {
    final o = orderEpisodes(episodes, order: EpisodeOrder.absolute);
    expect(o.order, EpisodeOrder.absolute);
    expect(o.seasons, [1, 0]);
    expect(o.episodes.map((e) => (e.item.id, e.item.season, e.item.episode)), [
      ('sp1', 0, 1),
      ('s1e1', 1, 1),
      ('s1e2', 1, 2),
      ('s2e1', 1, 3),
    ]);
    expect(o.episodes.last.tmdbSeason, 2);
  });

  test('AniDB from mappings, unmapped keep TMDB, no data → TMDB', () {
    expect(
      orderEpisodes(episodes, order: EpisodeOrder.anidb).order,
      EpisodeOrder.tmdb,
    );
    final o = orderEpisodes(
      episodes,
      order: EpisodeOrder.anidb,
      anidb: {'s1e1': (1, 1), 's1e2': (2, 1), 's2e1': (2, 2)},
    );
    expect(o.order, EpisodeOrder.anidb);
    expect(o.available, contains(EpisodeOrder.anidb));
    expect(o.inSeason(1).map((e) => e.item.id), ['s1e1']);
    expect(o.inSeason(2).map((e) => e.item.id), ['s1e2', 's2e1']);
    expect(o.inSeason(0).single.item.id, 'sp1');
  });

  test('progress is attached; unseen means not started', () {
    final o = orderEpisodes(
      episodes,
      progress: {
        's1e1': Progress(
          itemId: 's1e1',
          positionMs: 10,
          durationMs: 100,
          updatedAt: DateTime(2026),
          watched: true,
        ),
      },
    );
    final s1e1 = o.episodes.firstWhere((e) => e.item.id == 's1e1');
    expect((s1e1.watched, s1e1.unseen), (true, false));
    expect(o.episodes.firstWhere((e) => e.item.id == 's1e2').unseen, isTrue);
  });
}
