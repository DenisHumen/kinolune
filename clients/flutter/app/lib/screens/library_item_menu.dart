// SPDX-License-Identifier: MPL-2.0
//
// Context menu of a movie or show in the library grid (UI-008): long press,
// right click or the menu key. Entries without a library API yet are shown
// disabled with a "soon" mark. "Download" makes an offline copy of a
// network item (NET-019). Favorites and playlists (LIB-010) need a
// library with [LibraryLists].

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../library/library_queries.dart';
import '../offline/offline_downloads.dart';
import '../offline/offline_ui.dart';
import '../personal/personal_screen.dart' show moveItemToPersonal;
import '../services/app_services.dart';
import '../sources/jellyfin_link.dart';
import '../theme/kinolune_theme.dart';
import 'fix_match_dialog.dart';
import 'library_lists_ui.dart';
import 'playback.dart';

/// Entries of [showLibraryItemMenu].
enum LibraryItemAction {
  play,
  markWatched,
  markUnwatched,
  favorite,
  playlist,
  editMetadata,
  download,
  personal,
  delete,
}

/// Actions that have no library API yet (shown disabled, "soon").
const _soonAlways = {
  LibraryItemAction.markUnwatched, // no "unwatch" in the core yet
  LibraryItemAction.delete,
};

/// Actions that need [LibraryLists] (LIB-010).
const _needLists = {LibraryItemAction.favorite, LibraryItemAction.playlist};

/// Shows the menu of [item] at [position] (global) and runs the chosen
/// action.
Future<void> showLibraryItemMenu(
  BuildContext context,
  LibraryItem item, {
  required Offset position,
  ItemBadge? badge,
}) async {
  final l = context.l10n;
  final watched = badge?.state == WatchState.watched;
  final lists = libraryListsOf(context);
  final overlay = Overlay.of(context).context.findRenderObject()! as RenderBox;
  final local = overlay.globalToLocal(position);
  final favorite = await _isFavorite(lists, item);
  if (!context.mounted) return;
  // NET-019: a copy of a network item (a show: every episode) when the
  // core runs downloads.
  final offline = Services.read(context).offline;
  final downloaded = offline?.stateOf(item.id)?.state == OfflineState.done;
  final soon = {
    ..._soonAlways,
    if (lists == null) ..._needLists,
    if (offline == null || !offlineCapable(item)) LibraryItemAction.download,
  };
  // UI-025: movies and shows move to personal media where the library
  // keeps it; elsewhere the entry is not shown.
  final personal =
      Services.read(context).personal != null && item.kind != ItemKind.track;
  final actions = [
    LibraryItemAction.play,
    watched ? LibraryItemAction.markUnwatched : LibraryItemAction.markWatched,
    LibraryItemAction.favorite,
    LibraryItemAction.playlist,
    LibraryItemAction.editMetadata,
    LibraryItemAction.download,
    if (personal) LibraryItemAction.personal,
    LibraryItemAction.delete,
  ];
  final chosen = await showMenu<LibraryItemAction>(
    context: context,
    position: RelativeRect.fromRect(
      local & const Size(1, 1),
      Offset.zero & overlay.size,
    ),
    items: [
      for (final a in actions)
        PopupMenuItem(
          key: ValueKey('item-menu-${a.name}'),
          value: a,
          enabled: !soon.contains(a),
          child: _MenuRow(
            icon: _icon(a, favorite: favorite, downloaded: downloaded),
            label: _label(l, a, favorite: favorite, downloaded: downloaded),
            soon: soon.contains(a),
          ),
        ),
    ],
  );
  if (chosen == null || !context.mounted) return;
  switch (chosen) {
    case LibraryItemAction.play:
      await playLibraryItem(context, item);
    case LibraryItemAction.markWatched:
      final services = Services.read(context);
      await markWatched(services.library, item, server: services.jellyfin);
    case LibraryItemAction.favorite when lists != null:
      await setFavorite(context, lists, item, !favorite);
    case LibraryItemAction.playlist when lists != null:
      await showAddToPlaylistDialog(context, lists, item);
    case LibraryItemAction.editMetadata:
      await showFixMatchDialog(context, item);
    case LibraryItemAction.download when downloaded:
      await confirmDeleteDownload(context, item.id, item.title);
    case LibraryItemAction.download:
      await downloadItem(context, item);
    case LibraryItemAction.personal:
      await moveItemToPersonal(context, item);
    case LibraryItemAction.markUnwatched ||
        LibraryItemAction.favorite ||
        LibraryItemAction.playlist ||
        LibraryItemAction.delete:
      break;
  }
}

Future<bool> _isFavorite(LibraryLists? lists, LibraryItem item) async {
  if (lists == null) return false;
  try {
    return await lists.isFavorite(item.id);
  } catch (_) {
    return false;
  }
}

IconData _icon(
  LibraryItemAction a, {
  bool favorite = false,
  bool downloaded = false,
}) => switch (a) {
  LibraryItemAction.play => Icons.play_arrow_rounded,
  LibraryItemAction.markWatched => Icons.check_circle_outline,
  LibraryItemAction.markUnwatched => Icons.remove_done,
  LibraryItemAction.favorite =>
    favorite ? Icons.favorite : Icons.favorite_border,
  LibraryItemAction.playlist => Icons.playlist_add,
  LibraryItemAction.editMetadata => Icons.edit_outlined,
  LibraryItemAction.download =>
    downloaded ? Icons.download_done_rounded : Icons.download_outlined,
  LibraryItemAction.personal => Icons.folder_special_outlined,
  LibraryItemAction.delete => Icons.delete_outline,
};

String _label(
  AppLocalizations l,
  LibraryItemAction a, {
  bool favorite = false,
  bool downloaded = false,
}) => switch (a) {
  LibraryItemAction.play => l.commonPlay,
  LibraryItemAction.markWatched => l.libraryMenuMarkWatched,
  LibraryItemAction.markUnwatched => l.libraryMenuMarkUnwatched,
  LibraryItemAction.favorite =>
    favorite ? l.libraryMenuUnfavorite : l.libraryMenuFavorite,
  LibraryItemAction.playlist => l.libraryMenuPlaylist,
  LibraryItemAction.editMetadata => l.libraryMenuEditMetadata,
  LibraryItemAction.download =>
    downloaded ? l.offlineActionDelete : l.libraryMenuDownload,
  LibraryItemAction.personal => l.personalMoveTo,
  LibraryItemAction.delete => l.libraryMenuDelete,
};

class _MenuRow extends StatelessWidget {
  const _MenuRow({required this.icon, required this.label, required this.soon});

  final IconData icon;
  final String label;
  final bool soon;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final color = soon ? KColors.textTertiary : null;
    return Row(
      children: [
        Icon(icon, size: 20, color: color),
        const SizedBox(width: 12),
        Flexible(
          child: Text(
            label,
            overflow: TextOverflow.ellipsis,
            style: theme.textTheme.bodyMedium?.copyWith(color: color),
          ),
        ),
        if (soon) ...[
          const SizedBox(width: 16),
          Text(
            context.l10n.libraryMenuSoon,
            style: theme.textTheme.labelSmall?.copyWith(
              color: KColors.textTertiary,
            ),
          ),
        ],
      ],
    );
  }
}

/// Plays a movie from its saved position; a show from its episode in
/// progress, else its first unwatched episode, else the first one.
Future<void> playLibraryItem(BuildContext context, LibraryItem item) async {
  final library = Services.read(context).library;
  if (item.kind != ItemKind.show) {
    final start = resumePositionMs(await library.progress(item.id));
    if (context.mounted) {
      await playVideo(context, item, startPositionMs: start);
    }
    return;
  }
  final episodes = await library.episodes(item.id).first;
  if (episodes.isEmpty) return;
  LibraryItem? next;
  var start = 0;
  for (final (i, p) in await library.continueWatching().first) {
    if (i.showId == item.id) {
      next = i;
      start = resumePositionMs(p);
      break;
    }
  }
  if (next == null) {
    for (final e in episodes) {
      if ((await library.progress(e.id))?.watched != true) {
        next = e;
        break;
      }
    }
  }
  next ??= episodes.first;
  if (context.mounted) {
    await playVideo(context, next, startPositionMs: start);
  }
}

/// Marks a movie (or every episode of a show) as watched.
Future<void> markWatched(
  Library library,
  LibraryItem item, {
  JellyfinLink? server,
}) async {
  final targets = item.kind == ItemKind.show
      ? await library.episodes(item.id).first
      : [item];
  final now = DateTime.now();
  for (final t in targets) {
    final d = t.durationMs ?? 0;
    await library.saveProgress(
      Progress(
        itemId: t.id,
        positionMs: d,
        durationMs: d,
        updatedAt: now,
        watched: true,
      ),
    );
    // NET-014: a Jellyfin item is marked at the server too.
    try {
      await server?.setWatched(t.id, true);
    } catch (e) {
      debugPrint('Jellyfin watched mark of ${t.id} not sent: $e');
    }
  }
}
