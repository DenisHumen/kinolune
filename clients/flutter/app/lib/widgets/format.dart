// SPDX-License-Identifier: MPL-2.0

import '../integration/library.dart';
import '../l10n/l10n.dart';

String formatRuntime(AppLocalizations l, int? ms) {
  if (ms == null || ms <= 0) return '';
  final m = (ms / 60000).round();
  if (m < 60) return l.formatMinutes(m);
  return l.formatHoursMinutes(m ~/ 60, m % 60);
}

String formatClock(int ms) {
  final d = Duration(milliseconds: ms);
  final s = d.inSeconds % 60;
  return '${d.inMinutes}:${s.toString().padLeft(2, '0')}';
}

/// `1:23:45` from an hour on, `41:07` below.
String formatTimecode(int ms) {
  final d = Duration(milliseconds: ms < 0 ? 0 : ms);
  String two(int v) => v.toString().padLeft(2, '0');
  final s = two(d.inSeconds % 60);
  return d.inHours > 0
      ? '${d.inHours}:${two(d.inMinutes % 60)}:$s'
      : '${d.inMinutes}:$s';
}

String formatRemaining(AppLocalizations l, Progress p) {
  final left = ((p.durationMs - p.positionMs) / 60000).round();
  return left >= 60
      ? l.formatHoursMinutesLeft(left ~/ 60, left % 60)
      : l.formatMinutesLeft(left);
}

/// A span of a show, a season or a movie (UI-056): «40 мин», «7 ч 10 мин»,
/// «12 ч»; from 100 h on only hours. [approx]: «≈» in front (a duration
/// from metadata or unknown).
String formatSpan(AppLocalizations l, int ms, {bool approx = false}) {
  final m = ms <= 0 ? 0 : (ms / 60000).round().clamp(1, 1 << 30);
  final h = m ~/ 60;
  final s = m < 60
      ? l.formatMinutes(m)
      : (h >= 100 || m % 60 == 0)
      ? l.formatHours(h)
      : l.formatHoursMinutes(h, m % 60);
  return approx ? '≈$s' : s;
}

/// The time left of [t] (UI-056): «осталось 4 ч 10 мин», «просмотрено»
/// when every episode is watched; empty when nothing was watched or
/// started yet (the total says it all) or nothing is known to be left.
String formatLeftOf(AppLocalizations l, RuntimeTotal t) {
  if (t.allWatched) return l.formatAllWatched;
  if (t.untouched || t.remainingMs <= 0) return '';
  return l.formatLeft(
    formatSpan(l, t.remainingMs, approx: t.remainingEstimated),
  );
}

/// The runtime of a show (UI-056): «всего 21 ч 40 мин · осталось 12 ч 5 мин»;
/// empty when no duration is known.
String formatShowRuntime(AppLocalizations l, RuntimeTotal t) {
  if (t.totalMs <= 0) return '';
  return [
    l.formatTotal(formatSpan(l, t.totalMs, approx: t.estimated)),
    formatLeftOf(l, t),
  ].where((s) => s.isNotEmpty).join(' · ');
}

/// The line of a season tab (UI-056): «10 серий · 7 ч 10 мин · осталось
/// 2 ч 30 мин».
String formatSeasonRuntime(AppLocalizations l, RuntimeTotal t) => [
  if (t.episodes > 0) l.showEpisodeCount(t.episodes),
  if (t.totalMs > 0) formatSpan(l, t.totalMs, approx: t.estimated),
  if (t.totalMs > 0) formatLeftOf(l, t),
].where((s) => s.isNotEmpty).join(' · ');

/// A started movie (UI-056): «всего 2 ч 28 мин · осталось 40 мин», the
/// same formula as a show; just the time left when the length is unknown.
String formatMovieProgress(AppLocalizations l, Progress p, {int? totalMs}) {
  final total = (totalMs ?? 0) > 0 ? totalMs! : p.durationMs;
  if (total <= 0) return formatRemaining(l, p);
  final base = p.durationMs > 0 ? p.durationMs : total;
  final left = (total * (1 - p.positionMs / base)).round().clamp(0, total);
  return [
    l.formatTotal(formatSpan(l, total)),
    p.watched ? l.formatAllWatched : l.formatLeft(formatSpan(l, left)),
  ].join(' · ');
}

/// «ещё 23 мин» / «ещё 1 ч 5 мин» on the still of a started item (UI-037).
String watchedLeft(AppLocalizations l, Progress p) {
  final left = ((p.durationMs - p.positionMs) / 60000).round().clamp(
    0,
    1 << 20,
  );
  return left >= 60
      ? l.watchedHoursMinutesLeft(left ~/ 60, left % 60)
      : l.watchedMinutesLeft(left);
}

String episodeCode(AppLocalizations l, LibraryItem e) =>
    l.showEpisodeCode(e.season ?? 0, e.episode ?? 0);

/// Seed for generated album art: the folder of the track (shared by an album).
String albumSeedFor(LibraryItem track) {
  final i = track.filePath.lastIndexOf('/');
  return i > 0 ? track.filePath.substring(0, i) : (track.album ?? track.title);
}

/// Groups tracks into albums, preserving first-seen order.
List<Album> groupAlbums(AppLocalizations l, List<LibraryItem> tracks) {
  final map = <String, Album>{};
  for (final t in tracks) {
    final key = '${t.artist ?? ''}\u0000${t.album ?? ''}';
    map
        .putIfAbsent(
          key,
          () => Album(
            t.album ?? l.musicUnknownAlbum,
            t.artist ?? l.musicUnknownArtist,
            t.year,
            albumSeedFor(t),
            t.posterUrl,
          ),
        )
        .tracks
        .add(t);
  }
  for (final a in map.values) {
    a.tracks.sort((x, y) => (x.trackNumber ?? 0).compareTo(y.trackNumber ?? 0));
  }
  return map.values.toList();
}

class Album {
  Album(this.title, this.artist, this.year, this.artSeed, [this.artUrl]);
  final String title;
  final String? artUrl;
  final String artist;
  final int? year;
  final String artSeed;
  final List<LibraryItem> tracks = [];
}
