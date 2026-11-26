// SPDX-License-Identifier: MPL-2.0
//
// TWI-026: the «Медиатека» tab of the Twitch archive — saved streams as a
// library, streamer → game → parts ([TwitchArchiveLibrary]).
//
// - Every streamer is a heading with a row of playthroughs (box art,
//   «Прохождение X», the number of parts, where the user is). The row is
//   the UI-055 episode row: ‹ › on desktop, swipe on a phone, ←/→ on a TV.
// - Search (streamer, game, title, date) and filters by streamer, game and
//   a day of the stream calendar turn the shelves into a list of the parts
//   found, each with its playthrough and part number.
// - A playthrough opens its page: the parts in date order as a row that
//   opens on the part to watch next, «Продолжить · часть N». A part plays
//   from where it was left, else where its game starts (TWI-027), and «Далее»
//   at its end is the next part.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../../integration/library.dart' show Progress;
import '../../integration/player.dart';
import '../../l10n/l10n.dart';
import '../../services/app_services.dart';
import '../../theme/kinolune_theme.dart';
import '../../twitch/twitch_archive.dart';
import '../../twitch/twitch_archive_library.dart';
import '../../twitch/twitch_models.dart' show twitchBoxArtTemplate, twitchImage;
import '../../widgets/artwork.dart';
import '../../widgets/focusable.dart';
import '../episode_row.dart';
import '../library_grid_screen.dart' show FilterPill;
import '../playback.dart' show ProgressWriter;
import 'twitch_archive_screens.dart' show archiveDuration;

/// The title of a playthrough: «Прохождение X», «Прохождение X (2)».
String playthroughTitle(AppLocalizations l, TwitchPlaythrough p) {
  final game = p.gameName ?? l.twitchArchiveUnknownGame(p.gameId);
  return p.run > 1
      ? l.twitchArchivePlaythroughRun(game, p.run)
      : l.twitchArchivePlaythrough(game);
}

String _day(BuildContext context, DateTime? t) => t == null
    ? ''
    : MaterialLocalizations.of(context).formatMediumDate(t.toLocal());

/// Loads the progress of [files] by [TwitchSavedFile.progressKey].
Future<Map<String, Progress>> loadArchiveProgress(
  AppServices services,
  Iterable<TwitchSavedFile> files,
) async {
  final keys = {for (final f in files) ?f.progressKey};
  final out = <String, Progress>{};
  await Future.wait([
    for (final k in keys)
      services.library.progress(k).then((p) {
        if (p != null) out[k] = p;
      }, onError: (_) {}),
  ]);
  return out;
}

/// The «Медиатека» tab.
class TwitchArchiveLibraryView extends StatefulWidget {
  const TwitchArchiveLibraryView({
    super.key,
    required this.files,
    this.loaded = true,
  });

  /// Every saved file (any kind; clips are left out).
  final List<TwitchSavedFile> files;
  final bool loaded;

  @override
  State<TwitchArchiveLibraryView> createState() =>
      _TwitchArchiveLibraryViewState();
}

class _TwitchArchiveLibraryViewState extends State<TwitchArchiveLibraryView> {
  TwitchArchiveLibrary _lib = TwitchArchiveLibrary.empty;
  TwitchArchiveFilter _filter = const TwitchArchiveFilter();
  Map<String, Progress> _progress = const {};
  final _search = TextEditingController();
  List<TwitchSavedFile>? _built;

  @override
  void initState() {
    super.initState();
    _rebuild();
  }

  @override
  void didUpdateWidget(TwitchArchiveLibraryView old) {
    super.didUpdateWidget(old);
    _rebuild();
  }

  @override
  void dispose() {
    _search.dispose();
    super.dispose();
  }

  /// Regroups when the files changed (the archive polls every few seconds).
  void _rebuild() {
    final files = widget.files;
    final prev = _built;
    if (prev != null &&
        prev.length == files.length &&
        Iterable<int>.generate(files.length).every(
          (i) =>
              prev[i].assetId == files[i].assetId &&
              prev[i].games.length == files[i].games.length,
        )) {
      return;
    }
    _built = files;
    _lib = TwitchArchiveLibrary.build(files);
    unawaited(_loadProgress());
  }

  Future<void> _loadProgress() async {
    final p = await loadArchiveProgress(Services.read(context), widget.files);
    if (mounted) setState(() => _progress = p);
  }

  Future<void> _open(TwitchPlaythrough p) async {
    await openTwitchPlaythrough(context, p);
    await _loadProgress();
  }

  Future<void> _pickDay() async {
    final days = _lib.days;
    if (days.isEmpty) return;
    final day = await showArchiveCalendar(context, days, selected: _filter.day);
    if (day != null && mounted) {
      setState(() => _filter = _filter.copyWith(day: () => day));
    }
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final pad = EdgeInsets.symmetric(horizontal: d.pageInsets.left);
    final lib = _lib;
    final f = _filter;
    if (widget.loaded && lib.streamers.isEmpty) {
      return Padding(
        padding: pad.add(const EdgeInsets.symmetric(vertical: 24)),
        child: Text(
          l.twitchArchiveEmptyLibrary,
          key: const ValueKey('twitch-archive-library-empty'),
          style: Theme.of(context).textTheme.bodyMedium,
        ),
      );
    }
    final games = lib.games(userId: f.userId);
    final day = f.day;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: pad,
          child: Row(
            children: [
              Expanded(
                child: TextField(
                  key: const ValueKey('twitch-archive-search'),
                  controller: _search,
                  decoration: InputDecoration(
                    prefixIcon: const Icon(Icons.search_rounded),
                    hintText: l.twitchArchiveSearchHint,
                    isDense: true,
                  ),
                  onChanged: (q) =>
                      setState(() => _filter = _filter.copyWith(query: q)),
                ),
              ),
              const SizedBox(width: 8),
              IconButton(
                key: const ValueKey('twitch-archive-calendar'),
                tooltip: l.twitchArchiveCalendar,
                icon: const Icon(Icons.calendar_month_outlined),
                onPressed: _pickDay,
              ),
            ],
          ),
        ),
        const SizedBox(height: 10),
        if (lib.streamers.length > 1 || day != null)
          _Pills(
            children: [
              if (day != null)
                FilterPill(
                  key: const ValueKey('twitch-archive-day-chip'),
                  label: _day(context, day),
                  icon: Icons.close_rounded,
                  selected: true,
                  onTap: () => setState(
                    () => _filter = _filter.copyWith(day: () => null),
                  ),
                ),
              if (lib.streamers.length > 1) ...[
                FilterPill(
                  key: const ValueKey('twitch-archive-streamer-all'),
                  label: l.twitchArchiveAllStreamers,
                  selected: f.userId == null,
                  onTap: () => setState(
                    () => _filter = _filter.copyWith(
                      userId: () => null,
                      gameId: () => null,
                    ),
                  ),
                ),
                for (final s in lib.streamers)
                  FilterPill(
                    key: ValueKey('twitch-archive-streamer-${s.userId}'),
                    label: s.channel,
                    selected: f.userId == s.userId,
                    onTap: () => setState(
                      () => _filter = _filter.copyWith(
                        userId: () => s.userId,
                        gameId: () => null,
                      ),
                    ),
                  ),
              ],
            ],
          ),
        if (games.length > 1)
          _Pills(
            children: [
              FilterPill(
                key: const ValueKey('twitch-archive-game-all'),
                label: l.twitchArchiveAllGames,
                selected: f.gameId == null,
                onTap: () => setState(
                  () => _filter = _filter.copyWith(gameId: () => null),
                ),
              ),
              for (final (id, name) in games)
                FilterPill(
                  key: ValueKey('twitch-archive-game-$id'),
                  label: name ?? l.twitchArchiveUnknownGame(id),
                  selected: f.gameId == id,
                  onTap: () => setState(
                    () => _filter = _filter.copyWith(gameId: () => id),
                  ),
                ),
            ],
          ),
        const SizedBox(height: 4),
        if (f.query.trim().isNotEmpty || f.day != null || f.gameId != null)
          ..._results(context, lib.search(f))
        else
          for (final s in lib.streamers)
            if (f.userId == null || f.userId == s.userId) ..._shelf(context, s),
      ],
    );
  }

  List<Widget> _results(
    BuildContext context,
    ({
      List<(TwitchPlaythrough, TwitchArchivePart)> parts,
      List<TwitchSavedFile> other,
    })
    r,
  ) {
    final l = context.l10n;
    if (r.parts.isEmpty && r.other.isEmpty) {
      return [
        Padding(
          padding: EdgeInsets.symmetric(
            horizontal: KDensity.of(context).pageInsets.left,
            vertical: 16,
          ),
          child: Text(
            l.twitchArchiveNoMatches,
            key: const ValueKey('twitch-archive-no-matches'),
          ),
        ),
      ];
    }
    return [
      for (final (run, part) in r.parts)
        _PartTile(
          key: ValueKey('twitch-archive-result-${part.file.assetId}'),
          title: playthroughTitle(l, run),
          subtitle: [
            l.twitchArchivePartOf(part.number, run.parts.length),
            run.channel,
            _day(context, part.file.recordedAt),
          ].where((s) => s.isNotEmpty).join(' · '),
          progress: _progress[part.file.progressKey],
          art: run.boxArtUrl ?? twitchBoxArtTemplate(run.gameId),
          seed: 'category-${run.gameId}',
          onTap: () async {
            await playArchivePart(
              context,
              run,
              run.parts.indexOf(part),
              progress: _progress[part.file.progressKey],
            );
            await _loadProgress();
          },
        ),
      for (final file in r.other) _otherTile(context, file),
    ];
  }

  Widget _otherTile(BuildContext context, TwitchSavedFile file) => _PartTile(
    key: ValueKey('twitch-archive-result-${file.assetId}'),
    title: file.title,
    subtitle: [
      file.channel,
      _day(context, file.recordedAt),
      if (file.duration case final du?) archiveDuration(du),
    ].where((s) => s.isNotEmpty).join(' · '),
    progress: _progress[file.progressKey],
    art: null,
    seed: 'stream-${file.assetId}',
    onTap: () async {
      await playArchiveFile(
        context,
        file,
        progress: _progress[file.progressKey],
      );
      await _loadProgress();
    },
  );

  List<Widget> _shelf(BuildContext context, TwitchArchiveStreamer s) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final width = d.isTv ? 180.0 : (d.isPhone ? 132.0 : 160.0);
    final textScale = MediaQuery.textScalerOf(context).scale(1);
    return [
      Padding(
        key: ValueKey('twitch-archive-shelf-${s.userId}'),
        padding: EdgeInsets.fromLTRB(
          d.pageInsets.left,
          16,
          d.pageInsets.right,
          8,
        ),
        child: Text(s.channel, style: theme.textTheme.titleMedium),
      ),
      if (s.playthroughs.isNotEmpty)
        EpisodeRow(
          key: ValueKey('twitch-archive-row-${s.userId}'),
          count: s.playthroughs.length,
          cardWidth: width,
          cardHeight: width * 4 / 3 + 64 * textScale,
          itemBuilder: (context, i, node) {
            final p = s.playthroughs[i];
            final next = nextPartIndex(p.parts, _progress);
            final started = p.parts.any(
              (x) => _progress.containsKey(x.file.progressKey),
            );
            return _PlaythroughCard(
              key: ValueKey('twitch-playthrough-${p.id}'),
              focusNode: node,
              playthrough: p,
              width: width,
              status: started
                  ? l.twitchArchivePartOf(next + 1, p.parts.length)
                  : l.twitchArchiveParts(p.parts.length),
              onTap: () => _open(p),
            );
          },
        ),
      if (s.other.isNotEmpty) ...[
        Padding(
          padding: EdgeInsets.fromLTRB(
            d.pageInsets.left,
            12,
            d.pageInsets.right,
            4,
          ),
          child: Text(
            l.twitchArchiveOtherStreams,
            style: theme.textTheme.labelLarge,
          ),
        ),
        for (final f in s.other) _otherTile(context, f),
      ],
    ];
  }
}

class _Pills extends StatelessWidget {
  const _Pills({required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    return Padding(
      padding: EdgeInsets.fromLTRB(d.pageInsets.left, 0, d.pageInsets.right, 8),
      child: Wrap(spacing: 8, runSpacing: 8, children: children),
    );
  }
}

/// A playthrough on a streamer's shelf: box art, title, parts.
class _PlaythroughCard extends StatelessWidget {
  const _PlaythroughCard({
    super.key,
    required this.playthrough,
    required this.width,
    required this.status,
    required this.onTap,
    this.focusNode,
  });

  final TwitchPlaythrough playthrough;
  final double width;
  final String status;
  final VoidCallback onTap;
  final FocusNode? focusNode;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final p = playthrough;
    final title = playthroughTitle(l, p);
    return SizedBox(
      width: width,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          KFocusable(
            focusNode: focusNode,
            onActivate: onTap,
            semanticLabel: '$title, $status',
            builder: (context, focused) => AspectRatio(
              aspectRatio: 3 / 4,
              child: Artwork(
                url: twitchImage(
                  p.boxArtUrl ?? twitchBoxArtTemplate(p.gameId),
                  285,
                  380,
                ),
                seed: 'category-${p.gameId}',
                style: ArtStyle.poster,
              ),
            ),
          ),
          const SizedBox(height: 6),
          GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: onTap,
            excludeFromSemantics: true,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  title,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.titleSmall,
                ),
                Text(
                  status,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.labelSmall,
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// A found part or a stream without a game: a line with a progress bar.
class _PartTile extends StatelessWidget {
  const _PartTile({
    super.key,
    required this.title,
    required this.subtitle,
    required this.progress,
    required this.art,
    required this.seed,
    required this.onTap,
  });

  final String title;
  final String subtitle;
  final Progress? progress;
  final String? art;
  final String seed;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    final p = progress;
    return KFocusable(
      onActivate: onTap,
      radius: 8,
      light: false,
      semanticLabel: '$title, $subtitle',
      builder: (context, focused) => Padding(
        padding: EdgeInsets.fromLTRB(
          d.pageInsets.left,
          6,
          d.pageInsets.right,
          6,
        ),
        child: Row(
          children: [
            SizedBox(
              width: 42,
              height: 56,
              child: art == null
                  ? Icon(
                      Icons.radio_button_checked,
                      color: KColors.textSecondary,
                    )
                  : Artwork(
                      url: twitchImage(art, 90, 120),
                      seed: seed,
                      style: ArtStyle.poster,
                    ),
            ),
            const SizedBox(width: 12),
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
                  Text(
                    subtitle,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.bodySmall,
                  ),
                  if (p != null) ...[
                    const SizedBox(height: 6),
                    _ProgressBar(progress: p),
                  ],
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ProgressBar extends StatelessWidget {
  const _ProgressBar({required this.progress});

  final Progress progress;

  @override
  Widget build(BuildContext context) {
    final p = progress;
    final v = p.watched
        ? 1.0
        : p.durationMs <= 0
        ? 0.0
        : (p.positionMs / p.durationMs).clamp(0.0, 1.0);
    return ClipRRect(
      borderRadius: BorderRadius.circular(2),
      child: LinearProgressIndicator(
        value: v,
        minHeight: 3,
        backgroundColor: Colors.white.withValues(alpha: .12),
        color: KColors.accent,
      ),
    );
  }
}

// ------------------------------------------------------------ calendar

/// The stream calendar: only days with a saved stream can be picked.
Future<DateTime?> showArchiveCalendar(
  BuildContext context,
  Set<DateTime> days, {
  DateTime? selected,
}) {
  final sorted = days.toList()..sort();
  final initial = selected != null && days.contains(selected)
      ? selected
      : sorted.last;
  return showDialog<DateTime>(
    context: context,
    builder: (context) => AlertDialog(
      key: const ValueKey('twitch-archive-calendar-dialog'),
      title: Text(context.l10n.twitchArchiveCalendar),
      content: SizedBox(
        width: 340,
        height: 360,
        child: CalendarDatePicker(
          initialDate: initial,
          firstDate: DateTime(sorted.first.year, sorted.first.month),
          lastDate: DateTime(sorted.last.year, sorted.last.month + 1, 0),
          selectableDayPredicate: (d) =>
              days.contains(DateTime(d.year, d.month, d.day)),
          onDateChanged: (d) =>
              Navigator.of(context).pop(DateTime(d.year, d.month, d.day)),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(MaterialLocalizations.of(context).cancelButtonLabel),
        ),
      ],
    ),
  );
}

// ------------------------------------------------------------ playthrough

Future<void> openTwitchPlaythrough(
  BuildContext context,
  TwitchPlaythrough playthrough,
) => Navigator.of(context).push(
  MaterialPageRoute<void>(
    builder: (_) => TwitchPlaythroughScreen(playthrough: playthrough),
  ),
);

/// A playthrough: its parts in order, «Продолжить · часть N».
class TwitchPlaythroughScreen extends StatefulWidget {
  const TwitchPlaythroughScreen({super.key, required this.playthrough});

  final TwitchPlaythrough playthrough;

  @override
  State<TwitchPlaythroughScreen> createState() =>
      _TwitchPlaythroughScreenState();
}

class _TwitchPlaythroughScreenState extends State<TwitchPlaythroughScreen> {
  Map<String, Progress>? _progress;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => unawaited(_load()));
  }

  Future<void> _load() async {
    final p = await loadArchiveProgress(Services.read(context), [
      for (final x in widget.playthrough.parts) x.file,
    ]);
    if (mounted) setState(() => _progress = p);
  }

  Future<void> _play(int index) async {
    final run = widget.playthrough;
    await playArchivePart(
      context,
      run,
      index,
      progress: _progress?[run.parts[index].file.progressKey],
    );
    await _load();
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final run = widget.playthrough;
    final progress = _progress ?? const <String, Progress>{};
    final next = nextPartIndex(run.parts, progress);
    final started = progress.isNotEmpty;
    final range = [
      _day(context, run.firstAt),
      if (run.parts.length > 1) _day(context, run.lastAt),
    ].where((s) => s.isNotEmpty).join(' – ');
    final cardWidth = d.isPhone ? 260.0 : (d.isTv ? 260.0 : 280.0);
    final textScale = MediaQuery.textScalerOf(context).scale(1);
    return Scaffold(
      key: const ValueKey('twitch-playthrough-page'),
      backgroundColor: KColors.bgBase,
      appBar: AppBar(backgroundColor: KColors.bgBase),
      body: ListView(
        padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 24),
        children: [
          Padding(
            padding: EdgeInsets.fromLTRB(
              d.pageInsets.left,
              0,
              d.pageInsets.right,
              16,
            ),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SizedBox(
                  width: d.isPhone ? 96 : 140,
                  child: AspectRatio(
                    aspectRatio: 3 / 4,
                    child: Artwork(
                      url: twitchImage(
                        run.boxArtUrl ?? twitchBoxArtTemplate(run.gameId),
                        285,
                        380,
                      ),
                      seed: 'category-${run.gameId}',
                      style: ArtStyle.poster,
                    ),
                  ),
                ),
                const SizedBox(width: 16),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        playthroughTitle(l, run),
                        style: theme.textTheme.headlineSmall,
                      ),
                      const SizedBox(height: 4),
                      Text(run.channel, style: theme.textTheme.titleSmall),
                      const SizedBox(height: 4),
                      Text(
                        [
                          l.twitchArchiveParts(run.parts.length),
                          if (range.isNotEmpty) range,
                          if (run.gameTime > Duration.zero)
                            l.twitchArchiveGameTime(run.gameTime.inHours),
                        ].join(' · '),
                        style: theme.textTheme.bodySmall,
                      ),
                      const SizedBox(height: 12),
                      FilledButton.icon(
                        key: const ValueKey('twitch-playthrough-continue'),
                        onPressed: () => _play(next),
                        icon: const Icon(Icons.play_arrow_rounded),
                        label: Text(
                          started
                              ? l.twitchArchiveContinuePart(next + 1)
                              : l.twitchArchiveStartPart(next + 1),
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
          EpisodeRow(
            key: const ValueKey('twitch-playthrough-parts'),
            count: run.parts.length,
            cardWidth: cardWidth,
            cardHeight: cardWidth * 9 / 16 + 76 * textScale,
            startIndex: _progress == null ? null : next,
            rangeLabel: (first, last) => (
              l.showEpisodeRange(first + 1, last + 1),
              l.showEpisodeRangeLabel(first + 1, last + 1),
            ),
            itemBuilder: (context, i, node) => _PartCard(
              key: ValueKey('twitch-part-${run.parts[i].file.assetId}'),
              focusNode: node,
              part: run.parts[i],
              count: run.parts.length,
              width: cardWidth,
              next: i == next && started,
              progress: progress[run.parts[i].file.progressKey],
              onTap: () => _play(i),
            ),
          ),
        ],
      ),
    );
  }
}

/// A part: its number big, date, title, length and progress.
class _PartCard extends StatelessWidget {
  const _PartCard({
    super.key,
    required this.part,
    required this.count,
    required this.width,
    required this.next,
    required this.progress,
    required this.onTap,
    this.focusNode,
  });

  final TwitchArchivePart part;
  final int count;
  final double width;
  final bool next;
  final Progress? progress;
  final VoidCallback onTap;
  final FocusNode? focusNode;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final f = part.file;
    final p = progress;
    final label = l.twitchArchivePartOf(part.number, count);
    return SizedBox(
      width: width,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          KFocusable(
            focusNode: focusNode,
            onActivate: onTap,
            semanticLabel: '$label, ${f.title}',
            builder: (context, focused) => AspectRatio(
              aspectRatio: 16 / 9,
              child: DecoratedBox(
                decoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(10),
                  gradient: LinearGradient(
                    begin: Alignment.topLeft,
                    end: Alignment.bottomRight,
                    colors: [
                      KColors.accent.withValues(alpha: next ? .55 : .28),
                      KColors.bgBase,
                    ],
                  ),
                  border: next
                      ? Border.all(color: KColors.accent, width: 2)
                      : null,
                ),
                child: Stack(
                  children: [
                    Center(
                      child: Text(
                        '${part.number}',
                        style: theme.textTheme.displayMedium?.copyWith(
                          color: Colors.white.withValues(alpha: .9),
                          fontWeight: FontWeight.w700,
                        ),
                      ),
                    ),
                    if (p?.watched ?? false)
                      const Positioned(
                        right: 8,
                        top: 8,
                        child: Icon(
                          Icons.check_circle_rounded,
                          color: Colors.white,
                          size: 20,
                        ),
                      ),
                    if (f.duration case final du?)
                      Positioned(
                        right: 8,
                        bottom: 10,
                        child: Text(
                          archiveDuration(du),
                          style: theme.textTheme.labelSmall,
                        ),
                      ),
                    if (p != null && !p.watched)
                      Positioned(
                        left: 8,
                        right: 8,
                        bottom: 4,
                        child: _ProgressBar(progress: p),
                      ),
                  ],
                ),
              ),
            ),
          ),
          const SizedBox(height: 6),
          GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: onTap,
            excludeFromSemantics: true,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  [
                    label,
                    _day(context, f.recordedAt),
                  ].where((s) => s.isNotEmpty).join(' · '),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.labelMedium,
                ),
                Text(
                  f.title,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.bodySmall,
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

// ------------------------------------------------------------ playback

/// Where a saved file starts: where it was left, else at [gameStart].
int _startMs(Progress? p, Duration gameStart) =>
    p != null && !p.watched && p.positionMs > 0
    ? p.positionMs
    : gameStart.inMilliseconds;

PlayRequest _request(
  TwitchSavedFile f,
  String title,
  Progress? p, {
  Duration gameStart = Duration.zero,
}) => PlayRequest(
  uri: f.path,
  title: title,
  itemId: f.progressKey,
  durationMs: f.duration?.inMilliseconds,
  startPositionMs: _startMs(p, gameStart),
);

typedef _OnProgress = void Function(int positionMs, int durationMs);

/// Saves the positions of [f] under its progress key.
_OnProgress _saver(AppServices services, TwitchSavedFile f) {
  final key = f.progressKey;
  if (key == null) return (_, _) {};
  final writer = ProgressWriter(services.library.saveProgress);
  return (pos, dur) {
    if (pos <= 0 || dur <= 0) return;
    writer.add(
      Progress(
        itemId: key,
        positionMs: pos,
        durationMs: dur,
        updatedAt: services.clock(),
      ),
    );
  };
}

/// Plays part [index] of [run] from where it was left; «Далее» at the end
/// is the next part, from its game's start.
Future<void> playArchivePart(
  BuildContext context,
  TwitchPlaythrough run,
  int index, {
  Progress? progress,
}) async {
  final services = Services.read(context);
  final l = context.l10n;
  String title(int i) =>
      '${playthroughTitle(l, run)} · ${l.twitchArchivePart(run.parts[i].number)}';

  Future<UpNextItem?> Function() nextOf(int i) => () async {
    final n = i + 1;
    if (n >= run.parts.length) return null;
    final part = run.parts[n];
    final key = part.file.progressKey;
    final p = key == null ? null : await services.library.progress(key);
    return UpNextItem(
      request: _request(part.file, title(n), p, gameStart: part.gameStart),
      onProgress: _saver(services, part.file),
      loadNext: nextOf(n),
    );
  };

  final part = run.parts[index];
  await _openPlayer(
    context,
    services,
    _request(part.file, title(index), progress, gameStart: part.gameStart),
    onProgress: _saver(services, part.file),
    loadNext: nextOf(index),
  );
}

/// Plays a saved stream without a game from where it was left.
Future<void> playArchiveFile(
  BuildContext context,
  TwitchSavedFile file, {
  Progress? progress,
}) {
  final services = Services.read(context);
  return _openPlayer(
    context,
    services,
    _request(file, file.title, progress),
    onProgress: _saver(services, file),
  );
}

Future<void> _openPlayer(
  BuildContext context,
  AppServices services,
  PlayRequest request, {
  required _OnProgress onProgress,
  Future<UpNextItem?> Function()? loadNext,
}) async {
  services.music.pause();
  await Navigator.of(context, rootNavigator: true).push(
    MaterialPageRoute<void>(
      fullscreenDialog: true,
      builder: (_) => PlayerScreen(
        request: request,
        onProgress: onProgress,
        loadNext: loadNext,
        continuousPlayback: services.continuousPlayback.value,
        engineFactory: services.playerEngineFactory,
        subtitleSettings: services.subtitleSettings,
        subtitleLanguages: services.subtitleLanguages,
        seekStep: Duration(seconds: services.skipSeconds.value),
        speeds: const [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.5, 3],
      ),
    ),
  );
}
