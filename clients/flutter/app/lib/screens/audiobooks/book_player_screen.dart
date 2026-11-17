// SPDX-License-Identifier: MPL-2.0
//
// MUS-018: the book player — cover, the chapter and where in it, ±30 s,
// previous and next chapter, the speed (0.5–3×), the sleep timer (minutes
// or «до конца главы») and the chapter list.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../../integration/player.dart';
import '../../l10n/l10n.dart';
import '../../services/app_services.dart';
import '../../theme/kinolune_theme.dart';
import '../../widgets/artwork.dart';
import '../../widgets/format.dart';

/// Opens the book player over everything.
void openBookPlayer(BuildContext context) {
  Navigator.of(context, rootNavigator: true).push(
    MaterialPageRoute<void>(
      fullscreenDialog: true,
      builder: (_) => const BookPlayerScreen(),
    ),
  );
}

/// `1.25` for 1.25, `2` for 2.0.
String speedLabel(double s) {
  final t = s.toStringAsFixed(2);
  return t.replaceFirst(RegExp(r'\.?0+$'), '');
}

/// The minutes the sleep timer offers.
const sleepMinutes = [5, 10, 15, 30, 45, 60];

class BookPlayerScreen extends StatefulWidget {
  const BookPlayerScreen({super.key});

  @override
  State<BookPlayerScreen> createState() => _BookPlayerScreenState();
}

class _BookPlayerScreenState extends State<BookPlayerScreen> {
  /// Where the slider is dragged to, in the chapter.
  double? _drag;

  /// Ticks the sleep countdown.
  Timer? _tick;

  @override
  void initState() {
    super.initState();
    _tick = Timer.periodic(const Duration(seconds: 1), (_) {
      final s = Services.read(context).audiobooks.player;
      if (mounted && s.sleepTimer?.duration != null) setState(() {});
    });
  }

  @override
  void dispose() {
    _tick?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final player = services.audiobooks.player;
    final l = context.l10n;
    return Scaffold(
      key: const ValueKey('book-player'),
      backgroundColor: KColors.bgBase,
      appBar: AppBar(
        backgroundColor: Colors.transparent,
        leading: IconButton(
          tooltip: l.audiobookClose,
          icon: const Icon(Icons.keyboard_arrow_down_rounded),
          onPressed: () => Navigator.of(context).maybePop(),
        ),
        actions: [
          IconButton(
            key: const ValueKey('book-chapters'),
            tooltip: l.audiobookChapters,
            icon: const Icon(Icons.format_list_bulleted_rounded),
            onPressed: () => _chapters(context, player),
          ),
        ],
      ),
      body: SafeArea(
        child: ListenableBuilder(
          listenable: player,
          builder: (context, _) {
            final book = player.book;
            if (book == null) return const SizedBox.shrink();
            return LayoutBuilder(
              builder: (context, c) {
                final wide = c.maxWidth > 760;
                final cover = ClipRRect(
                  borderRadius: BorderRadius.circular(12),
                  child: AspectRatio(
                    aspectRatio: 1,
                    child: Artwork(
                      url: book.artworkUri,
                      seed: book.key ?? book.title,
                      style: ArtStyle.album,
                      title: book.title,
                    ),
                  ),
                );
                final controls = _controls(context, player);
                if (wide) {
                  return Padding(
                    padding: const EdgeInsets.all(32),
                    child: Row(
                      children: [
                        Flexible(
                          child: ConstrainedBox(
                            constraints: const BoxConstraints(maxWidth: 420),
                            child: cover,
                          ),
                        ),
                        const SizedBox(width: 48),
                        Expanded(child: controls),
                      ],
                    ),
                  );
                }
                return SingleChildScrollView(
                  padding: const EdgeInsets.fromLTRB(24, 8, 24, 24),
                  child: Column(
                    children: [
                      ConstrainedBox(
                        constraints: BoxConstraints(
                          maxWidth: (c.maxHeight * .42).clamp(160, 360),
                        ),
                        child: cover,
                      ),
                      const SizedBox(height: 24),
                      controls,
                    ],
                  ),
                );
              },
            );
          },
        ),
      ),
    );
  }

  Widget _controls(BuildContext context, BookPlayerController player) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final book = player.book!;
    final chapters = player.chapters;
    final i = player.chapterIndex;
    final (start, end) = player.chapterBounds(i);
    final length = (end - start).clamp(0, 1 << 40);
    final inChapter = (player.position.inMilliseconds - start).clamp(0, length);
    final shown = _drag?.round() ?? inChapter;
    final chapterTitle = chapters[i].title ?? l.audiobookChapter(i + 1);
    final left =
        player.duration.inMilliseconds - player.position.inMilliseconds;
    final sleep = player.sleepTimer;
    final sleepLeft = player.sleepLeft;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          book.title,
          key: const ValueKey('book-title'),
          style: theme.textTheme.headlineSmall,
          maxLines: 2,
          overflow: TextOverflow.ellipsis,
          textAlign: TextAlign.center,
        ),
        if (book.author != null)
          Text(
            book.author!,
            style: theme.textTheme.bodyMedium?.copyWith(
              color: KColors.textSecondary,
            ),
            textAlign: TextAlign.center,
          ),
        const SizedBox(height: 16),
        Text(
          chapterTitle,
          key: const ValueKey('book-chapter-title'),
          style: theme.textTheme.titleMedium,
          maxLines: 2,
          overflow: TextOverflow.ellipsis,
          textAlign: TextAlign.center,
        ),
        Text(
          l.audiobookPartOf(i + 1, chapters.length),
          style: theme.textTheme.labelSmall,
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 8),
        Slider(
          key: const ValueKey('book-slider'),
          value: shown.toDouble().clamp(0, length.toDouble()),
          max: length <= 0 ? 1 : length.toDouble(),
          onChanged: length <= 0 ? null : (v) => setState(() => _drag = v),
          onChangeEnd: length <= 0
              ? null
              : (v) {
                  setState(() => _drag = null);
                  player.seek(Duration(milliseconds: start + v.round()));
                },
        ),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16),
          child: Row(
            children: [
              Text(formatTimecode(shown), style: theme.textTheme.labelSmall),
              const Spacer(),
              if (left > 0)
                Text(
                  l.audiobookLeft(formatTimecode(left)),
                  key: const ValueKey('book-left'),
                  style: theme.textTheme.labelSmall,
                ),
              const Spacer(),
              Text(
                '-${formatTimecode(length - shown)}',
                style: theme.textTheme.labelSmall,
              ),
            ],
          ),
        ),
        const SizedBox(height: 12),
        FittedBox(
          fit: BoxFit.scaleDown,
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              IconButton(
                key: const ValueKey('book-prev-chapter'),
                tooltip: l.audiobookPreviousChapter,
                iconSize: 32,
                icon: const Icon(Icons.skip_previous_rounded),
                onPressed: player.previousChapter,
              ),
              const SizedBox(width: 8),
              IconButton(
                key: const ValueKey('book-back30'),
                tooltip: l.audiobookBack30,
                iconSize: 36,
                icon: const Icon(Icons.replay_30_rounded),
                onPressed: player.back,
              ),
              const SizedBox(width: 12),
              IconButton.filled(
                key: const ValueKey('book-play'),
                tooltip: player.playing ? l.audiobookPause : l.audiobookListen,
                iconSize: 44,
                style: IconButton.styleFrom(
                  backgroundColor: KColors.textPrimary,
                  foregroundColor: KColors.bgBase,
                  minimumSize: const Size(72, 72),
                ),
                icon: Icon(
                  player.playing
                      ? Icons.pause_rounded
                      : Icons.play_arrow_rounded,
                ),
                onPressed: player.togglePlay,
              ),
              const SizedBox(width: 12),
              IconButton(
                key: const ValueKey('book-forward30'),
                tooltip: l.audiobookForward30,
                iconSize: 36,
                icon: const Icon(Icons.forward_30_rounded),
                onPressed: player.forward,
              ),
              const SizedBox(width: 8),
              IconButton(
                key: const ValueKey('book-next-chapter'),
                tooltip: l.audiobookNextChapter,
                iconSize: 32,
                icon: const Icon(Icons.skip_next_rounded),
                onPressed: i + 1 < chapters.length ? player.nextChapter : null,
              ),
            ],
          ),
        ),
        const SizedBox(height: 16),
        Wrap(
          alignment: WrapAlignment.center,
          spacing: 12,
          runSpacing: 8,
          children: [
            TextButton.icon(
              key: const ValueKey('book-speed'),
              icon: const Icon(Icons.speed_rounded),
              label: Text(l.audiobookSpeedValue(speedLabel(player.speed))),
              onPressed: () => _speed(context),
            ),
            TextButton.icon(
              key: const ValueKey('book-sleep'),
              icon: Icon(
                sleep == null ? Icons.bedtime_outlined : Icons.bedtime_rounded,
              ),
              label: Text(
                sleep == null
                    ? l.audiobookSleepTimer
                    : sleep.endOfChapter
                    ? l.audiobookSleepEndOfChapter
                    : l.audiobookSleepIn(
                        formatTimecode(sleepLeft?.inMilliseconds ?? 0),
                      ),
              ),
              onPressed: () => _sleep(context, player),
            ),
          ],
        ),
      ],
    );
  }

  Future<void> _speed(BuildContext context) async {
    final service = Services.read(context).audiobooks;
    final l = context.l10n;
    await showModalBottomSheet<void>(
      context: context,
      backgroundColor: KColors.bgElevated,
      builder: (context) => SafeArea(
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              ListTile(
                title: Text(
                  l.audiobookSpeed,
                  style: Theme.of(context).textTheme.titleMedium,
                ),
              ),
              for (final s in BookPlayerController.speeds)
                ListTile(
                  key: ValueKey('book-speed-${speedLabel(s)}'),
                  title: Text(l.audiobookSpeedValue(speedLabel(s))),
                  trailing: s == service.player.speed
                      ? const Icon(Icons.check_rounded, color: KColors.accent)
                      : null,
                  onTap: () {
                    service.setSpeed(s);
                    Navigator.of(context).pop();
                  },
                ),
            ],
          ),
        ),
      ),
    );
  }

  Future<void> _sleep(BuildContext context, BookPlayerController player) async {
    final l = context.l10n;
    final current = player.sleepTimer;
    Widget option(String key, String label, SleepTimer? value) => ListTile(
      key: ValueKey('book-sleep-$key'),
      title: Text(label),
      trailing: value == current
          ? const Icon(Icons.check_rounded, color: KColors.accent)
          : null,
      onTap: () {
        player.setSleepTimer(value);
        Navigator.of(context).pop();
      },
    );
    await showModalBottomSheet<void>(
      context: context,
      backgroundColor: KColors.bgElevated,
      builder: (context) => SafeArea(
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              ListTile(
                title: Text(
                  l.audiobookSleepTimer,
                  style: Theme.of(context).textTheme.titleMedium,
                ),
              ),
              option('off', l.audiobookSleepOff, null),
              for (final m in sleepMinutes)
                option(
                  '$m',
                  l.audiobookSleepMinutes(m),
                  SleepTimer.after(Duration(minutes: m)),
                ),
              option(
                'end-of-chapter',
                l.audiobookSleepEndOfChapter,
                const SleepTimer.endOfChapter(),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Future<void> _chapters(
    BuildContext context,
    BookPlayerController player,
  ) async {
    final l = context.l10n;
    final chapters = player.chapters;
    final current = player.chapterIndex;
    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      backgroundColor: KColors.bgElevated,
      builder: (context) => SafeArea(
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxHeight: MediaQuery.sizeOf(context).height * .7,
          ),
          child: ListView.builder(
            key: const ValueKey('book-chapter-list'),
            shrinkWrap: true,
            itemCount: chapters.length + 1,
            itemBuilder: (context, n) {
              if (n == 0) {
                return ListTile(
                  title: Text(
                    l.audiobookChapters,
                    style: Theme.of(context).textTheme.titleMedium,
                  ),
                );
              }
              final i = n - 1;
              final (start, end) = player.chapterBounds(i);
              return ListTile(
                key: ValueKey('book-chapter-$i'),
                selected: i == current,
                selectedColor: KColors.accent,
                leading: Text('${i + 1}'),
                title: Text(chapters[i].title ?? l.audiobookChapter(i + 1)),
                trailing: end > start
                    ? Text(formatTimecode(end - start))
                    : null,
                onTap: () {
                  player.jumpToChapter(i);
                  Navigator.of(context).pop();
                },
              );
            },
          ),
        ),
      ),
    );
  }
}
