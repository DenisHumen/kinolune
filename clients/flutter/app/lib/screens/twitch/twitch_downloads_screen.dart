// SPDX-License-Identifier: MPL-2.0
//
// The Twitch downloads list (TWI-017, full builds only): state, progress
// and pause / resume / cancel / retry / remove of every download. Refreshes
// once a second while it is open.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../../l10n/l10n.dart';
import '../../services/app_services.dart';
import '../../theme/kinolune_theme.dart';
import '../../twitch/twitch_downloads.dart';
import '../../widgets/cards.dart';
import '../../widgets/undo_snack.dart';

class TwitchDownloadsScreen extends StatefulWidget {
  const TwitchDownloadsScreen({
    super.key,
    this.refresh = const Duration(seconds: 1),
  });

  final Duration refresh;

  @override
  State<TwitchDownloadsScreen> createState() => _TwitchDownloadsScreenState();
}

class _TwitchDownloadsScreenState extends State<TwitchDownloadsScreen> {
  List<TwitchDownloadItem>? _items;
  Timer? _timer;

  TwitchDownloads? get _downloads => Services.read(context).twitchDownloads;

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
    final items = await _downloads?.list() ?? const <TwitchDownloadItem>[];
    if (mounted) setState(() => _items = items);
  }

  /// Jobs removed from the list while «Отменить» is offered (QA-015).
  final _removing = <int>{};

  Future<void> _act(TwitchDownloadItem item, TwitchDownloadAction a) async {
    final downloads = _downloads;
    if (a == TwitchDownloadAction.remove) {
      // At once, with «Отменить»; the job goes when the snack bar closes.
      final l = context.l10n;
      setState(() => _removing.add(item.jobId));
      final undone = await showUndoSnack(
        ScaffoldMessenger.maybeOf(context),
        message: l.twitchDownloadRemoved,
        undoLabel: l.commonUndo,
      );
      if (!undone) await downloads?.control(item.jobId, a);
      if (!mounted) return;
      setState(() => _removing.remove(item.jobId));
    } else {
      await downloads?.control(item.jobId, a);
    }
    await _load();
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final items = _items;
    return Scaffold(
      key: const ValueKey('twitch-downloads-page'),
      backgroundColor: KColors.bgBase,
      body: ListView(
        padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 24),
        children: [
          PageTitle(l.twitchDownloads),
          if (items != null && items.isEmpty)
            Padding(
              padding: EdgeInsets.symmetric(
                horizontal: d.pageInsets.left,
                vertical: 24,
              ),
              child: Text(l.twitchDownloadsEmpty),
            ),
          for (final item in items ?? const <TwitchDownloadItem>[])
            if (!_removing.contains(item.jobId))
              _DownloadTile(item: item, onAction: (a) => _act(item, a)),
        ],
      ),
    );
  }
}

String _stateLabel(AppLocalizations l, TwitchDownloadState s) => switch (s) {
  TwitchDownloadState.queued => l.twitchDownloadQueued,
  TwitchDownloadState.waiting => l.twitchDownloadWaiting,
  TwitchDownloadState.running => l.twitchDownloadRunning,
  TwitchDownloadState.paused => l.twitchDownloadPaused,
  TwitchDownloadState.retrying => l.twitchDownloadRetrying,
  TwitchDownloadState.done => l.twitchDownloadDone,
  TwitchDownloadState.failed => l.twitchDownloadFailed,
  TwitchDownloadState.cancelled => l.twitchDownloadCancelled,
};

(IconData, String) _action(
  AppLocalizations l,
  TwitchDownloadAction a,
) => switch (a) {
  TwitchDownloadAction.pause => (Icons.pause_rounded, l.twitchDownloadPause),
  TwitchDownloadAction.resume => (
    Icons.play_arrow_rounded,
    l.twitchDownloadResume,
  ),
  TwitchDownloadAction.cancel => (Icons.close_rounded, l.twitchDownloadCancel),
  TwitchDownloadAction.retry => (Icons.refresh_rounded, l.twitchDownloadRetry),
  TwitchDownloadAction.remove => (
    Icons.delete_outline_rounded,
    l.twitchDownloadRemove,
  ),
};

String _size(int bytes) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB']; // l10n-ignore
  var v = bytes.toDouble();
  var u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return '${v.toStringAsFixed(u == 0 ? 0 : 1)} ${units[u]}';
}

class _DownloadTile extends StatelessWidget {
  const _DownloadTile({required this.item, required this.onAction});

  final TwitchDownloadItem item;
  final void Function(TwitchDownloadAction) onAction;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final title = item.file ?? 'v${item.videoId}'; // l10n-ignore
    final details = [
      _stateLabel(l, item.state),
      ?item.partLabel,
      if (item.bytesDone > 0) _size(item.bytesDone),
      ?item.waitingFor,
    ].join(' · ');
    return Padding(
      key: ValueKey('twitch-download-${item.jobId}'),
      padding: EdgeInsets.fromLTRB(d.pageInsets.left, 8, d.pageInsets.right, 8),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.titleSmall,
                ),
                const SizedBox(height: 2),
                Text(details, style: theme.textTheme.bodySmall),
                if (item.error case final error?)
                  Text(
                    error,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: theme.colorScheme.error,
                    ),
                  ),
                if (item.state.isActive ||
                    item.state == TwitchDownloadState.paused)
                  Padding(
                    padding: const EdgeInsets.only(top: 6),
                    child: KProgressBar(value: item.progress ?? 0),
                  ),
              ],
            ),
          ),
          for (final a in item.actions)
            IconButton(
              key: ValueKey('twitch-download-${item.jobId}-${a.name}'),
              tooltip: _action(l, a).$2,
              icon: Icon(_action(l, a).$1),
              onPressed: () => onAction(a),
            ),
        ],
      ),
    );
  }
}
