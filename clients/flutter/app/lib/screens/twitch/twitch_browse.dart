// SPDX-License-Identifier: MPL-2.0
//
// Browse of the "Streams" section (TWI-034, research 06 §1.1, §6.2): the
// grid of the most watched categories (`games/top`) with infinite scroll,
// the live streams of a category (`streams?game_id`, most viewers first)
// with the broadcast language filter (the UI language by default, "All
// languages" in one tap), and the stream menu (watch, open the channel,
// favourite). Helix answers only with a sign-in (DEC-006, research 06
// §1.3: an app token needs a client secret, which a client never has), so
// signed out the tab explains that and offers "Sign in".
//
// Layouts: a list-like single column on a phone, a grid with hover on a
// desktop, rows of focusable tiles for the remote on a TV (focus comes
// back to the tile after the category page closes: the route keeps it).

import 'dart:async';

import 'package:flutter/scheduler.dart' show SchedulerBinding;
import 'package:material_ui/material_ui.dart';

import '../../l10n/l10n.dart';
import '../../services/app_services.dart';
import '../../theme/kinolune_theme.dart';
import '../../twitch/twitch_backend.dart';
import '../../twitch/twitch_models.dart';
import '../../twitch/twitch_pager.dart';
import '../../twitch/twitch_service.dart';
import '../../widgets/artwork.dart';
import '../../widgets/cards.dart';
import '../../widgets/focusable.dart';
import '../library_grid_screen.dart' show FilterPill;
import 'twitch_channel_screen.dart';
import 'twitch_follow.dart';
import 'twitch_play.dart';
import 'twitch_widgets.dart';

/// "The list scrolled near its end": the pages above load the next page.
class TwitchScrollEnd extends ChangeNotifier {
  bool _queued = false;
  bool _disposed = false;

  /// Scroll notifications also come during layout: the listeners run
  /// after the frame (once per frame), where they may rebuild.
  void ping() {
    if (_queued || _disposed) return;
    _queued = true;
    SchedulerBinding.instance.addPostFrameCallback((_) {
      _queued = false;
      if (!_disposed) notifyListeners();
    });
    SchedulerBinding.instance.ensureVisualUpdate();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  /// Pings when [n] is a vertical scroll less than a screen from the end.
  bool onScroll(ScrollNotification n) {
    final m = n.metrics;
    if (m.axis == Axis.vertical && m.extentAfter < m.viewportDimension) {
      ping();
    }
    return false;
  }
}

/// The UI language as a Twitch broadcast language.
String twitchUiLanguage(BuildContext context) =>
    Localizations.localeOf(context).languageCode;

/// What to say when a Helix list could not be loaded.
String twitchListError(AppLocalizations l, Object? e) => switch (e) {
  TwitchUnavailable(reason: TwitchUnavailableReason.network) =>
    l.twitchBrowseOffline,
  TwitchUnavailable(
    reason: TwitchUnavailableReason.signedOut || TwitchUnavailableReason.signIn,
  ) =>
    l.twitchSignInNeeded,
  TwitchUnavailable(reason: TwitchUnavailableReason.notConfigured) =>
    l.twitchBrowseUnavailable,
  _ => l.twitchBrowseError,
};

bool _needsSignIn(Object? e) =>
    e is TwitchUnavailable &&
    (e.reason == TwitchUnavailableReason.signedOut ||
        e.reason == TwitchUnavailableReason.signIn);

/// Signed out: why the list is not there, and "Sign in" when this build
/// can sign in.
class TwitchBrowseSignIn extends StatelessWidget {
  const TwitchBrowseSignIn({super.key, required this.twitch});

  final TwitchService twitch;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return TwitchInfoCard(
      key: const ValueKey('twitch-browse-sign-in'),
      icon: Icons.explore_outlined,
      text: twitch.canSignIn ? l.twitchBrowseSignIn : l.twitchBrowseUnavailable,
      action: twitch.canSignIn
          ? KButton(
              key: const ValueKey('twitch-browse-sign-in-button'),
              label: l.twitchSignIn,
              icon: Icons.login,
              primary: true,
              onPressed: () => showTwitchSignIn(context),
            )
          : null,
    );
  }
}

/// The end of a paged list: a spinner while loading, the error with
/// "Retry" (or "Sign in"), else "Show more" while pages remain (the
/// keyboard and remote way to the next page; scrolling loads it too).
class TwitchPagerFooter extends StatelessWidget {
  const TwitchPagerFooter({super.key, required this.pager});

  final TwitchPager<Object?> pager;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final error = pager.error;
    final Widget child;
    if (pager.loading) {
      child = const Center(
        child: SizedBox.square(
          dimension: 28,
          child: CircularProgressIndicator(strokeWidth: 3),
        ),
      );
    } else if (error != null) {
      return TwitchInfoCard(
        key: const ValueKey('twitch-list-error'),
        icon: Icons.cloud_off_outlined,
        text: twitchListError(l, error),
        action: _needsSignIn(error)
            ? KButton(
                label: l.twitchSignIn,
                icon: Icons.login,
                onPressed: () async {
                  final account = await showTwitchSignIn(context);
                  if (account != null) unawaited(pager.loadMore());
                },
              )
            : KButton(
                key: const ValueKey('twitch-list-retry'),
                label: l.commonScreenErrorRetry,
                icon: Icons.refresh,
                onPressed: pager.loadMore,
              ),
      );
    } else if (pager.hasMore) {
      child = Center(
        child: KButton(
          key: const ValueKey('twitch-list-more'),
          label: l.twitchBrowseLoadMore,
          icon: Icons.expand_more,
          onPressed: pager.loadMore,
        ),
      );
    } else {
      return const SizedBox.shrink();
    }
    return Padding(
      padding: EdgeInsets.symmetric(
        horizontal: d.pageInsets.left,
        vertical: 12,
      ),
      child: child,
    );
  }
}

/// A category: box art (from the image cache), name and place in the top.
class TwitchCategoryCard extends StatelessWidget {
  const TwitchCategoryCard({
    super.key,
    required this.category,
    required this.onTap,
    this.rank,
    this.width,
  });

  final TwitchCategory category;
  final VoidCallback onTap;

  /// Place in "Top categories" (1-based); null in search.
  final int? rank;
  final double? width;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final r = rank;
    return SizedBox(
      width: width,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          KFocusable(
            onActivate: onTap,
            semanticLabel: category.name,
            builder: (context, focused) => AspectRatio(
              aspectRatio: 3 / 4,
              child: Artwork(
                url: twitchImage(
                  category.boxArtUrl ?? twitchBoxArtTemplate(category.id),
                  285,
                  380,
                ),
                seed: 'category-${category.id}',
                style: ArtStyle.poster,
              ),
            ),
          ),
          SizedBox(height: d.isTv ? 8 : 6),
          GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: onTap,
            // The art above is the button for screen readers (UI-021).
            excludeFromSemantics: true,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  category.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.titleSmall,
                ),
                Text(
                  r == null ? l.twitchCategoryKind : l.twitchCategoryRank(r),
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

void openTwitchCategory(BuildContext context, TwitchCategory category) =>
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => TwitchCategoryScreen(category: category),
      ),
    );

void openTwitchChannel(BuildContext context, TwitchChannel channel) =>
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => TwitchChannelScreen(channel: channel),
      ),
    );

/// Grid of fixed-aspect tiles, columns from the minimum tile width.
Widget twitchGrid(
  BuildContext context, {
  required int count,
  required double minItemWidth,
  required double artAspect,
  required Widget Function(int i) item,
  double extra = 0,
}) {
  final d = KDensity.of(context);
  return Padding(
    padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
    child: FocusTraversalGroup(
      child: GridView.builder(
        shrinkWrap: true,
        physics: const NeverScrollableScrollPhysics(),
        clipBehavior: Clip.none,
        padding: EdgeInsets.zero,
        gridDelegate: CardGridDelegate(
          minItemWidth: minItemWidth,
          crossSpacing: d.gap,
          mainSpacing: d.gap,
          artAspect: artAspect,
          extraHeight: cardCaptionHeight(d) + 8 + extra,
        ),
        itemCount: count,
        itemBuilder: (_, i) => item(i),
      ),
    ),
  );
}

/// The "Browse" tab: top categories with infinite scroll.
class TwitchBrowseTab extends StatefulWidget {
  const TwitchBrowseTab({
    super.key,
    required this.twitch,
    required this.scrollEnd,
  });

  final TwitchService twitch;
  final TwitchScrollEnd scrollEnd;

  @override
  State<TwitchBrowseTab> createState() => _TwitchBrowseTabState();
}

class _TwitchBrowseTabState extends State<TwitchBrowseTab> {
  TwitchPager<TwitchCategory>? _pager;
  String? _accountId;

  @override
  void initState() {
    super.initState();
    widget.scrollEnd.addListener(_onScrollEnd);
    _sync();
  }

  @override
  void didUpdateWidget(TwitchBrowseTab old) {
    super.didUpdateWidget(old);
    if (!identical(old.scrollEnd, widget.scrollEnd)) {
      old.scrollEnd.removeListener(_onScrollEnd);
      widget.scrollEnd.addListener(_onScrollEnd);
    }
    _sync();
  }

  @override
  void dispose() {
    widget.scrollEnd.removeListener(_onScrollEnd);
    _pager?.dispose();
    super.dispose();
  }

  /// A new account (signed in, out or another one) starts the list again.
  void _sync() {
    final id = widget.twitch.account?.userId;
    if (id == _accountId && (_pager != null || id == null)) return;
    _accountId = id;
    _pager?.dispose();
    _pager = null;
    if (id == null) return;
    final twitch = widget.twitch;
    final pager = TwitchPager<TwitchCategory>(
      (cursor) => twitch.topCategories(cursor: cursor),
      keyOf: (c) => c.id,
    )..addListener(_onPager);
    _pager = pager;
    unawaited(pager.loadMore());
  }

  void _onPager() {
    if (mounted) setState(() {});
  }

  void _onScrollEnd() {
    final p = _pager;
    if (p != null && !p.initial && p.error == null) unawaited(p.loadMore());
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final pager = _pager;
    if (pager == null) return TwitchBrowseSignIn(twitch: widget.twitch);
    if (pager.initial) {
      if (pager.error != null) {
        return TwitchPagerFooter(pager: pager);
      }
      return const Padding(
        key: ValueKey('twitch-browse-loading'),
        padding: EdgeInsets.all(48),
        child: Center(child: CircularProgressIndicator()),
      );
    }
    final items = pager.items;
    if (items.isEmpty) {
      return Padding(
        padding: EdgeInsets.symmetric(vertical: d.rowGap),
        child: EmptyState(
          key: const ValueKey('twitch-browse-empty'),
          icon: Icons.explore_outlined,
          title: l.twitchBrowseEmpty,
          message: l.twitchBrowseEmptyMessage,
        ),
      );
    }
    return Column(
      key: const ValueKey('twitch-browse'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        TwitchRowTitle(l.twitchBrowseTopCategories),
        twitchGrid(
          context,
          count: items.length,
          // A phone gets three columns, wider screens as many as fit.
          minItemWidth: d.isPhone ? 100 : d.posterWidth,
          artAspect: 4 / 3,
          item: (i) => TwitchCategoryCard(
            key: ValueKey('twitch-top-category-${items[i].id}'),
            category: items[i],
            rank: i + 1,
            onTap: () => openTwitchCategory(context, items[i]),
          ),
        ),
        TwitchPagerFooter(pager: pager),
      ],
    );
  }
}

/// Live streams of a category with the language filter (the top of the
/// category page).
class TwitchCategoryStreams extends StatefulWidget {
  const TwitchCategoryStreams({
    super.key,
    required this.category,
    required this.scrollEnd,
  });

  final TwitchCategory category;
  final TwitchScrollEnd scrollEnd;

  @override
  State<TwitchCategoryStreams> createState() => _TwitchCategoryStreamsState();
}

class _TwitchCategoryStreamsState extends State<TwitchCategoryStreams> {
  TwitchService? _twitch;
  TwitchPager<TwitchStream>? _pager;
  String? _accountId;

  /// The language of the list now (null = all); [_uiLanguage] decides the
  /// default.
  String? _language;
  String _uiLanguage = 'en';

  @override
  void initState() {
    super.initState();
    widget.scrollEnd.addListener(_onScrollEnd);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final twitch = Services.of(context).twitch;
    _uiLanguage = twitchUiLanguage(context);
    if (!identical(twitch, _twitch)) {
      _twitch?.removeListener(_onTwitch);
      _twitch = twitch..addListener(_onTwitch);
      _language = twitch.browseLanguage(_uiLanguage);
      _sync(force: true);
    }
  }

  @override
  void dispose() {
    widget.scrollEnd.removeListener(_onScrollEnd);
    _twitch?.removeListener(_onTwitch);
    _pager?.dispose();
    super.dispose();
  }

  void _onTwitch() {
    if (_twitch?.account?.userId != _accountId) {
      setState(() => _sync(force: true));
    }
  }

  void _sync({bool force = false}) {
    final twitch = _twitch!;
    final id = twitch.account?.userId;
    if (!force && id == _accountId) return;
    _accountId = id;
    _pager?.dispose();
    _pager = null;
    if (id == null) return;
    final language = _language;
    final pager = TwitchPager<TwitchStream>(
      (cursor) => twitch.categoryStreams(
        widget.category,
        language: language,
        cursor: cursor,
      ),
      keyOf: (s) => s.userId,
      compare: (a, b) => b.viewers.compareTo(a.viewers),
    )..addListener(_onPager);
    _pager = pager;
    unawaited(pager.loadMore());
  }

  void _onPager() {
    if (mounted) setState(() {});
  }

  void _onScrollEnd() {
    final p = _pager;
    if (p != null && !p.initial && p.error == null) unawaited(p.loadMore());
  }

  Future<void> _setLanguage(String? code) async {
    if (code == _language) return;
    setState(() {
      _language = code;
      _sync(force: true);
    });
    await _twitch!.setBrowseLanguage(code, uiLanguage: _uiLanguage);
  }

  Widget _languages(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final ui = _uiLanguage;
    final current = _language;
    final other = current != null && current != ui ? current : null;
    return Padding(
      padding: EdgeInsets.only(bottom: d.isTv ? 10 : 14),
      child: FocusTraversalGroup(
        child: SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          clipBehavior: Clip.none,
          padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
          child: Row(
            children: [
              FilterPill(
                key: ValueKey('twitch-language-$ui'),
                label: twitchLanguageName(ui),
                icon: Icons.translate,
                selected: current == ui,
                onTap: () => _setLanguage(ui),
              ),
              const SizedBox(width: 8),
              FilterPill(
                key: const ValueKey('twitch-language-all'),
                label: l.twitchLanguageAll,
                icon: Icons.public,
                selected: current == null,
                onTap: () => _setLanguage(null),
              ),
              if (other != null) ...[
                const SizedBox(width: 8),
                FilterPill(
                  key: ValueKey('twitch-language-$other'),
                  label: twitchLanguageName(other),
                  selected: true,
                  onTap: () {},
                ),
              ],
              const SizedBox(width: 8),
              PopupMenuButton<String>(
                key: const ValueKey('twitch-language-other'),
                tooltip: l.twitchLanguageOther,
                onSelected: _setLanguage,
                itemBuilder: (_) => [
                  for (final code in twitchBrowseLanguages)
                    if (code != ui)
                      CheckedPopupMenuItem(
                        key: ValueKey('twitch-language-pick-$code'),
                        value: code,
                        checked: code == current,
                        child: Text(twitchLanguageName(code)),
                      ),
                ],
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 8,
                    vertical: 6,
                  ),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(l.twitchLanguageOther),
                      const Icon(Icons.arrow_drop_down),
                    ],
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final twitch = _twitch!;
    final pager = _pager;
    final children = <Widget>[TwitchRowTitle(l.twitchLiveNow)];
    if (pager == null) {
      children.add(TwitchBrowseSignIn(twitch: twitch));
    } else {
      children.add(_languages(context));
      final items = pager.items;
      if (pager.initial && pager.error == null) {
        children.add(
          const Padding(
            key: ValueKey('twitch-category-streams-loading'),
            padding: EdgeInsets.all(32),
            child: Center(child: CircularProgressIndicator()),
          ),
        );
      } else if (pager.isEmpty) {
        final lang = _language;
        children.add(
          TwitchInfoCard(
            key: const ValueKey('twitch-category-streams-empty'),
            icon: Icons.sensors_off_outlined,
            text: lang == null
                ? l.twitchCategoryNoLive
                : l.twitchCategoryNoLiveInLanguage(twitchLanguageName(lang)),
            action: lang == null
                ? null
                : KButton(
                    key: const ValueKey('twitch-category-streams-all'),
                    label: l.twitchLanguageAll,
                    icon: Icons.public,
                    onPressed: () => _setLanguage(null),
                  ),
          ),
        );
      } else {
        if (items.isNotEmpty) {
          children.add(
            twitchGrid(
              context,
              count: items.length,
              minItemWidth: d.landscapeWidth,
              artAspect: 9 / 16,
              item: (i) {
                final s = items[i];
                return TwitchLiveCard(
                  key: ValueKey('twitch-category-stream-${s.login}'),
                  stream: s,
                  channel: twitch.channelOf(s.login),
                  width: double.infinity,
                  // Every stream here is of this category: the language
                  // instead, when all languages are listed.
                  showGame: false,
                  language: _language == null ? s.language : null,
                  onTap: () =>
                      playTwitch(context, TwitchPlayTarget.live(s.login)),
                  onChannel: () => openTwitchChannel(
                    context,
                    twitchChannelOfStream(twitch, s),
                  ),
                  onContextMenu: (at) =>
                      showTwitchStreamMenu(context, s, position: at),
                );
              },
            ),
          );
        }
        children.add(TwitchPagerFooter(pager: pager));
      }
    }
    return Column(
      key: const ValueKey('twitch-category-streams'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: children,
    );
  }
}

enum _StreamAction { watch, channel, favorite, follow }

/// The menu of a live stream (right click, long press, the menu key or a
/// held OK): watch, open the channel, add to or remove from favourites.
Future<void> showTwitchStreamMenu(
  BuildContext context,
  TwitchStream stream, {
  required Offset position,
}) async {
  final l = context.l10n;
  final twitch = Services.of(context).twitch;
  final channel = twitchChannelOfStream(twitch, stream);
  final favorite = twitch.isFavorite(stream.login);
  // TWI-033: signed in, the menu also follows on Twitch.
  final signedIn = twitch.account != null;
  final follows = twitch.followsOnTwitch(stream.login) == true;
  final overlay = Overlay.of(context).context.findRenderObject() as RenderBox;
  final chosen = await showMenu<_StreamAction>(
    context: context,
    position: RelativeRect.fromRect(
      position & const Size(1, 1),
      Offset.zero & overlay.size,
    ),
    items: [
      PopupMenuItem(
        key: const ValueKey('twitch-stream-menu-watch'),
        value: _StreamAction.watch,
        child: Text(l.twitchWatchLive),
      ),
      PopupMenuItem(
        key: const ValueKey('twitch-stream-menu-channel'),
        value: _StreamAction.channel,
        child: Text(l.twitchStreamOpenChannel),
      ),
      PopupMenuItem(
        key: const ValueKey('twitch-stream-menu-favorite'),
        value: _StreamAction.favorite,
        child: Text(favorite ? l.twitchRemoveFavorite : l.twitchAddFavorite),
      ),
      if (signedIn)
        PopupMenuItem(
          key: const ValueKey('twitch-stream-menu-follow'),
          value: _StreamAction.follow,
          child: Text(follows ? l.twitchUnfollow : l.twitchFollow),
        ),
    ],
  );
  if (chosen == null || !context.mounted) return;
  switch (chosen) {
    case _StreamAction.watch:
      await playTwitch(context, TwitchPlayTarget.live(stream.login));
    case _StreamAction.channel:
      openTwitchChannel(context, channel);
    case _StreamAction.favorite:
      await twitch.toggleFavorite(channel);
    case _StreamAction.follow:
      await followOnTwitch(context, channel, unfollow: follows);
  }
}

/// The channel of a live stream: the known one (avatar), else what the
/// stream says about it.
TwitchChannel twitchChannelOfStream(TwitchService twitch, TwitchStream s) {
  final c = twitch.channelOf(s.login);
  if (c.userId != null) return c;
  return TwitchChannel(
    login: s.login,
    displayName: s.displayName,
    userId: s.userId,
    avatarUrl: c.avatarUrl,
  );
}
