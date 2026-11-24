// SPDX-License-Identifier: MPL-2.0
//
// The card of an in-app notice (UI-026, ADR-NNNN) over the whole app: in
// the top right corner on TV and desktop, across the top on phones. It never
// takes focus by itself. Up from the top edge of a screen (or F6) moves the
// remote's focus onto its buttons, Down or Back returns it where it was;
// Back on the card closes it. It hides by itself after a few seconds, but
// not while it has the focus or the pointer. Reduce Motion drops the slide;
// screen readers hear it through a live region (Android) or an
// announcement (elsewhere), and get more time.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../platform/form_factor.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import 'in_app_notices.dart';

/// Wraps the app (inside `MaterialApp.builder`) and shows [notices] over
/// [child].
class InAppNoticeHost extends StatefulWidget {
  const InAppNoticeHost({
    super.key,
    required this.notices,
    required this.navigatorKey,
    required this.child,
    this.held,
  });

  final InAppNotices notices;

  /// While true no card shows and none hides by itself: they wait for it
  /// to turn false (a video player is open, BUG-016).
  final ValueListenable<bool>? held;

  /// The app's navigator: button actions run in its context.
  final GlobalKey<NavigatorState> navigatorKey;
  final Widget child;

  /// With a screen reader on, cards stay this many times longer.
  static const accessibleFactor = 3;

  @override
  State<InAppNoticeHost> createState() => _InAppNoticeHostState();
}

class _InAppNoticeHostState extends State<InAppNoticeHost> {
  Timer? _timer;
  InAppNotice? _timed;
  int _serial = -1;
  GlobalKey<_NoticeCardState> _cardKey = GlobalKey();
  bool _hovered = false;

  /// Focus on the card's buttons.
  bool _focused = false;

  /// Where the focus was before it moved onto the card.
  FocusNode? _returnTo;

  InAppNotices get _notices => widget.notices;

  bool get _held => widget.held?.value ?? false;

  /// The current notice was announced (not while [_held]).
  int _announced = -1;

  @override
  void initState() {
    super.initState();
    _notices.addListener(_onChange);
    widget.held?.addListener(_onChange);
    WidgetsBinding.instance.addPostFrameCallback((_) => _onChange());
  }

  @override
  void didUpdateWidget(InAppNoticeHost old) {
    super.didUpdateWidget(old);
    if (old.notices != widget.notices || old.held != widget.held) {
      old.notices.removeListener(_onChange);
      old.held?.removeListener(_onChange);
      widget.notices.addListener(_onChange);
      widget.held?.addListener(_onChange);
      _onChange();
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    _notices.removeListener(_onChange);
    widget.held?.removeListener(_onChange);
    super.dispose();
  }

  void _onChange() {
    if (!mounted) return;
    final n = _notices.current;
    if (_notices.serial != _serial) {
      _serial = _notices.serial;
      // The card the focus was on is going away: back to the page.
      if (_focused) _restoreFocus();
      _hovered = false;
      _cardKey = GlobalKey();
    }
    // A held card is announced when it shows.
    if (n != null && !_held && _announced != _serial) {
      _announced = _serial;
      _announce(n);
    }
    _restartTimer();
    setState(() {});
  }

  Duration _durationOf(InAppNotice n) {
    final reader = MediaQuery.maybeAccessibleNavigationOf(context) ?? false;
    return reader ? n.duration * InAppNoticeHost.accessibleFactor : n.duration;
  }

  /// Runs the hide timer for the current notice unless it is held (focus,
  /// pointer); a new text of the same notice starts it again.
  void _restartTimer() {
    final n = _held ? null : _notices.current;
    final held = _focused || _hovered;
    if (n == null || held) {
      _timer?.cancel();
      _timer = null;
      _timed = held ? n : null;
      return;
    }
    if (identical(n, _timed) && _timer != null) return;
    _timer?.cancel();
    _timed = n;
    _timer = Timer(_durationOf(n), () {
      _timer = null;
      if (identical(_notices.current, n)) _notices.expire();
    });
  }

  void _announce(InAppNotice n) {
    // Android reads the card's live region; elsewhere it is announced.
    if (defaultTargetPlatform == TargetPlatform.android) return;
    final l = lookupL10n(context);
    if (l == null) return;
    final text = [n.title(l), if (n.body case final b?) b(l)].join('. ');
    unawaited(
      SemanticsService.sendAnnouncement(
        View.of(context),
        text,
        Directionality.maybeOf(context) ?? TextDirection.ltr,
      ),
    );
  }

  // ---------------------------------------------------------------- focus

  void _enterCard() {
    final card = _cardKey.currentState;
    if (card == null) return;
    final now = FocusManager.instance.primaryFocus;
    if (now != null && !card.contains(now)) _returnTo = now;
    card.focusFirst();
  }

  void _restoreFocus() {
    final back = _returnTo;
    _returnTo = null;
    _focused = false;
    if (back != null && back.context != null && back.canRequestFocus) {
      back.requestFocus();
    } else {
      // The screen it was on has closed: the page under the card.
      FocusManager.instance.primaryFocus?.unfocus();
    }
  }

  void _onCardFocus(bool focused) {
    if (_focused == focused) return;
    _focused = focused;
    _restartTimer();
  }

  void _onHover(bool hovered) {
    if (_hovered == hovered) return;
    _hovered = hovered;
    _restartTimer();
  }

  /// Keys of the page under the card: Up at the top edge (and F6) move the
  /// focus onto the card.
  KeyEventResult _onPageKey(FocusNode _, KeyEvent e) {
    if (_notices.current == null || _held || _focused) {
      return KeyEventResult.ignored;
    }
    if (e is! KeyDownEvent) return KeyEventResult.ignored;
    if (e.logicalKey == LogicalKeyboardKey.f6) {
      _enterCard();
      return KeyEventResult.handled;
    }
    if (e.logicalKey != LogicalKeyboardKey.arrowUp) {
      return KeyEventResult.ignored;
    }
    final before = FocusManager.instance.primaryFocus;
    final ctx = before?.context;
    if (before == null || ctx == null) return KeyEventResult.ignored;
    // Text fields keep their arrow keys.
    if (ctx.widget is EditableText ||
        ctx.findAncestorStateOfType<EditableTextState>() != null) {
      return KeyEventResult.ignored;
    }
    Actions.maybeInvoke(
      ctx,
      const DirectionalFocusIntent(TraversalDirection.up),
    );
    if (identical(FocusManager.instance.primaryFocus, before)) _enterCard();
    return KeyEventResult.handled;
  }

  // -------------------------------------------------------------- actions

  void _dismiss() {
    if (_focused) _restoreFocus();
    _notices.dismiss();
  }

  void _activate() {
    if (_focused) _restoreFocus();
    final ctx = widget.navigatorKey.currentContext ?? context;
    unawaited(_notices.activate(ctx));
  }

  @override
  Widget build(BuildContext context) {
    final n = _held ? null : _notices.current;
    final reduce = MediaQuery.maybeDisableAnimationsOf(context) ?? false;
    final ff = FormFactorScope.of(context);
    final width = MediaQuery.sizeOf(context).width;
    final compact = ff == FormFactor.phone || width < 600;
    final tv = ff == FormFactor.tv;
    final serial = _serial;
    final switcher = AnimatedSwitcher(
      duration: reduce ? Duration.zero : KMotion.focus * 2,
      reverseDuration: reduce ? Duration.zero : KMotion.focus,
      switchInCurve: Curves.easeOutCubic,
      switchOutCurve: Curves.easeInCubic,
      layoutBuilder: (current, previous) => Stack(
        alignment: compact ? Alignment.topCenter : Alignment.topRight,
        children: [...previous, ?current],
      ),
      transitionBuilder: (child, animation) {
        final leaving = child.key != ValueKey(serial);
        final slide = Tween<Offset>(
          begin: compact ? const Offset(0, -.6) : const Offset(.25, 0),
          end: Offset.zero,
        ).animate(animation);
        Widget out = FadeTransition(
          opacity: animation,
          child: SlideTransition(position: slide, child: child),
        );
        if (leaving) {
          out = IgnorePointer(
            child: ExcludeFocus(child: ExcludeSemantics(child: out)),
          );
        }
        return out;
      },
      child: n == null
          ? const SizedBox.shrink(key: ValueKey(-1))
          : KeyedSubtree(
              key: ValueKey(serial),
              child: _NoticeCard(
                key: _cardKey,
                notice: n,
                more: _notices.queued.length,
                compact: compact,
                tv: tv,
                onActivate: _activate,
                onDismiss: _dismiss,
                onFocusChange: _onCardFocus,
                onHover: _onHover,
                onLeave: _restoreFocus,
              ),
            ),
    );
    final Widget placed = compact
        ? Positioned(
            top: 0,
            left: 0,
            right: 0,
            child: SafeArea(
              bottom: false,
              minimum: const EdgeInsets.all(8),
              child: Center(
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 560),
                  child: switcher,
                ),
              ),
            ),
          )
        : Positioned(
            top: tv ? 32 : 16,
            right: tv ? 48 : 16,
            child: SafeArea(
              left: false,
              bottom: false,
              child: SizedBox(
                width: (tv ? 460.0 : 380.0).clamp(0, width - 32),
                child: switcher,
              ),
            ),
          );
    return Focus(
      canRequestFocus: false,
      skipTraversal: true,
      onKeyEvent: _onPageKey,
      child: Stack(
        children: [
          Positioned.fill(child: widget.child),
          placed,
        ],
      ),
    );
  }
}

/// The l10n of [context], if the app's localizations are loaded.
AppLocalizations? lookupL10n(BuildContext context) =>
    Localizations.of<AppLocalizations>(context, AppLocalizations);

class _NoticeCard extends StatefulWidget {
  const _NoticeCard({
    super.key,
    required this.notice,
    required this.more,
    required this.compact,
    required this.tv,
    required this.onActivate,
    required this.onDismiss,
    required this.onFocusChange,
    required this.onHover,
    required this.onLeave,
  });

  final InAppNotice notice;

  /// Notices waiting after this one.
  final int more;
  final bool compact;
  final bool tv;
  final VoidCallback onActivate;
  final VoidCallback onDismiss;
  final ValueChanged<bool> onFocusChange;
  final ValueChanged<bool> onHover;

  /// Down on the card: the focus goes back to the page.
  final VoidCallback onLeave;

  @override
  State<_NoticeCard> createState() => _NoticeCardState();
}

class _NoticeCardState extends State<_NoticeCard> {
  final _scope = FocusScopeNode(debugLabel: 'in-app notice');
  final _action = FocusNode(debugLabel: 'notice action');
  final _close = FocusNode(debugLabel: 'notice dismiss');
  double _drag = 0;

  bool contains(FocusNode node) =>
      node == _scope || node.ancestors.contains(_scope);

  void focusFirst() =>
      (widget.notice.onAction != null ? _action : _close).requestFocus();

  @override
  void dispose() {
    _scope.dispose();
    _action.dispose();
    _close.dispose();
    super.dispose();
  }

  KeyEventResult _onKey(FocusNode _, KeyEvent e) {
    if (e is! KeyDownEvent && e is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    final k = e.logicalKey;
    if (k == LogicalKeyboardKey.escape ||
        k == LogicalKeyboardKey.goBack ||
        k == LogicalKeyboardKey.browserBack) {
      if (e is KeyDownEvent) widget.onDismiss();
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.arrowDown) {
      if (e is KeyDownEvent) widget.onLeave();
      return KeyEventResult.handled;
    }
    // Nothing above the card.
    if (k == LogicalKeyboardKey.arrowUp) return KeyEventResult.handled;
    return KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final n = widget.notice;
    final title = n.title(l);
    final body = n.body?.call(l);
    final pad = widget.tv ? 18.0 : 14.0;
    final buttons = Wrap(
      alignment: WrapAlignment.end,
      spacing: 8,
      runSpacing: 8,
      children: [
        KButton(
          key: const ValueKey('notice-dismiss'),
          focusNode: _close,
          label: n.dismissLabel?.call(l) ?? l.commonDismiss,
          onPressed: widget.onDismiss,
        ),
        if (n.onAction != null)
          KButton(
            key: const ValueKey('notice-action'),
            focusNode: _action,
            primary: true,
            label: n.actionLabel?.call(l) ?? l.noticeOpen,
            onPressed: widget.onActivate,
          ),
      ],
    );
    final card = DecoratedBox(
      decoration: BoxDecoration(
        color: KColors.bgElevated2,
        borderRadius: BorderRadius.circular(widget.tv ? 20 : 16),
        border: Border.all(color: KColors.stroke),
        boxShadow: const [
          BoxShadow(
            color: Color(0x99000000),
            blurRadius: 24,
            offset: Offset(0, 8),
          ),
        ],
      ),
      child: Padding(
        padding: EdgeInsets.all(pad),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                _KindIcon(kind: n.kind, tv: widget.tv),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        title,
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                        style: theme.textTheme.titleMedium?.copyWith(
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                      if (body != null) ...[
                        const SizedBox(height: 2),
                        Text(
                          body,
                          maxLines: 3,
                          overflow: TextOverflow.ellipsis,
                          style: theme.textTheme.bodyMedium?.copyWith(
                            color: KColors.textSecondary,
                          ),
                        ),
                      ],
                      if (widget.more > 0) ...[
                        const SizedBox(height: 4),
                        Text(
                          l.noticeMore(widget.more),
                          style: theme.textTheme.labelSmall?.copyWith(
                            color: KColors.textTertiary,
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
              ],
            ),
            SizedBox(height: widget.tv ? 14 : 10),
            buttons,
          ],
        ),
      ),
    );
    Widget out = Semantics(
      key: const ValueKey('in-app-notice'),
      container: true,
      liveRegion: true,
      explicitChildNodes: true,
      onDismiss: widget.onDismiss,
      child: Material(type: MaterialType.transparency, child: card),
    );
    if (widget.compact) {
      // Swipe up to close, like a system notification.
      out = GestureDetector(
        onVerticalDragUpdate: (d) => _drag += d.delta.dy,
        onVerticalDragEnd: (d) {
          final up = _drag < -24 || (d.primaryVelocity ?? 0) < -300;
          _drag = 0;
          if (up) widget.onDismiss();
        },
        child: out,
      );
    }
    return MouseRegion(
      onEnter: (_) => widget.onHover(true),
      onExit: (_) => widget.onHover(false),
      child: FocusScope(
        node: _scope,
        onFocusChange: widget.onFocusChange,
        onKeyEvent: _onKey,
        child: FocusTraversalGroup(child: out),
      ),
    );
  }
}

class _KindIcon extends StatelessWidget {
  const _KindIcon({required this.kind, required this.tv});

  final InAppNoticeKind kind;
  final bool tv;

  @override
  Widget build(BuildContext context) {
    final (icon, color) = switch (kind) {
      InAppNoticeKind.library => (Icons.video_library_outlined, KColors.accent),
      InAppNoticeKind.handoff => (Icons.devices, KColors.accentSoft),
      InAppNoticeKind.download => (
        Icons.download_done_rounded,
        KColors.accentSoft,
      ),
      InAppNoticeKind.live => (Icons.sensors, KColors.live),
      InAppNoticeKind.update => (Icons.system_update_alt, KColors.accentSoft),
      InAppNoticeKind.remoteInput => (Icons.phonelink, KColors.accent),
    };
    final size = tv ? 44.0 : 38.0;
    return ExcludeSemantics(
      child: Container(
        width: size,
        height: size,
        decoration: BoxDecoration(
          color: color.withValues(alpha: .16),
          borderRadius: BorderRadius.circular(size / 3.5),
        ),
        child: Icon(icon, color: color, size: size * .58),
      ),
    );
  }
}
