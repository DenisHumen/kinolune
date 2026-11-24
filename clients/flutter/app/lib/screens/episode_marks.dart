// SPDX-License-Identifier: MPL-2.0
//
// Watched marks on the show page (UI-049): the menu of an episode (long
// press, right click, a held OK on the remote, the menu key or its «⋯»
// button) with «Просмотрено» / «Не просмотрено» and «Отметить все
// предыдущие как просмотренные»; the menu of a season tab and the «Ещё»
// menu of the show mark a whole season or the whole show. The core marks
// them in one transaction (LibraryWatchMarks.setWatchedMany), sync and the
// Jellyfin, Emby or Plex server get every change, and after a bulk mark
// the snack bar offers «Отменить» (UI-035), which puts back the marks and
// positions the episodes had. The page, the stills (UI-037), «Далее» and
// Up Next (LIB-011) follow the library's progress stream at once.

import 'dart:async';

import 'package:material_ui/material_ui.dart';
import 'package:pa_library/pa_library.dart' show WatchMarkState, watchMarksOf;

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../sources/jellyfin_link.dart';
import '../widgets/undo_snack.dart';

/// Key of the «⋯» button of an episode card.
ValueKey<String> showEpisodeMenuKey(String id) =>
    ValueKey('show-episode-menu-$id');

/// Key of the «Ещё» button of the show.
const showMoreKey = ValueKey('show-more');

/// Entries of the watched-mark menus.
enum EpisodeMarkAction {
  /// This episode watched.
  watched,

  /// This episode not watched.
  unwatched,

  /// Every episode before this one watched.
  previous,

  /// The season of the tab watched / not watched.
  seasonWatched,
  seasonUnwatched,

  /// The whole show watched / not watched.
  showWatched,
  showUnwatched,
}

/// The library can mark episodes here (not the v0 library).
bool canMarkEpisodes(BuildContext context) =>
    watchMarksOf(Services.read(context).library) != null;

/// The episodes before [episode] in [all] (the page's order): regular
/// episodes for a regular one, specials for a special.
List<ShowEpisode> episodesBefore(ShowEpisode episode, List<ShowEpisode> all) {
  final special = (episode.item.season ?? 1) < 1;
  final out = <ShowEpisode>[];
  for (final e in all) {
    if (e.item.id == episode.item.id) return out;
    if (((e.item.season ?? 1) < 1) == special) out.add(e);
  }
  return const [];
}

/// Marks [items] watched or not, all at once. With [undo] (a bulk mark)
/// the snack bar offers «Отменить» for a few seconds. Completes when the
/// snack bar is gone; returns how many episodes changed.
Future<int> setEpisodesWatched(
  BuildContext context,
  List<LibraryItem> items, {
  required bool watched,
  bool undo = true,
}) async {
  final services = Services.read(context);
  final marks = watchMarksOf(services.library);
  if (marks == null || items.isEmpty) return 0;
  final l = context.l10n;
  final messenger = ScaffoldMessenger.maybeOf(context);
  final List<WatchMarkState> before;
  try {
    before = await marks.setWatchedMany([
      for (final i in items) i.id,
    ], watched: watched);
  } catch (e) {
    debugPrint('Watched marks of ${items.length} items not saved: $e');
    return 0;
  }
  final server = services.jellyfin;
  _toServer(server, [for (final s in before) (s.itemId, watched)]);
  if (!undo || before.isEmpty) return before.length;
  final undone = await showUndoSnack(
    messenger,
    message: watched
        ? l.showMarkedWatched(before.length)
        : l.showMarkedUnwatched(before.length),
    undoLabel: l.commonUndo,
  );
  if (!undone) return before.length;
  try {
    await marks.restoreWatchMarks(before);
  } catch (e) {
    debugPrint('Watched marks of ${before.length} items not restored: $e');
    return before.length;
  }
  _toServer(server, [for (final s in before) (s.itemId, s.watched)]);
  return before.length;
}

/// NET-014/NET-012: the marks of server items go to Jellyfin, Emby or
/// Plex too, one call per item (their APIs have no bulk call); local
/// items are skipped by the link. Not awaited: the page does not wait for
/// the network.
void _toServer(JellyfinLink? server, List<(String, bool)> marks) {
  if (server == null || marks.isEmpty) return;
  unawaited(() async {
    for (final (id, watched) in marks) {
      try {
        await server.setWatched(id, watched);
      } catch (e) {
        debugPrint('Server watched mark of $id not sent: $e');
      }
    }
  }());
}

/// The menu of [episode] at [position] (global); [all] are the episodes
/// of the show in the page's order.
Future<void> showEpisodeMarkMenu(
  BuildContext context,
  ShowEpisode episode,
  List<ShowEpisode> all, {
  required Offset position,
}) async {
  if (!canMarkEpisodes(context)) return;
  final before = episodesBefore(episode, all);
  final previous = [
    for (final e in before)
      if (!e.watched) e.item,
  ];
  final started = (episode.progress?.positionMs ?? 0) > 0;
  final chosen = await _showMarkMenu(context, position, [
    if (!episode.watched) EpisodeMarkAction.watched,
    // A started episode can be cleared too.
    if (episode.watched || started) EpisodeMarkAction.unwatched,
    if (previous.isNotEmpty) EpisodeMarkAction.previous,
  ]);
  if (chosen == null || !context.mounted) return;
  switch (chosen) {
    case EpisodeMarkAction.watched || EpisodeMarkAction.unwatched:
      await setEpisodesWatched(
        context,
        [episode.item],
        watched: chosen == EpisodeMarkAction.watched,
        undo: false,
      );
    case EpisodeMarkAction.previous:
      await setEpisodesWatched(context, previous, watched: true);
    case _:
      break;
  }
}

/// The menu of a season tab: the [season] episodes watched or not.
Future<void> showSeasonMarkMenu(
  BuildContext context,
  List<ShowEpisode> season, {
  required Offset position,
}) async {
  if (!canMarkEpisodes(context) || season.isEmpty) return;
  final chosen = await _showMarkMenu(context, position, [
    if (season.any((e) => !e.watched)) EpisodeMarkAction.seasonWatched,
    if (season.any((e) => !e.unseen)) EpisodeMarkAction.seasonUnwatched,
  ]);
  if (chosen == null || !context.mounted) return;
  await setEpisodesWatched(context, [
    for (final e in season) e.item,
  ], watched: chosen == EpisodeMarkAction.seasonWatched);
}

/// The «Ещё» menu of the show: the season of the tab ([season]) and the
/// whole show ([all]) watched or not.
Future<void> showShowMarkMenu(
  BuildContext context,
  List<ShowEpisode> season,
  List<ShowEpisode> all, {
  required Offset position,
}) async {
  if (!canMarkEpisodes(context) || all.isEmpty) return;
  final chosen = await _showMarkMenu(context, position, [
    if (season.any((e) => !e.watched)) EpisodeMarkAction.seasonWatched,
    if (season.any((e) => !e.unseen)) EpisodeMarkAction.seasonUnwatched,
    if (all.any((e) => !e.watched)) EpisodeMarkAction.showWatched,
    if (all.any((e) => !e.unseen)) EpisodeMarkAction.showUnwatched,
  ]);
  if (chosen == null || !context.mounted) return;
  final (episodes, watched) = switch (chosen) {
    EpisodeMarkAction.seasonWatched => (season, true),
    EpisodeMarkAction.seasonUnwatched => (season, false),
    EpisodeMarkAction.showWatched => (all, true),
    _ => (all, false),
  };
  await setEpisodesWatched(context, [
    for (final e in episodes) e.item,
  ], watched: watched);
}

/// The centre of [context]'s box, global: where a menu opened by a button
/// appears.
Offset centreOf(BuildContext context) {
  final box = context.findRenderObject() as RenderBox?;
  if (box == null || !box.hasSize) return Offset.zero;
  return box.localToGlobal(box.size.center(Offset.zero));
}

Future<EpisodeMarkAction?> _showMarkMenu(
  BuildContext context,
  Offset position,
  List<EpisodeMarkAction> actions,
) {
  if (actions.isEmpty) return Future.value();
  final l = context.l10n;
  final overlay = Overlay.of(context).context.findRenderObject()! as RenderBox;
  final local = overlay.globalToLocal(position);
  return showMenu<EpisodeMarkAction>(
    context: context,
    position: RelativeRect.fromRect(
      local & const Size(1, 1),
      Offset.zero & overlay.size,
    ),
    items: [
      for (final a in actions)
        PopupMenuItem(
          key: ValueKey('mark-menu-${a.name}'),
          value: a,
          child: Row(
            children: [
              Icon(_icon(a), size: 20),
              const SizedBox(width: 12),
              Flexible(
                child: Text(_label(l, a), overflow: TextOverflow.ellipsis),
              ),
            ],
          ),
        ),
    ],
  );
}

IconData _icon(EpisodeMarkAction a) => switch (a) {
  EpisodeMarkAction.watched => Icons.check_circle_outline,
  EpisodeMarkAction.unwatched => Icons.remove_done,
  EpisodeMarkAction.previous => Icons.done_all,
  EpisodeMarkAction.seasonWatched ||
  EpisodeMarkAction.showWatched => Icons.done_all,
  EpisodeMarkAction.seasonUnwatched ||
  EpisodeMarkAction.showUnwatched => Icons.remove_done,
};

String _label(AppLocalizations l, EpisodeMarkAction a) => switch (a) {
  EpisodeMarkAction.watched => l.showMarkWatched,
  EpisodeMarkAction.unwatched => l.showMarkUnwatched,
  EpisodeMarkAction.previous => l.showMarkPreviousWatched,
  EpisodeMarkAction.seasonWatched => l.showMarkSeasonWatched,
  EpisodeMarkAction.seasonUnwatched => l.showMarkSeasonUnwatched,
  EpisodeMarkAction.showWatched => l.showMarkShowWatched,
  EpisodeMarkAction.showUnwatched => l.showMarkShowUnwatched,
};
