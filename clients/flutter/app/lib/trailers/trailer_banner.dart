// SPDX-License-Identifier: MPL-2.0
//
// Pieces of the trailer banner (UI-036) used by the detail hero: the
// slowly moving backdrop, the frame of an online trailer, the «Трейлер»
// button and «вверх» on the TV remote.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import 'trailer_controller.dart';

/// Key of the «Трейлер» button (tests).
const trailerButtonKey = ValueKey('detail-trailer');

/// The smallest frame an online trailer plays in, logical pixels high.
const trailerFrameMinHeight = 108.0;

/// A backdrop that drifts and zooms very slowly while [moving] (the
/// «живой» фон of a page without a playable trailer). Still otherwise.
class LiveBackdrop extends StatefulWidget {
  const LiveBackdrop({super.key, required this.moving, required this.child});

  final bool moving;
  final Widget child;

  @override
  State<LiveBackdrop> createState() => _LiveBackdropState();
}

class _LiveBackdropState extends State<LiveBackdrop>
    with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController(
    vsync: this,
    duration: const Duration(seconds: 24),
  );

  @override
  void initState() {
    super.initState();
    _sync();
  }

  @override
  void didUpdateWidget(LiveBackdrop old) {
    super.didUpdateWidget(old);
    if (old.moving != widget.moving) _sync();
  }

  void _sync() {
    if (widget.moving) {
      unawaited(_c.repeat(reverse: true));
    } else {
      _c.stop();
    }
  }

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ClipRect(
    child: AnimatedBuilder(
      animation: _c,
      child: widget.child,
      builder: (context, child) {
        final t = Curves.easeInOut.transform(_c.value);
        return Transform.scale(
          key: const ValueKey('trailer-live-backdrop'),
          scale: 1 + .08 * t,
          alignment: Alignment(.6 - 1.2 * t, -.2),
          child: child,
        );
      },
    ),
  );
}

/// The «Трейлер» button of a page; hidden while there is none.
Widget? trailerButton(BuildContext context, TrailerController? trailer) {
  if (trailer?.trailer == null) return null;
  return KButton(
    key: trailerButtonKey,
    icon: Icons.movie_outlined,
    label: context.l10n.detailTrailer,
    onPressed: () => unawaited(trailer!.openFullScreen(context)),
  );
}

/// «Вверх» on the TV remote from the top row of the page's buttons opens
/// the trailer full screen. Arrow up from a lower row of buttons still
/// moves to the row above.
class TrailerUpKey extends StatelessWidget {
  const TrailerUpKey({
    super.key,
    required this.trailer,
    required this.enabled,
    required this.child,
  });

  final TrailerController? trailer;

  /// Only on TVs.
  final bool enabled;
  final Widget child;

  @override
  Widget build(BuildContext context) => Focus(
    canRequestFocus: false,
    skipTraversal: true,
    onKeyEvent: (node, event) {
      final t = trailer;
      if (!enabled ||
          t?.trailer == null ||
          event is! KeyDownEvent ||
          event.logicalKey != LogicalKeyboardKey.arrowUp) {
        return KeyEventResult.ignored;
      }
      final focused = FocusManager.instance.primaryFocus;
      if (focused == null) return KeyEventResult.ignored;
      final top = focused.rect.top;
      final above = node.traversalDescendants.any(
        (n) => n != focused && n.rect.bottom <= top + 1,
      );
      if (above) return KeyEventResult.ignored;
      unawaited(t!.openFullScreen(context));
      return KeyEventResult.handled;
    },
    child: child,
  );
}

/// The frame of an online trailer in the banner: the official player and
/// nothing drawn over it.
class TrailerFrame extends StatelessWidget {
  const TrailerFrame({super.key, required this.trailer});

  final TrailerController trailer;

  @override
  Widget build(BuildContext context) {
    final s = trailer.surface;
    return DecoratedBox(
      key: const ValueKey('trailer-frame'),
      decoration: BoxDecoration(color: KColors.bgBase),
      child: s == null ? const SizedBox.expand() : s.build(context),
    );
  }
}
