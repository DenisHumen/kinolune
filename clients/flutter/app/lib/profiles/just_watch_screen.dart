// SPDX-License-Identifier: MPL-2.0
//
// The "just watch" mode of a profile (UI-047): for children and elderly
// people. Instead of the app shell, one screen of large cards: Continue,
// One more episode (Up Next) and favourites. No navigation, settings,
// search or item menus; a card plays at once. The only other control is
// "Leave": the profile picker, where leaving this profile asks for the
// parent (adult) PIN ([ProfileController.needsFor]).

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../screens/home_feed.dart';
import '../screens/playback.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/artwork.dart';
import '../widgets/cards.dart';
import '../widgets/focusable.dart';
import '../widgets/format.dart';
import 'profile_avatar.dart';
import 'profile_picker.dart';

/// Key of the "Leave" button.
const justWatchExitKey = ValueKey('just-watch-exit');

class JustWatchScreen extends StatefulWidget {
  const JustWatchScreen({super.key});

  @override
  State<JustWatchScreen> createState() => _JustWatchScreenState();
}

class _JustWatchScreenState extends State<JustWatchScreen> {
  Library? _lib;
  late HomeFeed _feed;
  late Stream<List<ContinueEntry>> _continue;
  late Stream<List<UpNextEntry>> _upNext;
  late Stream<List<LibraryItem>> _favorites;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final lib = Services.of(context).library;
    if (identical(lib, _lib)) return;
    _lib = lib;
    _feed = HomeFeed(lib);
    // Library titles only: no Twitch VODs in this mode.
    _continue = _feed.continueRow(extra: const []);
    _upNext = _feed.upNext();
    _favorites = switch (lib) {
      final LibraryLists l => l.favorites(),
      _ => Stream.value(const <LibraryItem>[]),
    };
  }

  void _leave() => unawaited(
    Navigator.of(context).push(
      MaterialPageRoute<void>(builder: (_) => const ProfilePickerScreen()),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final services = Services.of(context);
    final active = services.profileControl.active;
    return Scaffold(
      key: const ValueKey('just-watch-screen'),
      body: SafeArea(
        child: StreamBuilder<List<ContinueEntry>>(
          stream: _continue,
          builder: (context, c) => StreamBuilder<List<UpNextEntry>>(
            stream: _upNext,
            builder: (context, n) => StreamBuilder<List<LibraryItem>>(
              stream: _favorites,
              builder: (context, f) {
                final cont = c.data ?? const <ContinueEntry>[];
                final next = n.data ?? const <UpNextEntry>[];
                final favs = f.data ?? const <LibraryItem>[];
                final loading =
                    c.connectionState == ConnectionState.waiting ||
                    f.connectionState == ConnectionState.waiting;
                final empty = cont.isEmpty && next.isEmpty && favs.isEmpty;
                var first = true;
                bool takeFirst() {
                  final r = first;
                  first = false;
                  return r;
                }

                return ListView(
                  padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 24),
                  children: [
                    Padding(
                      padding: EdgeInsets.fromLTRB(
                        d.pageInsets.left,
                        d.isTv ? 24 : 16,
                        d.pageInsets.right,
                        d.isTv ? 16 : 20,
                      ),
                      child: Row(
                        children: [
                          if (active != null) ...[
                            ProfileAvatar.of(active, size: d.isTv ? 56 : 48),
                            const SizedBox(width: 14),
                            Expanded(
                              child: Text(
                                profileName(l, active),
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: Theme.of(
                                  context,
                                ).textTheme.headlineMedium,
                              ),
                            ),
                          ] else
                            const Spacer(),
                          KButton(
                            key: justWatchExitKey,
                            icon: Icons.lock_outline,
                            label: l.justWatchExit,
                            onPressed: _leave,
                          ),
                        ],
                      ),
                    ),
                    if (empty && !loading)
                      Padding(
                        padding: const EdgeInsets.only(top: 48),
                        child: EmptyState(
                          key: const ValueKey('just-watch-empty'),
                          icon: Icons.favorite_border,
                          title: l.justWatchEmptyTitle,
                          message: l.justWatchEmptyMessage,
                        ),
                      ),
                    if (cont.isNotEmpty)
                      _BigRow(
                        key: const ValueKey('just-watch-continue'),
                        title: l.justWatchContinue,
                        children: [
                          for (final e in cont)
                            if (e.item case final item?)
                              _BigCard(
                                key: ValueKey('just-watch-continue-${e.id}'),
                                autofocus: takeFirst(),
                                seed: e.id,
                                imageUrl: e.imageUrl,
                                title: e.title,
                                subtitle: e.kind == ContinueKind.episode
                                    ? '${episodeCode(l, item)} · '
                                          '${formatRemaining(l, e.progress)}'
                                    : formatRemaining(l, e.progress),
                                progress: e.progress.fraction,
                                onTap: () => playVideo(
                                  context,
                                  item,
                                  startPositionMs: e.progress.positionMs,
                                ),
                              ),
                        ],
                      ),
                    if (next.isNotEmpty)
                      _BigRow(
                        key: const ValueKey('just-watch-up-next'),
                        title: l.justWatchUpNext,
                        children: [
                          for (final e in next)
                            _BigCard(
                              key: ValueKey('just-watch-next-${e.episode.id}'),
                              autofocus: takeFirst(),
                              seed: e.episode.id,
                              imageUrl:
                                  e.episode.backdropUrl ?? e.show.backdropUrl,
                              title: e.show.title,
                              subtitle:
                                  '${episodeCode(l, e.episode)} · '
                                  '${e.episode.title}',
                              onTap: () => playVideo(context, e.episode),
                            ),
                        ],
                      ),
                    if (favs.isNotEmpty)
                      _BigRow(
                        key: const ValueKey('just-watch-favorites'),
                        title: l.justWatchFavorites,
                        children: [
                          for (final item in favs)
                            _BigCard(
                              key: ValueKey('just-watch-fav-${item.id}'),
                              autofocus: takeFirst(),
                              seed: item.id,
                              imageUrl: item.backdropUrl ?? item.posterUrl,
                              title: item.title,
                              subtitle: item.year == null
                                  ? null
                                  : '${item.year}',
                              onTap: () => playJustWatch(context, item),
                            ),
                        ],
                      ),
                  ],
                );
              },
            ),
          ),
        ),
      ),
    );
  }
}

/// Plays a favourite at once: a movie or episode from its position, a show
/// from the episode to watch next (the started one, else the first not
/// watched, else the first).
Future<void> playJustWatch(BuildContext context, LibraryItem item) async {
  if (item.kind != ItemKind.show) {
    final p = await Services.read(context).library.progress(item.id);
    if (!context.mounted) return;
    return playVideo(context, item, startPositionMs: resumePositionMs(p));
  }
  final lib = Services.read(context).library;
  final episodes = await lib.episodes(item.id).first;
  if (episodes.isEmpty || !context.mounted) return;
  LibraryItem? target;
  var start = 0;
  for (final e in episodes) {
    final p = await lib.progress(e.id);
    if (p == null || !p.watched) {
      target = e;
      start = resumePositionMs(p);
      break;
    }
  }
  if (!context.mounted) return;
  return playVideo(context, target ?? episodes.first, startPositionMs: start);
}

/// A titled horizontal row of [_BigCard]s.
class _BigRow extends StatelessWidget {
  const _BigRow({super.key, required this.title, required this.children});

  final String title;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final width = _cardWidth(context);
    return Padding(
      padding: EdgeInsets.only(bottom: d.isTv ? 24 : 28),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: EdgeInsets.only(
              left: d.pageInsets.left,
              right: d.pageInsets.right,
              bottom: 12,
            ),
            child: Text(
              title,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: Theme.of(context).textTheme.headlineSmall,
            ),
          ),
          SizedBox(
            height: width * 9 / 16 + (d.isTv ? 20 : 8),
            child: FocusTraversalGroup(
              child: ListView.separated(
                scrollDirection: Axis.horizontal,
                clipBehavior: Clip.none,
                padding: EdgeInsets.only(
                  left: d.pageInsets.left,
                  right: d.pageInsets.right,
                  top: d.isTv ? 10 : 4,
                ),
                itemCount: children.length,
                separatorBuilder: (_, _) => SizedBox(width: d.gap * 1.5),
                itemBuilder: (_, i) =>
                    SizedBox(width: width, child: children[i]),
              ),
            ),
          ),
        ],
      ),
    );
  }

  /// Large cards: on a phone nearly the whole width (the next one peeks
  /// in), elsewhere about twice the Home card.
  static double _cardWidth(BuildContext context) {
    final d = KDensity.of(context);
    final screen = MediaQuery.sizeOf(context).width;
    if (d.isPhone) {
      return (screen - d.pageInsets.left - d.pageInsets.right - 40).clamp(
        200.0,
        560.0,
      );
    }
    return (d.landscapeWidth * 1.8).clamp(320.0, screen * 0.6);
  }
}

/// A large 16:9 card with the title on the picture and a progress bar.
class _BigCard extends StatelessWidget {
  const _BigCard({
    super.key,
    required this.seed,
    required this.title,
    required this.onTap,
    this.imageUrl,
    this.subtitle,
    this.progress,
    this.autofocus = false,
  });

  final String seed;
  final String title;
  final String? subtitle;
  final String? imageUrl;
  final double? progress;
  final VoidCallback onTap;
  final bool autofocus;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final d = KDensity.of(context);
    const shadow = [Shadow(blurRadius: 8, color: Colors.black87)];
    return KFocusable(
      autofocus: autofocus,
      onActivate: onTap,
      radius: 16,
      semanticLabel: subtitle == null ? title : '$title, $subtitle',
      builder: (context, focused) => ClipRRect(
        borderRadius: BorderRadius.circular(16),
        child: AspectRatio(
          aspectRatio: 16 / 9,
          child: Stack(
            fit: StackFit.expand,
            children: [
              Artwork(url: imageUrl, seed: seed, style: ArtStyle.backdrop),
              const DecoratedBox(
                decoration: BoxDecoration(
                  gradient: LinearGradient(
                    begin: Alignment.topCenter,
                    end: Alignment.bottomCenter,
                    stops: [.35, 1],
                    colors: [Colors.transparent, Color(0xD9000000)],
                  ),
                ),
              ),
              Positioned(
                left: 16,
                right: 16,
                bottom: progress == null ? 14 : 22,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      title,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style:
                          (d.isPhone
                                  ? theme.textTheme.titleLarge
                                  : theme.textTheme.headlineSmall)
                              ?.copyWith(
                                color: Colors.white,
                                fontWeight: FontWeight.w700,
                                shadows: shadow,
                              ),
                    ),
                    if (subtitle case final s?)
                      Text(
                        s,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: theme.textTheme.titleMedium?.copyWith(
                          color: Colors.white,
                          shadows: shadow,
                        ),
                      ),
                  ],
                ),
              ),
              if (progress case final p?)
                Positioned(
                  left: 16,
                  right: 16,
                  bottom: 10,
                  child: KProgressBar(value: p, height: 6),
                ),
            ],
          ),
        ),
      ),
    );
  }
}
