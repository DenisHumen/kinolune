// SPDX-License-Identifier: MPL-2.0
//
// Building blocks of the "Streams" section (TWI-013): avatars with the LIVE
// ring, live and VOD cards with the chapter bar, the attribution line and
// the "Open on Twitch" sheet. No Twitch logo anywhere (brand guidelines,
// research 06 §1.5): Twitch is named in text only.

import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';

import '../../l10n/l10n.dart';
import '../../platform/form_factor.dart';
import '../../services/external_links.dart';
import '../../theme/kinolune_theme.dart';
import '../../twitch/twitch_models.dart';
import '../../widgets/artwork.dart';
import '../../widgets/cards.dart';
import '../../widgets/focusable.dart';
import '../../widgets/qr_code.dart';
import '../../widgets/settings_tiles.dart' show copyLink;

/// `18.4K` in the UI language.
String compactCount(BuildContext context, int n) => NumberFormat.compact(
  locale: Localizations.localeOf(context).toLanguageTag(),
).format(n);

/// `3 hours ago`, `2 days ago`.
String timeAgo(AppLocalizations l, DateTime now, DateTime t) {
  final d = now.difference(t);
  if (d.inHours < 1) return l.twitchJustNow;
  if (d.inHours < 48) return l.twitchHoursAgo(d.inHours);
  return l.twitchDaysAgo(d.inDays);
}

/// `2:05:00` / `45:10`.
String formatLength(Duration d) {
  String two(int v) => v.toString().padLeft(2, '0');
  final h = d.inHours, m = d.inMinutes % 60, s = d.inSeconds % 60;
  return h > 0 ? '$h:${two(m)}:${two(s)}' : '$m:${two(s)}';
}

/// A stable colour of a category for the chapter bar.
Color categoryColor(String gameId) {
  const palette = [
    Color(0xFF7C9CFF),
    Color(0xFF4FC3A1),
    Color(0xFFFFB25B),
    Color(0xFFE86AA6),
    Color(0xFFA48BFF),
    Color(0xFF5BC0EB),
    Color(0xFFC6D65A),
    Color(0xFFFF7F6E),
  ];
  var h = 0;
  for (final c in gameId.codeUnits) {
    h = (h * 31 + c) & 0x7fffffff;
  }
  return palette[h % palette.length];
}

/// Round avatar; a red ring around it while the channel is live.
class TwitchAvatar extends StatelessWidget {
  const TwitchAvatar({
    super.key,
    required this.channel,
    required this.size,
    this.live = false,
  });

  final TwitchChannel channel;
  final double size;
  final bool live;

  @override
  Widget build(BuildContext context) {
    final ring = size * .06 + 1;
    final image = ClipOval(
      child: SizedBox.square(
        dimension: size - (ring + 2) * 2,
        child: channel.avatarUrl == null
            ? ColoredBox(
                color: categoryColor(channel.login).withValues(alpha: .35),
                child: Center(
                  child: Text(
                    channel.displayName.isEmpty
                        ? '?'
                        : channel.displayName.substring(0, 1).toUpperCase(),
                    style: TextStyle(
                      fontSize: size * .38,
                      fontWeight: FontWeight.w700,
                      color: Colors.white,
                    ),
                  ),
                ),
              )
            : Artwork(
                url: channel.avatarUrl,
                seed: 'avatar-${channel.login}',
                style: ArtStyle.album,
              ),
      ),
    );
    return SizedBox.square(
      dimension: size,
      child: DecoratedBox(
        decoration: BoxDecoration(
          shape: BoxShape.circle,
          border: Border.all(
            color: live ? KColors.live : Colors.transparent,
            width: ring,
          ),
        ),
        child: Center(child: image),
      ),
    );
  }
}

/// Red "LIVE" label.
class LiveBadge extends StatelessWidget {
  const LiveBadge({super.key, this.small = false});

  final bool small;

  @override
  Widget build(BuildContext context) => Container(
    padding: EdgeInsets.symmetric(horizontal: small ? 4 : 6, vertical: 1.5),
    decoration: BoxDecoration(
      color: KColors.live,
      borderRadius: BorderRadius.circular(4),
    ),
    child: Text(
      context.l10n.twitchLiveBadge,
      style: Theme.of(context).textTheme.labelSmall?.copyWith(
        color: Colors.white,
        fontWeight: FontWeight.w800,
        fontSize: small ? 9 : null,
        letterSpacing: .4,
      ),
    ),
  );
}

/// A dark pill over artwork (duration, viewers).
class _Pill extends StatelessWidget {
  const _Pill({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
    decoration: BoxDecoration(
      color: Colors.black.withValues(alpha: .7),
      borderRadius: BorderRadius.circular(4),
    ),
    child: DefaultTextStyle.merge(
      style: Theme.of(context).textTheme.labelSmall
          ?.copyWith(color: Colors.white, fontWeight: FontWeight.w600),
      child: child,
    ),
  );
}

/// Height of a 16:9 card row with two caption lines.
double twitchRowHeight(KDensity d, {double extra = 0}) =>
    d.landscapeWidth * 9 / 16 +
    cardCaptionHeight(d) +
    (d.isTv ? 12 : 6) +
    extra;

/// A 16:9 card: artwork with overlays and two caption lines.
class _MediaCard extends StatelessWidget {
  const _MediaCard({
    required this.seed,
    required this.imageUrl,
    required this.title,
    required this.subtitle,
    required this.width,
    required this.onTap,
    this.overlays = const [],
    this.below,
    this.leading,
  });

  final String seed;
  final String? imageUrl;
  final String title;
  final String subtitle;
  final double width;
  final VoidCallback? onTap;
  final List<Widget> overlays;
  final Widget? below;
  final Widget? leading;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    return SizedBox(
      width: width,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          KFocusable(
            onActivate: onTap,
            semanticLabel: title,
            builder: (context, focused) => AspectRatio(
              aspectRatio: 16 / 9,
              child: Stack(
                fit: StackFit.expand,
                children: [
                  Artwork(url: imageUrl, seed: seed, style: ArtStyle.backdrop),
                  ...overlays,
                ],
              ),
            ),
          ),
          ?below,
          SizedBox(height: d.isTv ? 8 : 6),
          GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: onTap,
            // The card above is the button for screen readers (UI-021).
            excludeFromSemantics: true,
            child: Row(
              children: [
                if (leading != null) ...[leading!, const SizedBox(width: 8)],
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
                      Text(
                        subtitle,
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
        ],
      ),
    );
  }
}

/// A live stream: preview frame, LIVE, viewers, avatar, name, category.
class TwitchLiveCard extends StatelessWidget {
  const TwitchLiveCard({
    super.key,
    required this.stream,
    required this.channel,
    required this.width,
    this.onTap,
  });

  final TwitchStream stream;
  final TwitchChannel channel;
  final double width;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final game = stream.gameName;
    return _MediaCard(
      seed: 'live-${stream.login}',
      imageUrl: twitchImage(stream.thumbnailUrl, 640, 360),
      title: stream.displayName,
      subtitle: [stream.title, ?game].join(' · '),
      width: width,
      onTap: onTap,
      leading: TwitchAvatar(
        channel: channel,
        size: d.isTv ? 30 : 32,
        live: true,
      ),
      overlays: [
        const Positioned(left: 8, top: 8, child: LiveBadge()),
        Positioned(
          left: 8,
          bottom: 8,
          child: _Pill(
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                const Icon(Icons.person, size: 12, color: Colors.white),
                const SizedBox(width: 3),
                Text(compactCount(context, stream.viewers)),
              ],
            ),
          ),
        ),
      ],
    );
  }
}

/// Coloured band of the categories along a VOD (TWI-008 chapters); an
/// approximate chapter is drawn faded.
class ChapterBar extends StatelessWidget {
  const ChapterBar({
    super.key,
    required this.chapters,
    required this.duration,
    this.height = 4,
  });

  final List<TwitchChapter> chapters;
  final Duration duration;
  final double height;

  @override
  Widget build(BuildContext context) {
    final total = duration.inMilliseconds;
    if (chapters.isEmpty || total <= 0) return SizedBox(height: height);
    final names = {for (final c in chapters) ?c.gameName}.join(' · ');
    return Semantics(
      label: names,
      child: ClipRRect(
        borderRadius: BorderRadius.circular(height),
        child: SizedBox(
          height: height,
          child: Row(
            children: [
              for (final c in chapters)
                Expanded(
                  flex:
                      (((c.end ?? duration) - c.start).inMilliseconds *
                              1000 ~/
                              total)
                          .clamp(1, 1000),
                  child: ColoredBox(
                    color: categoryColor(c.gameId)
                        .withValues(alpha: c.approximate ? .45 : 1),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The chapters of a VOD on the player's timeline (TWI-027): one segment
/// per chapter, as long as the chapter, with the game's cover and name when
/// they fit; the playing one is bright. Tapping a segment seeks to its
/// start. Not focusable: on a TV ↑/↓ jump between chapters instead.
class TwitchChapterStrip extends StatelessWidget {
  const TwitchChapterStrip({
    super.key,
    required this.chapters,
    required this.duration,
    required this.position,
    required this.onSeek,
    this.height = 30,
  });

  final List<TwitchChapter> chapters;
  final Duration duration;
  final Duration position;
  final ValueChanged<Duration> onSeek;
  final double height;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final total = duration.inMilliseconds;
    if (chapters.length < 2 || total <= 0) return const SizedBox.shrink();
    final sorted = [...chapters]..sort((a, b) => a.start.compareTo(b.start));
    var current = 0;
    for (final (i, c) in sorted.indexed) {
      if (c.start <= position) current = i;
    }
    return SizedBox(
      key: const ValueKey('twitch-chapter-strip'),
      height: height,
      child: Row(
        children: [
          for (final (i, c) in sorted.indexed)
            Expanded(
              flex:
                  (((i + 1 < sorted.length ? sorted[i + 1].start : duration) -
                                  c.start)
                              .inMilliseconds *
                          1000 ~/
                          total)
                      .clamp(1, 1000),
              child: Padding(
                padding: EdgeInsets.only(left: i == 0 ? 0 : 2),
                child: _ChapterSegment(
                  key: ValueKey('twitch-chapter-seg-$i'),
                  chapter: c,
                  current: i == current,
                  height: height,
                  label: l.twitchChapterAt(
                    c.gameName ?? l.twitchPlayerVod,
                    formatLength(c.start),
                  ),
                  onTap: () => onSeek(c.start),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _ChapterSegment extends StatelessWidget {
  const _ChapterSegment({
    super.key,
    required this.chapter,
    required this.current,
    required this.height,
    required this.label,
    required this.onTap,
  });

  final TwitchChapter chapter;
  final bool current;
  final double height;
  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final color = categoryColor(chapter.gameId);
    final name = chapter.gameName ?? '';
    final art = twitchImage(
      chapter.boxArtUrl ?? twitchBoxArtTemplate(chapter.gameId),
      52,
      72,
    );
    final iconW = height * .66;
    return Semantics(
      button: true,
      label: label,
      excludeSemantics: true,
      child: Tooltip(
        message: label,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: onTap,
          child: DecoratedBox(
            decoration: BoxDecoration(
              color: color.withValues(alpha: current ? .9 : .38),
              borderRadius: BorderRadius.circular(6),
              border: current
                  ? Border.all(color: Colors.white, width: 1.5)
                  : null,
            ),
            child: LayoutBuilder(
              builder: (context, c) {
                final showIcon = c.maxWidth >= iconW + 8;
                final showName = c.maxWidth >= iconW + 56 && name.isNotEmpty;
                if (!showIcon) return const SizedBox.expand();
                return Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 4),
                  child: Row(
                    children: [
                      ClipRRect(
                        borderRadius: BorderRadius.circular(3),
                        child: SizedBox(
                          width: iconW,
                          height: height - 6,
                          child: art == null
                              ? ColoredBox(
                                  color: Colors.black.withValues(alpha: .35),
                                  child: Center(
                                    child: Text(
                                      name.isEmpty
                                          ? '?'
                                          : name.characters.first.toUpperCase(),
                                      style: const TextStyle(
                                        color: Colors.white,
                                        fontWeight: FontWeight.w700,
                                        fontSize: 12,
                                      ),
                                    ),
                                  ),
                                )
                              : Artwork(
                                  url: art,
                                  seed: 'game-${chapter.gameId}',
                                  style: ArtStyle.poster,
                                ),
                        ),
                      ),
                      if (showName) ...[
                        const SizedBox(width: 6),
                        Expanded(
                          child: Text(
                            name,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              color: current ? Colors.black : Colors.white,
                              fontSize: 13,
                              fontWeight: FontWeight.w600,
                            ),
                          ),
                        ),
                      ],
                    ],
                  ),
                );
              },
            ),
          ),
        ),
      ),
    );
  }
}

/// A VOD: thumbnail, length, "deleted in N days", lock for sub-only,
/// progress and the chapter bar.
class TwitchVodCard extends StatelessWidget {
  const TwitchVodCard({
    super.key,
    required this.video,
    required this.width,
    required this.now,
    this.onTap,
    this.showChannel = true,
  });

  final TwitchVideo video;
  final double width;
  final DateTime now;
  final VoidCallback? onTap;
  final bool showChannel;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final days = video.daysLeft(now);
    final progress = video.progress?.fraction;
    final chapter = video.chapters.isEmpty
        ? null
        : video.chapters.first.approximate
        ? '≈ ${video.chapters.first.gameName ?? ''}'
        : video.chapters.map((c) => c.gameName).nonNulls.toSet().join(', ');
    final title = video.title.isEmpty ? video.displayName : video.title;
    return _MediaCard(
      seed: 'vod-${video.id}',
      imageUrl: twitchImage(video.thumbnailUrl, 640, 360),
      title: title,
      subtitle: [
        if (showChannel && video.displayName.isNotEmpty) video.displayName,
        timeAgo(l, now, video.publishedAt),
        ?chapter,
      ].join(' · '),
      width: width,
      onTap: onTap,
      below: video.chapters.isEmpty
          ? null
          : Padding(
              padding: const EdgeInsets.only(top: 4),
              child: ChapterBar(
                chapters: video.chapters,
                duration: video.duration,
              ),
            ),
      overlays: [
        if (video.subOnly) ...[
          ColoredBox(color: Colors.black.withValues(alpha: .45)),
          Center(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                const Icon(Icons.lock_outline, color: Colors.white, size: 26),
                const SizedBox(height: 4),
                _Pill(child: Text(l.twitchSubOnly)),
              ],
            ),
          ),
        ],
        if (days != null && days <= 7)
          Positioned(
            left: 8,
            top: 8,
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
              decoration: BoxDecoration(
                color: days <= 2
                    ? KColors.live.withValues(alpha: .9)
                    : Colors.black.withValues(alpha: .7),
                borderRadius: BorderRadius.circular(4),
              ),
              child: Text(
                l.twitchExpiresIn(days),
                style: Theme.of(context).textTheme.labelSmall?.copyWith(
                  color: Colors.white,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ),
          ),
        Positioned(
          right: 8,
          bottom: progress == null ? 8 : 16,
          child: _Pill(child: Text(formatLength(video.duration))),
        ),
        if (progress != null)
          Positioned(
            left: 8,
            right: 8,
            bottom: 7,
            child: KProgressBar(value: progress),
          ),
      ],
    );
  }
}

/// A clip card.
class TwitchClipCard extends StatelessWidget {
  const TwitchClipCard({
    super.key,
    required this.clip,
    required this.width,
    this.onTap,
  });

  final TwitchClip clip;
  final double width;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final creator = clip.creatorName;
    return _MediaCard(
      seed: 'clip-${clip.slug}',
      imageUrl: clip.thumbnailUrl,
      title: clip.title,
      subtitle: [
        compactCount(context, clip.viewCount),
        if (creator != null) context.l10n.twitchClipBy(creator),
      ].join(' · '),
      width: width,
      onTap: onTap,
      overlays: [
        Positioned(
          right: 8,
          bottom: 8,
          child: _Pill(child: Text(formatLength(clip.duration))),
        ),
      ],
    );
  }
}

/// The attribution line: data and video come from Twitch; we are not
/// Twitch (research 06 §1.5). Text only, no logo.
class TwitchAttribution extends StatelessWidget {
  const TwitchAttribution({super.key});

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    return Padding(
      key: const ValueKey('twitch-attribution'),
      padding: EdgeInsets.fromLTRB(d.pageInsets.left, 8, d.pageInsets.right, 8),
      child: Text(
        context.l10n.twitchAttribution,
        style: Theme.of(context).textTheme.labelSmall
            ?.copyWith(color: KColors.textTertiary),
      ),
    );
  }
}

/// "Open on Twitch": the link as a QR code (scan with the phone that has
/// the Twitch app; the https link opens it) and as text to copy, on every
/// device, TVs included. Phones and desktops also get "Open", which hands
/// the link to the Twitch app or the browser (`url_launcher`, TWI-005).
Future<void> showOpenOnTwitch(
  BuildContext context,
  String url, {
  String? message,
}) {
  final l = context.l10n;
  return showDialog<void>(
    context: context,
    builder: (context) {
      final theme = Theme.of(context);
      // A TV has nothing to open the link with: QR code and copy only.
      final tv = FormFactorScope.of(context) == FormFactor.tv;
      return AlertDialog(
        key: const ValueKey('twitch-open-dialog'),
        title: Text(l.twitchOpenOnTwitch),
        content: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (message != null) ...[
                Text(message, style: theme.textTheme.bodyMedium),
                const SizedBox(height: 12),
              ],
              QrCodeView(data: url, size: 180),
              const SizedBox(height: 12),
              SelectableText(url, style: theme.textTheme.bodySmall),
              const SizedBox(height: 4),
              Text(l.twitchOpenOnTwitchHint, style: theme.textTheme.bodySmall),
            ],
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => copyLink(context, url),
            child: Text(l.twitchCopyLink),
          ),
          TextButton(
            autofocus: tv,
            onPressed: () => Navigator.of(context).pop(),
            child: Text(l.twitchClose),
          ),
          if (!tv)
            FilledButton(
              key: const ValueKey('twitch-open-link'),
              autofocus: true,
              onPressed: () async {
                final navigator = Navigator.of(context);
                if (await openExternalLink(Uri.parse(url))) navigator.pop();
              },
              child: Text(l.twitchOpenLink),
            ),
        ],
      );
    },
  );
}

/// A row title with page insets, for rows that are not [MediaRow]s.
class TwitchRowTitle extends StatelessWidget {
  const TwitchRowTitle(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    return Padding(
      padding: EdgeInsets.only(
        left: d.pageInsets.left,
        right: d.pageInsets.right,
        bottom: d.isTv ? 6 : 10,
      ),
      child: Text(
        text,
        style: Theme.of(context).textTheme.titleLarge,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
      ),
    );
  }
}

/// An info card with an optional button (sign-in hints, placeholders).
class TwitchInfoCard extends StatelessWidget {
  const TwitchInfoCard({
    super.key,
    required this.icon,
    required this.text,
    this.action,
  });

  final IconData icon;
  final String text;
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    return Padding(
      padding: EdgeInsets.only(
        left: d.pageInsets.left,
        right: d.pageInsets.right,
        bottom: d.rowGap,
      ),
      child: Container(
        padding: const EdgeInsets.all(14),
        decoration: BoxDecoration(
          color: KColors.bgElevated,
          borderRadius: BorderRadius.circular(d.panelRadius),
          border: Border.all(color: KColors.stroke),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(icon, color: KColors.textSecondary),
                const SizedBox(width: 10),
                Expanded(child: Text(text, style: theme.textTheme.bodySmall)),
              ],
            ),
            if (action != null) ...[const SizedBox(height: 10), action!],
          ],
        ),
      ),
    );
  }
}
