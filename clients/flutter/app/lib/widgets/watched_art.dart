// SPDX-License-Identifier: MPL-2.0
//
// Watched state on stills and posters (UI-037, ADR-0100). A 16:9 still of a
// started episode or movie is sharp on the watched share (left) and blurred
// and darkened on the rest, with a thin light line at the split; a watched
// one is muted with a «✓»; one not watched yet is clean with an accent dot;
// the next episode gets an outline and «Далее»; with Hide Spoilers a future
// episode is blurred whole with its number on top. 2:3 posters carry the
// title, so they are never blurred: a bar and a mark instead.
//
// Blur is not the only signal: the bar, the time left and a screen-reader
// label («просмотрено 40 %») go with it. The blurred copy is made once from
// a small decode of the same art (LIB-013 cache) and kept in Flutter's image
// cache: nothing blurs live while a TV row scrolls.

import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart' show SynchronousFuture;
import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../theme/kinolune_theme.dart';
import 'artwork.dart';
import 'artwork_image.dart';
import 'cards.dart' show KProgressBar;

/// Watch state of a still or a poster.
enum WatchArt { unwatched, started, watched }

/// [p] in the LIB-011 thresholds: started from [continueWatchingMin] of the
/// runtime, watched when marked or past [continueWatchingMax]. Without a
/// known runtime there is no share to show: not started.
WatchArt watchArtOf(Progress? p) {
  if (p == null) return WatchArt.unwatched;
  if (p.watched) return WatchArt.watched;
  if (p.durationMs <= 0) return WatchArt.unwatched;
  final f = p.positionMs / p.durationMs;
  if (f > continueWatchingMax) return WatchArt.watched;
  if (f >= continueWatchingMin) return WatchArt.started;
  return WatchArt.unwatched;
}

/// Screen-reader text of a watch state.
String watchedSemantics(
  AppLocalizations l,
  WatchArt state, {
  double fraction = 0,
  String? remaining,
  bool next = false,
}) {
  final parts = [
    switch (state) {
      WatchArt.started => l.watchedPercent((fraction * 100).round()),
      WatchArt.watched => l.watchedDone,
      WatchArt.unwatched => l.watchedNotYet,
    },
    if (state == WatchArt.started && remaining != null) remaining,
    if (next) l.watchedNext,
  ];
  return parts.join(', ');
}

/// Key of the blurred, darkened part of a started still (tests).
const watchedBlurKey = ValueKey('watched-blur');

/// Key of the light line at the split.
const watchedSplitKey = ValueKey('watched-split');

/// Key of a still blurred whole (Hide Spoilers).
const watchedHiddenKey = ValueKey('watched-hidden');

/// Width of the blurred copy in pixels: blurred, it looks the same at any
/// size, and 64 × 36 is ~9 KB in memory.
const blurredArtWidth = 64;

/// A blurred copy of artwork: decoded small from the same source as the
/// sharp one (the LIB-013 disk cache for remote art), blurred once into a
/// new image and kept in Flutter's [ImageCache] under this key.
@immutable
class BlurredArtImage extends ImageProvider<BlurredArtImage> {
  const BlurredArtImage({
    required this.url,
    required this.seed,
    this.style = ArtStyle.backdrop,
    this.sourceWidth,
  });

  final String? url;

  /// Seed of the generated demo art.
  final String seed;
  final ArtStyle style;

  /// Width the sharp copy is decoded at: the same cached file is read, no
  /// second download.
  final int? sourceWidth;

  /// Blur radius on the small copy (~5 % of its width).
  static const sigma = 3.0;

  @override
  Future<BlurredArtImage> obtainKey(ImageConfiguration configuration) =>
      SynchronousFuture<BlurredArtImage>(this);

  @override
  ImageStreamCompleter loadImage(
    BlurredArtImage key,
    ImageDecoderCallback decode,
  ) => OneFrameImageStreamCompleter(key._load());

  Future<ImageInfo> _load() {
    final u = url;
    if (u != null && u.startsWith(kDemoArtScheme)) {
      // Demo art is drawn in code: draw it small and blur it right away.
      final h = style == ArtStyle.poster
          ? blurredArtWidth * 3 ~/ 2
          : blurredArtWidth * 9 ~/ 16;
      final size = Size(blurredArtWidth.toDouble(), h.toDouble());
      final rec = ui.PictureRecorder();
      paintGeneratedArt(Canvas(rec), size, seed, style);
      final pic = rec.endRecording();
      final small = pic.toImageSync(blurredArtWidth, h);
      pic.dispose();
      return SynchronousFuture(ImageInfo(image: blurImage(small)));
    }
    final ImageProvider source = (u == null || u.isEmpty)
        ? AssetImage(placeholderAsset(style))
        : artworkProvider(u, width: sourceWidth);
    return _resolve(ResizeImage(source, width: blurredArtWidth))
        .then((img) => ImageInfo(image: blurImage(img)));
  }

  static Future<ui.Image> _resolve(ImageProvider p) {
    final done = Completer<ui.Image>();
    final stream = p.resolve(ImageConfiguration.empty);
    late final ImageStreamListener listener;
    listener = ImageStreamListener(
      (info, _) {
        stream.removeListener(listener);
        if (!done.isCompleted) done.complete(info.image.clone());
        info.dispose();
      },
      onError: (e, st) {
        stream.removeListener(listener);
        if (!done.isCompleted) done.completeError(e, st);
      },
    );
    stream.addListener(listener);
    return done.future;
  }

  @override
  bool operator ==(Object other) =>
      other is BlurredArtImage &&
      other.url == url &&
      other.seed == seed &&
      other.style == style &&
      other.sourceWidth == sourceWidth;

  @override
  int get hashCode => Object.hash(url, seed, style, sourceWidth);

  @override
  String toString() => 'BlurredArtImage("$url", $seed, ${style.name})';
}

/// [src] blurred into a new image of its size; [src] is disposed.
ui.Image blurImage(ui.Image src, {double sigma = BlurredArtImage.sigma}) {
  final w = src.width, h = src.height;
  final rec = ui.PictureRecorder();
  Canvas(rec).drawImage(
    src,
    Offset.zero,
    Paint()
      ..filterQuality = FilterQuality.medium
      ..imageFilter = ui.ImageFilter.blur(
        sigmaX: sigma,
        sigmaY: sigma,
        tileMode: TileMode.clamp,
      ),
  );
  final pic = rec.endRecording();
  final out = pic.toImageSync(w, h);
  pic.dispose();
  src.dispose();
  return out;
}

/// The cached blurred copy of an art, stretched over the box.
class BlurredArt extends StatelessWidget {
  const BlurredArt({
    super.key,
    required this.url,
    required this.seed,
    this.style = ArtStyle.backdrop,
  });

  final String? url;
  final String seed;
  final ArtStyle style;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, c) => Image(
      image: BlurredArtImage(
        url: url,
        seed: seed,
        style: style,
        sourceWidth: decodeWidth(context, c.maxWidth),
      ),
      fit: BoxFit.cover,
      // The small copy stretched: smooth, not blocky.
      filterQuality: FilterQuality.medium,
      gaplessPlayback: true,
      excludeFromSemantics: true,
      errorBuilder: (_, _, _) => const ColoredBox(color: Color(0xFF15151A)),
    ),
  );
}

/// A 16:9 still with its watch state (UI-037). [fraction] is the watched
/// share of a started item; [remaining] its «ещё N мин». [hiddenLabel] (the
/// episode number) marks a still Hide Spoilers blurs whole.
class WatchedStill extends StatelessWidget {
  const WatchedStill({
    super.key,
    required this.url,
    required this.seed,
    this.state = WatchArt.unwatched,
    this.fraction = 0,
    this.remaining,
    this.next = false,
    this.hiddenLabel,
    this.dot = true,
    this.radius = 0,
  });

  final String? url;
  final String seed;
  final WatchArt state;
  final double fraction;
  final String? remaining;

  /// The next episode to watch: outline and «Далее».
  final bool next;

  /// Episode number over a still blurred whole (Hide Spoilers).
  final String? hiddenLabel;

  /// The accent dot of a still not watched yet (episodes; not shelves).
  final bool dot;

  /// Corner radius of the outline of [next].
  final double radius;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final hidden = hiddenLabel;
    final started = state == WatchArt.started && hidden == null;
    final f = fraction.clamp(0.0, 1.0);
    final art = Artwork(url: url, seed: seed, style: ArtStyle.backdrop);
    final label = hidden != null
        ? l.watchedSpoilerHidden(hidden)
        : watchedSemantics(
            l,
            state,
            fraction: f,
            remaining: remaining,
            next: next,
          );
    return Semantics(
      label: label,
      child: ExcludeSemantics(
        child: LayoutBuilder(
          builder: (context, c) {
            final w = c.maxWidth;
            final h = c.maxHeight;
            final small = w < 200;
            final split = w * f;
            return Stack(
              fit: StackFit.expand,
              children: [
                if (hidden != null)
                  BlurredArt(key: watchedHiddenKey, url: url, seed: seed)
                else
                  art,
                if (hidden != null)
                  ColoredBox(
                    color: Colors.black.withValues(alpha: .25),
                    child: Center(
                      child: Text(
                        hidden,
                        style: TextStyle(
                          color: Colors.white,
                          fontSize: (h * .2).clamp(12.0, 32.0),
                          fontWeight: FontWeight.w700,
                          shadows: const [
                            Shadow(blurRadius: 8, color: Colors.black54),
                          ],
                        ),
                      ),
                    ),
                  ),
                if (started) ...[
                  // The rest: the cached blurred copy, aligned with the
                  // sharp one and clipped to the right of the split.
                  Positioned(
                    key: watchedBlurKey,
                    left: split,
                    top: 0,
                    right: 0,
                    bottom: 0,
                    child: ClipRect(
                      child: OverflowBox(
                        alignment: Alignment.centerRight,
                        minWidth: w,
                        maxWidth: w,
                        child: Stack(
                          fit: StackFit.expand,
                          children: [
                            BlurredArt(url: url, seed: seed),
                            ColoredBox(
                              color: Colors.black.withValues(alpha: .3),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                  if (f > 0 && f < 1)
                    Positioned(
                      key: watchedSplitKey,
                      left: split - .75,
                      top: 0,
                      bottom: 0,
                      width: 1.5,
                      child: ColoredBox(
                        color: Colors.white.withValues(alpha: .85),
                      ),
                    ),
                ],
                if (state == WatchArt.watched && hidden == null)
                  // Muted: the watched still steps back.
                  ColoredBox(color: Colors.black.withValues(alpha: .38)),
                if (state == WatchArt.watched && hidden == null)
                  Positioned(
                    top: 6,
                    right: 6,
                    child: WatchMark(
                      key: const ValueKey('watched-check'),
                      state: WatchArt.watched,
                      size: small ? 18 : 22,
                    ),
                  ),
                if (state == WatchArt.unwatched && dot && hidden == null)
                  Positioned(
                    top: 8,
                    right: 8,
                    child: _AccentDot(size: small ? 8 : 10),
                  ),
                if (started)
                  Positioned(
                    left: 8,
                    right: 8,
                    bottom: 7,
                    child: KProgressBar(value: f, height: 3),
                  ),
                if (started && remaining != null)
                  Positioned(
                    right: 6,
                    bottom: 16,
                    child: _LeftChip(text: remaining!, small: small),
                  ),
                if (next) ...[
                  Positioned.fill(
                    child: DecoratedBox(
                      key: const ValueKey('watched-next'),
                      decoration: BoxDecoration(
                        border: Border.all(color: KColors.accent, width: 2.5),
                        borderRadius: BorderRadius.circular(radius),
                      ),
                    ),
                  ),
                  Positioned(
                    left: 8,
                    bottom: started ? 16 : 8,
                    child: _NextChip(text: l.watchedNext, small: small),
                  ),
                ],
              ],
            );
          },
        ),
      ),
    );
  }
}

/// «◐» (started) or «✓» (watched) in a circle.
class WatchMark extends StatelessWidget {
  const WatchMark({super.key, required this.state, this.size = 22});

  final WatchArt state;
  final double size;

  @override
  Widget build(BuildContext context) {
    final watched = state == WatchArt.watched;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: watched ? KColors.accent : Colors.black.withValues(alpha: .6),
        shape: BoxShape.circle,
      ),
      child: Icon(
        // Material's «contrast» is a half-filled circle: «◐».
        watched ? Icons.check_rounded : Icons.contrast_rounded,
        size: size * .68,
        color: Colors.white,
      ),
    );
  }
}

/// Count of unwatched episodes on a show poster.
class EpisodesLeftMark extends StatelessWidget {
  const EpisodesLeftMark({super.key, required this.count, this.size = 22});

  final int count;
  final double size;

  @override
  Widget build(BuildContext context) => Container(
    constraints: BoxConstraints(minWidth: size),
    height: size,
    padding: const EdgeInsets.symmetric(horizontal: 6),
    alignment: Alignment.center,
    decoration: BoxDecoration(
      color: KColors.accent,
      borderRadius: BorderRadius.circular(size / 2),
    ),
    child: Text(
      count > 99 ? '99+' : '$count', // l10n-ignore
      style: TextStyle(
        color: Colors.white,
        fontSize: size * .55,
        fontWeight: FontWeight.w700,
        height: 1,
      ),
    ),
  );
}

/// «?»: the review inbox has this item (META-034).
class ReviewMark extends StatelessWidget {
  const ReviewMark({super.key, this.size = 22});

  final double size;

  @override
  Widget build(BuildContext context) => Semantics(
    label: context.l10n.watchedNeedsReview,
    child: Container(
      width: size,
      height: size,
      alignment: Alignment.center,
      decoration: const BoxDecoration(
        color: Color(0xFFE0A100),
        shape: BoxShape.circle,
      ),
      child: Text(
        '?', // l10n-ignore
        style: TextStyle(
          color: Colors.black,
          fontSize: size * .62,
          fontWeight: FontWeight.w800,
          height: 1,
        ),
      ),
    ),
  );
}

/// Marks over a 2:3 poster (UI-037): never blurred, the title is on it. A
/// movie: a thin bar at the bottom and «◐»/«✓»; a show: the count of
/// unwatched episodes, or «✓» when all are watched.
class PosterWatchMarks extends StatelessWidget {
  const PosterWatchMarks({
    super.key,
    required this.itemId,
    required this.state,
    this.fraction,
    this.episodesLeft,
  });

  final String itemId;
  final WatchArt state;

  /// Watched share of a started movie.
  final double? fraction;

  /// Unwatched episodes of a show, when known.
  final int? episodesLeft;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final f = fraction;
    final left = episodesLeft;
    final allWatched = state == WatchArt.watched || (left != null && left == 0);
    final String label;
    if (allWatched) {
      label = l.watchedDone;
    } else if (left != null) {
      label = l.watchedEpisodesLeft(left);
    } else if (state == WatchArt.started) {
      label = f == null
          ? l.watchedStarted
          : l.watchedPercent((f * 100).round());
    } else {
      return const SizedBox.shrink();
    }
    return Semantics(
      label: label,
      child: ExcludeSemantics(
        child: Stack(
          fit: StackFit.expand,
          children: [
            Positioned(
              right: 6,
              top: 6,
              child: allWatched
                  ? WatchMark(
                      key: ValueKey('badge-watched-$itemId'),
                      state: WatchArt.watched,
                    )
                  : left != null
                  ? EpisodesLeftMark(
                      key: ValueKey('badge-left-$itemId'),
                      count: left,
                    )
                  : WatchMark(
                      key: ValueKey('badge-inProgress-$itemId'),
                      state: WatchArt.started,
                    ),
            ),
            if (!allWatched && f != null)
              Positioned(
                left: 8,
                right: 8,
                bottom: 8,
                child: KProgressBar(
                  key: ValueKey('badge-progress-$itemId'),
                  value: f,
                  height: 3,
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _AccentDot extends StatelessWidget {
  const _AccentDot({required this.size});

  final double size;

  @override
  Widget build(BuildContext context) => Container(
    key: const ValueKey('watched-dot'),
    width: size,
    height: size,
    decoration: BoxDecoration(
      color: KColors.accent,
      shape: BoxShape.circle,
      border: Border.all(color: Colors.black.withValues(alpha: .5), width: 1),
    ),
  );
}

class _LeftChip extends StatelessWidget {
  const _LeftChip({required this.text, required this.small});

  final String text;
  final bool small;

  @override
  Widget build(BuildContext context) => Container(
    key: const ValueKey('watched-left'),
    padding: const EdgeInsets.fromLTRB(4, 2, 6, 2),
    decoration: BoxDecoration(
      color: Colors.black.withValues(alpha: .62),
      borderRadius: BorderRadius.circular(4),
    ),
    child: Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Icon(
          Icons.contrast_rounded,
          size: small ? 10 : 12,
          color: Colors.white,
        ),
        const SizedBox(width: 3),
        Text(
          text,
          style: TextStyle(
            color: Colors.white,
            fontSize: small ? 9 : 11,
            fontWeight: FontWeight.w600,
            height: 1.2,
          ),
        ),
      ],
    ),
  );
}

class _NextChip extends StatelessWidget {
  const _NextChip({required this.text, required this.small});

  final String text;
  final bool small;

  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
    decoration: BoxDecoration(
      color: KColors.accent,
      borderRadius: BorderRadius.circular(4),
    ),
    child: Text(
      text,
      style: TextStyle(
        color: Colors.white,
        fontSize: small ? 9 : 11,
        fontWeight: FontWeight.w700,
        height: 1.2,
      ),
    ),
  );
}
