// SPDX-License-Identifier: MPL-2.0

import 'package:flutter/rendering.dart';
import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../library/show_airing.dart' show AiringPosterLabel;
import '../integration/library.dart';
import '../review/review_marks.dart';
import '../theme/kinolune_theme.dart';
import 'artwork.dart';
import 'focusable.dart';
import 'watched_art.dart';

/// 2:3 poster with a caption underneath.
class PosterCard extends StatelessWidget {
  const PosterCard({
    super.key,
    required this.item,
    required this.width,
    this.onTap,
    this.onFocus,
    this.caption,
    this.autofocus = false,
    this.overlay,
    this.onContextMenu,
    this.reviewMark = true,
  });

  final LibraryItem item;
  final double width;
  final VoidCallback? onTap;
  final ValueChanged<bool>? onFocus;
  final String? caption;
  final bool autofocus;

  /// Drawn over the artwork (badges, UI-008).
  final Widget? overlay;

  /// Long press, right click or the menu key ([KFocusable.onContextMenu]).
  final ValueChanged<Offset>? onContextMenu;

  /// «?» when the review inbox has the item (META-034, UI-037); off where
  /// the [overlay] draws it itself.
  final bool reviewMark;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    final review = reviewMark ? ReviewTileMark(itemId: item.id) : null;
    return SizedBox(
      width: width,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          KFocusable(
            autofocus: autofocus,
            onActivate: onTap,
            onFocusChange: onFocus,
            onContextMenu: onContextMenu,
            focusScale: d.focusScalePoster,
            semanticLabel: item.title,
            builder: (context, _) {
              final art = Artwork(
                url: item.posterUrl,
                seed: item.id,
                style: ArtStyle.poster,
                title: item.title,
                subtitle: item.year?.toString(),
              );
              // UI-057: «Новая серия в чт» / «Сериал завершён».
              final airing = item.kind == ItemKind.show
                  ? AiringPosterLabel(item: item)
                  : null;
              return AspectRatio(
                aspectRatio: 2 / 3,
                child: overlay == null && review == null && airing == null
                    ? art
                    : Stack(
                        fit: StackFit.expand,
                        children: [art, ?overlay, ?review, ?airing],
                      ),
              );
            },
          ),
          SizedBox(height: d.isTv ? 10 : 8),
          _Caption(
            onTap: onTap,
            children: [
              Text(
                item.title,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: theme.textTheme.titleSmall,
              ),
              Text(
                caption ?? (item.year?.toString() ?? ''),
                maxLines: 1,
                style: theme.textTheme.labelSmall,
              ),
            ],
          ),
        ],
      ),
    );
  }
}

/// 16:9 card (Continue watching, episodes) with optional progress.
class LandscapeCard extends StatelessWidget {
  const LandscapeCard({
    super.key,
    required this.seed,
    required this.title,
    required this.width,
    this.imageUrl,
    this.subtitle,
    this.progress,
    this.onTap,
    this.onFocus,
    this.badge,
    this.autofocus = false,
    this.remaining,
    this.art,
    this.onContextMenu,
  });

  final String seed;
  final String title;
  final String? subtitle;
  final String? imageUrl;
  final double width;

  /// Long press, right click, the menu key or a held OK ([KFocusable]).
  final ValueChanged<Offset>? onContextMenu;

  /// Drawn instead of [imageUrl] (UI-032: a frame of the file itself).
  final Widget? art;

  /// Watched share of a started item: the still is sharp on it and
  /// blurred on the rest (UI-037), with a bar.
  final double? progress;

  /// «ещё N мин» on the still of a started item.
  final String? remaining;
  final VoidCallback? onTap;
  final ValueChanged<bool>? onFocus;
  final String? badge;
  final bool autofocus;

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
            autofocus: autofocus,
            onActivate: onTap,
            onFocusChange: onFocus,
            onContextMenu: onContextMenu,
            semanticLabel: title,
            builder: (context, focused) => AspectRatio(
              aspectRatio: 16 / 9,
              child: Stack(
                fit: StackFit.expand,
                children: [
                  if (progress case final p?)
                    WatchedStill(
                      url: imageUrl,
                      seed: seed,
                      state: WatchArt.started,
                      fraction: p,
                      remaining: remaining,
                      dot: false,
                    )
                  else if (art case final a?)
                    a
                  else
                    Artwork(
                      url: imageUrl,
                      seed: seed,
                      style: ArtStyle.backdrop,
                    ),
                  if (badge != null)
                    Positioned(
                      left: 10,
                      top: 8,
                      child: Text(
                        badge!,
                        style: theme.textTheme.labelMedium?.copyWith(
                          color: Colors.white,
                          fontWeight: FontWeight.w700,
                          shadows: const [
                            Shadow(blurRadius: 6, color: Colors.black54),
                          ],
                        ),
                      ),
                    ),
                ],
              ),
            ),
          ),
          SizedBox(height: d.isTv ? 10 : 8),
          _Caption(
            onTap: onTap,
            children: [
              Text(
                title,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: theme.textTheme.titleSmall,
              ),
              if (subtitle != null)
                Text(
                  subtitle!,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.labelSmall,
                ),
            ],
          ),
        ],
      ),
    );
  }
}

/// 1:1 album tile.
class AlbumCard extends StatelessWidget {
  const AlbumCard({
    super.key,
    required this.seed,
    required this.title,
    required this.subtitle,
    required this.width,
    this.onTap,
    this.artUrl,
  });

  final String seed;
  final String? artUrl;
  final String title;
  final String subtitle;
  final double width;
  final VoidCallback? onTap;

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
            focusScale: d.focusScalePoster,
            semanticLabel: title,
            builder: (context, _) => AspectRatio(
              aspectRatio: 1,
              child: Artwork(url: artUrl, seed: seed, style: ArtStyle.album),
            ),
          ),
          SizedBox(height: d.isTv ? 10 : 8),
          _Caption(
            onTap: onTap,
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
        ],
      ),
    );
  }
}

class KProgressBar extends StatelessWidget {
  const KProgressBar({super.key, required this.value, this.height = 4});

  final double value;
  final double height;

  @override
  Widget build(BuildContext context) {
    return ClipRRect(
      borderRadius: BorderRadius.circular(height),
      child: SizedBox(
        height: height,
        child: Stack(
          fit: StackFit.expand,
          children: [
            ColoredBox(color: Colors.white.withValues(alpha: .28)),
            FractionallySizedBox(
              alignment: Alignment.centerLeft,
              widthFactor: value.clamp(0, 1),
              child: const ColoredBox(color: Colors.white),
            ),
          ],
        ),
      ),
    );
  }
}

/// Horizontal row with a header (tvOS shelf).
class MediaRow extends StatelessWidget {
  const MediaRow({
    super.key,
    required this.title,
    required this.itemCount,
    required this.itemBuilder,
    required this.height,
    this.onSeeAll,
    this.seeAllTileWidth,
  });

  final String title;
  final int itemCount;
  final IndexedWidgetBuilder itemBuilder;
  final double height;
  final VoidCallback? onSeeAll;

  /// On TV, where the header's «Все» is not shown, a focusable «Все» tile of
  /// this width ends the row (QA-015: «Мои видео» has no section in the
  /// menu, so the row is the only way to the whole list).
  final double? seeAllTileWidth;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    final tile = d.isTv ? seeAllTileWidth : null;
    final seeAll = onSeeAll;
    final count = itemCount + (tile != null && seeAll != null ? 1 : 0);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: EdgeInsets.only(
            left: d.pageInsets.left,
            right: d.pageInsets.right,
            bottom: d.isTv ? 6 : 10,
          ),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  title,
                  style: theme.textTheme.titleLarge,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              if (onSeeAll != null && !d.isTv)
                TextButton(
                  onPressed: onSeeAll,
                  child: Text(context.l10n.commonSeeAll),
                ),
            ],
          ),
        ),
        SizedBox(
          height: height,
          child: FocusTraversalGroup(
            child: ListView.separated(
              scrollDirection: Axis.horizontal,
              clipBehavior: Clip.none,
              padding: EdgeInsets.only(
                left: d.pageInsets.left,
                right: d.pageInsets.right,
                top: d.isTv ? 10 : 4,
              ),
              itemCount: count,
              separatorBuilder: (_, _) => SizedBox(width: d.gap),
              itemBuilder: (context, i) => i < itemCount
                  ? itemBuilder(context, i)
                  : _SeeAllTile(width: tile!, onTap: seeAll!),
            ),
          ),
        ),
      ],
    );
  }
}

/// The «Все» tile at the end of a TV row ([MediaRow.seeAllTileWidth]).
class _SeeAllTile extends StatelessWidget {
  const _SeeAllTile({required this.width, required this.onTap});

  final double width;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final label = context.l10n.commonSeeAll;
    return Align(
      alignment: Alignment.topLeft,
      child: SizedBox(
        width: width,
        child: KFocusable(
          key: const ValueKey('row-see-all-tile'),
          onActivate: onTap,
          semanticLabel: label,
          light: false,
          builder: (context, focused) => AspectRatio(
            aspectRatio: 16 / 9,
            child: ColoredBox(
              color: focused
                  ? KColors.focusFill
                  : Colors.white.withValues(alpha: .08),
              child: Center(
                child: ExcludeSemantics(
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(
                        label,
                        style: Theme.of(context).textTheme.titleMedium
                            ?.copyWith(
                              color: focused
                                  ? KColors.focusLabel
                                  : Colors.white,
                            ),
                      ),
                      const SizedBox(width: 8),
                      Icon(
                        Icons.arrow_forward_rounded,
                        color: focused ? KColors.focusLabel : Colors.white,
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Screen title used at the top of root screens.
class PageTitle extends StatelessWidget {
  const PageTitle(this.text, {super.key, this.trailing});

  final String text;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    return Padding(
      padding: EdgeInsets.fromLTRB(
        d.pageInsets.left,
        d.safeTop(context),
        d.pageInsets.right,
        d.isTv ? 12 : 16,
      ),
      child: Row(
        children: [
          Expanded(
            child: Text(text, style: Theme.of(context).textTheme.headlineLarge),
          ),
          ?trailing,
        ],
      ),
    );
  }
}

class EmptyState extends StatelessWidget {
  const EmptyState({
    super.key,
    required this.icon,
    required this.title,
    this.message,
    this.action,
  });

  final IconData icon;
  final String title;
  final String? message;
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(32),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: 56, color: KColors.textTertiary),
            const SizedBox(height: 16),
            Text(
              title,
              style: theme.textTheme.titleLarge,
              textAlign: TextAlign.center,
            ),
            if (message != null) ...[
              const SizedBox(height: 6),
              Text(
                message!,
                style: theme.textTheme.bodySmall,
                textAlign: TextAlign.center,
              ),
            ],
            if (action != null) ...[const SizedBox(height: 20), action!],
          ],
        ),
      ),
    );
  }
}

/// Height of the two caption lines under a card.
double cardCaptionHeight(KDensity d) =>
    (d.isTv ? 10 : 8) + d.type.caption * 1.45 + d.type.caption2 * 1.5 + 6;

/// Grid with columns computed from a minimum item width; each tile is
/// `width * artAspect + extraHeight` tall, so captions never overflow.
class CardGridDelegate extends SliverGridDelegate {
  const CardGridDelegate({
    required this.minItemWidth,
    required this.crossSpacing,
    required this.mainSpacing,
    required this.artAspect,
    required this.extraHeight,
  });

  final double minItemWidth, crossSpacing, mainSpacing, artAspect, extraHeight;

  @override
  SliverGridLayout getLayout(SliverConstraints constraints) {
    final cross = constraints.crossAxisExtent;
    final cols = ((cross + crossSpacing) / (minItemWidth + crossSpacing))
        .floor()
        .clamp(1, 99);
    final w = (cross - (cols - 1) * crossSpacing) / cols;
    final h = w * artAspect + extraHeight;
    return SliverGridRegularTileLayout(
      crossAxisCount: cols,
      mainAxisStride: h + mainSpacing,
      crossAxisStride: w + crossSpacing,
      childMainAxisExtent: h,
      childCrossAxisExtent: w,
      reverseCrossAxis: false,
    );
  }

  @override
  bool shouldRelayout(CardGridDelegate oldDelegate) =>
      oldDelegate.minItemWidth != minItemWidth ||
      oldDelegate.extraHeight != extraHeight ||
      oldDelegate.artAspect != artAspect;
}

/// Caption under a card: clickable with mouse/touch, not a separate focus stop.
class _Caption extends StatelessWidget {
  const _Caption({required this.children, this.onTap});

  final List<Widget> children;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) => MouseRegion(
    cursor: onTap == null ? MouseCursor.defer : SystemMouseCursors.click,
    child: GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      // The card above is the button for screen readers; a second "tap"
      // node with the caption would be a duplicate (UI-021).
      excludeFromSemantics: true,
      child: SizedBox(
        width: double.infinity,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: children,
        ),
      ),
    ),
  );
}
