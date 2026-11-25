// SPDX-License-Identifier: MPL-2.0
//
// BUG-026: a screen whose build, layout or initState throws must not leave
// a plain gray window (Flutter's release ErrorWidget) that only a restart
// clears. [ErrorBoundary] wraps a screen; [installErrorBoundaries] (once, in
// main) routes every ErrorWidget through it: inside a boundary the failed
// subtree gives way to a readable error with «Повторить» (the screen is
// built anew) and «Назад» (the route closes). Outside any boundary the
// previous builder keeps working. The exception itself is reported by the
// framework as usual (FlutterError.onError → the diagnostics log, CORE-008)
// and once more here with the boundary's name.

import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';

/// Routes ErrorWidgets inside an [ErrorBoundary] to it; elsewhere the
/// [ErrorWidget.builder] that was set before. Idempotent.
void installErrorBoundaries() {
  if (_installed) return;
  _installed = true;
  final previous = ErrorWidget.builder;
  ErrorWidget.builder = (details) =>
      _BoundaryErrorWidget(details: details, fallback: previous);
}

var _installed = false;

/// Undoes [installErrorBoundaries] (tests).
@visibleForTesting
void uninstallErrorBoundaries(ErrorWidgetBuilder builder) {
  ErrorWidget.builder = builder;
  _installed = false;
}

/// Shows [child]; if anything below it fails to build, a readable error
/// with «Повторить» and «Назад» instead (BUG-026).
class ErrorBoundary extends StatefulWidget {
  const ErrorBoundary({super.key, required this.name, required this.child});

  /// For the log: which screen failed.
  final String name;
  final Widget child;

  @override
  State<ErrorBoundary> createState() => ErrorBoundaryState();
}

/// Public for tests.
class ErrorBoundaryState extends State<ErrorBoundary> {
  FlutterErrorDetails? _error;
  var _attempt = 0;
  var _pending = false;

  /// The error shown now, if any.
  FlutterErrorDetails? get error => _error;

  void _report(FlutterErrorDetails details) {
    if (_error != null || _pending) return;
    _pending = true;
    debugPrint(
      'Screen "${widget.name}" failed to build: ${details.exceptionAsString()}',
    );
    // Not during this build: the next frame shows the error screen.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _pending = false;
      if (mounted) setState(() => _error = details);
    });
    WidgetsBinding.instance.scheduleFrame();
  }

  /// «Повторить»: the screen once more, with fresh state.
  void retry() => setState(() {
    _error = null;
    _attempt++;
  });

  @override
  Widget build(BuildContext context) {
    final error = _error;
    if (error != null) {
      return ScreenErrorView(
        onRetry: retry,
        onBack: () => Navigator.maybeOf(context)?.maybePop(),
      );
    }
    return _BoundaryScope(
      state: this,
      child: KeyedSubtree(key: ValueKey(_attempt), child: widget.child),
    );
  }
}

class _BoundaryScope extends InheritedWidget {
  const _BoundaryScope({required this.state, required super.child});

  final ErrorBoundaryState state;

  @override
  bool updateShouldNotify(_BoundaryScope oldWidget) => false;
}

/// What [ErrorWidget.builder] returns once [installErrorBoundaries] ran.
class _BoundaryErrorWidget extends StatelessWidget {
  const _BoundaryErrorWidget({required this.details, required this.fallback});

  final FlutterErrorDetails details;
  final ErrorWidgetBuilder fallback;

  @override
  Widget build(BuildContext context) {
    final scope = context.getInheritedWidgetOfExactType<_BoundaryScope>();
    if (scope == null) return fallback(details);
    scope.state._report(details);
    return const ColoredBox(color: Colors.black);
  }
}

/// The error screen of a boundary: a title, a hint, «Повторить», «Назад».
class ScreenErrorView extends StatelessWidget {
  const ScreenErrorView({
    super.key,
    required this.onRetry,
    required this.onBack,
  });

  final VoidCallback onRetry;
  final VoidCallback onBack;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    return Material(
      key: const ValueKey('screen-error'),
      color: Colors.black,
      child: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(24),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 520),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  const Icon(
                    Icons.error_outline_rounded,
                    color: Colors.white70,
                    size: 48,
                  ),
                  const SizedBox(height: 16),
                  Text(
                    l.commonScreenErrorTitle,
                    textAlign: TextAlign.center,
                    style: theme.textTheme.titleLarge?.copyWith(
                      color: Colors.white,
                    ),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    l.commonScreenErrorMessage,
                    textAlign: TextAlign.center,
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: Colors.white70,
                    ),
                  ),
                  const SizedBox(height: 24),
                  Wrap(
                    spacing: 12,
                    runSpacing: 8,
                    alignment: WrapAlignment.center,
                    children: [
                      FilledButton(
                        key: const ValueKey('screen-error-retry'),
                        autofocus: true,
                        onPressed: onRetry,
                        child: Text(l.commonScreenErrorRetry),
                      ),
                      OutlinedButton(
                        key: const ValueKey('screen-error-back'),
                        onPressed: onBack,
                        child: Text(l.commonScreenErrorBack),
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
