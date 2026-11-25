// SPDX-License-Identifier: MPL-2.0
//
// The "Streams" section (Twitch, TWI-013; research 06 §6.2, design 01
// §2.6). Four tabs:
// * Home — Live now, favourites with the LIVE ring, Continue, new VODs
//   with "deleted in N days", downloaded (placeholder until TWI-017),
//   Soon live from schedules, categories of the favourites;
// * Browse — the most watched categories, each with its live streams
//   (TWI-034, twitch/twitch_browse.dart);
// * Channels — favourites (sort and filter, pin) and, signed in, the
//   channels the user follows;
// * Search — channels (on air first, with viewers) and categories, recent
//   searches; without a login favourites, saved channels and "Add channel
//   <name>".
// The section is called "Streams", never after Twitch; Twitch is credited
// in text at the bottom (brand guidelines, research 06 §1.5).

import 'dart:async';

import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart' show MinTapTarget;

import '../flavor.dart';
import '../l10n/l10n.dart';
import '../platform/form_factor.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../twitch/twitch_models.dart';
import '../twitch/twitch_service.dart';
import '../widgets/cards.dart';
import '../widgets/focusable.dart';
import 'library_grid_screen.dart' show FilterPill;
import 'twitch/twitch_archive_screens.dart';
import 'twitch/twitch_browse.dart';
import 'twitch/twitch_channel_screen.dart';
import 'twitch/twitch_downloads_screen.dart';
import 'twitch/twitch_multiview_screen.dart';
import 'twitch/twitch_play.dart';
import 'twitch/twitch_widgets.dart';

enum TwitchTab { home, browse, channels, search }

class TwitchScreen extends StatefulWidget {
  const TwitchScreen({super.key, this.initialTab = TwitchTab.home});

  final TwitchTab initialTab;

  @override
  State<TwitchScreen> createState() => _TwitchScreenState();
}

class _TwitchScreenState extends State<TwitchScreen> {
  TwitchService? _twitch;
  late TwitchTab _tab = widget.initialTab;
  Future<TwitchHome>? _home;
  TwitchHome? _lastHome;
  String _signature = '';

  /// Infinite scroll of Browse (TWI-034).
  final _scrollEnd = TwitchScrollEnd();

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final twitch = Services.of(context).twitch;
    if (!identical(twitch, _twitch)) {
      _twitch?.removeListener(_onChange);
      _twitch = twitch..addListener(_onChange);
      _signature = _signatureOf(twitch);
      _reload(inBuild: true);
    }
  }

  @override
  void dispose() {
    _twitch?.removeListener(_onChange);
    _scrollEnd.dispose();
    super.dispose();
  }

  /// What the Home rows depend on: favourites and the account. Live status
  /// alone does not reload Home (it is refreshed by loading Home).
  static String _signatureOf(TwitchService t) => [
    t.account?.userId ?? '',
    for (final f in t.favorites) '${f.login}:${f.userId}:${f.pinned}',
  ].join(',');

  void _onChange() {
    final s = _signatureOf(_twitch!);
    if (s != _signature) {
      _signature = s;
      _reload();
    } else if (mounted) {
      setState(() {});
    }
  }

  /// Loads Home again; [inBuild]: called from [didChangeDependencies].
  void _reload({bool inBuild = false}) {
    final future = _twitch!.home();
    future.then((h) => _lastHome = h, onError: (_) {});
    if (inBuild) {
      _home = future;
    } else if (mounted) {
      setState(() {
        _home = future;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final twitch = _twitch!;
    return NotificationListener<ScrollNotification>(
      onNotification: _scrollEnd.onScroll,
      child: _slivers(context, d, l, twitch),
    );
  }

  Widget _slivers(
    BuildContext context,
    KDensity d,
    AppLocalizations l,
    TwitchService twitch,
  ) {
    return CustomScrollView(
      clipBehavior: Clip.none,
      slivers: [
        SliverToBoxAdapter(
          child: PageTitle(
            l.navTwitch,
            trailing:
                AppFlavor.twitchArchive &&
                    Services.of(context).twitchDownloads != null
                ? Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      IconButton(
                        key: const ValueKey('twitch-downloads'),
                        tooltip: l.twitchDownloads,
                        icon: const Icon(Icons.download_rounded),
                        onPressed: () => Navigator.of(context).push(
                          MaterialPageRoute<void>(
                            builder: (_) => const TwitchDownloadsScreen(),
                          ),
                        ),
                      ),
                      // TWI-019: recordings and saved files.
                      if (Services.of(context).visibleArchive != null)
                        IconButton(
                          key: const ValueKey('twitch-archive'),
                          tooltip: l.twitchArchiveTitle,
                          icon: const Icon(Icons.video_library_outlined),
                          onPressed: () => openTwitchArchive(context),
                        ),
                      _AccountButton(twitch),
                    ],
                  )
                : _AccountButton(twitch),
          ),
        ),
        SliverToBoxAdapter(
          child: _Pills(
            children: [
              for (final (tab, label) in [
                (TwitchTab.home, l.twitchTabHome),
                (TwitchTab.browse, l.twitchTabBrowse),
                (TwitchTab.channels, l.twitchTabChannels),
                (TwitchTab.search, l.twitchTabSearch),
              ])
                FilterPill(
                  key: ValueKey('twitch-tab-${tab.name}'),
                  label: label,
                  selected: _tab == tab,
                  onTap: () => setState(() => _tab = tab),
                ),
            ],
          ),
        ),
        ...switch (_tab) {
          TwitchTab.home => [
            SliverToBoxAdapter(
              child: FutureBuilder<TwitchHome>(
                future: _home,
                builder: (context, snap) {
                  final home = snap.data ?? _lastHome;
                  if (home == null) {
                    return const Padding(
                      padding: EdgeInsets.all(48),
                      child: Center(child: CircularProgressIndicator()),
                    );
                  }
                  return _HomeRows(
                    home: home,
                    onSearch: () => setState(() => _tab = TwitchTab.search),
                  );
                },
              ),
            ),
          ],
          TwitchTab.browse => [
            SliverToBoxAdapter(
              child: TwitchBrowseTab(twitch: twitch, scrollEnd: _scrollEnd),
            ),
          ],
          TwitchTab.channels => [
            SliverToBoxAdapter(child: _ChannelsTab(twitch: twitch)),
          ],
          TwitchTab.search => [
            SliverToBoxAdapter(child: _SearchTab(twitch: twitch)),
          ],
        },
        const SliverToBoxAdapter(child: TwitchAttribution()),
        SliverToBoxAdapter(child: SizedBox(height: d.pageInsets.bottom + 90)),
      ],
    );
  }
}

class _Pills extends StatelessWidget {
  const _Pills({required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    return Padding(
      padding: EdgeInsets.only(bottom: d.isTv ? 12 : 16),
      child: FocusTraversalGroup(
        child: SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          clipBehavior: Clip.none,
          padding: EdgeInsets.only(
            left: d.pageInsets.left,
            right: d.pageInsets.right,
          ),
          child: Row(
            children: [
              for (var i = 0; i < children.length; i++) ...[
                if (i > 0) const SizedBox(width: 8),
                children[i],
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// Sign-in button, or the account with "Sign out".
class _AccountButton extends StatelessWidget {
  const _AccountButton(this.twitch);

  final TwitchService twitch;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final account = twitch.account;
    if (account != null) {
      // UI-021: 48×48 to touch although the row is drawn smaller.
      return MinTapTarget(
        enabled: KDensity.of(context).isPhone,
        child: PopupMenuButton<void>(
          key: const ValueKey('twitch-account'),
          tooltip: l.twitchSignedInAs(account.login),
          itemBuilder: (_) => [
            // TWI-032: a new avatar or name on Twitch shows up now.
            PopupMenuItem(
              onTap: () => unawaited(twitch.refreshAccount(force: true)),
              child: Text(l.twitchAccountRefresh),
            ),
            PopupMenuItem(
              onTap: () => unawaited(twitch.signOut()),
              child: Text(l.twitchSignOut),
            ),
          ],
          child: Padding(
            padding: const EdgeInsets.all(8),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                TwitchAccountAvatar(
                  login: account.login,
                  name: twitch.accountName ?? account.login,
                  url: twitch.accountProfile?.avatarUrl,
                ),
                const SizedBox(width: 8),
                ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 160),
                  child: Text(
                    twitch.accountName ?? account.login,
                    key: const ValueKey('twitch-account-name'),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ],
            ),
          ),
        ),
      );
    }
    if (!twitch.canSignIn) return const SizedBox.shrink();
    return KButton(
      key: const ValueKey('twitch-sign-in-button'),
      label: l.twitchSignIn,
      icon: Icons.login,
      onPressed: () => showTwitchSignIn(context),
    );
  }
}

void _openChannel(BuildContext context, TwitchChannel channel) =>
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => TwitchChannelScreen(channel: channel),
      ),
    );

void _openCategory(BuildContext context, TwitchCategory category) =>
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => TwitchCategoryScreen(category: category),
      ),
    );

class _HomeRows extends StatelessWidget {
  const _HomeRows({required this.home, required this.onSearch});

  final TwitchHome home;
  final VoidCallback onSearch;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final services = Services.of(context);
    final twitch = services.twitch;
    final now = services.clock();
    Widget gap(Widget child) => Padding(
      padding: EdgeInsets.only(bottom: d.rowGap),
      child: child,
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (!home.liveKnown)
          TwitchInfoCard(
            key: const ValueKey('twitch-sign-in-hint'),
            icon: Icons.sensors_off_outlined,
            text: twitch.canSignIn
                ? l.twitchSignInNeeded
                : l.twitchSignInUnavailable,
            action: twitch.canSignIn
                ? KButton(
                    label: l.twitchSignIn,
                    icon: Icons.login,
                    onPressed: () => showTwitchSignIn(context),
                  )
                : null,
          ),
        if (home.live.isNotEmpty)
          gap(
            MediaRow(
              key: const ValueKey('twitch-row-live'),
              title: l.twitchLiveNow,
              itemCount: home.live.length,
              height: twitchRowHeight(d),
              itemBuilder: (context, i) {
                final s = home.live[i];
                return TwitchLiveCard(
                  key: ValueKey('twitch-live-${s.login}'),
                  stream: s,
                  channel: twitch.channelOf(s.login),
                  width: d.landscapeWidth,
                  onTap: () =>
                      playTwitch(context, TwitchPlayTarget.live(s.login)),
                );
              },
            ),
          ),
        // TWI-022: 2–4 of them at once on a desktop, a TV or a tablet.
        if (home.live.length >= 2 &&
            !multiviewIsPhone(
              FormFactorScope.of(context),
              MediaQuery.sizeOf(context),
            ))
          TwitchInfoCard(
            key: const ValueKey('twitch-multiview-card'),
            icon: Icons.grid_view,
            text: l.twitchMultiviewCard,
            action: KButton(
              key: const ValueKey('twitch-multiview-open'),
              label: l.twitchMultiviewOpen,
              icon: Icons.grid_view,
              onPressed: () => openTwitchMultiview(context, live: home.live),
            ),
          ),
        if (home.favorites.isEmpty)
          TwitchInfoCard(
            key: const ValueKey('twitch-favorites-empty'),
            icon: Icons.star_outline,
            text:
                '${l.twitchFavoritesEmptyTitle}. ${l.twitchFavoritesEmptyMessage}',
            action: KButton(
              label: l.twitchTabSearch,
              icon: Icons.search,
              onPressed: onSearch,
            ),
          )
        else
          gap(_FavoritesRow(entries: home.favorites, onAdd: onSearch)),
        if (home.continueVods.isNotEmpty)
          gap(
            _VodRow(
              rowKey: 'twitch-row-continue',
              title: l.twitchContinue,
              videos: home.continueVods,
              now: now,
            ),
          ),
        if (home.newVods.isNotEmpty)
          gap(
            _VodRow(
              rowKey: 'twitch-row-new',
              title: l.twitchNewVods,
              videos: home.newVods,
              now: now,
            ),
          ),
        // Downloads and recordings: full builds only (ADR-0011 §5).
        // ADR-NNNN: not at all for a kid profile (the recordings exist,
        // they are only hidden).
        if (AppFlavor.twitchArchive &&
            (Services.of(context).twitchArchive == null ||
                Services.of(context).visibleArchive != null)) ...[
          TwitchRowTitle(l.twitchDownloaded),
          // TWI-019: the library of recordings and downloads once the
          // queue is open (signed in, the remote config allows it).
          if (Services.of(context).visibleArchive != null)
            TwitchInfoCard(
              key: const ValueKey('twitch-archive-card'),
              icon: Icons.video_library_outlined,
              text: l.twitchArchiveCard,
              action: KButton(
                key: const ValueKey('twitch-archive-open'),
                icon: Icons.video_library_outlined,
                label: l.twitchArchiveOpen,
                onPressed: () => openTwitchArchive(context),
              ),
            )
          else
            TwitchInfoCard(
              key: const ValueKey('twitch-downloads-later'),
              icon: Icons.download_outlined,
              text: l.twitchDownloadedLater,
            ),
        ],
        if (home.soon.isNotEmpty)
          gap(
            MediaRow(
              key: const ValueKey('twitch-row-soon'),
              title: l.twitchSoonLive,
              itemCount: home.soon.length,
              height: d.isTv ? 74 : 80,
              itemBuilder: (context, i) => _UpcomingCard(
                upcoming: home.soon[i],
                now: now,
                width: d.landscapeWidth,
              ),
            ),
          ),
        if (home.categories.isNotEmpty) ...[
          TwitchRowTitle(l.twitchCategories),
          gap(
            Padding(
              padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
              child: FocusTraversalGroup(
                child: Wrap(
                  spacing: 8,
                  runSpacing: 8,
                  children: [
                    for (final c in home.categories)
                      FilterPill(
                        key: ValueKey('twitch-category-${c.category.id}'),
                        label: '${c.category.name} · ${c.count}',
                        icon: Icons.sports_esports_outlined,
                        selected: false,
                        onTap: () => _openCategory(context, c.category),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ],
      ],
    );
  }
}

class _VodRow extends StatelessWidget {
  const _VodRow({
    required this.rowKey,
    required this.title,
    required this.videos,
    required this.now,
  });

  final String rowKey;
  final String title;
  final List<TwitchVideo> videos;
  final DateTime now;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    return MediaRow(
      key: ValueKey(rowKey),
      title: title,
      itemCount: videos.length,
      height: twitchRowHeight(d, extra: 8),
      itemBuilder: (context, i) => TwitchVodCard(
        key: ValueKey('$rowKey-${videos[i].id}'),
        video: videos[i],
        width: d.landscapeWidth,
        now: now,
        onTap: () => playTwitchVideo(context, videos[i]),
      ),
    );
  }
}

/// Favourites as round avatars with the LIVE ring ("stories"), then "+".
class _FavoritesRow extends StatelessWidget {
  const _FavoritesRow({required this.entries, required this.onAdd});

  final List<TwitchFavoriteEntry> entries;
  final VoidCallback onAdd;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final size = d.isTv ? 64.0 : (d.isPhone ? 64.0 : 76.0);
    final theme = Theme.of(context);
    Widget tile({
      required Key key,
      required Widget avatar,
      required String label,
      required VoidCallback onTap,
    }) => SizedBox(
      width: size + 16,
      child: KFocusable(
        key: key,
        onActivate: onTap,
        radius: size,
        light: false,
        semanticLabel: label,
        builder: (context, focused) => Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            avatar,
            const SizedBox(height: 6),
            Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              textAlign: TextAlign.center,
              style: theme.textTheme.labelSmall,
            ),
          ],
        ),
      ),
    );
    return MediaRow(
      key: const ValueKey('twitch-row-favorites'),
      title: l.twitchFavorites,
      itemCount: entries.length + 1,
      height: size + 40,
      itemBuilder: (context, i) {
        if (i == entries.length) {
          return tile(
            key: const ValueKey('twitch-favorite-add'),
            avatar: SizedBox.square(
              dimension: size,
              child: DecoratedBox(
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  border: Border.all(color: KColors.stroke, width: 2),
                ),
                child: Icon(Icons.add, color: KColors.textSecondary),
              ),
            ),
            label: l.twitchAddFavorite,
            onTap: onAdd,
          );
        }
        final e = entries[i];
        return tile(
          key: ValueKey('twitch-favorite-${e.channel.login}'),
          avatar: Stack(
            clipBehavior: Clip.none,
            alignment: Alignment.bottomCenter,
            children: [
              TwitchAvatar(channel: e.channel, size: size, live: e.isLive),
              if (e.isLive)
                const Positioned(bottom: -4, child: LiveBadge(small: true)),
              if (e.favorite.pinned)
                Positioned(
                  top: 0,
                  right: 0,
                  child: Icon(
                    Icons.push_pin,
                    size: 14,
                    color: KColors.textSecondary,
                  ),
                ),
            ],
          ),
          label: e.channel.displayName,
          onTap: () => _openChannel(context, e.channel),
        );
      },
    );
  }
}

/// A scheduled broadcast: avatar, name, when, title.
class _UpcomingCard extends StatelessWidget {
  const _UpcomingCard({
    required this.upcoming,
    required this.now,
    required this.width,
  });

  final TwitchUpcoming upcoming;
  final DateTime now;
  final double width;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final s = upcoming.segment;
    final hours = s.start.difference(now).inHours.clamp(1, 9999);
    final cat = s.categoryName;
    return SizedBox(
      width: width,
      child: KFocusable(
        key: ValueKey('twitch-soon-${upcoming.channel.login}-${s.id}'),
        onActivate: () => _openChannel(context, upcoming.channel),
        light: false,
        radius: 12,
        semanticLabel: upcoming.channel.displayName,
        builder: (context, focused) => Container(
          padding: const EdgeInsets.all(10),
          decoration: BoxDecoration(
            color: KColors.bgElevated,
            borderRadius: BorderRadius.circular(12),
          ),
          child: Row(
            children: [
              TwitchAvatar(channel: upcoming.channel, size: 44),
              const SizedBox(width: 10),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisAlignment: MainAxisAlignment.center,
                  children: [
                    Text(
                      '${upcoming.channel.displayName} · ${l.twitchInHours(hours)}',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.titleSmall,
                    ),
                    Text(
                      [s.title, ?cat].join(' · '),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.labelSmall,
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

// ------------------------------------------------------------------ channels

String _sortLabel(AppLocalizations l, TwitchFavoriteSort s) => switch (s) {
  TwitchFavoriteSort.liveFirst => l.twitchSortLiveFirst,
  TwitchFavoriteSort.viewers => l.twitchSortViewers,
  TwitchFavoriteSort.name => l.twitchSortName,
  TwitchFavoriteSort.recentlyAdded => l.twitchSortRecentlyAdded,
  TwitchFavoriteSort.lastLive => l.twitchSortLastLive,
};

class _ChannelsTab extends StatefulWidget {
  const _ChannelsTab({required this.twitch});

  final TwitchService twitch;

  @override
  State<_ChannelsTab> createState() => _ChannelsTabState();
}

class _ChannelsTabState extends State<_ChannelsTab> {
  bool _following = false;
  Future<List<TwitchFavoriteEntry>>? _followed;

  void _loadFollowing() {
    _followed = widget.twitch.following().catchError(
      (_) => <TwitchFavoriteEntry>[],
    );
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final twitch = widget.twitch;
    final signedIn = twitch.account != null;
    final following = _following && signedIn;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _Pills(
          children: [
            FilterPill(
              key: const ValueKey('twitch-channels-favorites'),
              label: l.twitchFavorites,
              icon: Icons.star_outline,
              selected: !following,
              onTap: () => setState(() => _following = false),
            ),
            if (signedIn || twitch.canSignIn)
              FilterPill(
                key: const ValueKey('twitch-channels-following'),
                label: l.twitchFollowing,
                icon: Icons.people_outline,
                selected: following,
                onTap: () async {
                  if (!signedIn) {
                    await showTwitchSignIn(context);
                    if (twitch.account == null) return;
                  }
                  setState(() {
                    _following = true;
                    _loadFollowing();
                  });
                },
              ),
            const SizedBox(width: 8),
            for (final (f, label) in [
              (TwitchFavoriteFilter.all, l.twitchFilterAll),
              (TwitchFavoriteFilter.live, l.twitchFilterLive),
              (TwitchFavoriteFilter.offline, l.twitchFilterOffline),
            ])
              FilterPill(
                key: ValueKey('twitch-filter-${f.name}'),
                label: label,
                selected: twitch.filter == f,
                onTap: () async {
                  await twitch.setFilter(f);
                  if (following) setState(_loadFollowing);
                },
              ),
            PopupMenuButton<TwitchFavoriteSort>(
              key: const ValueKey('twitch-sort'),
              tooltip: l.twitchSort,
              initialValue: twitch.sort,
              onSelected: (s) async {
                await twitch.setSort(s);
                if (following) setState(_loadFollowing);
              },
              itemBuilder: (_) => [
                for (final s in TwitchFavoriteSort.values)
                  PopupMenuItem(
                    key: ValueKey('twitch-sort-${s.name}'),
                    value: s,
                    child: Text(_sortLabel(l, s)),
                  ),
              ],
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 8),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const Icon(Icons.sort, size: 18),
                    const SizedBox(width: 4),
                    Text(_sortLabel(l, twitch.sort)),
                  ],
                ),
              ),
            ),
          ],
        ),
        if (following)
          FutureBuilder<List<TwitchFavoriteEntry>>(
            future: _followed,
            builder: (context, snap) =>
                _ChannelList(entries: snap.data ?? const [], favorites: false),
          )
        else if (twitch.favorites.isEmpty)
          Padding(
            padding: EdgeInsets.symmetric(vertical: d.rowGap),
            child: EmptyState(
              icon: Icons.star_outline,
              title: l.twitchFavoritesEmptyTitle,
              message: l.twitchFavoritesEmptyMessage,
            ),
          )
        else
          _ChannelList(entries: twitch.favoriteEntries(), favorites: true),
      ],
    );
  }
}

class _ChannelList extends StatelessWidget {
  const _ChannelList({required this.entries, required this.favorites});

  final List<TwitchFavoriteEntry> entries;
  final bool favorites;

  @override
  Widget build(BuildContext context) => FocusTraversalGroup(
    child: Column(
      children: [
        for (final e in entries)
          ChannelTile(
            key: ValueKey('twitch-channel-${e.channel.login}'),
            channel: e.channel,
            live: e.live,
            pinned: favorites ? e.favorite.pinned : null,
          ),
      ],
    ),
  );
}

/// A channel in a list: avatar with the LIVE ring, name, status, and the
/// favourite (and pin) toggles.
class ChannelTile extends StatelessWidget {
  const ChannelTile({
    super.key,
    required this.channel,
    this.live,
    this.pinned,
    this.onOpened,
    this.showWatch = false,
  });

  final TwitchChannel channel;
  final TwitchStream? live;

  /// Shows the pin toggle (favourites list).
  final bool? pinned;

  /// Called when the tile opens the channel or the stream (search: the
  /// query goes to the recent searches).
  final VoidCallback? onOpened;

  /// A "Watch live" button while the channel is live (search, TWI-034).
  final bool showWatch;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final services = Services.of(context);
    final twitch = services.twitch;
    final fav = twitch.isFavorite(channel.login);
    final s = live;
    final last = channel.lastLiveAt;
    final status = s != null
        ? [
            l.twitchFilterLive,
            ?s.gameName,
            compactCount(context, s.viewers),
          ].join(' · ')
        : channel.userId == null
        ? l.twitchNotResolved
        : last != null
        ? l.twitchLastLive(timeAgo(l, services.clock(), last))
        : l.twitchOffline;
    return Padding(
      padding: EdgeInsets.symmetric(
        horizontal: d.pageInsets.left - 8,
        vertical: 2,
      ),
      child: KFocusable(
        onActivate: () {
          onOpened?.call();
          _openChannel(context, channel);
        },
        light: false,
        radius: 12,
        semanticLabel: channel.displayName,
        builder: (context, focused) => Padding(
          padding: const EdgeInsets.all(8),
          child: Row(
            children: [
              TwitchAvatar(
                channel: channel,
                size: d.isTv ? 44 : 48,
                live: s != null,
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      channel.displayName,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.titleSmall,
                    ),
                    Text(
                      status,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.labelSmall?.copyWith(
                        color: s != null ? KColors.live : null,
                      ),
                    ),
                  ],
                ),
              ),
              if (showWatch && s != null)
                IconButton(
                  key: ValueKey('twitch-search-watch-${channel.login}'),
                  tooltip: l.twitchWatchLive,
                  onPressed: () {
                    onOpened?.call();
                    playTwitch(context, TwitchPlayTarget.live(channel.login));
                  },
                  icon: Icon(Icons.play_circle_fill, color: KColors.live),
                ),
              if (pinned != null)
                IconButton(
                  key: ValueKey('twitch-pin-${channel.login}'),
                  tooltip: pinned! ? l.twitchUnpin : l.twitchPin,
                  onPressed: () => twitch.setPinned(channel.login, !pinned!),
                  icon: Icon(
                    pinned! ? Icons.push_pin : Icons.push_pin_outlined,
                    color: pinned! ? KColors.textPrimary : KColors.textTertiary,
                  ),
                ),
              IconButton(
                key: ValueKey('twitch-star-${channel.login}'),
                tooltip: fav ? l.twitchRemoveFavorite : l.twitchAddFavorite,
                onPressed: () => twitch.toggleFavorite(channel),
                icon: Icon(
                  fav ? Icons.star_rounded : Icons.star_outline_rounded,
                  color: fav ? KColors.accent : KColors.textTertiary,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

// -------------------------------------------------------------------- search

class _SearchTab extends StatefulWidget {
  const _SearchTab({required this.twitch});

  final TwitchService twitch;

  @override
  State<_SearchTab> createState() => _SearchTabState();
}

class _SearchTabState extends State<_SearchTab> {
  final _controller = TextEditingController();
  final _debounce = Debouncer(const Duration(milliseconds: 250));
  Future<TwitchSearchResults>? _results;

  @override
  void dispose() {
    _debounce.dispose();
    _controller.dispose();
    super.dispose();
  }

  /// The last answer, shown while the next one loads (no flicker while
  /// typing).
  TwitchSearchResults? _last;

  void _search(String q) {
    final results = widget.twitch.search(q);
    results.then((r) => _last = r, onError: (_) {});
    setState(() {
      _results = q.trim().isEmpty ? null : results;
      if (q.trim().isEmpty) _last = null;
    });
  }

  /// Enter, or a result opened: the query goes to the recent searches.
  void _remember() => unawaited(widget.twitch.rememberSearch(_controller.text));

  void _useRecent(String q) {
    _controller.text = q;
    _controller.selection = TextSelection.collapsed(offset: q.length);
    _search(q);
  }

  Widget _recent(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final recent = widget.twitch.recentSearches;
    if (recent.isEmpty) return const SizedBox.shrink();
    return Column(
      key: const ValueKey('twitch-search-recent'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.only(right: d.pageInsets.right - 8),
          child: Row(
            children: [
              Expanded(child: TwitchRowTitle(l.twitchSearchRecent)),
              TextButton(
                key: const ValueKey('twitch-search-recent-clear'),
                onPressed: widget.twitch.clearRecentSearches,
                child: Text(l.commonClear),
              ),
            ],
          ),
        ),
        Padding(
          padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
          child: FocusTraversalGroup(
            child: Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                for (final q in recent)
                  FilterPill(
                    key: ValueKey('twitch-search-recent-$q'),
                    label: q,
                    icon: Icons.history,
                    selected: false,
                    onTap: () => _useRecent(q),
                  ),
              ],
            ),
          ),
        ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
          child: TextField(
            key: const ValueKey('twitch-search-field'),
            controller: _controller,
            autofocus: !d.isPhone,
            textInputAction: TextInputAction.search,
            decoration: InputDecoration(
              hintText: l.twitchSearchHint,
              prefixIcon: const Icon(Icons.search),
            ),
            onChanged: (q) => _debounce(() => _search(q)),
            onSubmitted: (q) {
              _search(q);
              _remember();
            },
          ),
        ),
        const SizedBox(height: 12),
        if (widget.twitch.account == null)
          Padding(
            padding: EdgeInsets.fromLTRB(
              d.pageInsets.left,
              0,
              d.pageInsets.right,
              12,
            ),
            child: Text(
              l.twitchSearchOffline,
              style: theme.textTheme.bodySmall,
            ),
          ),
        if (_results == null) _recent(context),
        FutureBuilder<TwitchSearchResults>(
          future: _results,
          builder: (context, snap) {
            final r = _results == null ? null : (snap.data ?? _last);
            if (r == null) return const SizedBox.shrink();
            final add = r.addByName;
            if (r.channels.isEmpty && r.categories.isEmpty && add == null) {
              return Padding(
                padding: const EdgeInsets.all(24),
                child: Center(child: Text(l.twitchSearchEmpty)),
              );
            }
            return Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                if (add != null)
                  Padding(
                    padding: EdgeInsets.symmetric(
                      horizontal: d.pageInsets.left,
                      vertical: 4,
                    ),
                    child: Wrap(
                      spacing: 12,
                      runSpacing: 8,
                      children: [
                        KButton(
                          key: const ValueKey('twitch-add-by-name'),
                          label: l.twitchAddChannel(add),
                          icon: Icons.star_outline,
                          onPressed: () async {
                            await widget.twitch.addFavoriteByName(add);
                            _search(_controller.text);
                          },
                        ),
                        // TWI-012: open the channel's chat by name.
                        KButton(
                          key: const ValueKey('twitch-search-chat'),
                          label: l.twitchChatOpen,
                          icon: Icons.chat_bubble_outline,
                          onPressed: () => openTwitchChat(context, add),
                        ),
                      ],
                    ),
                  ),
                if (r.channels.isNotEmpty) ...[
                  const SizedBox(height: 8),
                  TwitchRowTitle(l.twitchSearchChannels),
                  // On air first, most viewers first (TWI-034).
                  FocusTraversalGroup(
                    child: Column(
                      children: [
                        for (final c in r.channels)
                          ChannelTile(
                            key: ValueKey('twitch-result-${c.login}'),
                            channel: c,
                            live: widget.twitch.liveOf(c),
                            onOpened: _remember,
                            showWatch: true,
                          ),
                      ],
                    ),
                  ),
                ],
                if (r.categories.isNotEmpty) ...[
                  const SizedBox(height: 12),
                  MediaRow(
                    key: const ValueKey('twitch-result-categories'),
                    title: l.twitchSearchCategories,
                    itemCount: r.categories.length,
                    height: d.posterWidth * 4 / 3 + cardCaptionHeight(d) + 8,
                    itemBuilder: (context, i) {
                      final c = r.categories[i];
                      return TwitchCategoryCard(
                        key: ValueKey('twitch-result-category-${c.id}'),
                        category: c,
                        width: d.posterWidth,
                        onTap: () {
                          _remember();
                          _openCategory(context, c);
                        },
                      );
                    },
                  ),
                ],
              ],
            );
          },
        ),
      ],
    );
  }
}

/// `Sun 15 Nov, 16:00` in the UI language.
String formatScheduleTime(BuildContext context, DateTime t) {
  final tag = Localizations.localeOf(context).toLanguageTag();
  return DateFormat.MMMEd(tag).add_Hm().format(t);
}
