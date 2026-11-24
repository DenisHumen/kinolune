// SPDX-License-Identifier: MPL-2.0
//
// Show Detail (UI-010, 01 §2.2): the hero of the movie page (UI-009) with
// "Resume S2E5" (initial focus), season tabs with Specials last, 16:9
// episode cards with progress, the episode order (TMDB, absolute, AniDB:
// the DisplayPreference of 03 §4.2) and Hide Spoilers (01 §1.4: overviews
// and stills of episodes not started yet are blurred). Below: cast and
// crew, and the technical information of the episode the button plays.
//
// Libraries with the core ([LibraryEpisodes]) number the episodes and store
// the order per show; the v0 library gets TMDB and absolute order computed
// here ([orderEpisodes]), not stored, and its progress from "Continue
// watching".
//
// UI-049: an episode, the episodes before it, a season or the whole show
// are marked watched or not from the menus of episode_marks.dart (long
// press, right click, a held OK, the «⋯» of a card, a season tab, «Ещё»).

import 'dart:async';
import 'dart:ui' show ImageFilter;

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../review/review_inbox_screen.dart';
import '../offline/offline_ui.dart';
import '../library/library_queries.dart' show combineLatest2;
import '../services/app_services.dart';
import '../sources/source_text.dart' show formatAgo;
import '../theme/kinolune_theme.dart';
import '../trailers/trailer_banner.dart';
import '../trailers/trailer_controller.dart';
import '../widgets/expandable_text.dart';
import '../widgets/focusable.dart';
import '../widgets/format.dart';
import '../widgets/watched_art.dart';
import 'detail_layout.dart' show MatchBadge;
import 'detail_sections.dart';
import 'episode_marks.dart';
import 'fix_match_dialog.dart';
import 'playback.dart';

/// Key of the Resume/Play button (tests: initial focus).
const showPlayKey = ValueKey('show-play');

/// Key of the episode order button.
const showOrderKey = ValueKey('show-order');

/// Key of an episode card.
ValueKey<String> showEpisodeKey(String id) => ValueKey('show-episode-$id');

/// Show → seasons → episodes.
class ShowDetailScreen extends StatefulWidget {
  const ShowDetailScreen({super.key, required this.show, this.initialSeason});

  final LibraryItem show;

  /// Season tab to open (TMDB number); else the season of the episode the
  /// button plays.
  final int? initialSeason;

  @override
  State<ShowDetailScreen> createState() => _ShowDetailScreenState();
}

/// What the main button plays.
class _Target {
  const _Target(this.episode, {this.resume = false});
  final ShowEpisode episode;
  final bool resume;
}

class _ShowDetailScreenState extends State<ShowDetailScreen> {
  Stream<ShowEpisodes>? _episodes;

  /// Order picked on this page (v0: kept here only).
  EpisodeOrder? _order;
  int? _season;
  bool _seasonFromRoute = false;
  ItemDetails _details = ItemDetails.empty;

  /// Probe data and resume source of the target episode, by its id.
  String? _targetId;
  TechInfo? _targetTech;
  String? _resumeDevice;

  LibraryItem get _show => widget.show;

  /// UI-036: the trailer banner and the «Трейлер» button.
  late final TrailerController _trailer = TrailerController(item: _show)
    ..addListener(_onTrailer);
  bool _trailerKnown = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_episodes != null) return;
    _episodes = _watch();
    _season = widget.initialSeason;
    _seasonFromRoute = widget.initialSeason != null;
    unawaited(_loadDetails());
    unawaited(_trailer.load(context));
  }

  void _onTrailer() {
    final known = _trailer.loaded && _trailer.trailer != null;
    if (known != _trailerKnown && mounted) {
      setState(() => _trailerKnown = known);
    }
  }

  @override
  void dispose() {
    _trailer.dispose();
    super.dispose();
  }

  Stream<ShowEpisodes> _watch() {
    final services = Services.read(context);
    // Settings → Shows → default episode order (UI-016) for a show whose
    // order was not picked on this page.
    if (_order == null &&
        services.defaultEpisodeOrder.value != EpisodeOrder.tmdb) {
      return () async* {
        final order = await services.initialEpisodeOrder(_show.id);
        yield* _watchIn(services, order);
      }();
    }
    return _watchIn(services, _order);
  }

  Stream<ShowEpisodes> _watchIn(AppServices services, EpisodeOrder? picked) {
    final library = services.library;
    if (library case final LibraryEpisodes l) {
      return l.showEpisodes(_show.id, order: picked);
    }
    // v0: progress of started episodes from "Continue watching".
    final order = picked ?? EpisodeOrder.tmdb;
    return combineLatest2(
      library.episodes(_show.id),
      library.continueWatching(),
      (List<LibraryItem> eps, List<(LibraryItem, Progress)> cw) =>
          orderEpisodes(
            eps,
            order: order,
            progress: {
              for (final (i, p) in cw)
                if (i.showId == _show.id) i.id: p,
            },
          ),
    );
  }

  Future<void> _loadDetails() async {
    final library = Services.read(context).library;
    if (library case final LibraryDetails d) {
      try {
        final details = await d.details(_show.id);
        if (mounted) setState(() => _details = details);
      } catch (e) {
        debugPrint('Details of ${_show.id} not read: $e');
      }
    }
  }

  /// Probe data and the device of the saved position (SYNC-003) of the
  /// episode the button plays; read once per episode.
  void _loadTarget(_Target? t) {
    final id = t?.episode.item.id;
    if (id == null || id == _targetId) return;
    _targetId = id;
    final resume = t?.resume ?? false;
    final services = Services.read(context);
    unawaited(() async {
      // Never during the build that found the target.
      await Future<void>.value();
      TechInfo? tech;
      String? device;
      if (services.library case final LibraryDetails d) {
        try {
          tech = (await d.details(id)).tech;
        } catch (e) {
          debugPrint('Details of $id not read: $e');
        }
      }
      if (resume && services.sync.available) {
        try {
          final hint = await services.sync.resumeHint(id);
          if (hint != null && hint.remote) device = hint.deviceName;
        } catch (e) {
          debugPrint('Resume hint of $id not read: $e');
        }
      }
      if (!mounted || _targetId != id) return;
      setState(() {
        _targetTech = tech;
        _resumeDevice = device;
      });
    }());
  }

  Future<void> _pickOrder(ShowEpisodes eps) async {
    final l = context.l10n;
    final picked = await showDialog<EpisodeOrder>(
      context: context,
      builder: (context) => SimpleDialog(
        title: Text(l.showEpisodeOrder),
        children: [
          for (final o in eps.available)
            SimpleDialogOption(
              key: ValueKey('show-order-${o.name}'),
              onPressed: () => Navigator.of(context).pop(o),
              child: Row(
                children: [
                  Icon(
                    o == eps.order ? Icons.check : null,
                    size: 20,
                    semanticLabel: null,
                  ),
                  const SizedBox(width: 12),
                  Expanded(child: Text(orderLabel(l, o))),
                ],
              ),
            ),
        ],
      ),
    );
    if (picked == null || picked == eps.order || !mounted) return;
    await _setOrder(picked);
  }

  /// Switches the episode order (and stores it when the library can).
  Future<void> _setOrder(EpisodeOrder order) async {
    final services = Services.read(context);
    final library = services.library;
    if (library case final LibraryEpisodes l) {
      await l.setEpisodeOrder(_show.id, order);
    }
    await services.episodeOrderPicked(_show.id);
    if (!mounted) return;
    setState(() {
      _order = order;
      _season = null;
      _seasonFromRoute = false;
      _episodes = _watch();
    });
  }

  /// Resume the latest started episode; after a finished one the next
  /// (PLAY-005 [nextEpisodeIn]); else the first regular episode.
  static _Target? _target(ShowEpisodes eps) {
    if (eps.episodes.isEmpty) return null;
    ShowEpisode? latest;
    for (final e in eps.episodes) {
      final p = e.progress;
      if (p == null) continue;
      if (latest == null || p.updatedAt.isAfter(latest.progress!.updatedAt)) {
        latest = e;
      }
    }
    if (latest != null) {
      if (resumePositionMs(latest.progress) > 0) {
        return _Target(latest, resume: true);
      }
      if (latest.watched || latest.progress!.fraction > continueWatchingMax) {
        final next = nextEpisodeIn([
          for (final e in eps.episodes) e.item,
        ], latest.item);
        if (next != null) {
          return _Target(eps.episodes.firstWhere((e) => e.item.id == next.id));
        }
      }
    }
    final first = eps.episodes.firstWhere(
      (e) => (e.item.season ?? 1) >= 1,
      orElse: () => eps.episodes.first,
    );
    return _Target(first);
  }

  @override
  Widget build(BuildContext context) {
    return StreamBuilder<ShowEpisodes>(
      stream: _episodes,
      builder: (context, snap) {
        final eps = snap.data ?? ShowEpisodes.empty;
        final target = _target(eps);
        if (snap.hasData) _loadTarget(target);
        return ValueListenableBuilder<bool>(
          valueListenable: Services.of(context).hideSpoilers,
          builder: (context, hide, _) => _page(context, eps, target, hide),
        );
      },
    );
  }

  Widget _page(
    BuildContext context,
    ShowEpisodes eps,
    _Target? target,
    bool hideSpoilers,
  ) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final show = _show;
    final details = _details;
    final seasons = eps.seasons;

    // The season tab: the route's (in TMDB numbers), else the target's.
    var season = _season;
    if (season != null && _seasonFromRoute && eps.order != EpisodeOrder.tmdb) {
      season = null;
    }
    if (season == null || !seasons.contains(season)) {
      season =
          target?.episode.item.season ?? (seasons.isEmpty ? 1 : seasons.first);
    }
    if (!seasons.contains(season) && seasons.isNotEmpty) season = seasons.first;
    final inSeason = eps.inSeason(season);

    final regularSeasons = seasons.where((s) => s >= 1).length;
    final meta = [
      if (show.year != null) '${show.year}',
      if (regularSeasons > 0 && eps.order != EpisodeOrder.absolute)
        l.showSeasonCount(regularSeasons),
      if (eps.episodes.isNotEmpty) l.showEpisodeCount(eps.episodes.length),
      if (details.genres.isNotEmpty) details.genres.take(3).join(', '),
    ].join('  ·  ');
    final badges = techBadges(_targetTech, null);
    // UI-049: watched marks need a library that can clear them.
    final canMark = canMarkEpisodes(context);

    final t = target;
    final tp = t?.episode.progress;
    final actions = <Widget>[
      if (t != null)
        KButton(
          key: showPlayKey,
          autofocus: !d.isPhone,
          primary: true,
          icon: Icons.play_arrow_rounded,
          label: t.resume
              ? l.showResumeEpisode(_code(l, eps.order, t.episode.item))
              : l.showPlayEpisode(_code(l, eps.order, t.episode.item)),
          onPressed: () => playVideo(
            context,
            t.episode.item,
            startPositionMs: t.resume ? resumePositionMs(tp) : 0,
          ),
          // NET-013: a server item can be transcoded on the server.
          onLongPress: () => playWithServerTranscode(
            context,
            t.episode.item,
            startPositionMs: t.resume ? resumePositionMs(tp) : 0,
          ),
        ),
      if (t != null && t.resume)
        KButton(
          key: const ValueKey('show-start-over'),
          icon: Icons.replay,
          label: l.detailFromBeginning,
          onPressed: () => playVideo(context, t.episode.item, fromStart: true),
        ),
      ?trailerButton(context, _trailer),
      if (eps.available.length > 1)
        KButton(
          key: showOrderKey,
          icon: Icons.format_list_numbered,
          label: orderLabel(l, eps.order),
          onPressed: () => _pickOrder(eps),
        ),
      // NET-019: the episodes of the season tab on the device.
      ?offlineListButton(
        context,
        key: const ValueKey('show-season-download'),
        label: l.offlineDownloadSeason,
        items: [for (final e in inSeason) e.item],
      ),
      // UI-049: the season of the tab or the whole show watched or not.
      if (canMark && eps.episodes.isNotEmpty)
        Builder(
          builder: (context) => KButton(
            key: showMoreKey,
            icon: Icons.more_horiz,
            label: l.showMore,
            onPressed: () => showShowMarkMenu(
              context,
              inSeason,
              eps.episodes,
              position: centreOf(context),
            ),
          ),
        ),
      KButton(
        icon: Icons.manage_search,
        label: l.commonFixMatch,
        onPressed: () => showFixMatchDialog(context, show),
      ),
    ];

    final resumeLine = (t == null || !t.resume || tp == null)
        ? null
        : _resumeDevice == null
        ? l.detailResumeSource(
            formatTimecode(tp.positionMs),
            formatAgo(l, tp.updatedAt, now: Services.read(context).clock()),
          )
        : l.detailResumeSourceDevice(
            formatTimecode(tp.positionMs),
            _resumeDevice!,
            formatAgo(l, tp.updatedAt, now: Services.read(context).clock()),
          );

    final info = ConstrainedBox(
      constraints: BoxConstraints(maxWidth: d.isPhone ? double.infinity : 760),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          DetailTitle(item: show, logoUrl: details.logoUrl),
          SizedBox(height: d.isTv ? 10 : 12),
          if (meta.isNotEmpty)
            Text(
              meta,
              key: const ValueKey('show-meta'),
              style: theme.textTheme.labelMedium?.copyWith(
                fontSize: d.type.caption + 1,
                color: KColors.textSecondaryOnArt,
              ),
            ),
          if (badges.isNotEmpty || details.ratings.isNotEmpty) ...[
            const SizedBox(height: 10),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              crossAxisAlignment: WrapCrossAlignment.center,
              children: [
                for (final b in badges) TechBadge(b),
                if (details.ratings.isNotEmpty) ...[
                  if (badges.isNotEmpty) const SizedBox(width: 6),
                  RatingsRow(ratings: details.ratings),
                ],
              ],
            ),
          ],
          SizedBox(height: d.isTv ? 18 : 20),
          // UI-036: «вверх» from the top row opens the trailer (TV).
          TrailerUpKey(
            trailer: _trailer,
            enabled: d.isTv,
            child: FocusTraversalGroup(
              child: Wrap(spacing: 12, runSpacing: 12, children: actions),
            ),
          ),
          if (resumeLine != null) ...[
            const SizedBox(height: 10),
            Text(
              resumeLine,
              key: const ValueKey('show-resume-source'),
              style: theme.textTheme.labelSmall?.copyWith(
                color: KColors.textSecondary,
              ),
            ),
          ],
          MatchBadge(confidence: show.matchConfidence),
          // META-034: fix an unsure recognition right here.
          ReviewStrip(itemId: show.id),
          if (show.overview != null) ...[
            SizedBox(height: d.isTv ? 14 : 18),
            ExpandableText(
              show.overview!,
              maxLines: d.isTv ? 3 : (d.isPhone ? 6 : 4),
              style: theme.textTheme.bodyLarge?.copyWith(
                color: KColors.textSecondary,
              ),
            ),
          ],
        ],
      ),
    );

    final cast = details.credits;
    final techItem = t?.episode.item;
    return Scaffold(
      backgroundColor: KColors.bgBase,
      body: CustomScrollView(
        // BUG-019: a screen of episodes ahead is built before it scrolls
        // in, so a wheel flick or the D-pad never meets an empty row.
        scrollCacheExtent: const ScrollCacheExtent.viewport(1),
        slivers: [
          SliverToBoxAdapter(
            child: DetailHero(item: show, trailer: _trailer, child: info),
          ),
          if (seasons.isNotEmpty)
            SliverToBoxAdapter(
              child: Padding(
                padding: EdgeInsets.only(
                  left: d.pageInsets.left,
                  right: d.pageInsets.right,
                  top: d.isTv ? 12 : 16,
                ),
                child: FocusTraversalGroup(
                  child: Wrap(
                    spacing: 10,
                    runSpacing: 10,
                    children: [
                      for (final s in seasons)
                        _SeasonChip(
                          key: ValueKey('show-season-$s'),
                          label: _seasonLabel(l, eps.order, s),
                          selected: s == season,
                          onTap: () => setState(() {
                            _season = s;
                            _seasonFromRoute = false;
                          }),
                          onMenu: canMark
                              ? (p) => showSeasonMarkMenu(
                                  context,
                                  eps.inSeason(s),
                                  position: p,
                                )
                              : null,
                        ),
                    ],
                  ),
                ),
              ),
            ),
          // BUG-019: the episodes are slivers of the page itself, built
          // lazily: the page scrolls down to the last one of a long season
          // with a wheel, a trackpad, the scrollbar, keys or the remote.
          if (inSeason.isNotEmpty)
            SliverPadding(
              padding: EdgeInsets.only(
                top: d.isTv ? 14 : 18,
                left: d.pageInsets.left,
                right: d.pageInsets.right,
              ),
              sliver: _EpisodeList(
                // A new list per order and season: focus starts at the
                // first card.
                key: ValueKey('episodes-${eps.order.name}-$season'),
                episodes: inSeason,
                order: eps.order,
                hideSpoilers: hideSpoilers,
                nextId: _nextId(eps, target),
                all: eps.episodes,
                canMark: canMark,
              ),
            ),
          if (cast.isNotEmpty)
            SliverToBoxAdapter(
              child: DetailSection(
                title: l.detailCastCrew,
                child: CreditsRow(credits: cast),
              ),
            ),
          if (techItem != null && (_targetTech != null || _probes))
            SliverToBoxAdapter(
              child: DetailSection(
                title: l.detailTechInfo,
                child: TechInfoSection(
                  item: techItem,
                  tech: _targetTech,
                  notProbedHint: _probes && _targetTech == null,
                ),
              ),
            ),
          SliverToBoxAdapter(child: SizedBox(height: d.pageInsets.bottom + 24)),
        ],
      ),
    );
  }

  /// The episode to watch next (UI-037: outline and «Далее»): what the
  /// button plays (a started one too), once the show was started.
  static String? _nextId(ShowEpisodes eps, _Target? t) {
    if (t == null || t.episode.watched) return null;
    if (!eps.episodes.any((e) => e.progress != null)) return null;
    return t.episode.item.id;
  }

  /// The library knows details (not v0).
  bool get _probes => Services.read(context).library is LibraryDetails;
}

/// Name of an episode order.
String orderLabel(AppLocalizations l, EpisodeOrder o) => switch (o) {
  EpisodeOrder.tmdb => l.showOrderTmdb,
  EpisodeOrder.absolute => l.showOrderAbsolute,
  EpisodeOrder.anidb => l.showOrderAnidb,
};

String _seasonLabel(AppLocalizations l, EpisodeOrder order, int s) {
  if (s == 0) return l.showSpecials;
  if (order == EpisodeOrder.absolute) return l.showAllEpisodes;
  return l.showSeason(s);
}

/// `S2 · E5`; in absolute order `E14` (specials keep the season code).
String _code(AppLocalizations l, EpisodeOrder order, LibraryItem e) =>
    order == EpisodeOrder.absolute && (e.season ?? 1) >= 1
    ? l.showEpisodeNumber(e.episode ?? 0)
    : episodeCode(l, e);

class _SeasonChip extends StatelessWidget {
  const _SeasonChip({
    super.key,
    required this.label,
    required this.selected,
    required this.onTap,
    this.onMenu,
  });

  final String label;
  final bool selected;
  final VoidCallback onTap;

  /// UI-049: the watched marks of the season.
  final ValueChanged<Offset>? onMenu;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    return KFocusable(
      onActivate: onTap,
      onContextMenu: onMenu,
      radius: 999,
      light: false,
      focusScale: d.focusScaleButton,
      semanticLabel: label,
      selected: selected,
      builder: (context, focused) {
        final on = focused || selected;
        return AnimatedContainer(
          duration: KMotion.focus,
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
          decoration: BoxDecoration(
            color: focused
                ? Colors.white
                : (selected
                      ? Colors.white.withValues(alpha: .22)
                      : Colors.white.withValues(alpha: .07)),
            borderRadius: BorderRadius.circular(999),
          ),
          child: Text(
            label,
            style: Theme.of(context).textTheme.labelLarge?.copyWith(
              color: focused
                  ? Colors.black
                  : (on ? Colors.white : KColors.textSecondary),
            ),
          ),
        );
      },
    );
  }
}

/// Episodes of a season (BUG-019): a list of wide cards on phones, a grid
/// of 16:9 cards elsewhere. Slivers built on demand, so a season of a
/// thousand episodes scrolls with the page like a short one.
class _EpisodeList extends StatelessWidget {
  const _EpisodeList({
    super.key,
    required this.episodes,
    required this.order,
    required this.hideSpoilers,
    this.nextId,
    this.all = const [],
    this.canMark = false,
  });

  final List<ShowEpisode> episodes;
  final EpisodeOrder order;
  final bool hideSpoilers;
  final String? nextId;

  /// Every episode of the show in the page's order (UI-049: «все
  /// предыдущие»).
  final List<ShowEpisode> all;

  /// The episode menu with the watched marks is there (UI-049).
  final bool canMark;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    void play(ShowEpisode e) => playVideo(
      context,
      e.item,
      startPositionMs: resumePositionMs(e.progress),
    );
    ValueChanged<Offset>? menu(ShowEpisode e) => canMark
        ? (p) => showEpisodeMarkMenu(context, e, all, position: p)
        : null;
    Widget card(ShowEpisode e, {bool horizontal = false}) => _EpisodeCard(
      key: showEpisodeKey(e.item.id),
      episode: e,
      order: order,
      hideSpoilers: hideSpoilers,
      next: e.item.id == nextId,
      width: double.infinity,
      horizontal: horizontal,
      onTap: () => play(e),
      onMenu: menu(e),
    );
    if (d.isPhone) {
      return FocusTraversalGroup(
        child: SliverList.builder(
          itemCount: episodes.length,
          itemBuilder: (context, i) => card(episodes[i], horizontal: true),
        ),
      );
    }
    final gap = d.isTv ? 20.0 : 16.0;
    final cardWidth = d.isTv ? 300.0 : 320.0;
    // Still + three text lines (code, title, two of overview).
    final textScale = MediaQuery.textScalerOf(context).scale(1);
    return FocusTraversalGroup(
      child: SliverLayoutBuilder(
        builder: (context, c) {
          // As many columns as keep a card at most [cardWidth] wide.
          final columns = ((c.crossAxisExtent + gap) / (cardWidth + gap))
              .ceil()
              .clamp(1, 1 << 20);
          final width = (c.crossAxisExtent - gap * (columns - 1)) / columns;
          return SliverGrid.builder(
            gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
              crossAxisCount: columns,
              crossAxisSpacing: gap,
              mainAxisSpacing: gap,
              mainAxisExtent: width * 9 / 16 + 112 * textScale,
            ),
            itemCount: episodes.length,
            itemBuilder: (context, i) => card(episodes[i]),
          );
        },
      ),
    );
  }
}

/// A 16:9 still with the watch state of the episode (UI-037), its number,
/// runtime, title and overview. With Hide Spoilers the still and the
/// overview of an episode not started yet are blurred.
class _EpisodeCard extends StatelessWidget {
  const _EpisodeCard({
    super.key,
    required this.episode,
    required this.order,
    required this.hideSpoilers,
    required this.width,
    required this.onTap,
    this.horizontal = false,
    this.next = false,
    this.onMenu,
  });

  final ShowEpisode episode;
  final EpisodeOrder order;
  final bool hideSpoilers;

  /// The episode to watch next.
  final bool next;
  final double width;
  final VoidCallback onTap;

  /// Still on the left, text on the right (phones).
  final bool horizontal;

  /// UI-049: the menu with the watched marks, at a global position (long
  /// press, right click, a held OK, the menu key, «⋯»).
  final ValueChanged<Offset>? onMenu;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    final l = context.l10n;
    final e = episode.item;
    final p = episode.progress;
    final hidden = hideSpoilers && episode.unseen;
    final Progress? started =
        (p != null && !episode.watched && p.positionMs > 0) ? p : null;
    final code = _code(l, order, e);

    final state = watchArtOf(p);
    final radius = d.cardRadius * .75;
    // UI-049: «⋯» for touch and mouse; the remote holds OK instead.
    final Widget? more = onMenu == null || d.isTv
        ? null
        : _EpisodeMoreButton(
            id: e.id,
            label: l.showEpisodeActions,
            onMenu: onMenu!,
            onArt: !horizontal,
          );
    Widget still(bool focused) => ClipRRect(
      borderRadius: BorderRadius.circular(radius),
      child: AspectRatio(
        aspectRatio: 16 / 9,
        child: Stack(
          fit: StackFit.expand,
          children: [
            WatchedStill(
              url: e.backdropUrl,
              seed: e.id,
              state: state,
              fraction: p?.fraction ?? 0,
              remaining: state == WatchArt.started && p != null
                  ? watchedLeft(l, p)
                  : null,
              next: next && !hidden,
              hiddenLabel: hidden ? code : null,
              radius: radius,
            ),
            if (!hidden)
              Center(
                child: Icon(
                  Icons.play_circle_outline,
                  color: Colors.white.withValues(alpha: focused ? .95 : .6),
                  size: 30,
                ),
              ),
            // NET-019: on the device / downloading.
            Positioned(
              top: 6,
              left: 6,
              child: OfflineBadge(itemId: e.id, size: 16),
            ),
            if (more != null && !horizontal)
              Positioned(top: 2, right: 2, child: more),
          ],
        ),
      ),
    );

    final overview = e.overview;
    Widget text() => Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          [
            code,
            formatRuntime(l, e.durationMs),
            if (started != null) formatRemaining(l, started),
          ].where((s) => s.isNotEmpty).join('  ·  '),
          style: theme.textTheme.labelSmall,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
        ),
        const SizedBox(height: 2),
        Text(
          e.title,
          style: theme.textTheme.titleMedium,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
        ),
        if (overview != null && overview.isNotEmpty) ...[
          const SizedBox(height: 4),
          Semantics(
            label: hidden ? l.showSpoilerHidden : null,
            excludeSemantics: hidden,
            child: _Blur(
              key: hidden ? const ValueKey('show-spoiler-blur') : null,
              on: hidden,
              sigma: 5,
              child: Text(
                overview,
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                style: theme.textTheme.bodySmall?.copyWith(
                  color: KColors.textSecondary,
                ),
              ),
            ),
          ),
        ],
      ],
    );

    return Padding(
      padding: EdgeInsets.only(bottom: horizontal ? 12 : 0),
      child: KFocusable(
        onActivate: onTap,
        focusScale: horizontal ? 1.01 : (d.isTv ? 1.05 : 1.03),
        light: false,
        semanticLabel: '$code ${e.title}',
        onContextMenu: onMenu,
        builder: (context, focused) => SizedBox(
          width: width,
          child: horizontal
              ? AnimatedContainer(
                  duration: KMotion.focus,
                  padding: const EdgeInsets.all(6),
                  color: focused
                      ? Colors.white.withValues(alpha: .08)
                      : Colors.transparent,
                  child: Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      SizedBox(width: 150, child: still(focused)),
                      const SizedBox(width: 14),
                      Expanded(child: text()),
                      ?more,
                    ],
                  ),
                )
              : Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    still(focused),
                    const SizedBox(height: 10),
                    text(),
                  ],
                ),
        ),
      ),
    );
  }
}

/// «⋯» of an episode card (UI-049): opens its menu. Out of the focus
/// order: on TV and with the keyboard the card itself opens the menu (a
/// held OK, the menu key).
class _EpisodeMoreButton extends StatelessWidget {
  const _EpisodeMoreButton({
    required this.id,
    required this.label,
    required this.onMenu,
    required this.onArt,
  });

  final String id;
  final String label;
  final ValueChanged<Offset> onMenu;

  /// Over the still: on a dark disc so it reads on any frame.
  final bool onArt;

  @override
  Widget build(BuildContext context) => ExcludeFocus(
    child: Builder(
      builder: (context) => IconButton(
        key: showEpisodeMenuKey(id),
        tooltip: label,
        // A 32 px disc, but a 48 px tap target on every form factor
        // (Android asks for 48, iOS for 44).
        visualDensity: VisualDensity.standard,
        iconSize: 20,
        color: Colors.white,
        style: IconButton.styleFrom(
          minimumSize: const Size.square(32),
          padding: const EdgeInsets.all(6),
          tapTargetSize: MaterialTapTargetSize.padded,
          backgroundColor: onArt ? Colors.black.withValues(alpha: .45) : null,
        ),
        icon: const Icon(Icons.more_horiz),
        onPressed: () => onMenu(centreOf(context)),
      ),
    ),
  );
}

/// Blurs [child] when [on] (Hide Spoilers).
class _Blur extends StatelessWidget {
  const _Blur({
    super.key,
    required this.on,
    required this.sigma,
    required this.child,
  });

  final bool on;
  final double sigma;
  final Widget child;

  @override
  Widget build(BuildContext context) => on
      ? ClipRect(
          child: ImageFiltered(
            imageFilter: ImageFilter.blur(sigmaX: sigma, sigmaY: sigma),
            child: child,
          ),
        )
      : child;
}
