// SPDX-License-Identifier: MPL-2.0
//
// «Всё ли в порядке» (UI-044): four lines a person understands — sources,
// storage space, sync and updates — each green, yellow or red (with an
// icon and a word, not only a colour) and, when something is wrong, one
// «Исправить» that opens the place where it is fixed (a NAS that does not
// let us in → its sign-in form). Codes and logs stay on «Состояние»
// (UI-017), one row below. Reads what the app already knows: the sources'
// checks, the offline folder's free space, the folder sync state and the
// update controller; nothing is sent anywhere. Opened from the basic
// section «Обновления и о программе» of Settings (UI-041), visible without
// the advanced switch.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../errors/error_view.dart';
import '../errors/friendly_errors.dart';
import '../features/updates/update_controller.dart';
import '../features/updates/update_section.dart' show updateProblemText;
import '../l10n/l10n.dart';
import '../offline/offline_downloads.dart';
import '../offline/offline_ui.dart' show formatOfflineSize;
import '../services/app_services.dart';
import '../sources/source_manager.dart';
import '../sources/source_text.dart';
import '../sync/device_sync.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import '../widgets/settings_tiles.dart';
import 'status_screen.dart' show openStatusScreen;

/// Green, yellow, red; [off] is grey: not set up or not on this device.
enum HealthLevel { ok, warning, problem, off }

/// One line of the screen.
@immutable
class HealthItem {
  const HealthItem({
    required this.id,
    required this.level,
    required this.status,
    this.text,
    this.fix,
    this.fixLabel,
    this.source,
  });

  /// `sources`, `storage`, `sync`, `updates`.
  final String id;
  final HealthLevel level;

  /// Short: «Все 3 на связи», «Свободно 48 ГБ».
  final String status;

  /// What is wrong and what to do; null when all is well.
  final String? text;

  /// The action of «Исправить»; null: no button.
  final ErrorFix? fix;

  /// Overrides the button's label («Настроить», «Обновить»).
  final String? fixLabel;

  /// The source [fix] is about.
  final ManagedSource? source;
}

/// The worst level of [items]; grey lines do not count.
HealthLevel overallHealth(Iterable<HealthItem> items) {
  var worst = HealthLevel.ok;
  for (final i in items) {
    if (i.level == HealthLevel.problem) return HealthLevel.problem;
    if (i.level == HealthLevel.warning) worst = HealthLevel.warning;
  }
  return worst;
}

/// Sources: red when one does not open (its error and action), yellow when
/// none is added or some are not checked yet.
HealthItem sourcesHealth(AppLocalizations l, List<ManagedSource> all) {
  final sources = all.where((s) => s.enabled).toList();
  if (sources.isEmpty) {
    return HealthItem(
      id: 'sources', // l10n-ignore
      level: HealthLevel.warning,
      status: l.healthSourcesNone,
      text: l.healthSourcesNoneText,
      fix: ErrorFix.openSources,
      fixLabel: l.healthAddSource,
    );
  }
  final failing = [
    for (final s in sources)
      if (s.status.currentError != null || s.status.online == false) s,
  ];
  if (failing.isNotEmpty) {
    final first = failing.first;
    final error = first.status.currentError;
    final code = error == null
        ? SourceErrorCode.unreachable
        : classifySourceMessage(error);
    return HealthItem(
      id: 'sources', // l10n-ignore
      level: HealthLevel.problem,
      status: l.healthSourcesFailing(failing.length, sources.length),
      text: l.healthSourceError(first.name, sourceErrorText(l, code)),
      fix: sourceErrorFix(code),
      source: first,
    );
  }
  if (sources.any((s) => s.status.online == null)) {
    return HealthItem(
      id: 'sources', // l10n-ignore
      level: HealthLevel.warning,
      status: l.healthSourcesUnchecked,
      fix: ErrorFix.retry,
      fixLabel: l.healthCheckNow,
    );
  }
  return HealthItem(
    id: 'sources', // l10n-ignore
    level: HealthLevel.ok,
    status: l.healthSourcesOk(sources.length),
  );
}

/// Less than this is red: downloads and the library database stop.
const healthCriticalFreeBytes = 500 * 1000 * 1000;

/// Storage space of the download folder's volume: red below 500 MB (or
/// 1 %), yellow below 2 GiB (or 5 %, as on Downloads).
HealthItem storageHealth(
  AppLocalizations l,
  Locale locale,
  OfflineSettings? settings,
) {
  final free = settings?.availableBytes;
  final total = settings?.totalBytes;
  if (free == null) {
    return HealthItem(
      id: 'storage', // l10n-ignore
      level: HealthLevel.off,
      status: l.healthStorageUnknown,
    );
  }
  final size = formatOfflineSize(l, locale, free);
  final t = total ?? 0;
  if (free < healthCriticalFreeBytes || (t > 0 && free * 100 < t)) {
    return HealthItem(
      id: 'storage', // l10n-ignore
      level: HealthLevel.problem,
      status: l.healthStorageFree(size),
      text: l.healthStorageCritical,
      fix: ErrorFix.freeSpace,
    );
  }
  if (free < (2 << 30) || (t > 0 && free * 20 < t)) {
    return HealthItem(
      id: 'storage', // l10n-ignore
      level: HealthLevel.warning,
      status: l.healthStorageFree(size),
      text: l.healthStorageLow,
      fix: ErrorFix.freeSpace,
    );
  }
  return HealthItem(
    id: 'storage', // l10n-ignore
    level: HealthLevel.ok,
    status: l.healthStorageFree(size),
  );
}

/// A folder sync older than this is yellow.
const healthSyncStale = Duration(days: 2);

/// Sync of the family: red when the last round failed, yellow when it has
/// not reached the folder for two days, grey when it is off.
HealthItem syncHealth(
  AppLocalizations l, {
  required bool available,
  required bool paired,
  required FolderSyncState? folder,
  required DateTime now,
}) {
  if (!available) {
    return HealthItem(
      id: 'sync', // l10n-ignore
      level: HealthLevel.off,
      status: l.healthSyncUnavailable,
    );
  }
  final f = folder;
  if (f != null && f.enabled) {
    final error = f.error;
    if (error != null) {
      return HealthItem(
        id: 'sync', // l10n-ignore
        level: HealthLevel.problem,
        status: l.healthSyncFailed,
        text: switch (error) {
          FolderSyncError.unavailable => l.devicesFolderErrorUnavailable,
          FolderSyncError.denied => l.devicesFolderErrorDenied,
          FolderSyncError.full => l.devicesFolderErrorFull,
          FolderSyncError.other => l.devicesFolderErrorOther,
        },
        fix: ErrorFix.openSync,
      );
    }
    final last = f.lastSync;
    if (last == null || now.difference(last) > healthSyncStale) {
      return HealthItem(
        id: 'sync', // l10n-ignore
        level: HealthLevel.warning,
        status: last == null
            ? l.healthSyncNever
            : l.healthSyncLast(formatAgo(l, last, now: now)),
        text: l.healthSyncStale,
        fix: ErrorFix.openSync,
      );
    }
    return HealthItem(
      id: 'sync', // l10n-ignore
      level: HealthLevel.ok,
      status: l.healthSyncLast(formatAgo(l, last, now: now)),
    );
  }
  if (paired) {
    return HealthItem(
      id: 'sync', // l10n-ignore
      level: HealthLevel.ok,
      status: l.healthSyncPaired,
    );
  }
  // SYNC-015: the folder picks itself once a writable network source
  // exists; until then sync waits, nothing to fix.
  if (f != null && f.mode == FolderSyncMode.auto) {
    return HealthItem(
      id: 'sync', // l10n-ignore
      level: HealthLevel.off,
      status: l.healthSyncAuto,
    );
  }
  return HealthItem(
    id: 'sync', // l10n-ignore
    level: HealthLevel.off,
    status: l.healthSyncOff,
    fix: ErrorFix.openSync,
    fixLabel: l.healthSetUp,
  );
}

/// Updates: yellow when a new version waits or the last step failed, grey
/// for store builds (the store updates them) and before the first check.
HealthItem updatesHealth(
  AppLocalizations l,
  UpdateController? c, {
  required String Function() problemText,
}) {
  if (c == null) {
    return HealthItem(
      id: 'updates', // l10n-ignore
      level: HealthLevel.off,
      status: l.healthUpdatesStore,
    );
  }
  final version = c.release?.version.text ?? '';
  switch (c.phase) {
    case UpdatePhase.failed:
      return HealthItem(
        id: 'updates', // l10n-ignore
        level: HealthLevel.warning,
        status: l.healthUpdatesFailed,
        text: problemText(),
        fix: ErrorFix.openUpdates,
      );
    case UpdatePhase.available ||
        UpdatePhase.downloading ||
        UpdatePhase.verifying ||
        UpdatePhase.installing ||
        UpdatePhase.readyToInstall ||
        UpdatePhase.restartNeeded ||
        UpdatePhase.handedOver:
      return HealthItem(
        id: 'updates', // l10n-ignore
        level: HealthLevel.warning,
        status: l.healthUpdatesAvailable(version),
        fix: ErrorFix.openUpdates,
        fixLabel: l.healthUpdate,
      );
    case UpdatePhase.installed:
    case UpdatePhase.upToDate:
      return HealthItem(
        id: 'updates', // l10n-ignore
        level: HealthLevel.ok,
        status: l.healthUpdatesOk(c.current.text),
      );
    case UpdatePhase.idle || UpdatePhase.checking:
      if (c.lastCheck == null) {
        return HealthItem(
          id: 'updates', // l10n-ignore
          level: HealthLevel.off,
          status: l.healthUpdatesNever,
          fix: ErrorFix.openUpdates,
          fixLabel: l.healthCheckNow,
        );
      }
      return HealthItem(
        id: 'updates', // l10n-ignore
        level: HealthLevel.ok,
        status: l.healthUpdatesOk(c.current.text),
      );
  }
}

/// Opens the screen over the current page.
Future<void> openHealthScreen(BuildContext context) =>
    Navigator.of(context)
        .push(MaterialPageRoute<void>(builder: (_) => const HealthScreen()));

class HealthScreen extends StatefulWidget {
  const HealthScreen({super.key, this.checkOnOpen = true});

  /// Check the sources and read the sync state when the screen opens.
  final bool checkOnOpen;

  @override
  State<HealthScreen> createState() => _HealthScreenState();
}

class _HealthScreenState extends State<HealthScreen> {
  FolderSyncState? _folder;
  bool _checking = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (widget.checkOnOpen) {
        unawaited(_check());
      } else {
        unawaited(_readSync());
      }
    });
  }

  Future<void> _readSync() async {
    if (!mounted) return;
    final sync = Services.read(context).sync;
    if (!sync.available) return;
    try {
      final f = await sync.folderSync();
      if (mounted) setState(() => _folder = f);
    } on Object {
      // The card shows sync as not set up.
    }
  }

  /// Checks every source, re-reads the free space and the sync state.
  Future<void> _check() async {
    if (!mounted || _checking) return;
    final services = Services.read(context);
    setState(() => _checking = true);
    try {
      await services.refreshSources();
      await Future.wait([
        for (final s in services.managedSources)
          if (s.enabled)
            services.checkSource(s.id).then<void>((_) {}, onError: (_) {}),
        if (services.offline != null)
          services.offline!.refresh().then<void>((_) {}, onError: (_) {}),
        _readSync(),
      ]);
    } finally {
      if (mounted) setState(() => _checking = false);
    }
  }

  List<HealthItem> _items(BuildContext context, AppServices services) {
    final l = context.l10n;
    return [
      sourcesHealth(l, services.managedSources),
      storageHealth(
        l,
        Localizations.localeOf(context),
        services.offline?.settings,
      ),
      syncHealth(
        l,
        available: services.sync.available,
        paired: services.sync.available && services.sync.paired,
        folder: _folder,
        now: services.clock(),
      ),
      updatesHealth(
        l,
        services.updates,
        problemText: () => updateProblemText(context, services.updates!),
      ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    final d = KDensity.of(context);
    return Scaffold(
      key: const ValueKey('health-page'),
      backgroundColor: KColors.bgBase,
      appBar: AppBar(
        backgroundColor: KColors.bgBase,
        title: Text(
          l.healthTitle,
          style: Theme.of(context).textTheme.titleLarge,
        ),
      ),
      body: ListenableBuilder(
        listenable: Listenable.merge([
          services,
          ?services.offline,
          ?services.updates,
        ]),
        builder: (context, _) {
          final items = _items(context, services);
          return FocusTraversalGroup(
            child: ListView(
              key: const ValueKey('health-list'),
              padding: EdgeInsets.fromLTRB(
                d.pageInsets.left,
                4,
                d.pageInsets.right,
                d.pageInsets.bottom + 24,
              ),
              children: [
                Align(
                  alignment: Alignment.topLeft,
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(maxWidth: 760),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        _Summary(
                          level: overallHealth(items),
                          checking: _checking,
                          onCheck: _check,
                        ),
                        const SizedBox(height: 12),
                        for (final i in items)
                          _HealthCard(item: i, onRetry: _check),
                        const SizedBox(height: 8),
                        SettingsTile(
                          key: const ValueKey('health-details'),
                          icon: Icons.monitor_heart_outlined,
                          title: l.healthDetails,
                          subtitle: l.healthDetailsSubtitle,
                          onTap: () => openStatusScreen(context),
                        ),
                      ],
                    ),
                  ),
                ),
              ],
            ),
          );
        },
      ),
    );
  }
}

(IconData, Color, String) _look(AppLocalizations l, HealthLevel level) =>
    switch (level) {
      HealthLevel.ok => (
        Icons.check_circle_rounded,
        KColors.statusOk,
        l.healthLevelOk,
      ),
      HealthLevel.warning => (
        Icons.warning_amber_rounded,
        KColors.statusWarning,
        l.healthLevelWarning,
      ),
      HealthLevel.problem => (
        Icons.error_rounded,
        KColors.statusProblem,
        l.healthLevelProblem,
      ),
      HealthLevel.off => (
        Icons.remove_circle_outline_rounded,
        KColors.textTertiary,
        l.healthLevelOff,
      ),
    };

class _Summary extends StatelessWidget {
  const _Summary({
    required this.level,
    required this.checking,
    required this.onCheck,
  });

  final HealthLevel level;
  final bool checking;
  final Future<void> Function() onCheck;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final d = KDensity.of(context);
    final (icon, color, _) = _look(l, level);
    final title = switch (level) {
      HealthLevel.ok || HealthLevel.off => l.healthSummaryOk,
      HealthLevel.warning => l.healthSummaryWarning,
      HealthLevel.problem => l.healthSummaryProblem,
    };
    return Container(
      key: ValueKey('health-summary-${level.name}'),
      padding: const EdgeInsets.all(18),
      decoration: BoxDecoration(
        color: KColors.bgElevated,
        borderRadius: BorderRadius.circular(d.panelRadius * .75),
        border: Border.all(color: color.withValues(alpha: .6)),
      ),
      // The button sits on the left, above the lines' «Исправить»: the
      // remote's Down goes from it straight to the first fix.
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(icon, color: color, size: 36),
          const SizedBox(width: 14),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Semantics(
                  liveRegion: true,
                  child: Text(title, style: theme.textTheme.titleMedium),
                ),
                const SizedBox(height: 10),
                KButton(
                  key: const ValueKey('health-check'),
                  label: checking ? l.healthChecking : l.healthCheckAgain,
                  icon: Icons.refresh_rounded,
                  autofocus: d.isTv,
                  onPressed: checking ? null : () => unawaited(onCheck()),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _HealthCard extends StatelessWidget {
  const _HealthCard({required this.item, required this.onRetry});

  final HealthItem item;
  final Future<void> Function() onRetry;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final d = KDensity.of(context);
    final (icon, color, word) = _look(l, item.level);
    final (areaIcon, area) = switch (item.id) {
      'sources' => (Icons.lan_outlined, l.healthAreaSources), // l10n-ignore
      'storage' => (
        Icons.sd_storage_outlined,
        l.healthAreaStorage,
      ), // l10n-ignore
      'sync' => (Icons.devices_outlined, l.healthAreaSync), // l10n-ignore
      _ => (Icons.system_update_alt_rounded, l.healthAreaUpdates),
    };
    final fix = item.fix;
    final text = item.text;
    return Container(
      key: ValueKey('health-${item.id}'),
      margin: EdgeInsets.only(bottom: d.isTv ? 10 : 12),
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: KColors.bgElevated,
        borderRadius: BorderRadius.circular(d.panelRadius * .75),
        border: Border.all(color: KColors.stroke),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Icon(areaIcon, color: KColors.textSecondary),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(area, style: theme.textTheme.titleMedium),
                    Text(
                      item.status,
                      key: ValueKey('health-${item.id}-status'),
                      style: theme.textTheme.bodySmall,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 8),
              Semantics(
                label: word,
                child: ExcludeSemantics(
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Icon(
                        icon,
                        key: ValueKey('health-${item.id}-${item.level.name}'),
                        color: color,
                        size: 22,
                      ),
                      if (!d.isPhone) ...[
                        const SizedBox(width: 6),
                        Text(
                          word,
                          style: theme.textTheme.labelMedium?.copyWith(
                            color: color,
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
              ),
            ],
          ),
          if (text != null) ...[
            const SizedBox(height: 10),
            Text(text, style: theme.textTheme.bodyMedium),
          ],
          if (fix != null) ...[
            const SizedBox(height: 10),
            Align(
              alignment: Alignment.centerLeft,
              child: Tooltip(
                message: errorFixHint(l, fix),
                child: KButton(
                  key: ValueKey('health-${item.id}-fix'),
                  label: item.fixLabel ?? errorFixLabel(l, fix),
                  icon: fix == ErrorFix.retry
                      ? Icons.refresh_rounded
                      : Icons.build_outlined,
                  primary: item.level == HealthLevel.problem,
                  onPressed: () => unawaited(
                    runErrorFix(
                      context,
                      fix,
                      source: item.source,
                      retry: onRetry,
                    ),
                  ),
                ),
              ),
            ),
          ],
        ],
      ),
    );
  }
}
