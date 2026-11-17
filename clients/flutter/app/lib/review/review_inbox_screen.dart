// SPDX-License-Identifier: MPL-2.0
//
// The review inbox «На проверку» (META-016): recognitions the app is not
// sure about (a result shown with a «проверьте» mark) and files no result
// fitted. Each entry shows its candidates with posters and a short
// «почему» line from the match explanation; the user answers «Верно»,
// picks another candidate, searches by title and year, or says it is not
// a movie. Kept simple on purpose: the redesign comes later (DSN).

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/artwork_image.dart';
import '../widgets/focusable.dart';
import '../widgets/settings_tiles.dart';
import 'folder_rules_section.dart';

/// The «почему» line of a candidate: its strongest signals.
String reviewWhy(AppLocalizations l, MatchSignals s) {
  // META-031: an entry the frames named — the share of frames says it all.
  if (s.provider == 'trace_moe') {
    return l.reviewWhy(l.reviewWhyFrames((s.title * 100).round()));
  }
  final parts = <String>[];
  if (s.provider == 'anime_offline_database') parts.add(l.reviewWhyAnimeDb);
  if (s.title > 0) parts.add(l.reviewWhyTitle((s.title * 100).round()));
  final marker = s.lostMarker;
  if (marker != null) parts.add(l.reviewWhyLostMarker(marker.toUpperCase()));
  final (fy, y) = (s.fileYear, s.year);
  if (fy != null && y != null) {
    parts.add(
      (fy - y).abs() <= 1
          ? l.reviewWhyYearSame(y)
          : l.reviewWhyYearDiffers(fy, y),
    );
  }
  if (s.episodesBad > 0) {
    parts.add(l.reviewWhyEpisodesMissing(s.episodesBad));
  } else if (s.episodesOk > 0) {
    parts.add(l.reviewWhyEpisodesFit);
  }
  final d = s.durationError;
  if (d != null && d <= 0.10) parts.add(l.reviewWhyDurationFits);
  if (d != null && d > 0.20) parts.add(l.reviewWhyDurationDiffers);
  if (s.language == true) parts.add(l.reviewWhyLanguage);
  if (s.tied) parts.add(l.reviewWhyTied);
  return parts.isEmpty ? '' : l.reviewWhy(parts.join(' · '));
}

/// The «На проверку» row of Settings → Медиатека, with the count as a
/// badge; nothing where the library has no inbox.
class ReviewInboxTile extends StatefulWidget {
  const ReviewInboxTile({super.key});

  @override
  State<ReviewInboxTile> createState() => _ReviewInboxTileState();
}

class _ReviewInboxTileState extends State<ReviewInboxTile> {
  ReviewInbox? _inbox;
  StreamSubscription<void>? _sub;
  ReviewCount? _count;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final inbox = Services.of(context).reviewInbox;
    if (!identical(inbox, _inbox)) {
      _sub?.cancel();
      _inbox = inbox;
      _sub = inbox?.changes.listen((_) => _load());
      _load();
    }
  }

  Future<void> _load() async {
    final inbox = _inbox;
    if (inbox == null) return;
    try {
      final c = await inbox.count();
      if (mounted) setState(() => _count = c);
    } catch (_) {
      // The row stays without a count.
    }
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final inbox = _inbox;
    if (inbox == null) return const SizedBox.shrink();
    final l = context.l10n;
    final c = _count;
    return SettingsTile(
      key: const ValueKey('settings-review-inbox'),
      icon: Icons.fact_check_outlined,
      title: l.reviewInboxTitle,
      subtitle: c == null
          ? null
          : c.total == 0
          ? l.reviewInboxSubtitleEmpty
          : l.reviewInboxSubtitle(c.check, c.manual),
      subtitleLines: 2,
      trailing: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (c != null && c.total > 0) _Badge(count: c.total),
          Icon(Icons.chevron_right, color: KColors.textTertiary),
        ],
      ),
      onTap: () async {
        await Navigator.of(context).push(
          MaterialPageRoute<void>(
            builder: (_) => ReviewInboxScreen(inbox: inbox),
          ),
        );
        _load();
      },
    );
  }
}

class _Badge extends StatelessWidget {
  const _Badge({required this.count});
  final int count;

  @override
  Widget build(BuildContext context) => Container(
    key: const ValueKey('review-inbox-badge'),
    margin: const EdgeInsets.only(right: 6),
    padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
    decoration: BoxDecoration(
      color: Theme.of(context).colorScheme.primary,
      borderRadius: BorderRadius.circular(999),
    ),
    child: Text(
      '$count',
      style: Theme.of(context).textTheme.labelMedium?.copyWith(
        color: Theme.of(context).colorScheme.onPrimary,
        fontWeight: FontWeight.w600,
      ),
    ),
  );
}

/// The list of the inbox.
class ReviewInboxScreen extends StatefulWidget {
  const ReviewInboxScreen({super.key, required this.inbox});
  final ReviewInbox inbox;

  @override
  State<ReviewInboxScreen> createState() => _ReviewInboxScreenState();
}

class _ReviewInboxScreenState extends State<ReviewInboxScreen> {
  List<ReviewEntry>? _entries;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final e = await widget.inbox.entries();
    if (mounted) setState(() => _entries = e);
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final entries = _entries;
    return Scaffold(
      backgroundColor: KColors.bgBase,
      appBar: AppBar(
        backgroundColor: KColors.bgBase,
        title: Text(
          l.reviewInboxTitle,
          style: Theme.of(context).textTheme.titleLarge,
        ),
      ),
      body: entries == null
          ? const Center(child: CircularProgressIndicator())
          : entries.isEmpty
          ? Center(
              child: Padding(
                padding: const EdgeInsets.all(32),
                child: Text(
                  l.reviewInboxEmpty,
                  key: const ValueKey('review-inbox-empty'),
                  textAlign: TextAlign.center,
                ),
              ),
            )
          : ListView.separated(
              padding: const EdgeInsets.all(16),
              itemCount: entries.length,
              separatorBuilder: (_, _) => const SizedBox(height: 8),
              itemBuilder: (context, i) => _EntryRow(
                entry: entries[i],
                onTap: () async {
                  await Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) => ReviewEntryScreen(
                        inbox: widget.inbox,
                        entry: entries[i],
                      ),
                    ),
                  );
                  _load();
                },
              ),
            ),
    );
  }
}

class _EntryRow extends StatelessWidget {
  const _EntryRow({required this.entry, required this.onTap});
  final ReviewEntry entry;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final shown = entry.current;
    final why = switch (entry.signals ?? shown?.signals) {
      final s? => reviewWhy(l, s),
      null => '',
    };
    final check = entry.state == ReviewState.check;
    return KFocusable(
      key: ValueKey('review-entry-${entry.workId}'),
      onActivate: onTap,
      focusScale: 1.0,
      light: false,
      radius: 12,
      semanticLabel: entry.title,
      builder: (context, focused) => Container(
        padding: const EdgeInsets.all(10),
        decoration: BoxDecoration(
          color: focused
              ? Colors.white.withValues(alpha: .09)
              : KColors.bgElevated,
          borderRadius: BorderRadius.circular(12),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            _Poster(url: entry.posterUrl ?? shown?.posterUrl, width: 56),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Flexible(
                        child: Text(
                          entry.title,
                          style: theme.textTheme.titleMedium,
                          maxLines: 2,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      const SizedBox(width: 8),
                      _StateChip(check: check),
                    ],
                  ),
                  if (entry.localTitle != entry.title ||
                      entry.samplePath.isNotEmpty)
                    Text(
                      entry.samplePath.isEmpty
                          ? entry.localTitle
                          : entry.samplePath,
                      style: theme.textTheme.labelSmall,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                  Text(
                    l.reviewFiles(entry.files),
                    style: theme.textTheme.labelSmall,
                  ),
                  if (why.isNotEmpty)
                    Text(
                      why,
                      style: theme.textTheme.bodySmall,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                    ),
                ],
              ),
            ),
            Icon(Icons.chevron_right, color: KColors.textTertiary),
          ],
        ),
      ),
    );
  }
}

class _StateChip extends StatelessWidget {
  const _StateChip({required this.check});
  final bool check;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final color = check ? Colors.amber : KColors.textTertiary;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
      decoration: BoxDecoration(
        border: Border.all(color: color),
        borderRadius: BorderRadius.circular(6),
      ),
      child: Text(
        check ? l.reviewStateCheck : l.reviewStateManual,
        style: Theme.of(context).textTheme.labelSmall?.copyWith(color: color),
      ),
    );
  }
}

class _Poster extends StatelessWidget {
  const _Poster({required this.url, required this.width});
  final String? url;
  final double width;

  @override
  Widget build(BuildContext context) {
    final u = url;
    final placeholder = Container(
      width: width,
      height: width * 1.5,
      decoration: BoxDecoration(
        color: KColors.stroke,
        borderRadius: BorderRadius.circular(6),
      ),
      child: Icon(Icons.movie_outlined, color: KColors.textTertiary),
    );
    if (u == null) return placeholder;
    return ClipRRect(
      borderRadius: BorderRadius.circular(6),
      child: Image(
        image: artworkProvider(u, width: decodeWidth(context, width)),
        width: width,
        height: width * 1.5,
        fit: BoxFit.cover,
        errorBuilder: (_, _, _) => placeholder,
      ),
    );
  }
}

/// One work: its candidates, manual search and the answers. Also the
/// «Исправить распознавание» page of an item ([showReviewForItem]).
class ReviewEntryScreen extends StatefulWidget {
  const ReviewEntryScreen({
    super.key,
    required this.inbox,
    required this.entry,
  });
  final ReviewInbox inbox;
  final ReviewEntry entry;

  @override
  State<ReviewEntryScreen> createState() => _ReviewEntryScreenState();
}

class _ReviewEntryScreenState extends State<ReviewEntryScreen> {
  late final _title = TextEditingController(text: widget.entry.localTitle);
  late final _year = TextEditingController(
    text: widget.entry.year?.toString() ?? '',
  );
  List<MatchCandidate>? _found;
  String? _error;
  bool _busy = false;

  ReviewEntry get _e => widget.entry;

  @override
  void dispose() {
    _title.dispose();
    _year.dispose();
    super.dispose();
  }

  Future<void> _act(Future<void> Function() action, String done) async {
    if (_busy) return;
    setState(() => _busy = true);
    final messenger = ScaffoldMessenger.maybeOf(context);
    final nav = Navigator.of(context);
    try {
      await action();
      messenger?.showSnackBar(SnackBar(content: Text(done)));
      nav.pop(true);
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _search() async {
    final title = _title.text.trim();
    if (title.isEmpty || _busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final r = await widget.inbox.search(
        _e.workId,
        title,
        year: int.tryParse(_year.text.trim()),
      );
      if (mounted) setState(() => _found = r);
    } catch (e) {
      if (mounted) {
        setState(() => _error = context.l10n.reviewSearchFailed('$e'));
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  void _choose(MatchCandidate c) {
    final l = context.l10n;
    if (c.current) {
      _act(() => widget.inbox.confirm(_e.workId), l.reviewDoneConfirmed);
    } else {
      _act(
        () => widget.inbox.choose(_e.workId, c),
        l.reviewDoneChosen(c.title),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final shown = _e.current;
    final found = _found;
    return Scaffold(
      backgroundColor: KColors.bgBase,
      appBar: AppBar(
        backgroundColor: KColors.bgBase,
        title: Text(_e.title, style: theme.textTheme.titleLarge),
      ),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          if (_e.samplePath.isNotEmpty)
            Text(
              _e.samplePath,
              key: const ValueKey('review-sample-path'),
              style: theme.textTheme.labelSmall,
            ),
          Text(l.reviewFiles(_e.files), style: theme.textTheme.labelSmall),
          const SizedBox(height: 12),
          if (shown != null) ...[
            Text(l.reviewShownNow, style: theme.textTheme.titleSmall),
            const SizedBox(height: 6),
            _CandidateCard(
              candidate: shown,
              action: l.reviewConfirm,
              actionKey: const ValueKey('review-confirm'),
              primary: true,
              onAction: _busy ? null : () => _choose(shown),
            ),
            const SizedBox(height: 16),
          ],
          Text(l.reviewCandidates, style: theme.textTheme.titleSmall),
          const SizedBox(height: 6),
          if (_e.candidates.where((c) => !c.current).isEmpty)
            Text(l.reviewNoCandidates, style: theme.textTheme.bodySmall),
          for (final c in _e.candidates.where((c) => !c.current))
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: _CandidateCard(
                candidate: c,
                action: l.reviewChoose,
                actionKey: ValueKey('review-choose-${c.extId}'),
                onAction: _busy ? null : () => _choose(c),
              ),
            ),
          const SizedBox(height: 16),
          Text(l.reviewSearchTitle, style: theme.textTheme.titleSmall),
          const SizedBox(height: 6),
          Row(
            children: [
              Expanded(
                child: TextField(
                  key: const ValueKey('review-search-title'),
                  controller: _title,
                  decoration: InputDecoration(labelText: l.reviewSearchField),
                  onSubmitted: (_) => _search(),
                ),
              ),
              const SizedBox(width: 8),
              SizedBox(
                width: 90,
                child: TextField(
                  key: const ValueKey('review-search-year'),
                  controller: _year,
                  keyboardType: TextInputType.number,
                  decoration: InputDecoration(labelText: l.reviewSearchYear),
                  onSubmitted: (_) => _search(),
                ),
              ),
              const SizedBox(width: 8),
              KButton(
                key: const ValueKey('review-search'),
                label: l.reviewSearchButton,
                icon: Icons.search,
                onPressed: _busy ? null : _search,
              ),
            ],
          ),
          if (_error != null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(
                _error!,
                style: theme.textTheme.bodySmall?.copyWith(
                  color: theme.colorScheme.error,
                ),
              ),
            ),
          if (found != null) ...[
            const SizedBox(height: 8),
            if (found.isEmpty)
              Text(
                l.reviewSearchNothing,
                key: const ValueKey('review-search-nothing'),
                style: theme.textTheme.bodySmall,
              ),
            for (final c in found)
              Padding(
                padding: const EdgeInsets.only(bottom: 8),
                child: _CandidateCard(
                  candidate: c,
                  action: l.reviewChoose,
                  actionKey: ValueKey('review-found-${c.extId}'),
                  onAction: _busy ? null : () => _choose(c),
                ),
              ),
          ],
          // META-035: type, numbering and the folder rule.
          const SizedBox(height: 24),
          FolderRulesSection(
            inbox: widget.inbox,
            itemId: _e.itemId,
            onKindChanged: () {
              if (mounted) Navigator.of(context).pop(true);
            },
          ),
          const SizedBox(height: 24),
          Align(
            alignment: Alignment.centerLeft,
            child: KButton(
              key: const ValueKey('review-not-media'),
              label: l.reviewNotMedia,
              icon: Icons.block,
              onPressed: _busy
                  ? null
                  : () => _act(
                      () => widget.inbox.notMedia(_e.workId),
                      l.reviewDoneNotMedia,
                    ),
            ),
          ),
          const SizedBox(height: 4),
          Text(l.reviewNotMediaHint, style: theme.textTheme.bodySmall),
        ],
      ),
    );
  }
}

class _CandidateCard extends StatelessWidget {
  const _CandidateCard({
    required this.candidate,
    required this.action,
    required this.actionKey,
    required this.onAction,
    this.primary = false,
  });

  final MatchCandidate candidate;
  final String action;
  final Key actionKey;
  final VoidCallback? onAction;
  final bool primary;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final c = candidate;
    final why = reviewWhy(l, c.signals);
    final meta = [
      if (c.year != null) '${c.year}',
      c.extKind == 'movie' ? l.reviewWhyMovie : l.reviewWhySeries,
      if (c.originalTitle != null && c.originalTitle != c.title)
        c.originalTitle!,
    ].join(' · ');
    return Container(
      padding: const EdgeInsets.all(10),
      decoration: BoxDecoration(
        color: KColors.bgElevated,
        borderRadius: BorderRadius.circular(12),
        border: primary ? Border.all(color: theme.colorScheme.primary) : null,
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          _Poster(url: c.posterUrl, width: 64),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(c.title, style: theme.textTheme.titleMedium),
                Text(meta, style: theme.textTheme.labelSmall),
                if (why.isNotEmpty)
                  Padding(
                    padding: const EdgeInsets.only(top: 4),
                    child: Text(why, style: theme.textTheme.bodySmall),
                  ),
                if (c.overview != null)
                  Padding(
                    padding: const EdgeInsets.only(top: 4),
                    child: Text(
                      c.overview!,
                      style: theme.textTheme.bodySmall,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                const SizedBox(height: 8),
                KButton(
                  key: actionKey,
                  label: action,
                  icon: primary ? Icons.check : null,
                  primary: primary,
                  onPressed: onAction,
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// «Исправить распознавание» of [item] on the review page; false when the
/// library has no inbox or the item has no work (the caller then shows
/// the plain fix dialog).
Future<bool> showReviewForItem(BuildContext context, LibraryItem item) async {
  final inbox = Services.read(context).reviewInbox;
  if (inbox == null) return false;
  final ReviewEntry? entry;
  try {
    entry = await inbox.entryFor(item.id);
  } catch (_) {
    return false;
  }
  final e = entry;
  if (e == null || !context.mounted) return false;
  await Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => ReviewEntryScreen(inbox: inbox, entry: e),
    ),
  );
  return true;
}

/// META-034: the media card shows the candidates of a work the app is not
/// sure about (or found nothing for) right under its title, so the user
/// fixes it where it is open instead of going to Settings. One tap on a
/// candidate chooses it; «Верно» keeps the shown result; «Другое…» opens
/// the full review page (manual search, «Это не фильм»). Nothing is shown
/// for a sure or already decided recognition, or without an inbox.
class ReviewStrip extends StatefulWidget {
  const ReviewStrip({super.key, required this.itemId});
  final String itemId;

  @override
  State<ReviewStrip> createState() => _ReviewStripState();
}

class _ReviewStripState extends State<ReviewStrip> {
  ReviewInbox? _inbox;
  ReviewEntry? _entry;
  StreamSubscription<void>? _sub;
  bool _busy = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final inbox = Services.of(context).reviewInbox;
    if (!identical(inbox, _inbox)) {
      _sub?.cancel();
      _inbox = inbox;
      _sub = inbox?.changes.listen((_) => _load());
      _load();
    }
  }

  @override
  void didUpdateWidget(ReviewStrip old) {
    super.didUpdateWidget(old);
    if (old.itemId != widget.itemId) _load();
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  Future<void> _load() async {
    final inbox = _inbox;
    ReviewEntry? e;
    try {
      e = await inbox?.inboxEntryFor(widget.itemId);
    } catch (_) {
      e = null;
    }
    if (mounted) setState(() => _entry = e);
  }

  Future<void> _act(Future<void> Function() action, String done) async {
    if (_busy) return;
    setState(() => _busy = true);
    final messenger = ScaffoldMessenger.maybeOf(context);
    try {
      await action();
      messenger?.showSnackBar(SnackBar(content: Text(done)));
      if (mounted) setState(() => _entry = null);
    } catch (e) {
      messenger?.showSnackBar(SnackBar(content: Text('$e')));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final inbox = _inbox;
    final e = _entry;
    if (inbox == null || e == null) return const SizedBox.shrink();
    final l = context.l10n;
    final theme = Theme.of(context);
    final others = e.candidates.where((c) => !c.current).take(5).toList();
    return Container(
      key: const ValueKey('review-strip'),
      margin: const EdgeInsets.only(top: 12),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: KColors.bgElevated,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: Colors.amber.withValues(alpha: .6)),
      ),
      child: FocusTraversalGroup(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                const Icon(Icons.help_outline, size: 18, color: Colors.amber),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    e.state == ReviewState.check
                        ? l.reviewStripCheck
                        : l.reviewStripManual,
                    style: theme.textTheme.titleSmall,
                  ),
                ),
              ],
            ),
            if (others.isNotEmpty) ...[
              const SizedBox(height: 10),
              SizedBox(
                height: 150,
                child: ListView.separated(
                  scrollDirection: Axis.horizontal,
                  clipBehavior: Clip.none,
                  itemCount: others.length,
                  separatorBuilder: (_, _) => const SizedBox(width: 10),
                  itemBuilder: (context, i) {
                    final c = others[i];
                    return KFocusable(
                      key: ValueKey('review-strip-candidate-${c.extId}'),
                      semanticLabel: l.reviewStripChoose(c.title),
                      onActivate: _busy
                          ? null
                          : () => _act(
                              () => inbox.choose(e.workId, c),
                              l.reviewDoneChosen(c.title),
                            ),
                      builder: (context, focused) => SizedBox(
                        width: 84,
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            _Poster(url: c.posterUrl, width: 72),
                            const SizedBox(height: 4),
                            Text(
                              c.year == null
                                  ? c.title
                                  : '${c.title} (${c.year})',
                              maxLines: 2,
                              overflow: TextOverflow.ellipsis,
                              style: theme.textTheme.labelSmall,
                            ),
                          ],
                        ),
                      ),
                    );
                  },
                ),
              ),
            ],
            const SizedBox(height: 10),
            Wrap(
              spacing: 10,
              runSpacing: 10,
              children: [
                if (e.state == ReviewState.check)
                  KButton(
                    key: const ValueKey('review-strip-confirm'),
                    label: l.reviewConfirm,
                    icon: Icons.check,
                    onPressed: _busy
                        ? null
                        : () => _act(
                            () => inbox.confirm(e.workId),
                            l.reviewDoneConfirmed,
                          ),
                  ),
                KButton(
                  key: const ValueKey('review-strip-more'),
                  label: l.reviewStripMore,
                  icon: Icons.search,
                  onPressed: _busy
                      ? null
                      : () async {
                          await Navigator.of(context).push(
                            MaterialPageRoute<void>(
                              builder: (_) =>
                                  ReviewEntryScreen(inbox: inbox, entry: e),
                            ),
                          );
                          _load();
                        },
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// META-034: a line on the home screen while there is something to check;
/// opens the review list. Hidden when the inbox is empty.
class HomeReviewBanner extends StatefulWidget {
  const HomeReviewBanner({super.key});

  @override
  State<HomeReviewBanner> createState() => _HomeReviewBannerState();
}

class _HomeReviewBannerState extends State<HomeReviewBanner> {
  ReviewInbox? _inbox;
  ReviewCount? _count;
  StreamSubscription<void>? _sub;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final inbox = Services.of(context).reviewInbox;
    if (!identical(inbox, _inbox)) {
      _sub?.cancel();
      _inbox = inbox;
      _sub = inbox?.changes.listen((_) => _load());
      _load();
    }
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  Future<void> _load() async {
    ReviewCount? c;
    try {
      c = await _inbox?.count();
    } catch (_) {
      c = null;
    }
    if (mounted) setState(() => _count = c);
  }

  @override
  Widget build(BuildContext context) {
    final inbox = _inbox;
    final c = _count;
    if (inbox == null || c == null || c.total == 0) {
      return const SizedBox.shrink();
    }
    final d = KDensity.of(context);
    return Padding(
      padding: EdgeInsets.fromLTRB(d.pageInsets.left, 8, d.pageInsets.right, 8),
      child: Align(
        alignment: Alignment.centerLeft,
        child: KButton(
          key: const ValueKey('home-review-banner'),
          label: context.l10n.reviewHomeBanner(c.total),
          icon: Icons.help_outline,
          onPressed: () => Navigator.of(context).push(
            MaterialPageRoute<void>(
              builder: (_) => ReviewInboxScreen(inbox: inbox),
            ),
          ),
        ),
      ),
    );
  }
}
