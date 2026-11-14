// SPDX-License-Identifier: MPL-2.0

// Episodes of a show in an episode order (UI-010, the DisplayPreference of
// 03 §4.2). RustLibrary reads them from the core (`Catalog::show_episodes`,
// the order is stored per show); libraries without the core order the
// list in Dart with [orderEpisodes] (TMDB and absolute only).

import 'models.dart';

/// How a show numbers its episodes.
enum EpisodeOrder {
  /// Seasons and episodes as TMDB has them (the default).
  tmdb,

  /// One run of regular episodes (season 1 holds all of them); specials
  /// stay in season 0.
  absolute,

  /// AniDB entries as seasons, from anime mappings.
  anidb,
}

/// An episode numbered in an order.
class ShowEpisode {
  /// [LibraryItem.season] and [LibraryItem.episode] are in the order.
  final LibraryItem item;

  /// Numbers in TMDB order.
  final int? tmdbSeason;
  final int? tmdbEpisode;
  final Progress? progress;

  const ShowEpisode({
    required this.item,
    this.tmdbSeason,
    this.tmdbEpisode,
    this.progress,
  });

  bool get watched => progress?.watched ?? false;

  /// Neither watched nor started: what Hide Spoilers blurs.
  bool get unseen => !watched && (progress?.positionMs ?? 0) <= 0;
}

/// Episodes of a show for the Show Detail screen.
class ShowEpisodes {
  /// The order [episodes] are numbered in.
  final EpisodeOrder order;

  /// Orders there is data for; [EpisodeOrder.tmdb] first.
  final List<EpisodeOrder> available;

  /// By season (specials first), then episode.
  final List<ShowEpisode> episodes;

  const ShowEpisodes({
    required this.order,
    required this.available,
    required this.episodes,
  });

  static const empty = ShowEpisodes(
    order: EpisodeOrder.tmdb,
    available: [EpisodeOrder.tmdb],
    episodes: [],
  );

  /// Season numbers for the season tabs: regular seasons ascending, then
  /// Specials (0).
  List<int> get seasons {
    final s = {for (final e in episodes) e.item.season ?? 1}.toList()
      ..sort((a, b) {
        if (a == 0) return 1;
        if (b == 0) return -1;
        return a.compareTo(b);
      });
    return s;
  }

  List<ShowEpisode> inSeason(int season) =>
      episodes.where((e) => (e.item.season ?? 1) == season).toList();
}

/// A library that numbers episodes in other orders and stores the choice
/// per show (the Rust core, the demo library).
abstract interface class LibraryEpisodes {
  /// Episodes of [showId] in [order], or in the order stored for the show
  /// when null. An order without data falls back to TMDB. Updates when the
  /// library or progress changes.
  Stream<ShowEpisodes> showEpisodes(String showId, {EpisodeOrder? order});

  /// Stores the episode order of [showId].
  Future<void> setEpisodeOrder(String showId, EpisodeOrder order);
}

int _season(LibraryItem e) => e.season ?? 1;

int _compare(ShowEpisode a, ShowEpisode b) {
  final s = _season(a.item).compareTo(_season(b.item));
  if (s != 0) return s;
  final e = (a.item.episode ?? 1 << 30).compareTo(b.item.episode ?? 1 << 30);
  if (e != 0) return e;
  return a.item.filePath.compareTo(b.item.filePath);
}

/// Numbers [episodes] (in TMDB order) in [order]: what the core does, for
/// libraries without it. Absolute numbers count the regular episodes in
/// TMDB order (a library without provider data knows no gaps). [anidb]
/// maps an episode id to its AniDB (entry, episode); episodes without one
/// keep their TMDB numbers. [progress] attaches progress by item id.
ShowEpisodes orderEpisodes(
  List<LibraryItem> episodes, {
  EpisodeOrder order = EpisodeOrder.tmdb,
  Map<String, Progress> progress = const {},
  Map<String, (int, int)> anidb = const {},
}) {
  final available = [
    EpisodeOrder.tmdb,
    if (episodes.any((e) => _season(e) >= 1)) EpisodeOrder.absolute,
    if (episodes.any((e) => anidb.containsKey(e.id))) EpisodeOrder.anidb,
  ];
  final applied = available.contains(order) ? order : EpisodeOrder.tmdb;
  final tmdb = [
    for (final e in episodes)
      ShowEpisode(
        item: e,
        tmdbSeason: e.season,
        tmdbEpisode: e.episode,
        progress: progress[e.id],
      ),
  ]..sort(_compare);
  var n = 0;
  final out = <ShowEpisode>[];
  for (final e in tmdb) {
    final item = e.item;
    final LibraryItem numbered = switch (applied) {
      EpisodeOrder.tmdb => item,
      EpisodeOrder.absolute =>
        _season(item) == 0 ? item : item.renumbered(1, ++n),
      EpisodeOrder.anidb => switch (anidb[item.id]) {
        final (int, int) a => item.renumbered(a.$1, a.$2),
        null => item,
      },
    };
    out.add(
      ShowEpisode(
        item: numbered,
        tmdbSeason: e.tmdbSeason,
        tmdbEpisode: e.tmdbEpisode,
        progress: e.progress,
      ),
    );
  }
  out.sort(_compare);
  return ShowEpisodes(order: applied, available: available, episodes: out);
}
