// SPDX-License-Identifier: MPL-2.0
//
// Downloads (NET-019, UI-018): the queue of offline copies with live
// progress from the core's event bus (CORE-009), pause / resume / cancel /
// retry for one or all; the copies on the device; the space of the
// download folder in one bar; auto-cleanup («delete watched», a
// pa-storage retention rule, on by default) and «Delete watched» now; the
// folder and Wi-Fi only. One column on a phone, the list and a side panel
// on a desktop, tablet and TV (remote focus on every action).
//
// LIB-027: files going up to the library on the server or NAS (dropped,
// shared or added here with «Добавить в медиатеку») are listed first, with
// their progress.

import 'dart:async';
import 'dart:io' show Platform;

import 'package:file_picker/file_picker.dart';
import 'package:material_ui/material_ui.dart';

import '../files/library_upload.dart';
import '../l10n/l10n.dart';
import '../offline/offline_downloads.dart';
import '../offline/offline_ui.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/cards.dart';
import '../widgets/focusable.dart';
import '../widgets/settings_tiles.dart';
import '../widgets/undo_snack.dart';

/// Choices of "delete watched": never, right away, after N days.
const offlineDeleteWatchedOptions = <int?>[null, 0, 1, 7, 30];

String offlineDeleteWatchedLabel(AppLocalizations l, int? days) =>
    switch (days) {
      null => l.offlineDeleteWatchedNever,
      0 => l.offlineDeleteWatchedNow,
      final n => l.offlineDeleteWatchedDays(n),
    };

/// From this width the screen has two columns: the list and a side panel
/// with the space and the settings (desktop, tablet, TV).
const downloadsWideFrom = 900.0;

class DownloadsScreen extends StatefulWidget {
  const DownloadsScreen({super.key});

  @override
  State<DownloadsScreen> createState() => _DownloadsScreenState();
}

class _DownloadsScreenState extends State<DownloadsScreen> {
  /// Entries removed from the list while «Отменить» is offered (QA-015).
  final _removing = <String>{};

  /// «Убрать из списка»: the entry goes at once; the core forgets it when
  /// the snack bar closes without «Отменить».
  Future<void> _remove(OfflineController offline, OfflineDownload d) async {
    final l = context.l10n;
    setState(() => _removing.add(d.itemId));
    final undone = await showUndoSnack(
      ScaffoldMessenger.maybeOf(context),
      message: l.offlineRemoved,
      undoLabel: l.commonUndo,
    );
    if (!undone) await offline.control(d, OfflineAction.remove);
    if (mounted) setState(() => _removing.remove(d.itemId));
  }

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final offline = Services.read(context).offline;
      if (offline != null) unawaited(offline.cleanup());
    });
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final services = Services.of(context);
    final offline = services.offline;
    final files = services.filesControl;
    return Scaffold(
      key: const ValueKey('downloads-page'),
      backgroundColor: KColors.bgBase,
      appBar: AppBar(
        backgroundColor: KColors.bgBase,
        title: Text(
          l.offlineScreenTitle,
          style: Theme.of(context).textTheme.titleLarge,
        ),
        actions: [
          // No file pickers on a TV: files come from a phone or a computer.
          if (files != null && !d.isTv) const AddToLibraryButton(),
        ],
      ),
      body: offline == null
          ? ListView(
              children: [
                if (files != null) LibraryUploadsSection(files: files),
                Padding(
                  padding: EdgeInsets.all(d.pageInsets.left),
                  child: Text(l.offlineUnavailable),
                ),
              ],
            )
          : ListenableBuilder(
              listenable: Listenable.merge([offline, ?files]),
              builder: (context, _) => LayoutBuilder(
                builder: (context, c) => c.maxWidth >= downloadsWideFrom
                    ? _wide(context, offline)
                    : _narrow(context, offline),
              ),
            ),
    );
  }

  List<OfflineDownload> _shown(List<OfflineDownload> list) => [
    for (final i in list)
      if (!_removing.contains(i.itemId)) i,
  ];

  /// Phone and narrow windows: one column — space, queue, copies,
  /// settings.
  Widget _narrow(BuildContext context, OfflineController offline) {
    final d = KDensity.of(context);
    final side = EdgeInsets.symmetric(horizontal: d.pageInsets.left);
    return FocusTraversalGroup(
      child: ListView(
        key: const ValueKey('downloads-narrow'),
        padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 24),
        children: [
          Padding(
            padding: side.copyWith(top: 4),
            child: DownloadsSpaceCard(offline: offline),
          ),
          ..._lists(context, offline),
          const SizedBox(height: 16),
          Padding(
            padding: side,
            child: OfflineSettingsSection(offline: offline),
          ),
        ],
      ),
    );
  }

  /// Desktop, tablet and TV: the list on the left, space and settings on
  /// the right.
  Widget _wide(BuildContext context, OfflineController offline) {
    final d = KDensity.of(context);
    return Row(
      key: const ValueKey('downloads-wide'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Expanded(
          child: FocusTraversalGroup(
            child: ListView(
              padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 24),
              children: _lists(context, offline),
            ),
          ),
        ),
        SizedBox(
          width: d.isTv ? 400 : 380,
          child: FocusTraversalGroup(
            child: ListView(
              key: const ValueKey('downloads-side'),
              padding: EdgeInsets.fromLTRB(
                0,
                4,
                d.pageInsets.right,
                d.pageInsets.bottom + 24,
              ),
              children: [
                DownloadsSpaceCard(offline: offline),
                const SizedBox(height: 16),
                OfflineSettingsSection(offline: offline),
              ],
            ),
          ),
        ),
      ],
    );
  }

  /// The queue and the copies with their headers, or the empty state.
  List<Widget> _lists(BuildContext context, OfflineController offline) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final locale = Localizations.localeOf(context);
    final queue = _shown(offline.queue);
    final copies = _shown(offline.copies);
    final files = Services.of(context).filesControl;
    final uploads = [
      if (files != null && files.libraryUploads.isNotEmpty)
        LibraryUploadsSection(files: files),
    ];
    if (queue.isEmpty && copies.isEmpty) {
      if (uploads.isNotEmpty) return uploads;
      return [
        Padding(
          key: const ValueKey('downloads-empty'),
          padding: EdgeInsets.symmetric(
            horizontal: d.pageInsets.left,
            vertical: 24,
          ),
          child: EmptyState(
            icon: Icons.download_for_offline_outlined,
            title: l.downloadsEmptyTitle,
            message: l.offlineEmpty,
          ),
        ),
      ];
    }
    // On a TV the remote's focus starts on the first control of the list.
    var first = d.isTv;
    bool takeFocus() {
      final f = first;
      first = false;
      return f;
    }

    Widget? bulk;
    if (offline.canPauseAll) {
      bulk = KButton(
        key: const ValueKey('downloads-pause-all'),
        icon: Icons.pause_rounded,
        label: l.downloadsPauseAll,
        autofocus: takeFocus(),
        onPressed: () => unawaited(offline.pauseAll()),
      );
    } else if (offline.canResumeAll) {
      bulk = KButton(
        key: const ValueKey('downloads-resume-all'),
        icon: Icons.play_arrow_rounded,
        label: l.downloadsResumeAll,
        autofocus: takeFocus(),
        onPressed: () => unawaited(offline.resumeAll()),
      );
    }
    void act(OfflineDownload item, OfflineAction a) {
      switch (a) {
        case OfflineAction.delete:
          unawaited(confirmDeleteDownload(context, item.itemId, item.title));
        case OfflineAction.remove:
          unawaited(_remove(offline, item));
        default:
          unawaited(offline.control(item, a));
      }
    }

    Widget tile(OfflineDownload item) => _DownloadTile(
      item: item,
      wifiOnly: offline.settings.wifiOnly,
      autofocus: takeFocus(),
      onAction: (a) => act(item, a),
    );

    return [
      ...uploads,
      if (queue.isNotEmpty) ...[
        _SectionHeader(
          key: const ValueKey('downloads-queue-header'),
          title: l.downloadsQueueTitle,
          count: '${queue.length}',
          trailing: bulk,
        ),
        for (final item in queue) tile(item),
      ],
      if (copies.isNotEmpty) ...[
        _SectionHeader(
          key: const ValueKey('downloads-copies-header'),
          title: l.downloadsCopiesTitle,
          count: l.downloadsCopiesSummary(
            copies.length,
            formatOfflineSize(l, locale, OfflineController.bytesOf(copies)),
          ),
        ),
        for (final item in copies) tile(item),
      ],
    ];
  }
}

class _SectionHeader extends StatelessWidget {
  const _SectionHeader({
    super.key,
    required this.title,
    required this.count,
    this.trailing,
  });

  final String title;
  final String count;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    return Padding(
      padding: EdgeInsets.fromLTRB(
        d.pageInsets.left,
        16,
        d.pageInsets.right,
        6,
      ),
      child: Wrap(
        spacing: 12,
        runSpacing: 8,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          Semantics(
            header: true,
            child: Text(title, style: theme.textTheme.titleMedium),
          ),
          Text(
            count,
            style: theme.textTheme.bodySmall?.copyWith(
              color: KColors.textSecondary,
            ),
          ),
          ?trailing,
        ],
      ),
    );
  }
}

/// Space of the download folder: downloads, other files and free space in
/// one bar, the auto-cleanup rule and «Delete watched» (UI-018).
class DownloadsSpaceCard extends StatelessWidget {
  const DownloadsSpaceCard({super.key, required this.offline});

  final OfflineController offline;

  /// Little space: less than 2 GiB or 5 % of the volume is free.
  static bool lowSpace(int free, int total) =>
      free < (2 << 30) || free * 20 < total;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final locale = Localizations.localeOf(context);
    String size(int b) => formatOfflineSize(l, locale, b);
    final s = offline.settings;
    final used = s.usedBytes > 0
        ? s.usedBytes
        : OfflineController.bytesOf(offline.copies);
    final total = s.totalBytes;
    final free = s.availableBytes;
    final watched = offline.watchedCopies;
    final days = s.deleteWatchedAfterDays;
    final volume = total != null && free != null && total > 0;
    final other = volume ? (total - free - used).clamp(0, total) : 0;
    return Container(
      key: const ValueKey('downloads-space'),
      padding: const EdgeInsets.all(18),
      decoration: BoxDecoration(
        color: KColors.bgElevated,
        borderRadius: BorderRadius.circular(d.panelRadius * .75),
        border: Border.all(color: KColors.stroke),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Semantics(
            header: true,
            child: Row(
              children: [
                Icon(
                  Icons.storage_rounded,
                  size: 22,
                  color: KColors.accentSoft,
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Text(
                    l.downloadsSpaceTitle,
                    style: theme.textTheme.titleLarge,
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(height: 14),
          if (volume) ...[
            _SpaceBar(used: used, other: other, free: free),
            const SizedBox(height: 10),
            Wrap(
              spacing: 16,
              runSpacing: 6,
              children: [
                _Legend(KColors.accent, l.downloadsSpaceDownloads(size(used))),
                _Legend(
                  KColors.textTertiary,
                  l.downloadsSpaceOther(size(other)),
                ),
                _Legend(KColors.stroke, l.downloadsSpaceFree(size(free))),
              ],
            ),
            if (lowSpace(free, total)) ...[
              const SizedBox(height: 10),
              Row(
                key: const ValueKey('downloads-space-low'),
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Icon(
                    Icons.warning_amber_rounded,
                    size: 18,
                    color: KColors.live,
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      l.downloadsSpaceLow,
                      style: theme.textTheme.bodySmall,
                    ),
                  ),
                ],
              ),
            ],
          ] else
            Text(
              l.offlineUsageNoVolume(size(used)),
              style: theme.textTheme.bodySmall,
            ),
          const SizedBox(height: 14),
          Text(
            days == null ? l.downloadsCleanupOff : l.downloadsCleanupRule(days),
            key: const ValueKey('downloads-cleanup-rule'),
            style: theme.textTheme.bodySmall?.copyWith(
              color: KColors.textSecondary,
            ),
          ),
          if (watched.isNotEmpty) ...[
            const SizedBox(height: 12),
            Align(
              alignment: Alignment.centerLeft,
              child: KButton(
                key: const ValueKey('downloads-delete-watched-now'),
                icon: Icons.auto_delete_outlined,
                label: l.downloadsDeleteWatched(watched.length),
                onPressed: () => _deleteWatched(context, watched),
              ),
            ),
          ],
        ],
      ),
    );
  }

  Future<void> _deleteWatched(
    BuildContext context,
    List<OfflineDownload> watched,
  ) async {
    final l = context.l10n;
    final size = formatOfflineSize(
      l,
      Localizations.localeOf(context),
      OfflineController.bytesOf(watched),
    );
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        key: const ValueKey('downloads-delete-watched-dialog'),
        title: Text(l.downloadsDeleteWatchedTitle),
        content: Text(l.downloadsDeleteWatchedBody(watched.length, size)),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: Text(l.offlineCancelDialog),
          ),
          TextButton(
            key: const ValueKey('downloads-delete-watched-confirm'),
            onPressed: () => Navigator.pop(context, true),
            child: Text(l.offlineDeleteConfirm),
          ),
        ],
      ),
    );
    if (ok != true) return;
    final n = await offline.deleteWatched();
    if (!context.mounted) return;
    ScaffoldMessenger.maybeOf(context)
      ?..removeCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(l.downloadsDeletedWatched(n))));
  }
}

/// Downloads, other files and free space of the volume.
class _SpaceBar extends StatelessWidget {
  const _SpaceBar({
    required this.used,
    required this.other,
    required this.free,
  });

  final int used, other, free;

  @override
  Widget build(BuildContext context) {
    final sum = used + other + free;
    // Flex in thousandths; a non-empty part stays visible.
    int part(int b) =>
        b <= 0 || sum <= 0 ? 0 : (b * 1000 ~/ sum).clamp(6, 1000);
    return ClipRRect(
      key: const ValueKey('downloads-space-bar'),
      borderRadius: BorderRadius.circular(4),
      child: SizedBox(
        height: 8,
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (part(used) > 0)
              Expanded(
                flex: part(used),
                child: const ColoredBox(color: KColors.accent),
              ),
            if (part(other) > 0)
              Expanded(
                flex: part(other),
                child: const ColoredBox(color: KColors.textTertiary),
              ),
            if (part(free) > 0)
              Expanded(
                flex: part(free),
                child: ColoredBox(color: KColors.stroke),
              ),
          ],
        ),
      ),
    );
  }
}

class _Legend extends StatelessWidget {
  const _Legend(this.color, this.text);

  final Color color;
  final String text;

  @override
  Widget build(BuildContext context) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      Container(
        width: 10,
        height: 10,
        decoration: BoxDecoration(color: color, shape: BoxShape.circle),
      ),
      const SizedBox(width: 6),
      Flexible(child: Text(text, style: Theme.of(context).textTheme.bodySmall)),
    ],
  );
}

/// Space and settings of the download folder (the Downloads screen and
/// Settings → Downloads).
class OfflineSettingsSection extends StatelessWidget {
  const OfflineSettingsSection({
    super.key,
    required this.offline,
    this.showOpen = false,
  });

  final OfflineController offline;

  /// Adds the "Downloads" row that opens [DownloadsScreen] (Settings).
  final bool showOpen;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final locale = Localizations.localeOf(context);
    final s = offline.settings;
    String size(int b) => formatOfflineSize(l, locale, b);
    final usage = s.availableBytes == null
        ? l.offlineUsageNoVolume(size(s.usedBytes))
        : l.offlineUsage(size(s.usedBytes), size(s.availableBytes!));
    final desktop = Platform.isLinux || Platform.isMacOS || Platform.isWindows;
    return SettingsSection(
      key: const ValueKey('settings-section-downloads'),
      icon: Icons.download_for_offline_outlined,
      title: l.offlineSettingsTitle,
      subtitle: l.offlineSettingsSubtitle,
      children: [
        if (showOpen)
          SettingsTile(
            key: const ValueKey('settings-downloads-open'),
            icon: Icons.download_outlined,
            title: l.offlineSettingsOpen,
            subtitle: l.offlineSettingsOpenCount(offline.activeCount),
            onTap: () => Navigator.of(context).push(
              MaterialPageRoute<void>(builder: (_) => const DownloadsScreen()),
            ),
          ),
        SettingsTile(
          key: const ValueKey('settings-downloads-folder'),
          icon: Icons.folder_outlined,
          title: l.offlineSettingsFolder,
          subtitle: [?s.folder, usage].join('\n'),
          subtitleLines: 3,
          onTap: desktop ? () => _pickFolder(context) : null,
        ),
        SettingsSwitchTile(
          key: const ValueKey('settings-downloads-wifi'),
          icon: Icons.wifi,
          title: l.offlineSettingsWifiOnly,
          subtitle: l.offlineSettingsWifiOnlyHint,
          value: s.wifiOnly,
          onChanged: (on) {
            unawaited(Services.read(context).setOfflineWifiOnly(on));
          },
        ),
        SettingsChoiceTile<int?>(
          key: const ValueKey('settings-downloads-delete-watched'),
          icon: Icons.auto_delete_outlined,
          title: l.offlineSettingsDeleteWatched,
          value: s.deleteWatchedAfterDays,
          options: offlineDeleteWatchedOptions,
          label: (v) => offlineDeleteWatchedLabel(l, v),
          onChanged: (v) => unawaited(offline.setDeleteWatched(v)),
        ),
      ],
    );
  }

  Future<void> _pickFolder(BuildContext context) async {
    final l = context.l10n;
    String? dir;
    try {
      dir = await FilePicker.getDirectoryPath(
        dialogTitle: l.offlineSettingsFolderPick,
      );
    } catch (e) {
      debugPrint('Folder picker: $e');
    }
    if (dir == null) return;
    try {
      await offline.setFolder(dir);
    } on OfflineException catch (_) {
      if (context.mounted) {
        ScaffoldMessenger.maybeOf(context)
            ?.showSnackBar(SnackBar(content: Text(l.offlineErrorBusy)));
      }
    }
  }
}

class _DownloadTile extends StatelessWidget {
  const _DownloadTile({
    required this.item,
    required this.wifiOnly,
    required this.onAction,
    this.autofocus = false,
  });

  final OfflineDownload item;
  final bool wifiOnly;
  final void Function(OfflineAction) onAction;
  final bool autofocus;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final locale = Localizations.localeOf(context);
    String size(int b) => formatOfflineSize(l, locale, b);
    final total = item.bytesTotal;
    final progress = item.progress;
    final details = [
      offlineStateText(l, item, wifiOnly: wifiOnly),
      ?item.version,
      if (item.state == OfflineState.done)
        size(item.bytesDone)
      else if (total != null && total > 0)
        l.offlineProgressBytes(size(item.bytesDone), size(total)),
      if (item.state == OfflineState.running && progress != null)
        formatOfflinePercent(locale, progress),
    ].join(' · ');
    final key = item.jobId != null ? 'j${item.jobId}' : 'c${item.copyId}';
    final done = item.state == OfflineState.done;
    final failed = item.state == OfflineState.failed;
    final circle = d.isTv ? 48.0 : 40.0;
    return Padding(
      key: ValueKey('download-$key'),
      padding: EdgeInsets.fromLTRB(
        d.pageInsets.left,
        d.isTv ? 10 : 8,
        d.pageInsets.right,
        d.isTv ? 10 : 8,
      ),
      child: Row(
        children: [
          Container(
            width: circle,
            height: circle,
            decoration: BoxDecoration(
              color: KColors.bgElevated2,
              shape: BoxShape.circle,
            ),
            child: Icon(
              done
                  ? Icons.download_done_rounded
                  : failed
                  ? Icons.error_outline_rounded
                  : Icons.downloading_rounded,
              size: d.isTv ? 24 : 20,
              color: done
                  ? KColors.accent
                  : failed
                  ? KColors.live
                  : KColors.textSecondary,
            ),
          ),
          const SizedBox(width: 14),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  item.title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.titleSmall,
                ),
                const SizedBox(height: 2),
                Text(
                  details,
                  key: ValueKey('download-$key-state'),
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.bodySmall,
                ),
                if (item.state.isActive)
                  Padding(
                    padding: const EdgeInsets.only(top: 6),
                    child: KProgressBar(
                      key: ValueKey('download-$key-progress'),
                      value: progress ?? 0,
                    ),
                  ),
              ],
            ),
          ),
          const SizedBox(width: 8),
          for (final (i, a) in item.actions.indexed)
            _ActionButton(
              key: ValueKey('download-$key-${a.name}'),
              icon: offlineActionLook(l, a).$1,
              tooltip: offlineActionLook(l, a).$2,
              autofocus: autofocus && i == 0,
              onPressed: () => onAction(a),
            ),
        ],
      ),
    );
  }
}

/// A round icon button with the app's focus look: a white fill under the
/// remote's focus on a TV and under the mouse on a desktop.
class _ActionButton extends StatelessWidget {
  const _ActionButton({
    super.key,
    required this.icon,
    required this.tooltip,
    required this.onPressed,
    this.autofocus = false,
  });

  final IconData icon;
  final String tooltip;
  final VoidCallback onPressed;
  final bool autofocus;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final side = d.isTv ? 48.0 : (d.isPhone ? 44.0 : 40.0);
    return Tooltip(
      message: tooltip,
      child: KFocusable(
        autofocus: autofocus,
        onActivate: onPressed,
        focusScale: d.focusScaleButton,
        radius: 999,
        light: false,
        semanticLabel: tooltip,
        builder: (context, active) => AnimatedContainer(
          duration: KMotion.focus,
          width: side,
          height: side,
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            color: active ? KColors.focusFill : Colors.transparent,
          ),
          child: Icon(
            icon,
            size: d.isTv ? 24 : 22,
            color: active ? KColors.focusLabel : KColors.textPrimary,
          ),
        ),
      ),
    );
  }
}
