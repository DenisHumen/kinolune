// SPDX-License-Identifier: MPL-2.0
//
// TWI-019 (full builds only): the recordings and downloads library, the
// record rule of a favourite channel, and the storage settings (folders,
// quotas, file name templates, import). Everything goes through
// `TwitchArchive` (the core's queue, or FakeTwitchArchive in tests). The
// app opens these only under `AppFlavor.twitchArchive`, and never for a
// profile that hides the Streams section (UI-015).

import 'dart:async';

import 'package:file_picker/file_picker.dart';
import 'package:material_ui/material_ui.dart';

import '../../l10n/l10n.dart';
import '../../platform/local_path.dart';
import '../../services/app_services.dart';
import '../../services/preferences.dart';
import '../../theme/kinolune_theme.dart';
import '../../twitch/twitch_archive.dart';
import '../../twitch/twitch_downloads.dart';
import '../../widgets/cards.dart';
import '../../widgets/settings_tiles.dart';
import '../../widgets/undo_snack.dart';
import '../library_grid_screen.dart' show FilterPill;

String archiveSize(int bytes) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB']; // l10n-ignore
  var v = bytes.toDouble();
  var u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return '${v.toStringAsFixed(u == 0 ? 0 : 1)} ${units[u]}';
}

String archiveDuration(Duration d) {
  final h = d.inHours;
  final m = d.inMinutes.remainder(60).toString().padLeft(2, '0');
  return h > 0
      ? '$h:$m:${d.inSeconds.remainder(60).toString().padLeft(2, '0')}'
      : '$m:${d.inSeconds.remainder(60).toString().padLeft(2, '0')}';
}

String _date(DateTime t) =>
    '${t.year}-${t.month.toString().padLeft(2, '0')}-${t.day.toString().padLeft(2, '0')}';

/// Paused and gap badges (no warning token in the design system yet).
const _amber = Color(0xFFF2B84B);

// ------------------------------------------------------------ the library

enum _Tab { recordings, downloads }

/// Recordings (active ones with their indicators, then saved) and saved
/// downloads and imports.
class TwitchArchiveScreen extends StatefulWidget {
  const TwitchArchiveScreen({
    super.key,
    this.refresh = const Duration(seconds: 2),
  });

  final Duration refresh;

  @override
  State<TwitchArchiveScreen> createState() => _TwitchArchiveScreenState();
}

class _TwitchArchiveScreenState extends State<TwitchArchiveScreen> {
  _Tab _tab = _Tab.recordings;
  List<TwitchRecordingItem>? _jobs;
  List<TwitchSavedFile>? _saved;
  Timer? _timer;

  TwitchArchive? get _archive => Services.read(context).visibleArchive;

  @override
  void initState() {
    super.initState();
    _timer = Timer.periodic(widget.refresh, (_) => unawaited(_load()));
    WidgetsBinding.instance.addPostFrameCallback((_) => unawaited(_load()));
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  Future<void> _load() async {
    final a = _archive;
    if (a == null) return;
    final jobs = await a.recordings();
    final saved = await a.saved();
    if (mounted) {
      setState(() {
        _jobs = jobs;
        _saved = saved;
      });
    }
  }

  /// Recordings removed from the list while «Отменить» is offered
  /// (QA-015).
  final _removing = <int>{};

  Future<void> _control(TwitchRecordingItem j, TwitchDownloadAction a) async {
    final archive = _archive;
    if (a == TwitchDownloadAction.remove) {
      // At once, with «Отменить»; the job goes when the snack bar closes.
      final l = context.l10n;
      setState(() => _removing.add(j.jobId));
      final undone = await showUndoSnack(
        ScaffoldMessenger.maybeOf(context),
        message: l.twitchDownloadRemoved,
        undoLabel: l.commonUndo,
      );
      if (!undone) await archive?.controlRecording(j.jobId, a);
      if (!mounted) return;
      setState(() => _removing.remove(j.jobId));
    } else {
      await archive?.controlRecording(j.jobId, a);
    }
    await _load();
  }

  Future<void> _import() async {
    final result = await importTwitchFolder(context);
    if (result != null) await _load();
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final jobs = (_jobs ?? const <TwitchRecordingItem>[])
        .where(
          (j) =>
              j.activity != RecordingActivity.done &&
              !_removing.contains(j.jobId),
        )
        .toList();
    final saved = _saved ?? const <TwitchSavedFile>[];
    final recordings = saved
        .where((f) => f.kind == TwitchSavedKind.recording)
        .toList();
    final downloads = saved
        .where((f) => f.kind != TwitchSavedKind.recording)
        .toList();
    final loaded = _saved != null;
    final pad = EdgeInsets.symmetric(horizontal: d.pageInsets.left);
    return Scaffold(
      key: const ValueKey('twitch-archive-page'),
      backgroundColor: KColors.bgBase,
      body: ListView(
        padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 24),
        children: [
          PageTitle(
            l.twitchArchiveTitle,
            trailing: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                IconButton(
                  key: const ValueKey('twitch-archive-import'),
                  tooltip: l.twitchArchiveImport,
                  icon: const Icon(Icons.drive_folder_upload_outlined),
                  onPressed: _import,
                ),
                IconButton(
                  key: const ValueKey('twitch-archive-settings'),
                  tooltip: l.twitchArchiveSettings,
                  icon: const Icon(Icons.tune_rounded),
                  onPressed: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) => const TwitchArchiveSettingsScreen(),
                    ),
                  ),
                ),
              ],
            ),
          ),
          Padding(
            padding: pad,
            child: Wrap(
              spacing: 8,
              children: [
                FilterPill(
                  key: const ValueKey('twitch-archive-tab-recordings'),
                  label: l.twitchArchiveRecordings,
                  selected: _tab == _Tab.recordings,
                  onTap: () => setState(() => _tab = _Tab.recordings),
                ),
                FilterPill(
                  key: const ValueKey('twitch-archive-tab-downloads'),
                  label: l.twitchArchiveDownloads,
                  selected: _tab == _Tab.downloads,
                  onTap: () => setState(() => _tab = _Tab.downloads),
                ),
              ],
            ),
          ),
          const SizedBox(height: 12),
          if (_tab == _Tab.recordings) ...[
            if (jobs.isNotEmpty) ...[
              _Heading(l.twitchArchiveNow),
              for (final j in jobs)
                _RecordingTile(item: j, onAction: (a) => _control(j, a)),
              const SizedBox(height: 8),
            ],
            if (loaded && recordings.isEmpty && jobs.isEmpty)
              _Empty(l.twitchArchiveNoRecordings),
            if (recordings.isNotEmpty) _Heading(l.twitchArchiveSaved),
            for (final f in recordings) _SavedTile(file: f),
          ] else ...[
            if (loaded && downloads.isEmpty) _Empty(l.twitchArchiveNoDownloads),
            for (final f in downloads) _SavedTile(file: f),
          ],
        ],
      ),
    );
  }
}

class _Heading extends StatelessWidget {
  const _Heading(this.text);
  final String text;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    return Padding(
      padding: EdgeInsets.fromLTRB(d.pageInsets.left, 8, d.pageInsets.right, 4),
      child: Text(text, style: Theme.of(context).textTheme.titleMedium),
    );
  }
}

class _Empty extends StatelessWidget {
  const _Empty(this.text);
  final String text;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    return Padding(
      padding: EdgeInsets.symmetric(
        horizontal: d.pageInsets.left,
        vertical: 24,
      ),
      child: Text(text),
    );
  }
}

/// A small coloured label: "Recording", "Paused", "2 gaps".
class _Badge extends StatelessWidget {
  const _Badge({required this.text, required this.color, this.icon, super.key});

  final String text;
  final Color color;
  final IconData? icon;

  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
    decoration: BoxDecoration(
      color: color.withValues(alpha: .16),
      borderRadius: BorderRadius.circular(6),
    ),
    child: Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (icon != null) ...[
          Icon(icon, size: 14, color: color),
          const SizedBox(width: 4),
        ],
        Text(
          text,
          style: Theme.of(context).textTheme.labelSmall?.copyWith(color: color),
        ),
      ],
    ),
  );
}

(IconData, String) _actionOf(
  AppLocalizations l,
  TwitchDownloadAction a,
) => switch (a) {
  TwitchDownloadAction.pause => (Icons.pause_rounded, l.twitchDownloadPause),
  TwitchDownloadAction.resume => (
    Icons.play_arrow_rounded,
    l.twitchDownloadResume,
  ),
  TwitchDownloadAction.cancel => (Icons.stop_rounded, l.twitchRecordingStop),
  TwitchDownloadAction.retry => (Icons.refresh_rounded, l.twitchDownloadRetry),
  TwitchDownloadAction.remove => (
    Icons.delete_outline_rounded,
    l.twitchDownloadRemove,
  ),
};

class _RecordingTile extends StatelessWidget {
  const _RecordingTile({required this.item, required this.onAction});

  final TwitchRecordingItem item;
  final void Function(TwitchDownloadAction) onAction;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final (badge, color, icon) = switch (item.activity) {
      RecordingActivity.active => (
        l.twitchRecordingActive,
        KColors.live,
        Icons.fiber_manual_record,
      ),
      RecordingActivity.paused => (
        item.state == TwitchDownloadState.waiting
            ? l.twitchRecordingWaiting
            : l.twitchRecordingPaused,
        _amber,
        Icons.pause_circle_outline,
      ),
      RecordingActivity.done => (
        l.twitchDownloadDone,
        KColors.textSecondary,
        Icons.check_circle_outline,
      ),
      RecordingActivity.failed => (
        l.twitchDownloadFailed,
        theme.colorScheme.error,
        Icons.error_outline,
      ),
    };
    final details = [
      archiveDuration(item.media),
      if (item.bytesDone > 0) archiveSize(item.bytesDone),
      ?item.waitingFor,
    ].join(' · ');
    return Padding(
      key: ValueKey('twitch-recording-${item.jobId}'),
      padding: EdgeInsets.fromLTRB(d.pageInsets.left, 8, d.pageInsets.right, 8),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  item.login ?? item.userId,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.titleSmall,
                ),
                const SizedBox(height: 4),
                Wrap(
                  spacing: 6,
                  runSpacing: 4,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  children: [
                    _Badge(
                      key: ValueKey(
                        'twitch-recording-${item.jobId}-${item.activity.name}',
                      ),
                      text: badge,
                      color: color,
                      icon: icon,
                    ),
                    if (item.gaps > 0)
                      _Badge(
                        key: ValueKey('twitch-recording-${item.jobId}-gaps'),
                        text: l.twitchRecordingGaps(item.gaps),
                        color: _amber,
                        icon: Icons.content_cut_rounded,
                      ),
                    Text(details, style: theme.textTheme.bodySmall),
                  ],
                ),
                if (item.error case final e?)
                  Text(
                    e,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: theme.colorScheme.error,
                    ),
                  ),
              ],
            ),
          ),
          for (final a in item.actions)
            IconButton(
              key: ValueKey('twitch-recording-${item.jobId}-${a.name}'),
              tooltip: _actionOf(l, a).$2,
              icon: Icon(_actionOf(l, a).$1),
              onPressed: () => onAction(a),
            ),
        ],
      ),
    );
  }
}

class _SavedTile extends StatelessWidget {
  const _SavedTile({required this.file});

  final TwitchSavedFile file;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final origin = switch (file.origin) {
      'import' => l.twitchArchiveOriginImport,
      'recording' => l.twitchArchiveOriginRecording,
      _ => l.twitchArchiveOriginDownload,
    };
    final details = [
      file.channel,
      if (file.recordedAt case final t?) _date(t),
      if (file.duration case final du?) archiveDuration(du),
      if (file.sizeBytes > 0) archiveSize(file.sizeBytes),
      origin,
    ].join(' · ');
    return Padding(
      key: ValueKey('twitch-saved-${file.assetId}'),
      padding: EdgeInsets.fromLTRB(d.pageInsets.left, 8, d.pageInsets.right, 8),
      child: Row(
        children: [
          Icon(
            file.kind == TwitchSavedKind.recording
                ? Icons.radio_button_checked
                : file.kind == TwitchSavedKind.clip
                ? Icons.content_cut
                : Icons.video_file_outlined,
            color: KColors.textSecondary,
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  file.title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.titleSmall,
                ),
                const SizedBox(height: 2),
                Text(
                  details,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.bodySmall,
                ),
                const SizedBox(height: 4),
                Wrap(
                  spacing: 6,
                  children: [
                    if (file.chapters > 0)
                      _Badge(
                        text: l.twitchArchiveChapters(file.chapters),
                        color: KColors.textSecondary,
                        icon: Icons.segment_rounded,
                      ),
                    if (file.hasChat)
                      _Badge(
                        text: l.twitchArchiveChat,
                        color: KColors.textSecondary,
                        icon: Icons.chat_bubble_outline,
                      ),
                    if (file.hasAdsMarked)
                      _Badge(
                        text: l.twitchArchiveAds,
                        color: KColors.textSecondary,
                        icon: Icons.campaign_outlined,
                      ),
                  ],
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

// ------------------------------------------------------------------ import

/// Asks for a folder and imports it; shows what happened. Null when the
/// user cancelled or nothing could be imported.
Future<TwitchImportResult?> importTwitchFolder(
  BuildContext context, {
  String? path,
}) async {
  final archive = Services.read(context).visibleArchive;
  if (archive == null) return null;
  final l = context.l10n;
  final messenger = ScaffoldMessenger.maybeOf(context);
  var dir = path;
  if (dir == null) {
    try {
      dir = await FilePicker.getDirectoryPath(
        dialogTitle: l.twitchArchiveImportPick,
      );
    } catch (e) {
      debugPrint('Folder picker: $e');
    }
  }
  if (dir == null) return null;
  try {
    final r = await archive.importFolder(dir);
    messenger?.showSnackBar(
      SnackBar(
        content: Text(
          l.twitchArchiveImportDone(
            r.added,
            r.duplicates + r.known,
            r.noUser + r.skipped,
            r.chapters,
          ),
        ),
      ),
    );
    return r;
  } catch (e) {
    messenger?.showSnackBar(
      SnackBar(content: Text(l.twitchArchiveImportError('$e'))),
    );
    return null;
  }
}

// -------------------------------------------------------------- the rule

/// The ToS / personal-use warning (06 §2.7): once per device, before the
/// first rule is turned on. True when accepted.
Future<bool> confirmRecordWarning(BuildContext context) async {
  final services = Services.read(context);
  final prefs = services.preferences;
  if (await prefs.read(Preferences.twitchRecordWarningAccepted) == 'true') {
    return true;
  }
  if (!context.mounted) return false;
  final l = context.l10n;
  final ok = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      key: const ValueKey('twitch-record-warning'),
      title: Text(l.twitchRecordWarningTitle),
      content: Text(l.twitchRecordWarningBody),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(false),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('twitch-record-warning-accept'),
          onPressed: () => Navigator.of(context).pop(true),
          child: Text(l.twitchRecordWarningAccept),
        ),
      ],
    ),
  );
  if (ok == true) {
    await prefs.write(Preferences.twitchRecordWarningAccepted, 'true');
    return true;
  }
  return false;
}

/// Auto-record of one favourite channel for the active profile: on/off,
/// categories, quality, keep N / X days, chat.
class TwitchRecordRuleScreen extends StatefulWidget {
  const TwitchRecordRuleScreen({
    super.key,
    required this.userId,
    required this.displayName,
    this.categories = const [],
  });

  final String userId;
  final String displayName;

  /// Categories seen on the channel `(game id, name)`: the filter's
  /// choices.
  final List<(String, String)> categories;

  @override
  State<TwitchRecordRuleScreen> createState() => _TwitchRecordRuleScreenState();
}

class _TwitchRecordRuleScreenState extends State<TwitchRecordRuleScreen> {
  TwitchRecordRule? _rule;
  bool _saved = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => unawaited(_load()));
  }

  Future<void> _load() async {
    final services = Services.read(context);
    final archive = services.visibleArchive;
    if (archive == null) return;
    final rules = await archive.rules(services.archiveProfileId);
    final existing = rules.where((r) => r.userId == widget.userId).firstOrNull;
    if (!mounted) return;
    setState(() {
      _saved = existing != null;
      _rule =
          existing ?? TwitchRecordRule(userId: widget.userId, enabled: false);
    });
  }

  Future<void> _update(TwitchRecordRule next) async {
    final services = Services.read(context);
    final archive = services.visibleArchive;
    if (archive == null) return;
    final turningOn = next.enabled && !(_rule?.enabled ?? false);
    if (turningOn && !await confirmRecordWarning(context)) return;
    setState(() => _rule = next);
    await archive.setRule(services.archiveProfileId, next);
    _saved = true;
    await services.refreshRecordingChannels();
  }

  /// At once, with «Отменить» that saves the rule back (QA-015).
  Future<void> _delete() async {
    final services = Services.read(context);
    final archive = services.visibleArchive;
    final profileId = services.archiveProfileId;
    final rule = _rule;
    final l = context.l10n;
    final messenger = ScaffoldMessenger.maybeOf(context);
    await archive?.deleteRule(profileId, widget.userId);
    await services.refreshRecordingChannels();
    if (mounted) Navigator.of(context).maybePop();
    final undone = await showUndoSnack(
      messenger,
      message: l.twitchRuleDeleted,
      undoLabel: l.commonUndo,
    );
    if (undone && rule != null && archive != null) {
      await archive.setRule(profileId, rule);
      await services.refreshRecordingChannels();
    }
  }

  Future<int?> _askNumber(String title, int? current) async {
    final result = await showDialog<(int?,)>(
      context: context,
      builder: (context) => _NumberDialog(title: title, current: current),
    );
    return result == null ? current : result.$1;
  }

  String _quality(AppLocalizations l, String q) => switch (q) {
    'best' => l.twitchRuleQualityBest,
    'audio_only' => l.twitchRuleQualityAudio,
    _ => q,
  };

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final rule = _rule;
    return Scaffold(
      key: ValueKey('twitch-rule-page-${widget.userId}'),
      backgroundColor: KColors.bgBase,
      body: ListView(
        padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 24),
        children: [
          PageTitle(l.twitchRuleTitle(widget.displayName)),
          if (rule != null) ...[
            Padding(
              padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
              child: SettingsSection(
                icon: Icons.radio_button_checked,
                title: l.twitchRuleAuto,
                subtitle: l.twitchRuleAutoSubtitle,
                children: [
                  SettingsSwitchTile(
                    key: const ValueKey('twitch-rule-enabled'),
                    icon: Icons.fiber_manual_record_outlined,
                    title: l.twitchRuleAuto,
                    subtitle: rule.enabled
                        ? l.twitchRuleOnHint
                        : l.twitchRuleOffHint,
                    value: rule.enabled,
                    onChanged: (v) => _update(rule.copyWith(enabled: v)),
                  ),
                  SettingsChoiceTile<String>(
                    key: const ValueKey('twitch-rule-quality'),
                    icon: Icons.high_quality_outlined,
                    title: l.twitchRuleQuality,
                    value: rule.quality,
                    options: TwitchRecordRule.qualities,
                    label: (q) => _quality(l, q),
                    onChanged: (q) => _update(rule.copyWith(quality: q)),
                  ),
                  SettingsSwitchTile(
                    key: const ValueKey('twitch-rule-chat'),
                    icon: Icons.chat_bubble_outline,
                    title: l.twitchRuleChat,
                    subtitle: l.twitchRuleChatSubtitle,
                    value: rule.withChat,
                    onChanged: (v) => _update(rule.copyWith(withChat: v)),
                  ),
                ],
              ),
            ),
            Padding(
              padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
              child: SettingsSection(
                icon: Icons.auto_delete_outlined,
                title: l.twitchRuleRetention,
                subtitle: l.twitchRuleRetentionSubtitle,
                children: [
                  SettingsTile(
                    key: const ValueKey('twitch-rule-keep-last'),
                    icon: Icons.filter_9_plus_outlined,
                    title: l.twitchRuleKeepLast,
                    subtitle: rule.keepLast == null
                        ? l.twitchRuleKeepAll
                        : l.twitchRuleKeepLastValue(rule.keepLast!),
                    onTap: () async {
                      final n = await _askNumber(
                        l.twitchRuleKeepLast,
                        rule.keepLast,
                      );
                      await _update(rule.copyWith(keepLast: () => n));
                    },
                  ),
                  SettingsTile(
                    key: const ValueKey('twitch-rule-keep-days'),
                    icon: Icons.calendar_month_outlined,
                    title: l.twitchRuleKeepDays,
                    subtitle: rule.keepDays == null
                        ? l.twitchRuleKeepAll
                        : l.twitchRuleKeepDaysValue(rule.keepDays!),
                    onTap: () async {
                      final n = await _askNumber(
                        l.twitchRuleKeepDays,
                        rule.keepDays,
                      );
                      await _update(rule.copyWith(keepDays: () => n));
                    },
                  ),
                ],
              ),
            ),
            Padding(
              padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
              child: SettingsSection(
                icon: Icons.category_outlined,
                title: l.twitchRuleCategories,
                subtitle: rule.categories == null
                    ? l.twitchRuleCategoriesAll
                    : l.twitchRuleCategoriesSome(rule.categories!.length),
                children: [
                  Padding(
                    padding: const EdgeInsets.all(12),
                    child: Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      children: [
                        FilterPill(
                          key: const ValueKey('twitch-rule-category-all'),
                          label: l.twitchRuleCategoriesAll,
                          selected: rule.categories == null,
                          onTap: () =>
                              _update(rule.copyWith(categories: () => null)),
                        ),
                        for (final (id, name) in widget.categories)
                          FilterPill(
                            key: ValueKey('twitch-rule-category-$id'),
                            label: name,
                            selected: rule.categories?.contains(id) ?? false,
                            onTap: () {
                              final next = {...?rule.categories};
                              if (!next.remove(id)) next.add(id);
                              _update(
                                rule.copyWith(
                                  categories: () => next.isEmpty ? null : next,
                                ),
                              );
                            },
                          ),
                      ],
                    ),
                  ),
                  if (widget.categories.isEmpty)
                    Padding(
                      padding: const EdgeInsets.fromLTRB(12, 0, 12, 12),
                      child: Text(
                        l.twitchRuleCategoriesNone,
                        style: Theme.of(context).textTheme.bodySmall,
                      ),
                    ),
                ],
              ),
            ),
            if (_saved)
              Padding(
                padding: EdgeInsets.symmetric(
                  horizontal: d.pageInsets.left,
                  vertical: 8,
                ),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: TextButton.icon(
                    key: const ValueKey('twitch-rule-delete'),
                    icon: const Icon(Icons.delete_outline),
                    label: Text(l.twitchRuleDelete),
                    onPressed: _delete,
                  ),
                ),
              ),
          ],
        ],
      ),
    );
  }
}

// ---------------------------------------------------------------- settings

/// Folders and quotas of downloads and recordings, file name templates,
/// import.
class TwitchArchiveSettingsScreen extends StatefulWidget {
  const TwitchArchiveSettingsScreen({super.key});

  @override
  State<TwitchArchiveSettingsScreen> createState() =>
      _TwitchArchiveSettingsScreenState();
}

class _TwitchArchiveSettingsScreenState
    extends State<TwitchArchiveSettingsScreen> {
  TwitchArchiveSettings? _s;
  List<TwitchArchiveUsage> _usage = const [];
  final _folder = TextEditingController();
  final _vod = TextEditingController();
  final _live = TextEditingController();
  String? _error;
  (String, String)? _preview;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => unawaited(_load()));
  }

  @override
  void dispose() {
    _folder.dispose();
    _vod.dispose();
    _live.dispose();
    super.dispose();
  }

  TwitchArchive? get _archive => Services.read(context).visibleArchive;

  Future<void> _load() async {
    final a = _archive;
    if (a == null) return;
    final s = await a.settings();
    final usage = await a.usage();
    if (!mounted) return;
    _folder.text = s.folderTemplate;
    _vod.text = s.vodTemplate;
    _live.text = s.liveTemplate;
    setState(() {
      _s = s;
      _usage = usage;
    });
    _check();
  }

  void _check() {
    final a = _archive;
    if (a == null) return;
    try {
      final p = a.preview(_folder.text, _vod.text, _live.text);
      setState(() {
        _preview = p;
        _error = null;
      });
    } on TwitchTemplateError catch (e) {
      setState(() => _error = e.message);
    }
  }

  Future<void> _save(TwitchArchiveSettings next) async {
    final l = context.l10n;
    final messenger = ScaffoldMessenger.maybeOf(context);
    try {
      await Services.read(context).saveArchiveSettings(next);
      setState(() => _s = next);
      final usage = await _archive?.usage();
      if (mounted && usage != null) setState(() => _usage = usage);
    } on TwitchTemplateError catch (e) {
      setState(() => _error = e.message);
    } catch (e) {
      messenger?.showSnackBar(
        SnackBar(content: Text(l.twitchArchiveSaveError('$e'))),
      );
    }
  }

  Future<void> _pickDir(bool recordings) async {
    final s = _s;
    if (s == null) return;
    String? dir;
    try {
      dir = await FilePicker.getDirectoryPath(
        dialogTitle: context.l10n.twitchArchiveFolderPick,
      );
    } catch (e) {
      debugPrint('Folder picker: $e');
    }
    if (dir == null) return;
    await _save(
      recordings
          ? s.copyWith(recordingsDir: dir)
          : s.copyWith(downloadsDir: dir),
    );
  }

  Future<void> _pickQuota(bool recordings) async {
    final s = _s;
    if (s == null) return;
    final l = context.l10n;
    const gb = 1024 * 1024 * 1024;
    final current = recordings ? s.recordingsQuotaBytes : s.downloadsQuotaBytes;
    const options = [0, 50, 100, 250, 500, 1000, 2000];
    final picked = await showDialog<int>(
      context: context,
      builder: (context) => SimpleDialog(
        title: Text(l.twitchArchiveQuota),
        children: [
          for (final o in options)
            SimpleDialogOption(
              key: ValueKey('twitch-archive-quota-$o'),
              onPressed: () => Navigator.of(context).pop(o),
              child: Text(
                o == 0 ? l.twitchArchiveQuotaNone : archiveSize(o * gb),
              ),
            ),
        ],
      ),
    );
    if (picked == null) return;
    final q = picked == 0 ? null : picked * gb;
    if (q == current) return;
    await _save(
      recordings
          ? s.copyWith(recordingsQuotaBytes: () => q)
          : s.copyWith(downloadsQuotaBytes: () => q),
    );
  }

  String _usageText(AppLocalizations l, bool recordings) {
    final u = _usage.where((u) => u.recordings == recordings).firstOrNull;
    if (u == null) return '';
    final used = archiveSize(u.used);
    final free = u.available == null ? null : archiveSize(u.available!);
    return [
      l.twitchArchiveUsed(used),
      if (u.quota case final q?) l.twitchArchiveQuotaOf(archiveSize(q)),
      if (free != null) l.twitchArchiveFree(free),
      if (u.low) l.twitchArchiveLow,
    ].join(' · ');
  }

  Widget _templateField(String key, String label, TextEditingController c) =>
      Padding(
        padding: const EdgeInsets.fromLTRB(12, 6, 12, 6),
        child: TextField(
          key: ValueKey(key),
          controller: c,
          decoration: InputDecoration(labelText: label),
          onChanged: (_) => _check(),
        ),
      );

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final s = _s;
    final theme = Theme.of(context);
    final pad = EdgeInsets.symmetric(horizontal: d.pageInsets.left);
    return Scaffold(
      key: const ValueKey('twitch-archive-settings-page'),
      backgroundColor: KColors.bgBase,
      body: ListView(
        padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 24),
        children: [
          PageTitle(l.twitchArchiveSettings),
          if (s != null) ...[
            Padding(
              padding: pad,
              child: SettingsSection(
                icon: Icons.folder_outlined,
                title: l.twitchArchiveStorage,
                subtitle: l.twitchArchiveStorageSubtitle,
                children: [
                  for (final rec in [false, true]) ...[
                    SettingsTile(
                      key: ValueKey(
                        'twitch-archive-dir-${rec ? 'rec' : 'vod'}',
                      ),
                      icon: rec
                          ? Icons.radio_button_checked
                          : Icons.download_outlined,
                      title: rec
                          ? l.twitchArchiveRecordingsFolder
                          : l.twitchArchiveDownloadsFolder,
                      subtitle: displayLocalPath(
                        rec ? s.recordingsDir : s.downloadsDir,
                      ),
                      subtitleLines: 2,
                      onTap: () => _pickDir(rec),
                    ),
                    SettingsTile(
                      key: ValueKey(
                        'twitch-archive-quota-${rec ? 'rec' : 'vod'}',
                      ),
                      icon: Icons.data_usage_outlined,
                      title: l.twitchArchiveQuota,
                      subtitle: _usageText(l, rec),
                      subtitleLines: 2,
                      onTap: () => _pickQuota(rec),
                    ),
                  ],
                ],
              ),
            ),
            Padding(
              padding: pad,
              child: SettingsSection(
                icon: Icons.drive_file_rename_outline,
                title: l.twitchArchiveNames,
                subtitle: l.twitchArchiveNamesSubtitle,
                children: [
                  _templateField(
                    'twitch-archive-template-folder',
                    l.twitchArchiveTemplateFolder,
                    _folder,
                  ),
                  _templateField(
                    'twitch-archive-template-vod',
                    l.twitchArchiveTemplateVod,
                    _vod,
                  ),
                  _templateField(
                    'twitch-archive-template-live',
                    l.twitchArchiveTemplateLive,
                    _live,
                  ),
                  Padding(
                    padding: const EdgeInsets.fromLTRB(12, 4, 12, 4),
                    child: _error != null
                        ? Text(
                            _error!,
                            key: const ValueKey(
                              'twitch-archive-template-error',
                            ),
                            style: theme.textTheme.bodySmall?.copyWith(
                              color: theme.colorScheme.error,
                            ),
                          )
                        : Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text(
                                l.twitchArchiveTemplateHelp,
                                style: theme.textTheme.labelSmall,
                              ),
                              if (_preview case final p?) ...[
                                const SizedBox(height: 6),
                                Text(
                                  p.$1,
                                  key: const ValueKey(
                                    'twitch-archive-preview-vod',
                                  ),
                                  style: theme.textTheme.bodySmall,
                                ),
                                Text(p.$2, style: theme.textTheme.bodySmall),
                              ],
                            ],
                          ),
                  ),
                  Padding(
                    padding: const EdgeInsets.all(12),
                    child: Wrap(
                      spacing: 12,
                      children: [
                        FilledButton(
                          key: const ValueKey('twitch-archive-template-save'),
                          onPressed: _error != null
                              ? null
                              : () => _save(
                                  s.copyWith(
                                    folderTemplate: _folder.text,
                                    vodTemplate: _vod.text,
                                    liveTemplate: _live.text,
                                  ),
                                ),
                          child: Text(l.commonSave),
                        ),
                        TextButton(
                          key: const ValueKey('twitch-archive-template-reset'),
                          onPressed: () {
                            _folder.text = TwitchArchiveSettings.defaultFolder;
                            _vod.text = TwitchArchiveSettings.defaultVod;
                            _live.text = TwitchArchiveSettings.defaultLive;
                            _check();
                          },
                          child: Text(l.twitchArchiveTemplateReset),
                        ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
            Padding(
              padding: pad,
              child: SettingsSection(
                icon: Icons.drive_folder_upload_outlined,
                title: l.twitchArchiveImport,
                subtitle: l.twitchArchiveImportSubtitle,
                children: [
                  SettingsTile(
                    key: const ValueKey('twitch-archive-settings-import'),
                    icon: Icons.folder_open_outlined,
                    title: l.twitchArchiveImportPick,
                    onTap: () => importTwitchFolder(context),
                  ),
                ],
              ),
            ),
          ],
        ],
      ),
    );
  }
}

/// Opens the recordings and downloads library.
Future<void> openTwitchArchive(BuildContext context) => Navigator.of(context)
    .push(MaterialPageRoute<void>(builder: (_) => const TwitchArchiveScreen()));

/// Opens the record rule of a favourite channel; the categories of its
/// recent VODs are the filter's choices.
Future<void> openTwitchRecordRule(
  BuildContext context, {
  required String userId,
  required String displayName,
  List<(String, String)> categories = const [],
}) => Navigator.of(context).push(
  MaterialPageRoute<void>(
    builder: (_) => TwitchRecordRuleScreen(
      userId: userId,
      displayName: displayName,
      categories: categories,
    ),
  ),
);

/// A positive number or empty ("keep all"); owns its controller so it
/// lives as long as the dialog's route.
class _NumberDialog extends StatefulWidget {
  const _NumberDialog({required this.title, this.current});

  final String title;
  final int? current;

  @override
  State<_NumberDialog> createState() => _NumberDialogState();
}

class _NumberDialogState extends State<_NumberDialog> {
  late final _controller = TextEditingController(
    text: widget.current?.toString() ?? '',
  );

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return AlertDialog(
      title: Text(widget.title),
      content: TextField(
        key: const ValueKey('twitch-rule-number'),
        controller: _controller,
        autofocus: true,
        keyboardType: TextInputType.number,
        decoration: InputDecoration(hintText: l.twitchRuleKeepAllHint),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('twitch-rule-number-ok'),
          onPressed: () {
            final n = int.tryParse(_controller.text.trim());
            Navigator.of(context).pop((n != null && n > 0 ? n : null,));
          },
          child: Text(l.commonSave),
        ),
      ],
    );
  }
}
