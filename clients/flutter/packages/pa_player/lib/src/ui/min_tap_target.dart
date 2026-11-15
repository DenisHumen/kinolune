// SPDX-License-Identifier: MPL-2.0
import 'dart:math' as math;

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

/// Makes the touch and screen-reader target of [child] at least [minSize]
/// (48×48 dp, the Android and WCAG 2.5.8 minimum; iOS asks for 44 pt)
/// without changing the layout or the look (UI-021).
///
/// The extra area around a smaller child is centred on it: taps there go to
/// the child, and the semantics node (what TalkBack and VoiceOver focus and
/// activate) covers it too. Material's `MaterialTapTargetSize.padded` does
/// the same by growing the layout, which would move the design's pixels.
class MinTapTarget extends SingleChildRenderObjectWidget {
  const MinTapTarget({
    super.key,
    this.minSize = const Size(48, 48),
    this.enabled = true,
    required Widget super.child,
  });

  final Size minSize;

  /// Off: a plain pass-through (e.g. on desktop and TV, where targets are
  /// reached with a pointer or focus).
  final bool enabled;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      RenderMinTapTarget(minSize: minSize, enabled: enabled);

  @override
  void updateRenderObject(
    BuildContext context,
    RenderMinTapTarget renderObject,
  ) {
    renderObject
      ..minSize = minSize
      ..enabled = enabled;
  }
}

class RenderMinTapTarget extends RenderProxyBox {
  RenderMinTapTarget({required Size minSize, required bool enabled})
    : _minSize = minSize,
      _enabled = enabled;

  Size get minSize => _minSize;
  Size _minSize;
  set minSize(Size v) {
    if (v == _minSize) return;
    _minSize = v;
    markNeedsSemanticsUpdate();
  }

  bool get enabled => _enabled;
  bool _enabled;
  set enabled(bool v) {
    if (v == _enabled) return;
    _enabled = v;
    markNeedsSemanticsUpdate();
  }

  /// The target in local coordinates: the child's box grown to [minSize].
  Rect get target {
    final s = size;
    final dx = math.max(0.0, (_minSize.width - s.width) / 2);
    final dy = math.max(0.0, (_minSize.height - s.height) / 2);
    return Rect.fromLTRB(-dx, -dy, s.width + dx, s.height + dy);
  }

  bool get _grows =>
      _enabled &&
      hasSize &&
      (size.width < _minSize.width || size.height < _minSize.height);

  @override
  Rect get semanticBounds => _grows ? target : super.semanticBounds;

  @override
  void describeSemanticsConfiguration(SemanticsConfiguration config) {
    super.describeSemanticsConfiguration(config);
    // Own node, so its rectangle is [semanticBounds] and the child's label
    // and actions merge into it.
    if (_grows) config.isSemanticBoundary = true;
  }

  @override
  bool hitTest(BoxHitTestResult result, {required Offset position}) {
    if (!_grows || size.contains(position)) {
      return super.hitTest(result, position: position);
    }
    if (!target.contains(position)) return false;
    // A tap in the extra area counts as a tap on the nearest point of the
    // child.
    final inside = Offset(
      position.dx.clamp(0.0, math.max(0.0, size.width - .01)),
      position.dy.clamp(0.0, math.max(0.0, size.height - .01)),
    );
    if (hitTestChildren(result, position: inside)) {
      result.add(BoxHitTestEntry(this, position));
      return true;
    }
    return false;
  }
}
