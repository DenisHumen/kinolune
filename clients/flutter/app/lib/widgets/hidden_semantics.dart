// SPDX-License-Identifier: MPL-2.0
//
// BUG-015: a page that stays built but out of the semantics tree (an
// inactive section of the shell's IndexedStack, the shell under the
// fullscreen player) keeps the semantics caches of the moment it was hidden.
// Flutter 3.50 reads one of them, the nearest ancestor with clean geometry,
// without refreshing it (flutter/flutter#195316, fixed on master by
// 1c858ec59d, not yet in a stable release). When the hidden page changes —
// a library stream updates a row, a banner resizes the navigator — that stale
// cache points at a node whose geometry was cleared, and `flushSemantics`
// throws `Null check operator used on a null value` in
// `_updateSemanticsNodeGeometry`. On Linux and Windows the accessibility
// bridge keeps semantics on, so it hit every live run, and screen reader
// users lost the semantics update of that frame.

import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

/// Drops the semantics caches of [child] once it is hidden, so nothing in
/// the hidden page has a clean geometry for a stale cache to point at.
///
/// Hidden is [hidden] (an inactive section) or a disabled [TickerMode]: the
/// page sits in a route under an opaque one (the fullscreen player) or under
/// App Lock, both out of the semantics tree.
///
/// While hidden the page stays out of the semantics tree; when it is shown
/// again Flutter builds its semantics afresh, as for a new page. Remove this
/// once the pinned Flutter has the fix of flutter/flutter#195316.
class HiddenSemantics extends StatefulWidget {
  const HiddenSemantics({super.key, required this.hidden, required this.child});

  /// The parent no longer shows [child] to the semantics tree.
  final bool hidden;
  final Widget child;

  @override
  State<HiddenSemantics> createState() => _HiddenSemanticsState();
}

class _HiddenSemanticsState extends State<HiddenSemantics> {
  bool _tickers = true;
  bool _wasHidden = false;

  bool get _hidden => widget.hidden || !_tickers;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _tickers = TickerMode.valuesOf(context).enabled;
    _check();
  }

  @override
  void didUpdateWidget(HiddenSemantics old) {
    super.didUpdateWidget(old);
    _check();
  }

  void _check() {
    if (_hidden && !_wasHidden) _resetAfterFrame();
    _wasHidden = _hidden;
  }

  /// After the frame in which the parent stopped showing the page, so the
  /// semantics tree no longer refers to anything in it.
  void _resetAfterFrame() {
    SchedulerBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_hidden) return;
      final RenderObject? page = context.findRenderObject();
      if (page == null || !page.attached) return;
      page.clearSemantics();
      // The parent re-adopts the cleared page in its next semantics update,
      // still hidden; when the page is shown again Flutter updates it from
      // there as usual.
      page.parent?.markNeedsSemanticsUpdate();
    });
    SchedulerBinding.instance.ensureVisualUpdate();
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
