// SPDX-License-Identifier: MPL-2.0
//
// TWI-026: the saved streams as a library — streamer → game → parts.
//
// The core gives every saved file the categories on air in it (TWI-008
// chapters of its VOD or live session, the longest first); this file
// decides what the library shows:
//
// - A part is a saved VOD or recording (clips are not parts). Copies of the
//   same broadcast (the recording and the downloaded VOD) are one part: the
//   longer copy, a download on a tie.
// - The game of a part is its longest category that is a game
//   ([isNonGameCategory]: Just Chatting and the like are talk). Another game
//   played at least [kMinPartPlay] in the same stream makes it a part of
//   that game's playthrough too, opened where that game starts.
// - A playthrough: the parts of one game by one streamer in date order,
//   numbered from 1. A break longer than [kNewPlaythroughGap] between two
//   parts starts a new playthrough of the game (a replay a year later);
//   other games in between do not break it.
// - Streams without a game (only talk, or no chapters) stay under the
//   streamer as «Другие эфиры».
//
// Search and filters ([TwitchArchiveFilter]) pick parts; part numbers
// always come from the whole library, so a found part keeps its number.

import '../integration/library.dart' show Progress;
import 'twitch_archive.dart';
import 'twitch_skip_chatting.dart';

/// A break between two parts of a game that starts a new playthrough.
const kNewPlaythroughGap = Duration(days: 120);

/// A second game counts as played in a stream from this long.
const kMinPartPlay = Duration(minutes: 15);

/// One saved broadcast in a playthrough.
class TwitchArchivePart {
  const TwitchArchivePart({
    required this.file,
    required this.number,
    required this.game,
  });

  final TwitchSavedFile file;

  /// 1-based, in date order.
  final int number;

  /// The playthrough's game in this file.
  final TwitchSavedGame game;

  /// Where the game starts in the file: a new part opens there, not at the
  /// talk before it (TWI-027).
  Duration get gameStart => game.start;
}

/// The parts of one game by one streamer.
class TwitchPlaythrough {
  const TwitchPlaythrough({
    required this.userId,
    required this.channel,
    required this.gameId,
    required this.gameName,
    required this.boxArtUrl,
    required this.run,
    required this.parts,
  });

  final String userId;
  final String channel;
  final String gameId;

  /// Null: the category was never fetched.
  final String? gameName;
  final String? boxArtUrl;

  /// 1 for the first playthrough of the game, 2 for a replay after
  /// [kNewPlaythroughGap]…
  final int run;

  /// In date order; never empty.
  final List<TwitchArchivePart> parts;

  /// Stable id within the library.
  String get id => '$userId/$gameId/$run';

  DateTime? get firstAt => parts.first.file.recordedAt;
  DateTime? get lastAt => parts.last.file.recordedAt;

  /// Time on air in this game, all parts together.
  Duration get gameTime =>
      parts.fold(Duration.zero, (sum, p) => sum + p.game.duration);
}

/// A streamer's shelf: playthroughs (the latest first) and the streams
/// without a game.
class TwitchArchiveStreamer {
  const TwitchArchiveStreamer({
    required this.userId,
    required this.channel,
    required this.login,
    required this.playthroughs,
    required this.other,
  });

  final String userId;
  final String channel;
  final String? login;
  final List<TwitchPlaythrough> playthroughs;

  /// Saved streams with no game, the newest first.
  final List<TwitchSavedFile> other;

  DateTime? get lastAt {
    DateTime? last;
    for (final t in [
      for (final p in playthroughs) p.lastAt,
      for (final f in other) f.recordedAt,
    ]) {
      if (t != null && (last == null || t.isAfter(last))) last = t;
    }
    return last;
  }
}

/// What the search and the filters keep.
class TwitchArchiveFilter {
  const TwitchArchiveFilter({
    this.query = '',
    this.userId,
    this.gameId,
    this.day,
  });

  /// Words that all must be found in the title, the streamer, the game or
  /// the date (`2026-11-03`, `03.11.2026`, `2026-11`, `2026`).
  final String query;
  final String? userId;
  final String? gameId;

  /// A local calendar day.
  final DateTime? day;

  bool get isEmpty =>
      query.trim().isEmpty && userId == null && gameId == null && day == null;

  TwitchArchiveFilter copyWith({
    String? query,
    String? Function()? userId,
    String? Function()? gameId,
    DateTime? Function()? day,
  }) => TwitchArchiveFilter(
    query: query ?? this.query,
    userId: userId == null ? this.userId : userId(),
    gameId: gameId == null ? this.gameId : gameId(),
    day: day == null ? this.day : day(),
  );

  /// [file], whose games for the library are [games], passes.
  bool matches(TwitchSavedFile file, {Iterable<TwitchSavedGame>? games}) {
    final g = games ?? file.games;
    if (userId != null && file.userId != userId) return false;
    if (gameId != null && !g.any((x) => x.id == gameId)) return false;
    final at = file.recordedAt?.toLocal();
    if (day != null && (at == null || !sameDay(at, day!))) return false;
    final words = query.toLowerCase().split(RegExp(r'\s+'))
      ..removeWhere((w) => w.isEmpty);
    if (words.isEmpty) return true;
    final hay = [
      file.title,
      file.channel,
      file.login ?? '',
      for (final x in g) x.name ?? '',
      if (at != null) ...dateTokens(at),
    ].join('\n').toLowerCase();
    return words.every(hay.contains);
  }
}

/// The same local calendar day.
bool sameDay(DateTime a, DateTime b) =>
    a.year == b.year && a.month == b.month && a.day == b.day;

/// How a date can be typed in the search.
List<String> dateTokens(DateTime at) {
  String two(int n) => n.toString().padLeft(2, '0');
  final y = '${at.year}';
  final m = two(at.month);
  final d = two(at.day);
  return ['$y-$m-$d', '$d.$m.$y', '$d.$m', '$y-$m', '$m.$y'];
}

/// The library of saved streams.
class TwitchArchiveLibrary {
  const TwitchArchiveLibrary(this.streamers);

  /// The streamer with the latest stream first.
  final List<TwitchArchiveStreamer> streamers;

  static const empty = TwitchArchiveLibrary([]);

  /// Builds the library from the saved files (any order).
  factory TwitchArchiveLibrary.build(List<TwitchSavedFile> files) {
    final byUser = <String, List<TwitchSavedFile>>{};
    for (final f in _distinct(files)) {
      byUser.putIfAbsent(f.userId, () => []).add(f);
    }
    final streamers = <TwitchArchiveStreamer>[];
    for (final MapEntry(key: userId, value: own) in byUser.entries) {
      own.sort(_byDate);
      final byGame = <String, List<(TwitchSavedFile, TwitchSavedGame)>>{};
      final other = <TwitchSavedFile>[];
      for (final f in own) {
        final games = partGames(f);
        if (games.isEmpty) other.add(f);
        for (final g in games) {
          byGame.putIfAbsent(g.id, () => []).add((f, g));
        }
      }
      final playthroughs = <TwitchPlaythrough>[];
      for (final MapEntry(key: gameId, value: list) in byGame.entries) {
        var run = <(TwitchSavedFile, TwitchSavedGame)>[];
        var runs = 0;
        void close() {
          if (run.isEmpty) return;
          runs++;
          playthroughs.add(_playthrough(userId, gameId, runs, run));
          run = [];
        }

        for (final e in list) {
          final prev = run.isEmpty ? null : run.last.$1.recordedAt;
          final at = e.$1.recordedAt;
          if (prev != null &&
              at != null &&
              at.difference(prev) > kNewPlaythroughGap) {
            close();
          }
          run.add(e);
        }
        close();
      }
      playthroughs.sort((a, b) => _laterFirst(a.lastAt, b.lastAt));
      final first = own.first;
      streamers.add(
        TwitchArchiveStreamer(
          userId: userId,
          channel: first.channel,
          login: first.login,
          playthroughs: playthroughs,
          other: other.reversed.toList(),
        ),
      );
    }
    streamers.sort((a, b) => _laterFirst(a.lastAt, b.lastAt));
    return TwitchArchiveLibrary(streamers);
  }

  /// Every playthrough, of every streamer.
  Iterable<TwitchPlaythrough> get playthroughs =>
      streamers.expand((s) => s.playthroughs);

  /// The playthrough with [id], if still there.
  TwitchPlaythrough? playthrough(String id) {
    for (final p in playthroughs) {
      if (p.id == id) return p;
    }
    return null;
  }

  /// The parts the [filter] keeps, the newest first, with their
  /// playthrough; then the streams without a game it keeps.
  ({
    List<(TwitchPlaythrough, TwitchArchivePart)> parts,
    List<TwitchSavedFile> other,
  })
  search(TwitchArchiveFilter filter) {
    final parts = <(TwitchPlaythrough, TwitchArchivePart)>[];
    final other = <TwitchSavedFile>[];
    for (final s in streamers) {
      for (final t in s.playthroughs) {
        for (final p in t.parts) {
          if (filter.matches(p.file, games: [p.game])) parts.add((t, p));
        }
      }
      if (filter.gameId == null) {
        other.addAll(s.other.where((f) => filter.matches(f, games: const [])));
      }
    }
    parts.sort(
      (a, b) => _laterFirst(a.$2.file.recordedAt, b.$2.file.recordedAt),
    );
    other.sort((a, b) => _laterFirst(a.recordedAt, b.recordedAt));
    return (parts: parts, other: other);
  }

  /// Local days with a saved stream (the calendar).
  Set<DateTime> get days => {
    for (final s in streamers) ...[
      for (final t in s.playthroughs)
        for (final p in t.parts)
          if (p.file.recordedAt case final at?) _day(at),
      for (final f in s.other)
        if (f.recordedAt case final at?) _day(at),
    ],
  };

  /// Games of the library for the filter: (id, name) by name.
  List<(String, String?)> games({String? userId}) {
    final seen = <String, String?>{};
    for (final t in playthroughs) {
      if (userId != null && t.userId != userId) continue;
      seen.putIfAbsent(t.gameId, () => t.gameName);
    }
    final out = [for (final e in seen.entries) (e.key, e.value)]
      ..sort(
        (a, b) => (a.$2 ?? a.$1).toLowerCase().compareTo(
          (b.$2 ?? b.$1).toLowerCase(),
        ),
      );
    return out;
  }
}

/// The games [file] is a part of: its longest game, and any other game
/// played at least [kMinPartPlay].
List<TwitchSavedGame> partGames(TwitchSavedFile file) {
  if (file.kind == TwitchSavedKind.clip) return const [];
  final games = [
    for (final g in file.games)
      if (!isNonGameCategory(g.id, g.name)) g,
  ];
  if (games.isEmpty) return const [];
  return [
    games.first,
    for (final g in games.skip(1))
      if (g.duration >= kMinPartPlay) g,
  ];
}

DateTime _day(DateTime at) {
  final l = at.toLocal();
  return DateTime(l.year, l.month, l.day);
}

TwitchPlaythrough _playthrough(
  String userId,
  String gameId,
  int run,
  List<(TwitchSavedFile, TwitchSavedGame)> list,
) {
  final named = list
      .map((e) => e.$2)
      .firstWhere((g) => g.name != null, orElse: () => list.first.$2);
  final art = list.map((e) => e.$2.boxArtUrl).whereType<String>();
  return TwitchPlaythrough(
    userId: userId,
    channel: list.first.$1.channel,
    gameId: gameId,
    gameName: named.name,
    boxArtUrl: art.isEmpty ? null : art.first,
    run: run,
    parts: [
      for (final (i, (f, g)) in list.indexed)
        TwitchArchivePart(file: f, number: i + 1, game: g),
    ],
  );
}

/// Oldest first; files without a date last.
int _byDate(TwitchSavedFile a, TwitchSavedFile b) {
  final x = a.recordedAt;
  final y = b.recordedAt;
  if (x == null || y == null) {
    return (x == null ? 1 : 0) - (y == null ? 1 : 0);
  }
  final c = x.compareTo(y);
  return c != 0 ? c : a.assetId.compareTo(b.assetId);
}

int _laterFirst(DateTime? a, DateTime? b) {
  if (a == null || b == null) return (a == null ? 1 : 0) - (b == null ? 1 : 0);
  return b.compareTo(a);
}

/// One file per broadcast: a recording and the downloaded VOD of the same
/// stream (or two copies of a VOD) are the same part.
List<TwitchSavedFile> _distinct(List<TwitchSavedFile> files) {
  final out = <TwitchSavedFile>[];
  for (final f in files) {
    if (f.kind == TwitchSavedKind.clip) continue;
    final i = out.indexWhere(
      (o) =>
          (f.videoId != null && o.videoId == f.videoId) ||
          (f.streamId != null && o.streamId == f.streamId),
    );
    if (i < 0) {
      out.add(f);
    } else if (_better(f, out[i])) {
      out[i] = f;
    }
  }
  return out;
}

bool _better(TwitchSavedFile a, TwitchSavedFile b) {
  final x = a.duration ?? Duration.zero;
  final y = b.duration ?? Duration.zero;
  if (x != y) return x > y;
  return a.kind == TwitchSavedKind.vod && b.kind != TwitchSavedKind.vod;
}

/// Where to go on in [parts] with their [progress] (by
/// [TwitchSavedFile.progressKey]): the last part started, or the one after
/// it once it is watched; the first part when none was started.
int nextPartIndex(
  List<TwitchArchivePart> parts,
  Map<String, Progress> progress,
) {
  for (var i = parts.length - 1; i >= 0; i--) {
    final p = progress[parts[i].file.progressKey];
    if (p == null) continue;
    if (!p.watched) return i;
    return i + 1 < parts.length ? i + 1 : i;
  }
  return 0;
}
