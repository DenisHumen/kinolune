// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/widgets.dart';

/// Whether the shell section around [child] is the one on screen.
///
/// The shell keeps every section alive in an `IndexedStack`, so a page's
/// `initState` and `autofocus` run once, possibly while it is hidden. A page
/// that must react to being shown again (Search focuses its field) depends on
/// this instead.
class SectionActive extends InheritedWidget {
  const SectionActive({super.key, required this.active, required super.child});

  final bool active;

  /// True outside a shell (tests, pushed routes without one).
  static bool of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<SectionActive>()?.active ??
      true;

  @override
  bool updateShouldNotify(SectionActive oldWidget) =>
      active != oldWidget.active;
}
