// SPDX-License-Identifier: MPL-2.0
//
// The PIN pad (UI-015): ten digits, erase and OK on a 3×4 grid that the TV
// remote walks with the D-pad (OK on a key presses it), plus the number
// keys of a keyboard or remote, Backspace and Enter. After too many wrong
// PINs it shows the wait of the shared [PinAttemptLimiter] and takes no
// input until it is over.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import 'pin_limiter.dart';
import 'profile_controller.dart' show PinCheck;

/// `m:ss` of a wait.
String formatPinWait(Duration d) {
  final s = (d.inMilliseconds / 1000).ceil();
  final m = s ~/ 60;
  return '$m:${(s % 60).toString().padLeft(2, '0')}';
}

final _digitKeys = <LogicalKeyboardKey, String>{
  LogicalKeyboardKey.digit0: '0',
  LogicalKeyboardKey.digit1: '1',
  LogicalKeyboardKey.digit2: '2',
  LogicalKeyboardKey.digit3: '3',
  LogicalKeyboardKey.digit4: '4',
  LogicalKeyboardKey.digit5: '5',
  LogicalKeyboardKey.digit6: '6',
  LogicalKeyboardKey.digit7: '7',
  LogicalKeyboardKey.digit8: '8',
  LogicalKeyboardKey.digit9: '9',
  LogicalKeyboardKey.numpad0: '0',
  LogicalKeyboardKey.numpad1: '1',
  LogicalKeyboardKey.numpad2: '2',
  LogicalKeyboardKey.numpad3: '3',
  LogicalKeyboardKey.numpad4: '4',
  LogicalKeyboardKey.numpad5: '5',
  LogicalKeyboardKey.numpad6: '6',
  LogicalKeyboardKey.numpad7: '7',
  LogicalKeyboardKey.numpad8: '8',
  LogicalKeyboardKey.numpad9: '9',
};

class PinPad extends StatefulWidget {
  const PinPad({
    super.key,
    required this.title,
    required this.onSubmit,
    this.subtitle,
    this.limiter,
    this.onCancel,
    this.wrongMessage,
    this.initialError,
    this.autofocus = true,
    this.minLength = 4,
    this.maxLength = 8,
  });

  final String title;
  final String? subtitle;

  /// Checks the PIN. On [PinCheck.ok] the caller moves on; the pad clears.
  final Future<PinCheck> Function(String pin) onSubmit;

  /// Shows its wait and blocks input while it lasts.
  final PinAttemptLimiter? limiter;

  /// Shows a Cancel key under the pad.
  final VoidCallback? onCancel;

  /// Shown for [PinCheck.wrong]; default "Wrong PIN".
  final String? wrongMessage;

  /// An error to show from the start (the previous step failed).
  final String? initialError;

  final bool autofocus;
  final int minLength;
  final int maxLength;

  @override
  State<PinPad> createState() => _PinPadState();
}

class _PinPadState extends State<PinPad> {
  String _pin = '';
  late String? _error = widget.initialError;
  bool _busy = false;

  /// The last input came from a keyboard: Enter submits (on a remote OK
  /// presses the focused key instead).
  bool _typed = false;
  Timer? _tick;

  /// The "1" key: focused when the pad appears ([PinPad.autofocus]), also
  /// when another part of the screen held the focus before.
  final _firstKey = FocusNode(debugLabel: 'pin-key-1');

  @override
  void initState() {
    super.initState();
    widget.limiter?.addListener(_onLimiter);
    _onLimiter();
  }

  /// Whether the pad is on screen (not in a hidden section of the shell).
  bool _shown = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    // Hidden sections of the shell (IndexedStack) take no focus; the pad
    // focuses "1" each time it comes on screen.
    final shown = Visibility.of(context);
    if (shown && !_shown && widget.autofocus) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _firstKey.requestFocus();
      });
    }
    _shown = shown;
  }

  @override
  void didUpdateWidget(PinPad old) {
    super.didUpdateWidget(old);
    if (old.limiter != widget.limiter) {
      old.limiter?.removeListener(_onLimiter);
      widget.limiter?.addListener(_onLimiter);
      _onLimiter();
    }
  }

  @override
  void dispose() {
    widget.limiter?.removeListener(_onLimiter);
    _tick?.cancel();
    _firstKey.dispose();
    super.dispose();
  }

  bool get _locked => widget.limiter?.locked ?? false;

  void _onLimiter() {
    if (_locked) {
      _tick ??= Timer.periodic(const Duration(seconds: 1), (_) {
        if (!mounted) return;
        if (!_locked) {
          _tick?.cancel();
          _tick = null;
        }
        setState(() {});
      });
    }
    if (mounted) setState(() {});
  }

  void _add(String digit) {
    if (_busy || _locked || _pin.length >= widget.maxLength) return;
    setState(() {
      _pin += digit;
      _error = null;
    });
  }

  void _erase() {
    if (_busy || _pin.isEmpty) return;
    setState(() {
      _pin = _pin.substring(0, _pin.length - 1);
      _error = null;
    });
  }

  Future<void> _submit() async {
    if (_busy || _locked) return;
    final l = context.l10n;
    if (_pin.length < widget.minLength) {
      setState(() => _error = l.profilesPinTooShort(widget.minLength));
      return;
    }
    setState(() => _busy = true);
    final r = await widget.onSubmit(_pin);
    if (!mounted) return;
    final limiter = widget.limiter;
    setState(() {
      _busy = false;
      _pin = '';
      _error = switch (r) {
        PinCheck.ok => null,
        PinCheck.locked => null,
        PinCheck.wrong =>
          limiter != null &&
                  !limiter.locked &&
                  limiter.attemptsBeforeWait <= 2 &&
                  limiter.attemptsBeforeWait > 0
              ? l.profilesPinWrongLeft(limiter.attemptsBeforeWait)
              : (widget.wrongMessage ?? l.profilesPinWrong),
      };
    });
    _onLimiter();
  }

  KeyEventResult _onKey(FocusNode node, KeyEvent e) {
    if (e is! KeyDownEvent && e is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    final digit = _digitKeys[e.logicalKey];
    if (digit != null) {
      _typed = true;
      _add(digit);
      return KeyEventResult.handled;
    }
    if (e.logicalKey == LogicalKeyboardKey.backspace ||
        e.logicalKey == LogicalKeyboardKey.delete) {
      _typed = true;
      _erase();
      return KeyEventResult.handled;
    }
    if (_typed &&
        (e.logicalKey == LogicalKeyboardKey.enter ||
            e.logicalKey == LogicalKeyboardKey.numpadEnter)) {
      unawaited(_submit());
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  void _press(VoidCallback f) {
    // The focused key takes Enter as "press"; after digits typed on a
    // keyboard Enter means "done" instead (D-pad OK is `select`).
    final keys = HardwareKeyboard.instance;
    if (_typed &&
        (keys.isLogicalKeyPressed(LogicalKeyboardKey.enter) ||
            keys.isLogicalKeyPressed(LogicalKeyboardKey.numpadEnter))) {
      unawaited(_submit());
      return;
    }
    _typed = false;
    f();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final d = KDensity.of(context);
    final keySize = d.isTv ? 60.0 : (d.isPhone ? 68.0 : 56.0);
    final gap = d.isTv ? 10.0 : 12.0;
    final locked = _locked;
    final remaining = widget.limiter?.remaining ?? Duration.zero;
    final status = locked
        ? l.profilesPinLockedOut(formatPinWait(remaining))
        : _error;

    Widget key(
      String id, {
      String? label,
      IconData? icon,
      VoidCallback? onTap,
    }) => _PinKey(
      key: ValueKey('pin-key-$id'),
      size: keySize,
      label: label,
      icon: icon,
      semanticLabel: switch (id) {
        'back' => l.profilesPinErase,
        'ok' => l.profilesPinOk,
        _ => id,
      },
      focusNode: id == '1' ? _firstKey : null,
      onTap: onTap == null || locked || _busy ? null : () => _press(onTap),
    );

    Widget row(List<Widget> keys) => Padding(
      padding: EdgeInsets.only(bottom: gap),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (final (i, k) in keys.indexed) ...[
            if (i > 0) SizedBox(width: gap),
            k,
          ],
        ],
      ),
    );

    Widget digit(String n) => key(n, label: n, onTap: () => _add(n));

    final dots = Row(
      key: const ValueKey('pin-dots'),
      mainAxisSize: MainAxisSize.min,
      children: [
        for (var i = 0; i < (_pin.length < 4 ? 4 : _pin.length); i++)
          Container(
            margin: const EdgeInsets.symmetric(horizontal: 6),
            width: 14,
            height: 14,
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              color: i < _pin.length ? KColors.textPrimary : Colors.transparent,
              border: Border.all(color: KColors.textSecondary, width: 1.5),
            ),
          ),
      ],
    );

    return Focus(
      onKeyEvent: _onKey,
      child: FocusTraversalGroup(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              widget.title,
              textAlign: TextAlign.center,
              style: theme.textTheme.titleLarge,
            ),
            if (widget.subtitle != null) ...[
              const SizedBox(height: 6),
              Text(
                widget.subtitle!,
                textAlign: TextAlign.center,
                style: theme.textTheme.bodySmall,
              ),
            ],
            SizedBox(height: d.isTv ? 14 : 20),
            Semantics(
              label: l.profilesPinEntered(_pin.length),
              child: ExcludeSemantics(child: dots),
            ),
            const SizedBox(height: 10),
            SizedBox(
              height: 40,
              child: status == null
                  ? null
                  : Text(
                      status,
                      key: ValueKey(locked ? 'pin-locked' : 'pin-error'),
                      textAlign: TextAlign.center,
                      maxLines: 2,
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.error,
                      ),
                    ),
            ),
            row([digit('1'), digit('2'), digit('3')]),
            row([digit('4'), digit('5'), digit('6')]),
            row([digit('7'), digit('8'), digit('9')]),
            row([
              key('back', icon: Icons.backspace_outlined, onTap: _erase),
              digit('0'),
              key(
                'ok',
                icon: Icons.check_rounded,
                onTap: () => unawaited(_submit()),
              ),
            ]),
            if (widget.onCancel != null)
              TextButton(
                key: const ValueKey('pin-cancel'),
                onPressed: widget.onCancel,
                child: Text(l.commonCancel),
              ),
          ],
        ),
      ),
    );
  }
}

class _PinKey extends StatelessWidget {
  const _PinKey({
    super.key,
    required this.size,
    required this.semanticLabel,
    this.label,
    this.icon,
    this.onTap,
    this.focusNode,
  });

  final double size;
  final String? label;
  final IconData? icon;
  final String semanticLabel;
  final VoidCallback? onTap;
  final FocusNode? focusNode;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final enabled = onTap != null;
    return KFocusable(
      onActivate: onTap,
      focusNode: focusNode,
      radius: 999,
      light: false,
      focusScale: 1.08,
      semanticLabel: semanticLabel,
      builder: (context, focused) {
        final fg = focused
            ? KColors.focusLabel
            : (enabled ? KColors.textPrimary : KColors.textTertiary);
        return AnimatedContainer(
          duration: KMotion.focus,
          width: size,
          height: size,
          alignment: Alignment.center,
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            color: focused ? KColors.focusFill : KColors.bgElevated2,
          ),
          child: label != null
              ? Text(
                  label!,
                  style: theme.textTheme.headlineSmall?.copyWith(color: fg),
                )
              : Icon(icon, color: fg, size: size * .4),
        );
      },
    );
  }
}

/// Asks for a PIN in a dialog until [check] says ok (true) or the user
/// cancels (false).
Future<bool> showPinDialog(
  BuildContext context, {
  required String title,
  String? subtitle,
  required Future<PinCheck> Function(String pin) check,
  PinAttemptLimiter? limiter,
}) async {
  final ok = await showDialog<bool>(
    context: context,
    builder: (c) => _PinDialog(
      child: PinPad(
        key: const ValueKey('pin-dialog'),
        title: title,
        subtitle: subtitle,
        limiter: limiter,
        onCancel: () => Navigator.pop(c, false),
        onSubmit: (pin) async {
          final r = await check(pin);
          if (r == PinCheck.ok && c.mounted) Navigator.pop(c, true);
          return r;
        },
      ),
    ),
  );
  return ok ?? false;
}

/// Asks for a new PIN twice; null if cancelled.
Future<String?> showNewPinDialog(
  BuildContext context, {
  required String title,
  String? subtitle,
}) => showDialog<String>(
  context: context,
  builder: (c) => _PinDialog(
    child: _NewPin(title: title, subtitle: subtitle),
  ),
);

class _NewPin extends StatefulWidget {
  const _NewPin({required this.title, this.subtitle});

  final String title;
  final String? subtitle;

  @override
  State<_NewPin> createState() => _NewPinState();
}

class _NewPinState extends State<_NewPin> {
  String? _first;
  bool _mismatch = false;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final first = _first;
    return PinPad(
      key: ValueKey(
        first == null
            ? 'pin-new-${_mismatch ? 'again' : 'first'}'
            : 'pin-confirm',
      ),
      title: first == null ? widget.title : l.profilesPinConfirm,
      subtitle: first == null
          ? (widget.subtitle ?? l.profilesPinNewHint)
          : null,
      initialError: _mismatch && first == null ? l.profilesPinMismatch : null,
      onCancel: () => Navigator.pop(context),
      onSubmit: (pin) async {
        if (first == null) {
          setState(() => _first = pin);
          return PinCheck.ok;
        }
        if (pin != first) {
          setState(() {
            _first = null;
            _mismatch = true;
          });
          return PinCheck.wrong;
        }
        Navigator.pop(context, pin);
        return PinCheck.ok;
      },
    );
  }
}

class _PinDialog extends StatelessWidget {
  const _PinDialog({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) => Dialog(
    backgroundColor: KColors.bgElevated,
    child: SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(24, 24, 24, 12),
      child: child,
    ),
  );
}
