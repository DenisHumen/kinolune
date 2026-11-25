// SPDX-License-Identifier: MPL-2.0
//
// The focus engine visual (01 §4.2): a focused element scales up (posters
// ~1.10, cards ~1.08 on TV), gets a soft glow and drop shadow and a moving
// specular "light" (parallax-light). Desktop hover uses a gentler version,
// phones get a press effect (scale 0.97) instead.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart' show MinTapTarget;

import '../theme/kinolune_theme.dart';

class KFocusable extends StatefulWidget {
  const KFocusable({
    super.key,
    required this.builder,
    this.onActivate,
    this.onFocusChange,
    this.focusScale,
    this.radius,
    this.autofocus = false,
    this.focusNode,
    this.semanticLabel,
    this.light = true,
    this.onContextMenu,
    this.selected,
    this.focusable = true,
  });

  final Widget Function(BuildContext context, bool focused) builder;
  final VoidCallback? onActivate;

  /// Long press, right click or the menu key (UI-008), or holding OK /
  /// Select on a remote (TV-002, QA-015); the global position of the
  /// pointer, or the centre of the element for a key.
  final ValueChanged<Offset>? onContextMenu;
  final ValueChanged<bool>? onFocusChange;
  final double? focusScale;
  final double? radius;
  final bool autofocus;
  final FocusNode? focusNode;
  final String? semanticLabel;

  /// Draw the specular light overlay (cards/posters; off for text buttons).
  final bool light;

  /// Selected state for screen readers (a navigation item, a chosen chip);
  /// null when the element is not selectable.
  final bool? selected;

  /// False for a row that only holds other focusable controls: the row
  /// itself is then no Tab stop (UI-021: Shift+Tab got stuck on a row
  /// around its own buttons).
  final bool focusable;

  @override
  State<KFocusable> createState() => _KFocusableState();
}

class _KFocusableState extends State<KFocusable>
    with SingleTickerProviderStateMixin {
  bool _focused = false;
  bool _hovered = false;
  bool _pressed = false;
  Offset _pointer = const Offset(.3, .2);

  /// Held OK / Select (QA-015): a short press activates on release, a long
  /// one opens the context menu.
  Timer? _hold;
  bool _held = false;
  late final AnimationController _breath = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 3200),
  );

  @override
  void dispose() {
    _hold?.cancel();
    _breath.dispose();
    super.dispose();
  }

  void _openContextMenuAtCentre() {
    final box = context.findRenderObject() as RenderBox?;
    if (widget.onContextMenu != null && box != null && box.hasSize) {
      widget.onContextMenu!(box.localToGlobal(box.size.center(Offset.zero)));
    }
  }

  /// OK / Select / Enter / A on an element with a context menu: activates
  /// on release, or opens the menu once held for [kLongSelect]. Other keys
  /// go on to the shortcuts.
  KeyEventResult _onKey(FocusNode node, KeyEvent e) {
    if (widget.onContextMenu == null ||
        !widget.focusable ||
        !_activateKeys.contains(e.logicalKey)) {
      return KeyEventResult.ignored;
    }
    switch (e) {
      case KeyDownEvent():
        _hold?.cancel();
        _held = false;
        _hold = Timer(kLongSelect, () {
          _hold = null;
          _held = true;
          if (mounted) _openContextMenuAtCentre();
        });
      case KeyRepeatEvent():
        break;
      case KeyUpEvent():
        final pending = _hold != null;
        _hold?.cancel();
        _hold = null;
        if (pending && !_held) widget.onActivate?.call();
        _held = false;
    }
    return KeyEventResult.handled;
  }

  void _setFocus(bool f) {
    if (!f) {
      _hold?.cancel();
      _hold = null;
    }
    setState(() => _focused = f);
    widget.onFocusChange?.call(f);
    final d = KDensity.of(context);
    if (f && d.isTv && !_reduceMotion) {
      _breath.repeat(reverse: true);
    } else {
      _breath.stop();
    }
  }

  bool get _reduceMotion =>
      MediaQuery.maybeDisableAnimationsOf(context) ?? false;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final active = _focused || _hovered;
    final scale = _pressed
        ? .97
        : _focused
        ? (widget.focusScale ?? d.focusScaleCard)
        : _hovered
        ? 1 + ((widget.focusScale ?? d.focusScaleCard) - 1) * .5
        : 1.0;
    final radius = BorderRadius.circular(widget.radius ?? d.cardRadius);
    final showGlow = _focused && (d.isTv || d.isDesktop);

    final content = AnimatedBuilder(
      animation: _breath,
      builder: (context, child) {
        // Light drifts slowly on TV (no touchpad tilt), follows the pointer on desktop.
        final t = Curves.easeInOut.transform(_breath.value);
        final light = d.isTv
            ? Offset(.15 + .5 * t, .1 + .15 * math.sin(t * math.pi))
            : _pointer;
        return Stack(
          fit: StackFit.passthrough,
          children: [
            child!,
            if (widget.light)
              Positioned.fill(
                child: IgnorePointer(
                  child: AnimatedOpacity(
                    opacity: active ? 1 : 0,
                    duration: KMotion.focus,
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        borderRadius: radius,
                        gradient: RadialGradient(
                          center: Alignment(light.dx * 2 - 1, light.dy * 2 - 1),
                          radius: 1.1,
                          colors: [
                            Colors.white.withValues(
                              alpha: _focused ? .22 : .12,
                            ),
                            Colors.white.withValues(alpha: 0),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
              ),
          ],
        );
      },
      child: widget.builder(context, active),
    );

    // UI-021: on touch screens the target is at least 48×48 even when the
    // drawn element is smaller (chips, capsule buttons).
    return MinTapTarget(
      enabled: d.isPhone && widget.onActivate != null,
      child: Semantics(
        button: widget.onActivate != null,
        selected: widget.selected,
        label: widget.semanticLabel,
        // Sees the key events of the detector's node first only for the
        // activate keys it leaves out when there is a context menu.
        child: Focus(
          canRequestFocus: false,
          skipTraversal: true,
          onKeyEvent: _onKey,
          child: FocusableActionDetector(
            focusNode: widget.focusNode,
            autofocus: widget.autofocus,
            enabled: widget.focusable,
            onShowFocusHighlight: (_) {},
            onFocusChange: _setFocus,
            onShowHoverHighlight: (h) => setState(() => _hovered = h),
            mouseCursor: widget.onActivate != null
                ? SystemMouseCursors.click
                : MouseCursor.defer,
            // With a context menu the activate keys go to [_onKey] (a long
            // press of OK opens the menu); without one they activate here.
            shortcuts: widget.onContextMenu == null
                ? _activateShortcuts
                : _menuShortcuts,
            actions: {
              _ContextMenuIntent: CallbackAction<_ContextMenuIntent>(
                onInvoke: (_) {
                  _openContextMenuAtCentre();
                  return null;
                },
              ),
              ActivateIntent: CallbackAction<ActivateIntent>(
                onInvoke: (_) {
                  widget.onActivate?.call();
                  return null;
                },
              ),
            },
            child: MouseRegion(
              onHover: (e) {
                final box = context.findRenderObject() as RenderBox?;
                if (box == null || !box.hasSize) return;
                setState(
                  () => _pointer = Offset(
                    e.localPosition.dx / box.size.width,
                    e.localPosition.dy / box.size.height,
                  ),
                );
              },
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTapDown: d.isPhone
                    ? (_) => setState(() => _pressed = true)
                    : null,
                onTapUp: d.isPhone
                    ? (_) => setState(() => _pressed = false)
                    : null,
                onTapCancel: d.isPhone
                    ? () => setState(() => _pressed = false)
                    : null,
                onTap: widget.onActivate,
                onLongPressStart: widget.onContextMenu == null
                    ? null
                    : (e) {
                        setState(() => _pressed = false);
                        widget.onContextMenu!(e.globalPosition);
                      },
                onSecondaryTapUp: widget.onContextMenu == null
                    ? null
                    : (e) => widget.onContextMenu!(e.globalPosition),
                child: AnimatedScale(
                  scale: scale,
                  duration: _focused ? KMotion.focus : KMotion.focusOut,
                  curve: _focused ? KMotion.focusCurve : Curves.easeOut,
                  child: AnimatedContainer(
                    duration: KMotion.focus,
                    decoration: BoxDecoration(
                      borderRadius: radius,
                      boxShadow: showGlow
                          ? [
                              const BoxShadow(
                                color: Color(0x73000000),
                                blurRadius: 24,
                                offset: Offset(0, 10),
                              ),
                              BoxShadow(
                                color: KColors.focusGlow,
                                blurRadius: 22,
                                spreadRadius: 1,
                              ),
                            ]
                          : const [],
                    ),
                    // Always a decoration (transparent without focus): adding
                    // and removing it changed the widget tree and re-created
                    // the focus nodes of buttons inside (UI-021: Shift+Tab
                    // got stuck next to a source row's Rescan and Remove).
                    foregroundDecoration: BoxDecoration(
                      borderRadius: radius,
                      border: Border.all(
                        color: _focused && !d.isPhone
                            ? Colors.white.withValues(alpha: .85)
                            : Colors.transparent,
                        width: d.isTv ? 2 : 1.5,
                      ),
                    ),
                    child: ClipRRect(borderRadius: radius, child: content),
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

/// How long OK / Select is held to open the context menu (QA-015), as a
/// long press on Android.
const kLongSelect = Duration(milliseconds: 500);

/// The keys that activate an element (the remote's OK, Enter, gamepad A).
final _activateKeys = <LogicalKeyboardKey>{
  LogicalKeyboardKey.select,
  LogicalKeyboardKey.enter,
  LogicalKeyboardKey.numpadEnter,
  LogicalKeyboardKey.gameButtonA,
};

const _menuShortcuts = <ShortcutActivator, Intent>{
  SingleActivator(LogicalKeyboardKey.contextMenu): _ContextMenuIntent(),
  SingleActivator(LogicalKeyboardKey.f10, shift: true): _ContextMenuIntent(),
  SingleActivator(LogicalKeyboardKey.gameButtonY): _ContextMenuIntent(),
};

const _activateShortcuts = <ShortcutActivator, Intent>{
  SingleActivator(LogicalKeyboardKey.select): ActivateIntent(),
  SingleActivator(LogicalKeyboardKey.enter): ActivateIntent(),
  SingleActivator(LogicalKeyboardKey.numpadEnter): ActivateIntent(),
  SingleActivator(LogicalKeyboardKey.gameButtonA): ActivateIntent(),
  ..._menuShortcuts,
};

/// Scrolls the element that takes keyboard focus into view (UI-021, WCAG
/// 2.4.11). Flutter's Tab traversal does this for a move inside a page, but
/// not when focus enters a page from the sidebar: the page's scope restores
/// a child that may be scrolled far away. Only for keyboard focus (not
/// touch) and not on TV, whose rows scroll focus themselves.
class KeepFocusVisible extends StatefulWidget {
  const KeepFocusVisible({super.key, this.enabled = true, required this.child});

  final bool enabled;
  final Widget child;

  @override
  State<KeepFocusVisible> createState() => _KeepFocusVisibleState();
}

class _KeepFocusVisibleState extends State<KeepFocusVisible> {
  FocusNode? _last;

  @override
  void initState() {
    super.initState();
    FocusManager.instance.addListener(_changed);
  }

  @override
  void dispose() {
    FocusManager.instance.removeListener(_changed);
    super.dispose();
  }

  void _changed() {
    final node = FocusManager.instance.primaryFocus;
    if (!widget.enabled || identical(node, _last)) return;
    _last = node;
    if (FocusManager.instance.highlightMode != FocusHighlightMode.traditional) {
      return;
    }
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final ctx = node?.context;
      if (!identical(FocusManager.instance.primaryFocus, node) ||
          ctx == null ||
          !ctx.mounted ||
          Scrollable.maybeOf(ctx) == null) {
        return;
      }
      // Scrolls only when the element is out of view, at either edge.
      for (final policy in const [
        ScrollPositionAlignmentPolicy.keepVisibleAtStart,
        ScrollPositionAlignmentPolicy.keepVisibleAtEnd,
      ]) {
        unawaited(Scrollable.ensureVisible(ctx, alignmentPolicy: policy));
      }
    });
  }

  @override
  Widget build(BuildContext context) => widget.child;
}

class _ContextMenuIntent extends Intent {
  const _ContextMenuIntent();
}

/// Capsule button: white fill + dark label when focused on TV (01 §4.5).
class KButton extends StatefulWidget {
  const KButton({
    super.key,
    required this.label,
    this.icon,
    this.onPressed,
    this.onLongPress,
    this.primary = false,
    this.autofocus = false,
    this.focusNode,
  });

  final String label;
  final IconData? icon;
  final VoidCallback? onPressed;

  /// Long press, right click, the menu key or holding OK on a remote
  /// (NET-013: more ways to play).
  final VoidCallback? onLongPress;
  final bool primary;
  final bool autofocus;
  final FocusNode? focusNode;

  @override
  State<KButton> createState() => _KButtonState();
}

class _KButtonState extends State<KButton> {
  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    return KFocusable(
      autofocus: widget.autofocus,
      focusNode: widget.focusNode,
      onActivate: widget.onPressed,
      onContextMenu: widget.onLongPress == null
          ? null
          : (_) => widget.onLongPress!(),
      focusScale: d.focusScaleButton,
      radius: 999,
      light: false,
      semanticLabel: widget.label,
      builder: (context, active) {
        final focused = active;
        final Color bg, fg;
        if (focused) {
          bg = KColors.focusFill;
          fg = KColors.focusLabel;
        } else if (widget.primary) {
          bg = KColors.accent;
          fg = Colors.white;
        } else {
          bg = Colors.white.withValues(alpha: .12);
          fg = Colors.white;
        }
        final vPad = d.isTv ? 10.0 : 11.0;
        return AnimatedContainer(
          duration: KMotion.focus,
          padding: EdgeInsets.symmetric(
            horizontal: d.isTv ? 22 : 20,
            vertical: vPad,
          ),
          decoration: BoxDecoration(
            color: bg,
            borderRadius: BorderRadius.circular(999),
          ),
          // The label is already the button's semantics label (UI-021:
          // TalkBack read "Play, Play").
          child: ExcludeSemantics(
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (widget.icon != null) ...[
                  Icon(widget.icon, color: fg, size: d.type.callout + 6),
                  const SizedBox(width: 8),
                ],
                // A long label on a phone (ru/uk) ends with «…» instead of
                // overflowing the pill (UI-056).
                Flexible(
                  child: Text(
                    widget.label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.labelLarge?.copyWith(color: fg),
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

/// Debounces focus-driven background changes (01 §4.7 crossfade-art).
class Debouncer {
  Debouncer(this.delay);
  final Duration delay;
  Timer? _t;
  void call(VoidCallback f) {
    _t?.cancel();
    _t = Timer(delay, f);
  }

  void dispose() => _t?.cancel();
}
