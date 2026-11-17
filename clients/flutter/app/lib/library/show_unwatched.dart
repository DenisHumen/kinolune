// SPDX-License-Identifier: MPL-2.0
//
// Unwatched episodes of a show for its poster (UI-037): the count, or «✓»
// when every episode is watched. Counted only for the show posters on
// screen, from the same episode list the show page reads (with progress in
// the LIB-011 thresholds); the last count of a show is kept, so a poster
// scrolled back into view shows it at once and only refreshes it.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../widgets/watched_art.dart' show WatchArt, watchArtOf;
import 'library_queries.dart' show combineLatest2;

/// Unwatched regular episodes in [episodes] (specials do not count).
int unwatchedEpisodes(Iterable<ShowEpisode> episodes) => episodes
    .where(
      (e) =>
          (e.item.season ?? 1) >= 1 &&
          !e.watched &&
          watchArtOf(e.progress) != WatchArt.watched,
    )
    .length;

/// Unwatched episode counts of the shows of a library, by show id.
class ShowUnwatchedCounts {
  ShowUnwatchedCounts._(this.library);

  final Library library;

  static final _byLibrary = Expando<ShowUnwatchedCounts>('show counts');

  static ShowUnwatchedCounts of(Library library) =>
      _byLibrary[library] ??= ShowUnwatchedCounts._(library);

  final _shows = <String, _Show>{};

  /// The count of [showId], kept up to date while held; release with
  /// [release].
  ValueListenable<int?> hold(String showId) {
    final s = _shows.putIfAbsent(showId, () => _Show());
    if (s.holders++ == 0) {
      s.sub = _episodes(showId).listen(
        (eps) => s.count.value = unwatchedEpisodes(eps.episodes),
        onError: (Object e) => debugPrint('Episodes of $showId: $e'),
      );
    }
    return s.count;
  }

  void release(String showId) {
    final s = _shows[showId];
    if (s == null || --s.holders > 0) return;
    unawaited(s.sub?.cancel());
    s.sub = null;
  }

  Stream<ShowEpisodes> _episodes(String showId) {
    final l = library;
    if (l case final LibraryEpisodes e) return e.showEpisodes(showId);
    // v0: progress of started episodes from "Continue watching".
    return combineLatest2(
      l.episodes(showId),
      l.continueWatching(),
      (List<LibraryItem> eps, List<(LibraryItem, Progress)> cw) =>
          orderEpisodes(
            eps,
            progress: {
              for (final (i, p) in cw)
                if (i.showId == showId) i.id: p,
            },
          ),
    );
  }
}

class _Show {
  final count = ValueNotifier<int?>(null);
  int holders = 0;
  StreamSubscription<ShowEpisodes>? sub;
}

/// Builds with the unwatched episode count of [showId] (null until known).
class ShowUnwatchedBuilder extends StatefulWidget {
  const ShowUnwatchedBuilder({
    super.key,
    required this.library,
    required this.showId,
    required this.builder,
  });

  final Library library;
  final String showId;
  final Widget Function(BuildContext context, int? count) builder;

  @override
  State<ShowUnwatchedBuilder> createState() => _ShowUnwatchedBuilderState();
}

class _ShowUnwatchedBuilderState extends State<ShowUnwatchedBuilder> {
  late ShowUnwatchedCounts _counts;
  late ValueListenable<int?> _count;

  @override
  void initState() {
    super.initState();
    _counts = ShowUnwatchedCounts.of(widget.library);
    _count = _counts.hold(widget.showId);
  }

  @override
  void didUpdateWidget(ShowUnwatchedBuilder old) {
    super.didUpdateWidget(old);
    if (old.showId == widget.showId && old.library == widget.library) return;
    _counts.release(old.showId);
    _counts = ShowUnwatchedCounts.of(widget.library);
    _count = _counts.hold(widget.showId);
  }

  @override
  void dispose() {
    _counts.release(widget.showId);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<int?>(
    valueListenable: _count,
    builder: (context, n, _) => widget.builder(context, n),
  );
}
