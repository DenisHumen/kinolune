// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/material.dart';

import '../l10n/pa_player_strings.dart';
import '../play_request.dart';
import '../ui/focus_button.dart';
import '../ui/tokens.dart';
import 'artwork.dart';
import 'music_player_controller.dart';
import 'music_queue.dart';

/// Saves the tracks of the queue as a playlist (the app knows the
/// library; MUS-003 "Save as playlist").
typedef SaveQueueCallback =
    Future<void> Function(BuildContext context, List<PlayRequest> tracks);

/// The queue panel of Now Playing (01 §4.11): "Playing next" (Play Next /
/// Add to Queue), "Up next from: <source>", drag to reorder, swipe or ✕ to
/// remove, clear, save as playlist; and the History tab.
class NowPlayingQueue extends StatefulWidget {
  const NowPlayingQueue({super.key, required this.controller, this.onSave});

  final MusicPlayerController controller;

  /// Shows "Save as playlist" when set.
  final SaveQueueCallback? onSave;

  @override
  State<NowPlayingQueue> createState() => _NowPlayingQueueState();
}

class _NowPlayingQueueState extends State<NowPlayingQueue> {
  bool _history = false;

  MusicPlayerController get c => widget.controller;

  @override
  Widget build(BuildContext context) {
    final l = PaPlayerStrings.of(context);
    final save = widget.onSave;
    return FocusTraversalGroup(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              _Tab(
                key: const ValueKey('np-tab-queue'),
                label: l.queueTab,
                selected: !_history,
                onPressed: () => setState(() => _history = false),
              ),
              _Tab(
                key: const ValueKey('np-tab-history'),
                label: l.historyTab,
                selected: _history,
                onPressed: () => setState(() => _history = true),
              ),
              const Spacer(),
              if (!_history && save != null)
                FocusIconButton(
                  key: const ValueKey('np-save'),
                  icon: Icons.playlist_add_rounded,
                  tooltip: l.saveAsPlaylist,
                  size: 40,
                  onPressed: c.hasQueue
                      ? () => save(context, c.queue.ordered)
                      : null,
                ),
              if (!_history)
                FocusIconButton(
                  key: const ValueKey('np-clear'),
                  icon: Icons.clear_all_rounded,
                  tooltip: l.clearQueue,
                  size: 40,
                  onPressed: c.queue.upNext.isEmpty ? null : c.clearUpNext,
                ),
            ],
          ),
          const SizedBox(height: 4),
          Expanded(child: _history ? _historyList(l) : _queueList(l)),
        ],
      ),
    );
  }

  Widget _message(String text) => Padding(
    padding: const EdgeInsets.all(14),
    child: Text(text, style: const TextStyle(color: PaTokens.textSecondary)),
  );

  Widget _historyList(PaPlayerStrings l) {
    final recent = c.recent;
    if (recent.isEmpty) return _message(l.historyEmpty);
    return ListView.builder(
      key: const ValueKey('np-history'),
      itemCount: recent.length,
      itemBuilder: (context, i) {
        final t = recent[i];
        return FocusListRow(
          key: ValueKey('history-$i'),
          title: t.title,
          subtitle: t.artist,
          leading: Artwork(request: t, size: 40, radius: 4),
          trailing: const SizedBox.shrink(),
          onPressed: () => c.playNow(t),
        );
      },
    );
  }

  Widget _queueList(PaPlayerStrings l) {
    final up = c.queue.upNextEntries;
    if (up.isEmpty) return _message(l.endOfQueue);
    final queued = c.queue.queuedCount;
    final source = c.source;
    return CustomScrollView(
      key: const ValueKey('np-queue'),
      slivers: [
        if (queued > 0) ...[
          _header(l.playingNext, 'np-section-queued'),
          _section(l, up, 0, queued),
        ],
        if (queued < up.length) ...[
          _header(
            source == null ? l.upNext : l.upNextFrom(source.label),
            'np-section-source',
          ),
          _section(l, up, queued, up.length),
        ],
      ],
    );
  }

  Widget _header(String text, String key) => SliverToBoxAdapter(
    child: Padding(
      padding: const EdgeInsets.fromLTRB(14, 12, 14, 6),
      child: Text(
        text,
        key: ValueKey(key),
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: const TextStyle(
          color: PaTokens.text,
          fontSize: 16,
          fontWeight: FontWeight.w700,
        ),
      ),
    ),
  );

  /// Up-next entries [from]..[to] as a list reorderable within itself.
  Widget _section(PaPlayerStrings l, List<QueueEntry> up, int from, int to) {
    final base = c.queue.position + 1;
    return SliverReorderableList(
      itemCount: to - from,
      onReorder: (a, b) {
        // [b] counts the dragged row itself when it moves down.
        if (b > a) b--;
        c.moveUpNext(from + a, from + b);
      },
      itemBuilder: (context, i) {
        final index = from + i;
        final e = up[index];
        final t = e.request;
        return Dismissible(
          key: ValueKey('queue-entry-${e.key}'),
          direction: DismissDirection.endToStart,
          onDismissed: (_) => c.removeUpNext(index),
          background: Container(
            color: const Color(0x55FF4D4D),
            alignment: Alignment.centerRight,
            padding: const EdgeInsets.only(right: 20),
            child: const Icon(Icons.delete_outline_rounded),
          ),
          child: FocusListRow(
            key: ValueKey('queue-${base + index}'),
            title: t.title,
            subtitle: t.artist,
            leading: Artwork(request: t, size: 40, radius: 4),
            trailing: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                FocusIconButton(
                  key: ValueKey('queue-remove-${base + index}'),
                  icon: Icons.close_rounded,
                  tooltip: l.removeFromQueue,
                  size: 36,
                  onPressed: () => c.removeUpNext(index),
                ),
                ReorderableDragStartListener(
                  key: ValueKey('queue-drag-${base + index}'),
                  index: i,
                  child: Tooltip(
                    message: l.dragToReorder,
                    child: const Padding(
                      padding: EdgeInsets.all(6),
                      child: Icon(
                        Icons.drag_handle_rounded,
                        color: PaTokens.textSecondary,
                      ),
                    ),
                  ),
                ),
              ],
            ),
            onPressed: () => c.jumpTo(base + index),
          ),
        );
      },
    );
  }
}

class _Tab extends StatelessWidget {
  const _Tab({
    super.key,
    required this.label,
    required this.selected,
    required this.onPressed,
  });

  final String label;
  final bool selected;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => TextButton(
    onPressed: onPressed,
    style: TextButton.styleFrom(
      foregroundColor: selected ? PaTokens.text : PaTokens.textSecondary,
    ),
    child: Text(
      label,
      style: TextStyle(
        fontSize: 18,
        fontWeight: selected ? FontWeight.w700 : FontWeight.w500,
      ),
    ),
  );
}
