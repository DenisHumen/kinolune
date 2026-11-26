// SPDX-License-Identifier: MPL-2.0
//
// Channel page of the "Streams" section (TWI-013, research 06 §6.3): banner,
// avatar with the LIVE ring, status, "Watch live", favourite, "Open on
// Twitch"; tabs VODs (with the chapter bar), highlights, clips, schedule
// and about. Also the category page: live streams of the category (TWI-034)
// and VODs of the favourites where the category was on air (TWI-008
// chapters, not the channel's current one).

import 'package:material_ui/material_ui.dart';

import '../../flavor.dart';
import '../../l10n/l10n.dart';
import '../../services/app_services.dart';
import '../../theme/kinolune_theme.dart';
import '../../twitch/twitch_models.dart';
import '../../twitch/twitch_service.dart';
import '../../widgets/artwork.dart';
import '../../widgets/cards.dart';
import '../../widgets/focusable.dart';
import '../library_grid_screen.dart' show FilterPill;
import '../twitch_screen.dart' show formatScheduleTime;
import 'twitch_archive_screens.dart';
import 'twitch_browse.dart';
import 'twitch_follow.dart';
import 'twitch_play.dart';
import 'twitch_record_live.dart';
import 'twitch_widgets.dart';

enum _ChannelTab { vods, highlights, clips, schedule, about }

/// Categories seen on a channel `(game id, name)`: live now, then the
/// chapters of its VODs (TWI-019 record rule filter).
List<(String, String)> _categories(TwitchChannelPage page) {
  final seen = <String, String>{};
  if (page.live case TwitchStream(:final gameId?, :final gameName?)) {
    seen[gameId] = gameName;
  }
  for (final v in [...page.vods, ...page.highlights]) {
    for (final c in v.chapters) {
      if (c.gameName case final name?) seen.putIfAbsent(c.gameId, () => name);
    }
  }
  return [for (final e in seen.entries) (e.key, e.value)];
}

class TwitchChannelScreen extends StatefulWidget {
  const TwitchChannelScreen({super.key, required this.channel});

  final TwitchChannel channel;

  @override
  State<TwitchChannelScreen> createState() => _TwitchChannelScreenState();
}

class _TwitchChannelScreenState extends State<TwitchChannelScreen> {
  Future<TwitchChannelPage>? _page;
  _ChannelTab _tab = _ChannelTab.vods;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _page ??= Services.of(context).twitch.channelPage(widget.channel);
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      key: ValueKey('twitch-channel-page-${widget.channel.login}'),
      backgroundColor: KColors.bgBase,
      body: FutureBuilder<TwitchChannelPage>(
        future: _page,
        builder: (context, snap) {
          final page = snap.data ?? TwitchChannelPage(channel: widget.channel);
          return ListenableBuilder(
            listenable: Services.of(context).twitch,
            builder: (context, _) => _content(context, page, snap.hasData),
          );
        },
      ),
    );
  }

  Widget _content(BuildContext context, TwitchChannelPage page, bool loaded) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final services = Services.of(context);
    final twitch = services.twitch;
    final now = services.clock();
    final c = page.channel;
    final live = page.live;
    final fav = twitch.isFavorite(c.login);
    final width = MediaQuery.sizeOf(context).width;
    final bannerH = (width * (d.isPhone ? .42 : .22)).clamp(120.0, 300.0);
    final avatar = d.isPhone ? 76.0 : (d.isTv ? 84.0 : 104.0);
    final last = c.lastLiveAt;
    final status = live != null
        ? [live.title, ?live.gameName].join(' · ')
        : last != null
        ? l.twitchLastLive(timeAgo(l, now, last))
        : l.twitchOffline;

    final header = Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(
          height: bannerH,
          child: Stack(
            fit: StackFit.expand,
            children: [
              Artwork(
                url: c.bannerUrl,
                seed: 'banner-${c.login}',
                style: ArtStyle.backdrop,
              ),
              DecoratedBox(
                decoration: BoxDecoration(
                  gradient: LinearGradient(
                    begin: Alignment.topCenter,
                    end: Alignment.bottomCenter,
                    colors: [const Color(0x550B0B0F), KColors.bgBase],
                  ),
                ),
              ),
              if (Navigator.of(context).canPop() && !d.isTv)
                SafeArea(
                  child: Align(
                    alignment: Alignment.topLeft,
                    child: Padding(
                      padding: const EdgeInsets.all(8),
                      child: IconButton(
                        tooltip: l.commonBack,
                        onPressed: () => Navigator.of(context).maybePop(),
                        icon: const Icon(Icons.arrow_back),
                      ),
                    ),
                  ),
                ),
            ],
          ),
        ),
        Padding(
          padding: EdgeInsets.fromLTRB(
            d.pageInsets.left,
            0,
            d.pageInsets.right,
            12,
          ),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              TwitchAvatar(channel: c, size: avatar, live: live != null),
              const SizedBox(width: 16),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Flexible(
                          child: Text(
                            c.displayName,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: theme.textTheme.headlineMedium,
                          ),
                        ),
                        if (live != null) ...[
                          const SizedBox(width: 8),
                          const LiveBadge(),
                        ],
                      ],
                    ),
                    const SizedBox(height: 4),
                    Text(
                      status,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.bodySmall,
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
        Padding(
          padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
          child: FocusTraversalGroup(
            child: Wrap(
              spacing: 12,
              runSpacing: 12,
              children: [
                if (live != null)
                  KButton(
                    key: const ValueKey('twitch-watch-live'),
                    primary: true,
                    autofocus: true,
                    icon: Icons.play_arrow_rounded,
                    label: l.twitchWatchLive,
                    onPressed: () =>
                        playTwitch(context, TwitchPlayTarget.live(c.login)),
                  ),
                KButton(
                  key: const ValueKey('twitch-channel-favorite'),
                  autofocus: live == null,
                  icon: fav ? Icons.star_rounded : Icons.star_outline_rounded,
                  // TWI-033: the label says it is Kinolune's, not Twitch's.
                  label: fav
                      ? l.twitchInKinoluneFavorites
                      : l.twitchAddFavorite,
                  onPressed: () => twitch.toggleFavorite(c),
                ),
                // TWI-033: follow on Twitch (signed in only, ADR-NNNN).
                if (twitch.account != null) TwitchFollowButton(channel: c),
                // TWI-009: the favourite's "X is live" bell.
                if (fav)
                  KButton(
                    key: const ValueKey('twitch-channel-notify'),
                    icon: twitch.notifyOf(c.login)
                        ? Icons.notifications_active_outlined
                        : Icons.notifications_off_outlined,
                    label: twitch.notifyOf(c.login)
                        ? l.twitchLiveNotifyOn
                        : l.twitchLiveNotifyOff,
                    onPressed: () =>
                        twitch.setNotify(c.login, !twitch.notifyOf(c.login)),
                  ),
                // TWI-019: the favourite's record rule (full builds).
                if (AppFlavor.twitchArchive &&
                    fav &&
                    c.userId != null &&
                    services.visibleArchive != null)
                  KButton(
                    key: const ValueKey('twitch-channel-record'),
                    icon: Icons.radio_button_checked,
                    label: l.twitchRuleButton,
                    onPressed: () => openTwitchRecordRule(
                      context,
                      userId: c.userId!,
                      displayName: c.displayName,
                      categories: _categories(page),
                    ),
                  ),
                // TWI-035: record the broadcast on air (full builds).
                if (AppFlavor.twitchArchive &&
                    live != null &&
                    services.visibleArchive != null)
                  TwitchRecordLiveButton(
                    login: c.login,
                    userId: c.userId,
                    onDark: false,
                  ),
                // TWI-012: the read-only chat works without a login.
                KButton(
                  key: const ValueKey('twitch-open-chat'),
                  icon: Icons.chat_bubble_outline,
                  label: l.twitchChatOpen,
                  onPressed: () => openTwitchChat(context, c.login),
                ),
                KButton(
                  key: const ValueKey('twitch-open-on-twitch'),
                  icon: Icons.open_in_new,
                  label: l.twitchOpenOnTwitch,
                  onPressed: () => showOpenOnTwitch(context, c.url),
                ),
              ],
            ),
          ),
        ),
        SizedBox(height: d.isTv ? 14 : 18),
        if (loaded && !page.online)
          TwitchInfoCard(
            icon: Icons.cloud_off_outlined,
            text: l.twitchCachedData,
          ),
        Padding(
          padding: EdgeInsets.only(bottom: d.isTv ? 10 : 14),
          child: FocusTraversalGroup(
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              clipBehavior: Clip.none,
              padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
              child: Row(
                children: [
                  for (final (tab, label) in [
                    (_ChannelTab.vods, l.twitchTabVods),
                    (_ChannelTab.highlights, l.twitchTabHighlights),
                    (_ChannelTab.clips, l.twitchTabClips),
                    (_ChannelTab.schedule, l.twitchTabSchedule),
                    (_ChannelTab.about, l.twitchTabAbout),
                  ]) ...[
                    FilterPill(
                      key: ValueKey('twitch-channel-tab-${tab.name}'),
                      label: label,
                      selected: _tab == tab,
                      onTap: () => setState(() => _tab = tab),
                    ),
                    const SizedBox(width: 8),
                  ],
                ],
              ),
            ),
          ),
        ),
      ],
    );

    final List<Widget> body;
    switch (_tab) {
      case _ChannelTab.vods:
      case _ChannelTab.highlights:
        final vods = _tab == _ChannelTab.vods ? page.vods : page.highlights;
        body = [
          if (vods.isEmpty)
            _empty(context, loaded, l.twitchNoVods)
          else
            _grid(
              context,
              vods.length,
              extra: 8,
              (i) => TwitchVodCard(
                key: ValueKey('twitch-vod-${vods[i].id}'),
                video: vods[i],
                width: double.infinity,
                now: now,
                showChannel: false,
                onTap: () => playTwitchVideo(context, vods[i]),
              ),
            ),
        ];
      case _ChannelTab.clips:
        body = [
          if (page.clips.isEmpty)
            _empty(context, loaded, l.twitchNoClips)
          else
            _grid(
              context,
              page.clips.length,
              (i) => TwitchClipCard(
                clip: page.clips[i],
                width: double.infinity,
                onTap: () => showOpenOnTwitch(context, page.clips[i].url),
              ),
            ),
        ];
      case _ChannelTab.schedule:
        body = [
          if (page.schedule.isEmpty)
            _empty(context, loaded, l.twitchNoSchedule)
          else
            for (final s in page.schedule)
              Padding(
                padding: EdgeInsets.symmetric(
                  horizontal: d.pageInsets.left,
                  vertical: 4,
                ),
                child: Container(
                  padding: const EdgeInsets.all(12),
                  decoration: BoxDecoration(
                    color: KColors.bgElevated,
                    borderRadius: BorderRadius.circular(12),
                  ),
                  child: Row(
                    children: [
                      Icon(Icons.event_outlined, color: KColors.textSecondary),
                      const SizedBox(width: 12),
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Text(
                              formatScheduleTime(context, s.start),
                              style: theme.textTheme.titleSmall,
                            ),
                            Text(
                              [s.title, ?s.categoryName].join(' · '),
                              maxLines: 2,
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
        ];
      case _ChannelTab.about:
        body = [
          Padding(
            padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
            child: Text(c.description ?? '', style: theme.textTheme.bodyMedium),
          ),
        ];
    }

    return ListView(
      padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 24),
      children: [
        header,
        ...body,
        const SizedBox(height: 16),
        const TwitchAttribution(),
      ],
    );
  }

  Widget _empty(BuildContext context, bool loaded, String text) => Padding(
    padding: const EdgeInsets.all(24),
    child: Center(
      child: loaded ? Text(text) : const CircularProgressIndicator(),
    ),
  );
}

/// A grid of 16:9 cards with page insets.
Widget _grid(
  BuildContext context,
  int count,
  Widget Function(int i) item, {
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
        gridDelegate: CardGridDelegate(
          minItemWidth: d.landscapeWidth,
          crossSpacing: d.gap,
          mainSpacing: d.gap,
          artAspect: 9 / 16,
          extraHeight: cardCaptionHeight(d) + 8 + extra,
        ),
        itemCount: count,
        itemBuilder: (_, i) => item(i),
      ),
    ),
  );
}

/// The category page: live streams now, most viewers first, in the chosen
/// broadcast language (TWI-034), then the VODs of the favourites where the
/// category was on air (TWI-008 chapters).
class TwitchCategoryScreen extends StatefulWidget {
  const TwitchCategoryScreen({super.key, required this.category});

  final TwitchCategory category;

  @override
  State<TwitchCategoryScreen> createState() => _TwitchCategoryScreenState();
}

class _TwitchCategoryScreenState extends State<TwitchCategoryScreen> {
  Future<List<TwitchVideo>>? _vods;
  final _scrollEnd = TwitchScrollEnd();

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _vods ??= Services.of(context).twitch.categoryVideos(widget.category);
  }

  @override
  void dispose() {
    _scrollEnd.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final services = Services.of(context);
    final now = services.clock();
    return Scaffold(
      key: ValueKey('twitch-category-page-${widget.category.id}'),
      backgroundColor: KColors.bgBase,
      body: NotificationListener<ScrollNotification>(
        onNotification: _scrollEnd.onScroll,
        child: FutureBuilder<List<TwitchVideo>>(
          future: _vods,
          builder: (context, snap) {
            final vods = snap.data ?? const <TwitchVideo>[];
            return ListView(
              padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 24),
              children: [
                PageTitle(
                  widget.category.name,
                  trailing: KButton(
                    icon: Icons.open_in_new,
                    label: l.twitchOpenOnTwitch,
                    onPressed: () =>
                        showOpenOnTwitch(context, widget.category.url),
                  ),
                ),
                if (Navigator.of(context).canPop() && !d.isTv)
                  Padding(
                    padding: EdgeInsets.only(left: d.pageInsets.left - 8),
                    child: Align(
                      alignment: Alignment.centerLeft,
                      child: TextButton.icon(
                        onPressed: () => Navigator.of(context).maybePop(),
                        icon: const Icon(Icons.arrow_back),
                        label: Text(l.commonBack),
                      ),
                    ),
                  ),
                TwitchCategoryStreams(
                  category: widget.category,
                  scrollEnd: _scrollEnd,
                ),
                // The favourites' VODs with this game (TWI-008), after the
                // live streams and only when there are some.
                if (vods.isNotEmpty) ...[
                  SizedBox(height: d.rowGap),
                  TwitchRowTitle(l.twitchCategoryVodsTitle),
                  Padding(
                    padding: EdgeInsets.fromLTRB(
                      d.pageInsets.left,
                      0,
                      d.pageInsets.right,
                      12,
                    ),
                    child: Text(
                      l.twitchCategoryVodsHint,
                      style: Theme.of(context).textTheme.bodySmall,
                    ),
                  ),
                  _grid(
                    context,
                    vods.length,
                    extra: 8,
                    (i) => TwitchVodCard(
                      key: ValueKey('twitch-category-vod-${vods[i].id}'),
                      video: vods[i],
                      width: double.infinity,
                      now: now,
                      // TWI-027: the part of the playthrough of this game.
                      onTap: () => playTwitchVideo(
                        context,
                        vods[i],
                        gameId: widget.category.id,
                      ),
                    ),
                  ),
                ],
                const SizedBox(height: 16),
                const TwitchAttribution(),
              ],
            );
          },
        ),
      ),
    );
  }
}
