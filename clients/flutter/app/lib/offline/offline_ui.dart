// SPDX-License-Identifier: MPL-2.0
//
// "Download for offline" in the item pages and menus (NET-019): the
// button with the state of the item's copy, the version picker, the
// season / album / playlist actions and the delete confirmation.

import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import 'offline_downloads.dart';

/// `700 MB`, `4.2 GB`: binary units.
String formatOfflineSize(AppLocalizations l, Locale locale, int bytes) {
  final tag = locale.toLanguageTag();
  const gib = 1 << 30;
  if (bytes >= gib) {
    return l.detailSizeGb(NumberFormat('0.#', tag).format(bytes / gib));
  }
  return l.detailSizeMb(NumberFormat('0', tag).format(bytes / (1 << 20)));
}

/// `45 %` in the locale's format.
String formatOfflinePercent(Locale locale, double fraction) =>
    NumberFormat.percentPattern(locale.toLanguageTag()).format(fraction);

/// One line about the state of a download; [wifiOnly] tells "waits for
/// Wi-Fi" from "no connection".
String offlineStateText(
  AppLocalizations l,
  OfflineDownload d, {
  bool wifiOnly = false,
}) => switch (d.state) {
  OfflineState.queued => l.offlineStateQueued,
  OfflineState.waiting || OfflineState.paused => switch (d.waitingFor) {
    'network' when wifiOnly => l.offlineStateWaitingWifi,
    'network' => l.offlineStateWaitingNetwork,
    'disk_reserve' ||
    'quota' ||
    'too_large' ||
    'resources' => l.offlineStateWaitingSpace,
    _ when d.state == OfflineState.paused => l.offlineStatePaused,
    _ => l.offlineStateWaiting,
  },
  OfflineState.running => l.offlineStateRunning,
  OfflineState.retrying => l.offlineStateRetrying,
  OfflineState.failed => l.offlineStateFailed,
  OfflineState.cancelled => l.offlineStateCancelled,
  OfflineState.done => d.watched ? l.offlineStateWatched : l.offlineStateDone,
};

(IconData, String) offlineActionLook(AppLocalizations l, OfflineAction a) =>
    switch (a) {
      OfflineAction.pause => (Icons.pause_rounded, l.offlineActionPause),
      OfflineAction.resume => (Icons.play_arrow_rounded, l.offlineActionResume),
      OfflineAction.cancel => (Icons.close_rounded, l.offlineActionCancel),
      OfflineAction.retry => (Icons.refresh_rounded, l.offlineActionRetry),
      OfflineAction.remove => (Icons.clear_all_rounded, l.offlineActionRemove),
      OfflineAction.delete => (
        Icons.delete_outline_rounded,
        l.offlineActionDelete,
      ),
    };

/// The item is on a network source: a copy makes sense.
bool offlineCapable(LibraryItem item) => item.streamUri != null;

String _errorText(AppLocalizations l, OfflineException e) => switch (e.code) {
  'no_space' => l.offlineErrorNoSpace,
  'local' => l.offlineErrorLocal,
  'unplayable' => l.offlineErrorUnplayable,
  'busy' => l.offlineErrorBusy,
  _ => l.offlineErrorGeneric,
};

void _say(BuildContext context, String text) {
  ScaffoldMessenger.maybeOf(context)
    ?..removeCurrentSnackBar()
    ..showSnackBar(SnackBar(content: Text(text)));
}

/// Runs a download action and tells the result in a snackbar.
Future<void> runOfflineDownload(
  BuildContext context,
  Future<int> Function(OfflineController offline) start,
) async {
  final offline = Services.read(context).offline;
  if (offline == null) return;
  final l = context.l10n;
  try {
    final n = await start(offline);
    if (context.mounted) _say(context, l.offlineQueued(n));
  } on OfflineException catch (e) {
    if (context.mounted) _say(context, _errorText(l, e));
  } catch (e) {
    debugPrint('Download not started: $e');
    if (context.mounted) _say(context, l.offlineErrorGeneric);
  }
}

/// Downloads [item] (a movie, an episode, a track, or every episode of a
/// show); a movie with several versions asks which one.
Future<void> downloadItem(
  BuildContext context,
  LibraryItem item, {
  List<MediaVersion> versions = const [],
}) async {
  int? version;
  final playable = versions.where((v) => v.unplayable == null).toList();
  if (playable.length > 1) {
    final chosen = await showDialog<MediaVersion>(
      context: context,
      builder: (context) => SimpleDialog(
        key: const ValueKey('offline-version-dialog'),
        title: Text(context.l10n.offlineChooseVersion),
        children: [
          for (final v in playable)
            SimpleDialogOption(
              key: ValueKey('offline-version-${v.id}'),
              onPressed: () => Navigator.pop(context, v),
              child: Text(
                [
                  v.label,
                  formatOfflineSize(
                    context.l10n,
                    Localizations.localeOf(context),
                    v.sizeBytes,
                  ),
                ].join(' · '),
              ),
            ),
        ],
      ),
    );
    if (chosen == null) return;
    version = int.tryParse(chosen.id);
  }
  if (!context.mounted) return;
  await runOfflineDownload(
    context,
    (o) => o.download(OfflineTarget.item(item.id, versionFileId: version)),
  );
}

/// Asks, then deletes the copy (and downloads) of an item.
Future<void> confirmDeleteDownload(
  BuildContext context,
  String itemId,
  String title,
) async {
  final l = context.l10n;
  final ok = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      key: const ValueKey('offline-delete-dialog'),
      title: Text(l.offlineDeleteTitle),
      content: Text(l.offlineDeleteBody(title)),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: Text(l.offlineCancelDialog),
        ),
        TextButton(
          key: const ValueKey('offline-delete-confirm'),
          onPressed: () => Navigator.pop(context, true),
          child: Text(l.offlineDeleteConfirm),
        ),
      ],
    ),
  );
  if (ok != true || !context.mounted) return;
  await Services.read(context).offline?.deleteItem(itemId);
}

/// The "Download" button of a movie or an episode page: its label follows
/// the copy (queued, 45 %, downloaded). Hidden for local files and without
/// the core.
class OfflineButton extends StatelessWidget {
  const OfflineButton({
    super.key,
    required this.item,
    this.versions = const [],
  });

  final LibraryItem item;
  final List<MediaVersion> versions;

  @override
  Widget build(BuildContext context) {
    final offline = Services.of(context).offline;
    if (offline == null || !offlineCapable(item)) {
      return const SizedBox.shrink();
    }
    return ListenableBuilder(
      listenable: offline,
      builder: (context, _) {
        final l = context.l10n;
        final locale = Localizations.localeOf(context);
        final d = offline.stateOf(item.id);
        final (IconData icon, String label) = switch (d?.state) {
          null ||
          OfflineState.cancelled ||
          OfflineState.failed => (Icons.download_outlined, l.offlineDownload),
          OfflineState.done => (
            Icons.download_done_rounded,
            l.offlineDownloaded,
          ),
          OfflineState.running => (
            Icons.downloading_rounded,
            l.offlineDownloading(
              formatOfflinePercent(locale, d!.progress ?? 0),
            ),
          ),
          _ => (
            Icons.downloading_rounded,
            offlineStateText(l, d!, wifiOnly: offline.settings.wifiOnly),
          ),
        };
        return KButton(
          key: const ValueKey('offline-button'),
          icon: icon,
          label: label,
          onPressed: () => switch (d?.state) {
            null || OfflineState.cancelled => downloadItem(
              context,
              item,
              versions: versions,
            ),
            OfflineState.failed => offline.control(d!, OfflineAction.retry),
            OfflineState.done => confirmDeleteDownload(
              context,
              item.id,
              item.title,
            ),
            _ => _activeMenu(context, offline, d!),
          },
        );
      },
    );
  }

  Future<void> _activeMenu(
    BuildContext context,
    OfflineController offline,
    OfflineDownload d,
  ) async {
    final l = context.l10n;
    final action = await showModalBottomSheet<OfflineAction>(
      context: context,
      builder: (context) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final a in d.actions)
              ListTile(
                key: ValueKey('offline-sheet-${a.name}'),
                leading: Icon(offlineActionLook(l, a).$1),
                title: Text(offlineActionLook(l, a).$2),
                onTap: () => Navigator.pop(context, a),
              ),
          ],
        ),
      ),
    );
    if (action != null) await offline.control(d, action);
  }
}

/// A small mark on an episode or track: downloaded, or downloading with
/// its progress. Nothing for items without a copy.
class OfflineBadge extends StatelessWidget {
  const OfflineBadge({super.key, required this.itemId, this.size = 18});

  final String itemId;
  final double size;

  @override
  Widget build(BuildContext context) {
    final offline = Services.of(context).offline;
    if (offline == null) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: offline,
      builder: (context, _) {
        final d = offline.stateOf(itemId);
        if (d == null) return const SizedBox.shrink();
        if (d.state == OfflineState.done) {
          return Icon(
            Icons.download_done_rounded,
            key: ValueKey('offline-badge-$itemId'),
            size: size,
            color: KColors.accent,
            semanticLabel: context.l10n.offlineDownloaded,
          );
        }
        if (!d.state.isActive) return const SizedBox.shrink();
        return SizedBox(
          key: ValueKey('offline-badge-$itemId'),
          width: size,
          height: size,
          child: CircularProgressIndicator(
            strokeWidth: 2,
            value: d.progress,
            color: KColors.accent,
          ),
        );
      },
    );
  }
}

/// "Download season / album / playlist": queues every network item of
/// [items] in their order. Null (nothing to show) without the core or when
/// every item is local.
Widget? offlineListButton(
  BuildContext context, {
  required Key key,
  required String label,
  required List<LibraryItem> items,
}) {
  final offline = Services.of(context).offline;
  final network = [
    for (final i in items)
      if (offlineCapable(i)) i.id,
  ];
  if (offline == null || network.isEmpty) return null;
  return ListenableBuilder(
    key: key,
    listenable: offline,
    builder: (context, _) {
      final done = network.every(
        (id) => offline.stateOf(id)?.state == OfflineState.done,
      );
      final busy = network.any(
        (id) => offline.stateOf(id)?.state.isActive ?? false,
      );
      return KButton(
        icon: done
            ? Icons.download_done_rounded
            : (busy ? Icons.downloading_rounded : Icons.download_outlined),
        label: done ? context.l10n.offlineDownloaded : label,
        onPressed: done
            ? null
            : () =>
                  runOfflineDownload(context, (o) => o.downloadItems(network)),
      );
    },
  );
}
