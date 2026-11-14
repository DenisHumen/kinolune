// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/material.dart';

import '../trickplay.dart';
import '../video/osd_model.dart';
import 'tokens.dart';

/// Thin progress bar that grows on hover/focus/drag. Tap or drag to seek.
/// Keyboard/D-pad seeking is handled by the owning screen while this is
/// focused (left/right ±10 s).
///
/// With [trickplay] (PLAY-009), dragging or hovering shows the preview of
/// the position under the pointer above the bar.
///
/// With [chapters] (UI-013), the bar carries a tick at the start of each
/// chapter and the preview names the chapter under the pointer.
class Scrubber extends StatefulWidget {
  const Scrubber({
    super.key,
    required this.position,
    required this.duration,
    this.buffer = Duration.zero,
    required this.onSeek,
    this.focusNode,
    this.height = 6,
    this.showTimes = true,
    this.endLabel,
    this.trickplay,
    this.chapters = const [],
    this.chapterName,
  });

  final Duration position;
  final Duration duration;
  final Duration buffer;
  final ValueChanged<Duration> onSeek;
  final FocusNode? focusNode;
  final double height;
  final bool showTimes;

  /// Overrides the right-hand label (default: "-remaining").
  final String? endLabel;

  /// Seek previews of the file; null: none.
  final TrickplaySource? trickplay;

  /// Chapter markers, sorted by start.
  final List<PlaybackChapter> chapters;

  /// Name of chapter `i` for the preview; default: its title.
  final String Function(int index)? chapterName;

  @override
  State<Scrubber> createState() => _ScrubberState();
}

class _ScrubberState extends State<Scrubber> {
  double? _drag; // 0..1 while dragging
  double? _hover; // 0..1 while the mouse is over the bar
  bool _focused = false;
  bool _hovered = false;
  TrickplayLoader? _loader;

  @override
  void initState() {
    super.initState();
    _attach();
  }

  @override
  void didUpdateWidget(Scrubber old) {
    super.didUpdateWidget(old);
    if (!identical(old.trickplay, widget.trickplay)) {
      _loader?.dispose();
      _loader = null;
      _attach();
    }
  }

  @override
  void dispose() {
    _loader?.dispose();
    super.dispose();
  }

  void _attach() {
    final source = widget.trickplay;
    if (source != null) _loader = TrickplayLoader(source);
  }

  /// The position under the pointer, while dragging or hovering.
  double? get _pointer => _drag ?? _hover;

  void _preview() {
    final f = _pointer;
    if (f != null && widget.duration > Duration.zero) {
      _loader?.request(_at(f));
    }
  }

  double get _fraction {
    final d = widget.duration.inMilliseconds;
    if (d <= 0) return 0;
    return (widget.position.inMilliseconds / d).clamp(0.0, 1.0);
  }

  double get _bufferFraction {
    final d = widget.duration.inMilliseconds;
    if (d <= 0) return 0;
    return (widget.buffer.inMilliseconds / d).clamp(0.0, 1.0);
  }

  Duration _at(double f) => Duration(
    milliseconds: (widget.duration.inMilliseconds * f.clamp(0.0, 1.0)).round(),
  );

  void _update(Offset local, double width) {
    setState(() => _drag = (local.dx / width).clamp(0.0, 1.0));
    _preview();
  }

  /// The preview bubble has something to show: a picture or a chapter.
  bool get _previews => _loader != null || widget.chapters.isNotEmpty;

  /// "12:34 · Chapter title" for the preview at [at].
  String _bubbleLabel(Duration at) {
    final time = formatDuration(at);
    final i = chapterIndexAt(widget.chapters, at);
    if (i < 0) return time;
    final name = widget.chapterName?.call(i) ?? widget.chapters[i].title;
    return name == null || name.isEmpty ? time : '$time · $name';
  }

  void _hoverAt(Offset local, double width) {
    if (!_previews) return;
    setState(() => _hover = (local.dx / width).clamp(0.0, 1.0));
    _preview();
  }

  @override
  Widget build(BuildContext context) {
    final f = _drag ?? _fraction;
    final active = _focused || _hovered || _drag != null;
    final shown = _drag != null ? _at(_drag!) : widget.position;
    final remaining = widget.duration - shown;
    final bar = LayoutBuilder(
      builder: (context, c) {
        final w = c.maxWidth;
        final h = active ? widget.height * 1.6 : widget.height;
        final gesture = GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTapUp: (d) {
            widget.onSeek(_at(d.localPosition.dx / w));
          },
          onHorizontalDragStart: (d) => _update(d.localPosition, w),
          onHorizontalDragUpdate: (d) => _update(d.localPosition, w),
          onHorizontalDragEnd: (_) {
            final v = _drag;
            setState(() => _drag = null);
            if (v != null) widget.onSeek(_at(v));
          },
          onHorizontalDragCancel: () => setState(() => _drag = null),
          child: SizedBox(
            height: 28,
            child: Center(
              child: AnimatedContainer(
                duration: const Duration(milliseconds: 120),
                width: w,
                height: h,
                decoration: BoxDecoration(
                  color: PaTokens.track,
                  borderRadius: BorderRadius.circular(h),
                  border: _focused
                      ? Border.all(color: PaTokens.focusRing, width: 1.5)
                      : null,
                ),
                child: Stack(
                  fit: StackFit.expand,
                  children: [
                    FractionallySizedBox(
                      alignment: Alignment.centerLeft,
                      widthFactor: _bufferFraction,
                      child: Container(
                        decoration: BoxDecoration(
                          color: const Color(0x40FFFFFF),
                          borderRadius: BorderRadius.circular(h),
                        ),
                      ),
                    ),
                    FractionallySizedBox(
                      alignment: Alignment.centerLeft,
                      widthFactor: f,
                      child: Container(
                        decoration: BoxDecoration(
                          color: PaTokens.text,
                          borderRadius: BorderRadius.circular(h),
                        ),
                      ),
                    ),
                    if (widget.duration > Duration.zero)
                      for (final c in widget.chapters)
                        if (c.start > Duration.zero &&
                            c.start < widget.duration)
                          Positioned(
                            key: const ValueKey('chapter-marker'),
                            left:
                                w *
                                    c.start.inMilliseconds /
                                    widget.duration.inMilliseconds -
                                1.5,
                            top: 0,
                            bottom: 0,
                            width: 3,
                            child: const ColoredBox(
                              color: PaTokens.chapterMark,
                            ),
                          ),
                  ],
                ),
              ),
            ),
          ),
        );
        final loader = _loader;
        final pointer = _pointer;
        final barRegion = MouseRegion(
          onHover: (e) => _hoverAt(e.localPosition, w),
          onExit: (_) => setState(() => _hover = null),
          child: gesture,
        );
        // Always a Stack: switching the tree would restart a drag.
        return Stack(
          clipBehavior: Clip.none,
          children: [
            barRegion,
            if (_previews && pointer != null)
              Positioned(
                bottom: 32,
                left: 0,
                right: 0,
                child: IgnorePointer(
                  child: _TrickplayBubble(
                    loader: loader,
                    fraction: pointer,
                    barWidth: w,
                    label: _bubbleLabel(_at(pointer)),
                  ),
                ),
              ),
          ],
        );
      },
    );
    return Focus(
      focusNode: widget.focusNode,
      onFocusChange: (v) => setState(() => _focused = v),
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        onEnter: (_) => setState(() => _hovered = true),
        onExit: (_) => setState(() => _hovered = false),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            bar,
            if (widget.showTimes)
              Row(
                children: [
                  Text(formatDuration(shown), style: _timeStyle),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Text(
                      widget.endLabel ?? '-${formatDuration(remaining)}',
                      textAlign: TextAlign.end,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: _timeStyle,
                    ),
                  ),
                ],
              ),
          ],
        ),
      ),
    );
  }
}

/// The preview above the bar at [fraction] of its width, kept inside it.
class _TrickplayBubble extends StatelessWidget {
  const _TrickplayBubble({
    required this.loader,
    required this.fraction,
    required this.barWidth,
    required this.label,
  });

  final TrickplayLoader? loader;
  final double fraction;
  final double barWidth;
  final String label;

  @override
  Widget build(BuildContext context) {
    final width = (barWidth * 0.28).clamp(160.0, 240.0);
    final loader = this.loader;
    return ListenableBuilder(
      listenable: loader ?? const _NoListenable(),
      builder: (context, _) {
        final frame = loader?.frame;
        final image = loader?.image;
        final crop = frame?.crop;
        final aspect = crop != null && crop.height > 0
            ? crop.width / crop.height
            : image != null && image.height > 0
            ? image.width / image.height
            : 16 / 9;
        final left = (barWidth * fraction - width / 2).clamp(
          0.0,
          (barWidth - width).clamp(0.0, double.infinity),
        );
        return Align(
          alignment: Alignment.bottomLeft,
          child: Padding(
            padding: EdgeInsets.only(left: left),
            child: Column(
              key: const ValueKey('trickplay-preview'),
              mainAxisSize: MainAxisSize.min,
              children: [
                if (frame != null && image != null)
                  Container(
                    width: width,
                    height: width / aspect,
                    decoration: BoxDecoration(
                      border: Border.all(color: PaTokens.text, width: 1.5),
                      borderRadius: BorderRadius.circular(6),
                    ),
                    clipBehavior: Clip.antiAlias,
                    child: TrickplayImage(image: image, frame: frame),
                  ),
                const SizedBox(height: 4),
                SizedBox(
                  width: width,
                  child: Text(
                    label,
                    key: const ValueKey('scrubber-preview-label'),
                    textAlign: TextAlign.center,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: _timeStyle.copyWith(
                      color: PaTokens.text,
                      shadows: const [
                        Shadow(color: Color(0xCC000000), blurRadius: 4),
                      ],
                    ),
                  ),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}

/// A listenable that never fires (a bubble without trickplay).
class _NoListenable implements Listenable {
  const _NoListenable();

  @override
  void addListener(VoidCallback listener) {}

  @override
  void removeListener(VoidCallback listener) {}
}

const _timeStyle = TextStyle(
  color: PaTokens.textSecondary,
  fontSize: 14,
  fontFeatures: [FontFeature.tabularFigures()],
);
