// SPDX-License-Identifier: MPL-2.0

// Small overlays of the player OSD (UI-013): the gesture feedback pill
// (volume, brightness, double-tap seek, zoom), the "2×" badge while the
// video is held, the Skip Intro button and the live statistics panel.

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../engine/player_engine.dart';
import '../l10n/pa_player_strings.dart';
import '../ui/tokens.dart';
import 'osd_model.dart';
import 'osd_panels.dart' show formatRate;

/// What the feedback pill shows: an icon, a text and, for volume and
/// brightness, a level 0..1.
@immutable
class OsdFeedback {
  const OsdFeedback({
    required this.icon,
    required this.label,
    this.level,
    this.alignment = Alignment.center,
  });

  final IconData icon;
  final String label;
  final double? level;

  /// Where on the screen: double-tap seeks show on their side.
  final Alignment alignment;

  @override
  bool operator ==(Object other) =>
      other is OsdFeedback &&
      other.icon == icon &&
      other.label == label &&
      other.level == level &&
      other.alignment == alignment;

  @override
  int get hashCode => Object.hash(icon, label, level, alignment);
}

/// The transient pill in the middle (or on a side) of the video.
class OsdFeedbackPill extends StatelessWidget {
  const OsdFeedbackPill({super.key, required this.feedback});

  final ValueListenable<OsdFeedback?> feedback;

  @override
  Widget build(BuildContext context) {
    return IgnorePointer(
      child: ValueListenableBuilder<OsdFeedback?>(
        valueListenable: feedback,
        builder: (context, f, _) {
          if (f == null) return const SizedBox.shrink();
          // Phones in portrait: a shorter level bar, the text shrinks last.
          final compact = MediaQuery.sizeOf(context).width < 600;
          return Align(
            alignment: f.alignment,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 16),
              child: Container(
                key: const ValueKey('osd-feedback'),
                padding: const EdgeInsets.symmetric(
                  horizontal: 18,
                  vertical: 12,
                ),
                decoration: BoxDecoration(
                  color: PaTokens.glassStrong,
                  borderRadius: BorderRadius.circular(PaTokens.radius),
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon(f.icon, color: PaTokens.text, size: 26),
                    const SizedBox(width: 12),
                    if (f.level != null) ...[
                      SizedBox(
                        width: compact ? 72 : 120,
                        child: ClipRRect(
                          borderRadius: BorderRadius.circular(3),
                          child: LinearProgressIndicator(
                            value: f.level!.clamp(0.0, 1.0),
                            minHeight: 6,
                            color: PaTokens.text,
                            backgroundColor: PaTokens.track,
                          ),
                        ),
                      ),
                      const SizedBox(width: 12),
                    ],
                    Flexible(
                      child: Text(
                        f.label,
                        key: const ValueKey('osd-feedback-label'),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(
                          color: PaTokens.text,
                          fontSize: 17,
                          fontWeight: FontWeight.w600,
                          fontFeatures: [FontFeature.tabularFigures()],
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}

/// The "2×" badge at the top while the video is held (speed hold).
class HoldSpeedBadge extends StatelessWidget {
  const HoldSpeedBadge({super.key, required this.holding});

  final ValueListenable<bool> holding;

  @override
  Widget build(BuildContext context) {
    final l = PaPlayerStrings.of(context);
    return IgnorePointer(
      child: ValueListenableBuilder<bool>(
        valueListenable: holding,
        builder: (context, on, _) => on
            ? Align(
                alignment: Alignment.topCenter,
                child: SafeArea(
                  child: Container(
                    key: const ValueKey('osd-hold-speed'),
                    margin: const EdgeInsets.only(top: 24),
                    padding: const EdgeInsets.symmetric(
                      horizontal: 14,
                      vertical: 8,
                    ),
                    decoration: BoxDecoration(
                      color: PaTokens.glassStrong,
                      borderRadius: BorderRadius.circular(20),
                    ),
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        const Icon(
                          Icons.fast_forward_rounded,
                          color: PaTokens.text,
                          size: 20,
                        ),
                        const SizedBox(width: 8),
                        Text(
                          l.holdSpeed,
                          style: const TextStyle(
                            color: PaTokens.text,
                            fontSize: 15,
                            fontWeight: FontWeight.w600,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              )
            : const SizedBox.shrink(),
      ),
    );
  }
}

/// Label of the skip button for [kind].
String skipLabel(SkipKind kind, PaPlayerStrings l) => switch (kind) {
  SkipKind.intro => l.skipIntro,
  SkipKind.recap => l.skipRecap,
  SkipKind.credits => l.skipCredits,
};

/// "Skip Intro" / "Skip Credits" at the bottom right while the playhead is
/// in a [SkipSegment] (the segments come from PLAY-011 through
/// `PlayRequest.skipSegments`). Shown with or without the OSD.
class SkipSegmentButton extends StatelessWidget {
  const SkipSegmentButton({
    super.key,
    required this.segment,
    required this.onSkip,
    required this.compact,
    required this.osdVisible,
    this.focusNode,
    this.countdown,
  });

  final SkipSegment segment;

  /// Seconds until the player skips by itself ([SkipMode.auto]); null:
  /// it waits for the button.
  final int? countdown;
  final VoidCallback onSkip;
  final bool compact;
  final bool osdVisible;
  final FocusNode? focusNode;

  @override
  Widget build(BuildContext context) {
    final l = PaPlayerStrings.of(context);
    // Above the bottom bar while the OSD is up, low in the corner otherwise.
    final bottom = osdVisible ? (compact ? 200.0 : 150.0) : 48.0;
    return AnimatedPositioned(
      duration: MediaQuery.of(context).disableAnimations
          ? Duration.zero
          : PaTokens.osdFade,
      right: compact ? 16 : 48,
      bottom: bottom,
      child: SafeArea(
        child: FilledButton.icon(
          key: const ValueKey('osd-skip-segment'),
          focusNode: focusNode,
          style: FilledButton.styleFrom(
            backgroundColor: PaTokens.text,
            foregroundColor: PaTokens.bg,
            padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 14),
            textStyle: const TextStyle(
              fontSize: 16,
              fontWeight: FontWeight.w600,
            ),
          ),
          onPressed: onSkip,
          icon: const Icon(Icons.skip_next_rounded),
          label: Text(
            countdown == null
                ? skipLabel(segment.kind, l)
                : '${skipLabel(segment.kind, l)} ($countdown)',
          ),
        ),
      ),
    );
  }
}

/// Live statistics ("stats for nerds") at the top left: the engine's
/// technical summary plus the numbers it measures while shown
/// ([StatsEngine]). Does not take focus or input.
class StatsOverlay extends StatelessWidget {
  const StatsOverlay({super.key, required this.engine, required this.zoom});

  final PlayerEngine engine;
  final ValueListenable<VideoZoom> zoom;

  @override
  Widget build(BuildContext context) {
    final l = PaPlayerStrings.of(context);
    final size = MediaQuery.sizeOf(context);
    final compact = size.width < 600;
    // Phones in landscape are short: smaller rows, and what does not fit is
    // cut off at the bottom rather than overflowing.
    final short = size.height < 500;
    return IgnorePointer(
      child: Align(
        alignment: Alignment.topLeft,
        child: SafeArea(
          child: Container(
            key: const ValueKey('osd-stats'),
            margin: EdgeInsets.fromLTRB(
              compact ? 12 : 32,
              short ? 64 : 84,
              12,
              12,
            ),
            padding: EdgeInsets.all(short ? 8 : 12),
            constraints: BoxConstraints(maxWidth: compact ? 280 : 360),
            clipBehavior: Clip.hardEdge,
            decoration: BoxDecoration(
              color: const Color(0xCC000000),
              borderRadius: BorderRadius.circular(10),
            ),
            child: ValueListenableBuilder<EngineState>(
              valueListenable: engine.state,
              builder: (context, s, _) {
                final rows = <(String, String)>[
                  for (final e in s.info.entries)
                    (l.engineInfoKey(e.key), e.value),
                  for (final e in s.stats.entries)
                    (l.engineInfoKey(e.key), e.value),
                  (l.speed, formatRate(s.rate)),
                  (l.zoom, OsdSidePanelLabels.zoom(zoom.value, l)),
                  (l.volume, '${s.volume.round()} %'),
                  (l.infoPosition, formatDuration(s.position)),
                ];
                return SingleChildScrollView(
                  physics: const NeverScrollableScrollPhysics(),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        l.statistics,
                        style: const TextStyle(
                          color: PaTokens.text,
                          fontSize: 14,
                          fontWeight: FontWeight.w700,
                        ),
                      ),
                      const SizedBox(height: 6),
                      for (final (k, v) in rows)
                        Padding(
                          padding: const EdgeInsets.symmetric(vertical: 1),
                          child: Text.rich(
                            TextSpan(
                              children: [
                                TextSpan(
                                  text: '$k  ',
                                  style: const TextStyle(
                                    color: PaTokens.textSecondary,
                                  ),
                                ),
                                TextSpan(text: v),
                              ],
                            ),
                            style: TextStyle(
                              color: PaTokens.text,
                              fontSize: short ? 10.5 : 12,
                              fontFeatures: const [
                                FontFeature.tabularFigures(),
                              ],
                            ),
                          ),
                        ),
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

/// Labels shared by the panels and overlays.
abstract final class OsdSidePanelLabels {
  static String zoom(VideoZoom z, PaPlayerStrings l) => switch (z) {
    VideoZoom.fit => l.zoomFit,
    VideoZoom.fill => l.zoomFill,
    VideoZoom.stretch => l.zoomStretch,
    VideoZoom.ratio16x9 => '16:9', // l10n-ignore
    VideoZoom.ratio4x3 => '4:3', // l10n-ignore
    VideoZoom.ratio235 => '2.35:1', // l10n-ignore
  };
}
