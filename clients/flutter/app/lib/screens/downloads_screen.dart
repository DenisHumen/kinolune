// SPDX-License-Identifier: MPL-2.0
//
// Downloads (NET-019, UI-018 basics): offline copies on the device and the
// downloads in the queue with their progress (from the core's job events),
// pause / resume / cancel / retry / delete, the space of the download
// folder and its settings: the folder, Wi-Fi only and "delete watched"
// (a pa-storage retention rule).

import 'dart:async';
import 'dart:io' show Platform;

import 'package:file_picker/file_picker.dart';
import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../offline/offline_downloads.dart';
import '../offline/offline_ui.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/cards.dart';
import '../widgets/settings_tiles.dart';

/// Choices of "delete watched": never, right away, after N days.
const offlineDeleteWatchedOptions = <int?>[null, 0, 1, 7, 30];

String offlineDeleteWatchedLabel(AppLocalizations l, int? days) =>
    switch (days) {
      null => l.offlineDeleteWatchedNever,
      0 => l.offlineDeleteWatchedNow,
      final n => l.offlineDeleteWatchedDays(n),
    };

class DownloadsScreen extends StatefulWidget {
  const DownloadsScreen({super.key});

  @override
  State<DownloadsScreen> createState() => _DownloadsScreenState();
}

class _DownloadsScreenState extends State<DownloadsScreen> {
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
    final offline = Services.of(context).offline;
    return Scaffold(
      key: const ValueKey('downloads-page'),
      backgroundColor: KColors.bgBase,
      appBar: AppBar(
        backgroundColor: KColors.bgBase,
        title: Text(
          l.offlineScreenTitle,
          style: Theme.of(context).textTheme.titleLarge,
        ),
      ),
      body: offline == null
          ? Padding(
              padding: EdgeInsets.all(d.pageInsets.left),
              child: Text(l.offlineUnavailable),
            )
          : ListenableBuilder(
              listenable: offline,
              builder: (context, _) => _list(context, offline),
            ),
    );
  }

  Widget _list(BuildContext context, OfflineController offline) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final items = offline.downloads;
    return ListView(
      padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 24),
      children: [
        Padding(
          padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
          child: Align(
            alignment: Alignment.topLeft,
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 760),
              child: OfflineSettingsSection(offline: offline),
            ),
          ),
        ),
        if (items.isEmpty)
          Padding(
            key: const ValueKey('downloads-empty'),
            padding: EdgeInsets.symmetric(
              horizontal: d.pageInsets.left,
              vertical: 24,
            ),
            child: Text(
              l.offlineEmpty,
              style: Theme.of(context).textTheme.bodyMedium
                  ?.copyWith(color: KColors.textSecondary),
            ),
          ),
        for (final item in items)
          _DownloadTile(
            item: item,
            wifiOnly: offline.settings.wifiOnly,
            onAction: (a) => a == OfflineAction.delete
                ? confirmDeleteDownload(context, item.itemId, item.title)
                : offline.control(item, a),
          ),
      ],
    );
  }
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
  });

  final OfflineDownload item;
  final bool wifiOnly;
  final void Function(OfflineAction) onAction;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final locale = Localizations.localeOf(context);
    String size(int b) => formatOfflineSize(l, locale, b);
    final total = item.bytesTotal;
    final details = [
      offlineStateText(l, item, wifiOnly: wifiOnly),
      ?item.version,
      if (item.state == OfflineState.done)
        size(item.bytesDone)
      else if (total != null && total > 0)
        l.offlineProgressBytes(size(item.bytesDone), size(total)),
    ].join(' · ');
    final key = item.jobId != null ? 'j${item.jobId}' : 'c${item.copyId}';
    return Padding(
      key: ValueKey('download-$key'),
      padding: EdgeInsets.fromLTRB(d.pageInsets.left, 8, d.pageInsets.right, 8),
      child: Row(
        children: [
          Icon(
            item.state == OfflineState.done
                ? Icons.download_done_rounded
                : Icons.downloading_rounded,
            color: item.state == OfflineState.done
                ? KColors.accent
                : KColors.textSecondary,
          ),
          const SizedBox(width: 12),
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
                  style: theme.textTheme.bodySmall,
                ),
                if (item.state.isActive)
                  Padding(
                    padding: const EdgeInsets.only(top: 6),
                    child: KProgressBar(
                      key: ValueKey('download-$key-progress'),
                      value: item.progress ?? 0,
                    ),
                  ),
              ],
            ),
          ),
          for (final a in item.actions)
            IconButton(
              key: ValueKey('download-$key-${a.name}'),
              tooltip: offlineActionLook(l, a).$2,
              icon: Icon(offlineActionLook(l, a).$1),
              onPressed: () => onAction(a),
            ),
        ],
      ),
    );
  }
}
