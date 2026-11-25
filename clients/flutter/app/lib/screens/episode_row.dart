// SPDX-License-Identifier: MPL-2.0
//
// UI-055: the episodes of a season as one horizontal row of 16:9 cards
// that runs past the edge of the screen. The row is lazy (BUG-019: seasons
// of a thousand episodes and more) and every card has the same width, so
// any episode's place in the row is known without building it.
//
// - Desktop: round ‹ › buttons at the ends of the row, shown on hover and
//   whenever the keyboard is in use, page by the visible part and hide at
//   the start and the end. The row also moves by mouse drag, Shift+wheel and
//   a horizontal trackpad swipe; a plain wheel over it scrolls the page.
//   A thin scrollbar on hover.
// - Phone and tablet: a swipe with inertia that settles on a card.
// - TV and keyboard: ←/→ move the focus to the next card and the row
//   follows it (the focus stays at most the second card from an edge), a
//   held key speeds up, Home/End go to the ends, ↑ goes to the range chips
//   or the season tabs, ↓ to the next block.
// - The row opens on the episode to watch next (UI-037), else on the first.
// - A season longer than [episodeRangeSize]: range chips above the row
//   («1–50», «51–100», …) jump to a part of it.

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';

/// Episodes per range chip.
const episodeRangeSize = 50;

/// Key of the row's horizontal list.
const episodeRowKey = ValueKey('show-episode-row');

/// Keys of the ‹ › buttons.
const episodeRowBackKey = ValueKey('show-episode-row-back');
const episodeRowForwardKey = ValueKey('show-episode-row-forward');

/// Key of the range chip that starts at the [first] card (1-based place in
/// the row).
ValueKey<String> episodeRangeKey(int first) => ValueKey('show-range-$first');

/// A horizontal, lazily built row of [count] cards of the same width.
class EpisodeRow extends StatefulWidget {
  const EpisodeRow({
    super.key,
    required this.count,
    required this.cardWidth,
    required this.cardHeight,
    required this.itemBuilder,
    this.startIndex,
    this.rangeLabel,
    this.onUp,
  });

  final int count;
  final double cardWidth;
  final double cardHeight;

  /// Builds the card at [index]; [focusNode] goes to its focusable.
  final Widget Function(BuildContext context, int index, FocusNode focusNode)
  itemBuilder;

  /// The card the row opens on (and moves to when it changes).
  final int? startIndex;

  /// Text and screen-reader label of the range chip from card [first] to
  /// card [last] (0-based); no chips without it.
  final (String, String) Function(int first, int last)? rangeLabel;

  /// ↑ from a card when there are no range chips (the season tabs).
  final VoidCallback? onUp;

  @override
  State<EpisodeRow> createState() => _EpisodeRowState();
}

class _EpisodeRowState extends State<EpisodeRow> {
  /// Opens on [EpisodeRow.startIndex]: the offset is read when the list
  /// first lays out, once the width of the row is known.
  late final ScrollController _scroll = _OpeningController(
    () => _openOffset(widget.startIndex ?? 0),
  );
  final _nodes = <int, FocusNode>{};
  final _rangeNodes = <int, FocusNode>{};

  /// The mice over the row.
  final _mice = <int>{};

  bool get _hover => _mice.isNotEmpty;

  /// Where a running animation of the row goes.
  double? _dest;

  /// Repeats of a held arrow key (speeds up the row).
  int _repeats = 0;

  // Geometry of the last layout.
  double _viewport = 0;
  double _gap = 16;
  double _padLeft = 0;
  double _padRight = 0;
  bool _phone = false;

  double get _extent => widget.cardWidth + _gap;

  bool get _hasRanges =>
      widget.rangeLabel != null && widget.count > episodeRangeSize;

  /// The range chip of the part in view.
  final _part = ValueNotifier(0);

  @override
  void initState() {
    super.initState();
    FocusManager.instance.addHighlightModeListener(_onHighlightMode);
    _scroll.addListener(_onScroll);
  }

  void _onScroll() {
    if (!_hasRanges) return;
    final parts = (widget.count + episodeRangeSize - 1) ~/ episodeRangeSize;
    final at = _offset;
    _part.value = at >= _max - .5 && _max > 0
        ? parts - 1
        : ((at / _extent + .01).floor() ~/ episodeRangeSize).clamp(
            0,
            parts - 1,
          );
  }

  @override
  void didUpdateWidget(EpisodeRow old) {
    super.didUpdateWidget(old);
    final start = widget.startIndex;
    if (start != null && start != old.startIndex && start < widget.count) {
      // UI-037: after an episode the row shows the next one.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _reveal(start, animate: true);
      });
    }
  }

  @override
  void dispose() {
    FocusManager.instance.removeHighlightModeListener(_onHighlightMode);
    _scroll.dispose();
    _part.dispose();
    for (final n in [..._nodes.values, ..._rangeNodes.values]) {
      n.dispose();
    }
    super.dispose();
  }

  void _onHighlightMode(FocusHighlightMode _) {
    if (mounted) setState(() {});
  }

  bool get _reduceMotion =>
      MediaQuery.maybeDisableAnimationsOf(context) ?? false;

  FocusNode _node(int i) => _nodes.putIfAbsent(i, () {
    late final FocusNode node;
    node = FocusNode(debugLabel: 'episode-$i')
      ..addListener(() {
        // Focus that came by Tab or from the tabs above: into the frame.
        if (node.hasPrimaryFocus) _reveal(i, animate: true);
      });
    return node;
  });

  FocusNode _rangeNode(int r) =>
      _rangeNodes.putIfAbsent(r, () => FocusNode(debugLabel: 'range-$r'));

  /// The largest scroll offset of the row.
  double get _max {
    final s = _scroll;
    if (s.hasClients && s.position.hasContentDimensions) {
      return s.position.maxScrollExtent;
    }
    return math.max(
      0,
      _padLeft +
          widget.count * _extent +
          math.max(0, _padRight - _gap) -
          _viewport,
    );
  }

  double get _offset => _scroll.hasClients ? _scroll.offset : 0;

  /// Offset that shows card [i] where the row opens on it: the first card
  /// on a phone (about one card fits), else the second, the one before it
  /// in view too.
  double _openOffset(int i) =>
      (math.max(0, _phone ? i : i - 1) * _extent).clamp(0, _max).toDouble();

  /// Scrolls the row as little as needed for card [i] to be in view with a
  /// neighbour on each side, when they fit (TV: the focus is at most the
  /// second card from an edge).
  void _reveal(int i, {required bool animate}) {
    if (_viewport <= 0) return;
    final cur = _dest ?? _offset;
    final left = _padLeft + i * _extent;
    final right = left + widget.cardWidth;
    // Neighbours wholly inside the row.
    var lo = (i < widget.count - 1 ? right + _extent : right) - _viewport;
    var hi = i > 0 ? left - _extent : 0.0;
    if (lo > hi) {
      // No room for them: the card itself inside the page margins.
      lo = right + _padRight - _viewport;
      hi = left - _padLeft;
      if (lo > hi) lo = hi;
    }
    final target = cur.clamp(lo, hi).clamp(0, _max).toDouble();
    if ((target - cur).abs() < .5) return;
    _moveTo(target, animate: animate);
  }

  void _moveTo(double target, {required bool animate}) {
    final s = _scroll;
    if (!s.hasClients) return;
    if (!animate || _reduceMotion) {
      _dest = null;
      s.jumpTo(target);
      return;
    }
    _dest = target;
    s
        .animateTo(
          target,
          duration: const Duration(milliseconds: 240),
          curve: Curves.easeOutCubic,
        )
        .whenComplete(() {
          if (_dest == target) _dest = null;
        });
  }

  /// ‹ ›: the row moves by the cards that fit in the visible part.
  void _page(int dir) {
    final cur = _dest ?? _offset;
    final fit = math.max(
      1,
      ((_viewport - _padLeft - _padRight + _gap) / _extent).floor(),
    );
    final at = dir > 0
        ? (cur / _extent + .01).floor() + fit
        : (cur / _extent - .01).ceil() - fit;
    _moveTo(
      (math.max(0, at) * _extent).clamp(0, _max).toDouble(),
      animate: true,
    );
  }

  /// A range chip: the row moves to its first card.
  void _jump(int first) =>
      _moveTo((first * _extent).clamp(0, _max).toDouble(), animate: true);

  int? get _focused {
    for (final e in _nodes.entries) {
      if (e.value.hasPrimaryFocus) return e.key;
    }
    return null;
  }

  KeyEventResult _onKey(FocusNode _, KeyEvent e) {
    if (e is KeyUpEvent) {
      _repeats = 0;
      return KeyEventResult.ignored;
    }
    final i = _focused;
    if (i == null) return KeyEventResult.ignored;
    final key = e.logicalKey;
    if (key == LogicalKeyboardKey.arrowUp) {
      if (_hasRanges) {
        final chip = _rangeNode(i ~/ episodeRangeSize)..requestFocus();
        WidgetsBinding.instance.addPostFrameCallback((_) {
          final ctx = chip.context;
          if (ctx != null && ctx.mounted) {
            Scrollable.ensureVisible(ctx, duration: KMotion.focus);
          }
        });
        return KeyEventResult.handled;
      }
      final up = widget.onUp;
      if (up == null) return KeyEventResult.ignored;
      up();
      return KeyEventResult.handled;
    }
    _repeats = e is KeyRepeatEvent ? _repeats + 1 : 0;
    // A held key speeds up: one card, then two, then four at a time.
    final step = _repeats > 24 ? 4 : (_repeats > 8 ? 2 : 1);
    final last = widget.count - 1;
    final int to;
    if (key == LogicalKeyboardKey.arrowRight) {
      to = math.min(last, i + step);
    } else if (key == LogicalKeyboardKey.arrowLeft) {
      to = math.max(0, i - step);
    } else if (key == LogicalKeyboardKey.home) {
      to = 0;
    } else if (key == LogicalKeyboardKey.end) {
      to = last;
    } else {
      return KeyEventResult.ignored;
    }
    // At an end the key goes on (← to the side menu).
    if (to == i) return KeyEventResult.ignored;
    _reveal(to, animate: e is KeyDownEvent && (to - i).abs() == 1);
    // A card not built yet takes the focus as soon as it is.
    _node(to).requestFocus();
    return KeyEventResult.handled;
  }

  /// Shift+wheel of a mouse moves the row. Flutter flips the wheel's axis
  /// itself; this covers a platform that sends the turn of the wheel with
  /// Shift only as a vertical delta to a row whose own flip is off (below),
  /// so the row never scrolls twice.
  void _onPointerSignal(PointerSignalEvent e) {
    final s = _scroll;
    if (e is! PointerScrollEvent ||
        e.kind != PointerDeviceKind.mouse ||
        !s.hasClients ||
        !HardwareKeyboard.instance.isShiftPressed ||
        e.scrollDelta.dx != 0 ||
        e.scrollDelta.dy == 0) {
      return;
    }
    final delta = e.scrollDelta.dy;
    final target = (s.offset + delta).clamp(0, _max);
    if (target == s.offset) return;
    GestureBinding.instance.pointerSignalResolver.register(e, (event) {
      if (!s.hasClients) return;
      s.position.pointerScroll(delta);
      (event as PointerScrollEvent).respond(allowPlatformDefault: false);
    });
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    _gap = d.isTv ? 20 : 16;
    _padLeft = d.pageInsets.left;
    _padRight = d.pageInsets.right;
    _phone = d.isPhone;
    final keyboard =
        FocusManager.instance.highlightMode == FocusHighlightMode.traditional;
    final buttons = !d.isTv && (_hover || keyboard);
    const top = 10.0;
    final height = widget.cardHeight + top + 14;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        if (_hasRanges) _ranges(d),
        LayoutBuilder(
          builder: (context, c) {
            _viewport = c.maxWidth;
            final scroll = _scroll;
            Widget list = ListView.builder(
              key: episodeRowKey,
              controller: scroll,
              scrollDirection: Axis.horizontal,
              clipBehavior: Clip.none,
              itemExtent: _extent,
              // A few cards past the edge, so the next card under the remote
              // is there before the row moves.
              scrollCacheExtent: ScrollCacheExtent.pixels(_extent * 2),
              physics: d.isPhone ? _SnapPhysics(extent: _extent) : null,
              padding: EdgeInsets.fromLTRB(
                _padLeft,
                top,
                math.max(0, _padRight - _gap),
                14,
              ),
              itemCount: widget.count,
              itemBuilder: (context, i) => Align(
                alignment: Alignment.topLeft,
                child: SizedBox(
                  width: widget.cardWidth,
                  child: widget.itemBuilder(context, i, _node(i)),
                ),
              ),
            );
            if (!d.isPhone && !d.isTv) {
              list = Scrollbar(
                controller: scroll,
                thumbVisibility: _hover,
                thickness: 4,
                radius: const Radius.circular(2),
                child: list,
              );
            }
            return SizedBox(
              height: height,
              child: MouseRegion(
                onEnter: (e) {
                  if (e.kind == PointerDeviceKind.mouse) {
                    setState(() => _mice.add(e.device));
                  }
                },
                onExit: (e) {
                  if (_mice.contains(e.device)) {
                    setState(() => _mice.remove(e.device));
                  }
                },
                child: Stack(
                  children: [
                    Positioned.fill(
                      child: ScrollConfiguration(
                        behavior: ScrollConfiguration.of(context).copyWith(
                          scrollbars: false,
                          // The mouse drags the row too; the trackpad swipes
                          // it (pan gestures).
                          dragDevices: PointerDeviceKind.values.toSet(),
                          // Shift+wheel: [_onPointerSignal].
                          pointerAxisModifiers: const {},
                        ),
                        child: Listener(
                          onPointerSignal: _onPointerSignal,
                          child: Focus(
                            canRequestFocus: false,
                            skipTraversal: true,
                            onKeyEvent: _onKey,
                            child: FocusTraversalGroup(child: list),
                          ),
                        ),
                      ),
                    ),
                    if (!d.isTv)
                      ListenableBuilder(
                        listenable: scroll,
                        builder: (context, _) {
                          final at = scroll.hasClients ? scroll.offset : 0.0;
                          final y =
                              top +
                              widget.cardWidth * 9 / 32 -
                              _PageButton.size / 2;
                          return Stack(
                            children: [
                              Positioned(
                                left: math.max(
                                  4,
                                  _padLeft / 2 - _PageButton.size / 2,
                                ),
                                top: y,
                                child: _PageButton(
                                  key: episodeRowBackKey,
                                  visible: buttons && at > .5,
                                  icon: Icons.chevron_left_rounded,
                                  label: l.showEpisodesPrevious,
                                  onPressed: () => _page(-1),
                                ),
                              ),
                              Positioned(
                                right: math.max(
                                  4,
                                  _padRight / 2 - _PageButton.size / 2,
                                ),
                                top: y,
                                child: _PageButton(
                                  key: episodeRowForwardKey,
                                  visible: buttons && at < _max - .5,
                                  icon: Icons.chevron_right_rounded,
                                  label: l.showEpisodesNext,
                                  onPressed: () => _page(1),
                                ),
                              ),
                            ],
                          );
                        },
                      ),
                  ],
                ),
              ),
            );
          },
        ),
      ],
    );
  }

  /// «1–50», «51–100», …: the chip of the part in view is selected.
  Widget _ranges(KDensity d) {
    final count = widget.count;
    final parts = (count + episodeRangeSize - 1) ~/ episodeRangeSize;
    final label = widget.rangeLabel!;
    return Padding(
      padding: EdgeInsets.only(top: d.isTv ? 6 : 8),
      child: SizedBox(
        height: d.isTv ? 52 : 48,
        child: ValueListenableBuilder<int>(
          valueListenable: _part,
          builder: (context, current, _) {
            return FocusTraversalGroup(
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                clipBehavior: Clip.none,
                padding: EdgeInsets.only(
                  left: d.pageInsets.left,
                  right: d.pageInsets.right,
                  top: 4,
                  bottom: 4,
                ),
                child: Row(
                  children: [
                    for (var r = 0; r < parts; r++) ...[
                      if (r > 0) const SizedBox(width: 8),
                      _chip(r, current, label),
                    ],
                  ],
                ),
              ),
            );
          },
        ),
      ),
    );
  }

  Widget _chip(int r, int current, (String, String) Function(int, int) label) {
    final first = r * episodeRangeSize;
    final last = math.min(widget.count, first + episodeRangeSize) - 1;
    final (text, semantics) = label(first, last);
    return ShowChip(
      key: episodeRangeKey(first + 1),
      focusNode: _rangeNode(r),
      label: text,
      semanticLabel: semantics,
      selected: r == current,
      compact: true,
      onTap: () => _jump(first),
    );
  }
}

/// A round ‹ or › at an end of the row (desktop): out of the Tab order, the
/// arrow keys do its work.
class _PageButton extends StatelessWidget {
  const _PageButton({
    super.key,
    required this.visible,
    required this.icon,
    required this.label,
    required this.onPressed,
  });

  static const size = 44.0;

  final bool visible;
  final IconData icon;
  final String label;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => IgnorePointer(
    ignoring: !visible,
    child: AnimatedOpacity(
      opacity: visible ? 1 : 0,
      duration: KMotion.focus,
      child: ExcludeSemantics(
        excluding: !visible,
        child: ExcludeFocus(
          child: IconButton(
            tooltip: label,
            onPressed: onPressed,
            iconSize: 28,
            color: KColors.textPrimary,
            style: IconButton.styleFrom(
              fixedSize: const Size.square(size),
              minimumSize: const Size.square(size),
              backgroundColor: KColors.bgElevated2.withValues(alpha: .92),
              hoverColor: KColors.bgElevated,
              side: BorderSide(color: KColors.stroke),
              shape: const CircleBorder(),
              elevation: 2,
              shadowColor: Colors.black,
            ),
            icon: Icon(icon),
          ),
        ),
      ),
    ),
  );
}

/// A controller whose first offset is computed when the list attaches.
class _OpeningController extends ScrollController {
  _OpeningController(this._initial);

  final double Function() _initial;

  @override
  double get initialScrollOffset => _initial();
}

/// Swipes on a phone settle on a card: the fling's end, rounded to the
/// nearest card.
class _SnapPhysics extends ScrollPhysics {
  const _SnapPhysics({required this.extent, super.parent});

  final double extent;

  @override
  _SnapPhysics applyTo(ScrollPhysics? ancestor) =>
      _SnapPhysics(extent: extent, parent: buildParent(ancestor));

  @override
  Simulation? createBallisticSimulation(
    ScrollMetrics position,
    double velocity,
  ) {
    if (position.outOfRange || extent <= 0) {
      return super.createBallisticSimulation(position, velocity);
    }
    final tolerance = toleranceFor(position);
    final free = super.createBallisticSimulation(position, velocity);
    final end = free?.x(double.infinity) ?? position.pixels;
    final target = ((end / extent).roundToDouble() * extent)
        .clamp(position.minScrollExtent, position.maxScrollExtent)
        .toDouble();
    if ((target - position.pixels).abs() < tolerance.distance &&
        velocity.abs() < tolerance.velocity) {
      return null;
    }
    return ScrollSpringSimulation(
      spring,
      position.pixels,
      target,
      velocity,
      tolerance: tolerance,
    );
  }

  @override
  bool get allowImplicitScrolling => false;
}

/// A pill: the season tabs and the range chips of the show page.
class ShowChip extends StatelessWidget {
  const ShowChip({
    super.key,
    required this.label,
    required this.selected,
    required this.onTap,
    this.onMenu,
    this.focusNode,
    this.semanticLabel,
    this.compact = false,
  });

  final String label;
  final bool selected;
  final VoidCallback onTap;

  /// UI-049: the watched marks of the season.
  final ValueChanged<Offset>? onMenu;
  final FocusNode? focusNode;
  final String? semanticLabel;

  /// Smaller (range chips under the season tabs).
  final bool compact;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    return KFocusable(
      focusNode: focusNode,
      onActivate: onTap,
      onContextMenu: onMenu,
      radius: 999,
      light: false,
      focusScale: d.focusScaleButton,
      semanticLabel: semanticLabel ?? label,
      selected: selected,
      builder: (context, focused) {
        final on = focused || selected;
        return AnimatedContainer(
          duration: KMotion.focus,
          padding: EdgeInsets.symmetric(
            horizontal: compact ? 12 : 16,
            vertical: compact ? 6 : 8,
          ),
          decoration: BoxDecoration(
            color: focused
                ? Colors.white
                : (selected
                      ? Colors.white.withValues(alpha: .22)
                      : Colors.white.withValues(alpha: .07)),
            borderRadius: BorderRadius.circular(999),
          ),
          child: ExcludeSemantics(
            excluding: semanticLabel != null,
            child: Text(
              label,
              style:
                  (compact
                          ? theme.textTheme.labelMedium
                          : theme.textTheme.labelLarge)
                      ?.copyWith(
                        color: focused
                            ? Colors.black
                            : (on ? Colors.white : KColors.textSecondary),
                      ),
            ),
          ),
        );
      },
    );
  }
}
