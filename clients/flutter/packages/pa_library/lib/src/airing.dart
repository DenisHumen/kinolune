// SPDX-License-Identifier: MPL-2.0

// When the next episode of an ongoing show or anime airs (UI-057). The
// core asks TMDB (`next_episode_to_air`, `status`), AniList
// (`nextAiringEpisode`, anime) and Shikimori as the fallback (ADR-0142), and
// keeps the last answer: offline it comes back marked [ShowAiring.offline].

/// Where a show is in its run.
enum AiringStatus {
  /// New episodes come; [ShowAiring.next] has the next one when its date is
  /// known.
  airing,

  /// On a break, no date yet.
  hiatus,

  /// Finished or cancelled: «Сериал завершён».
  ended,
}

/// The next episode of a show, still to come.
class NextAiring {
  /// Season (TMDB numbering); null when the provider numbers the episodes
  /// in one run (anime): the card goes to the end of the last season.
  final int? season;
  final int episode;

  /// Only the day is known (TMDB): a local calendar date, no time.
  final DateTime? date;

  /// The exact moment (anime), UTC: a countdown to the hour.
  final DateTime? at;

  const NextAiring({required this.episode, this.season, this.date, this.at});

  /// The day it airs on the device: [at] in local time, else [date].
  DateTime? get day {
    final a = at;
    if (a != null) {
      final l = a.toLocal();
      return DateTime(l.year, l.month, l.day);
    }
    return date;
  }

  @override
  bool operator ==(Object other) =>
      other is NextAiring &&
      other.season == season &&
      other.episode == episode &&
      other.date == date &&
      other.at == at;

  @override
  int get hashCode => Object.hash(season, episode, date, at);
}

/// The airing state of a show.
class ShowAiring {
  final String showId;
  final AiringStatus status;
  final NextAiring? next;

  /// `tmdb`, `anilist` or `shikimori`.
  final String source;

  /// When a provider last answered.
  final DateTime checkedAt;

  /// The last refresh failed (offline): this is an older answer.
  final bool offline;

  const ShowAiring({
    required this.showId,
    required this.status,
    required this.checkedAt,
    this.next,
    this.source = '',
    this.offline = false,
  });

  @override
  bool operator ==(Object other) =>
      other is ShowAiring &&
      other.showId == showId &&
      other.status == status &&
      other.next == next &&
      other.source == source &&
      other.checkedAt == checkedAt &&
      other.offline == offline;

  @override
  int get hashCode =>
      Object.hash(showId, status, next, source, checkedAt, offline);
}

/// A library that knows when shows air (the core).
abstract interface class LibraryAiring {
  /// The airing state of [showId] as last fetched; null without data.
  Future<ShowAiring?> showAiring(String showId);

  /// Asks the providers about [showId] now (its page opened). Offline the
  /// last value comes back with [ShowAiring.offline]; null without data.
  Future<ShowAiring?> refreshAiring(String showId);

  /// Every show with an airing state (labels on the posters).
  Future<List<ShowAiring>> airingAll();

  /// The daily background refresh of the shows that are due; how many a
  /// provider answered for.
  Future<int> refreshAiringDue();

  /// Fires after a refresh changed something.
  Stream<void> get airingChanges;
}
