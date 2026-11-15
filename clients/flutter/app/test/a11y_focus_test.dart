// SPDX-License-Identifier: MPL-2.0
//
// Keyboard focus order of the key screens (UI-021): Tab walks every screen
// without a trap, in reading order (navigation first, then the page from top
// to bottom), Shift+Tab walks back the same way, and every element that
// takes focus has a name for screen readers. On TV the D-pad leaves the
// navigation for the page and comes back.

import 'package:flutter/rendering.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

typedef _Step = Future<void> Function(WidgetTester t, AppLocalizations l);

Future<void> _tap(WidgetTester t, Finder f) async {
  await t.ensureVisible(f.first);
  await settle(t, 4);
  await t.tap(f.first);
  await settle(t, 10);
}

/// One focus stop: where it is and what a screen reader calls it.
class _Stop {
  _Stop(this.node, this.rect, this.name);
  final FocusNode node;
  final Rect rect;
  final String name;

  @override
  String toString() => '"$name" at ${rect.topLeft}';
}

_Stop? _current(WidgetTester t) {
  final node = FocusManager.instance.primaryFocus;
  final ctx = node?.context;
  if (node == null || ctx == null || !ctx.mounted) return null;
  final box = ctx.findRenderObject();
  if (box is! RenderBox || !box.hasSize) return null;
  final rect = box.localToGlobal(Offset.zero) & box.size;
  return _Stop(node, rect, _name());
}

/// The label, tooltip, value or hint of the semantics node that has input
/// focus: what TalkBack, VoiceOver, Narrator and Orca announce for it.
String _name() {
  final focused = find.semantics.byFlag(SemanticsFlag.isFocused).evaluate();
  for (final node in focused) {
    final data = node.getSemanticsData();
    final name = [
      data.label,
      data.tooltip,
      data.value,
      data.hint,
    ].where((s) => s.trim().isNotEmpty).join(' / ');
    if (name.isNotEmpty) return name;
  }
  return '';
}

/// Presses Tab (or Shift+Tab) until focus comes back to where it started.
/// The first stop is the focus before the first key press, when there is
/// one. Fails on a keyboard trap: focus not moving, a loop that does not
/// come back to the start, or no cycle in [max] steps.
Future<List<_Stop>> _cycle(
  WidgetTester t, {
  bool back = false,
  int max = 1500,
}) async {
  final initial = _current(t);
  final stops = <_Stop>[?initial];
  final seen = <FocusNode>{?initial?.node};
  for (var i = 0; i < max; i++) {
    if (back) {
      await t.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await t.sendKeyEvent(LogicalKeyboardKey.tab);
      await t.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    } else {
      await t.sendKeyEvent(LogicalKeyboardKey.tab);
    }
    await settle(t, 2);
    final s = _current(t);
    expect(s, isNotNull, reason: 'Tab #$i left the focus nowhere');
    if (stops.isNotEmpty && identical(s!.node, stops.last.node)) {
      fail('Tab #$i did not move the focus from ${stops.last}');
    }
    // Back at the start: one full cycle.
    if (stops.isNotEmpty && identical(s!.node, stops.first.node)) {
      return stops;
    }
    // The initial focus was a node Tab does not stop at (a page's own
    // Focus): the cycle closes on the first real stop.
    if (stops.length > 1 && identical(s!.node, stops[1].node)) {
      return stops.sublist(1);
    }
    if (seen.contains(s!.node)) {
      final from = stops.indexWhere((x) => identical(x.node, s.node));
      fail(
        'Tab #$i loops back to stop #$from $s without returning to the '
        'start; the loop: ${stops.skip(from).join(', ')}',
      );
    }
    seen.add(s.node);
    stops.add(s);
  }
  fail('no Tab cycle in $max steps: ${stops.take(12).join(', ')} …');
}

final _screens = <String, (Section, _Step?)>{
  'home': (Section.home, null),
  'movies': (Section.movies, null),
  'movie-detail': (
    Section.movies,
    (t, _) => _tap(t, find.text('The Quiet Orbit')),
  ),
  'show-detail': (
    Section.shows,
    (t, _) => _tap(t, find.text('Moonlit Bureau')),
  ),
  'music': (Section.music, null),
  'album': (Section.music, (t, _) => _tap(t, find.text('Nocturne Avenue'))),
  'search': (Section.search, null),
  'settings': (Section.settings, null),
  'sources': (Section.settings, (t, l) => _tap(t, find.text(l.sourcesManage))),
  'twitch': (Section.twitch, null),
};

Future<AppServices> _services(Section section) async {
  final s = section == Section.twitch
      ? await twitchServices()
      : await demoServices();
  await s.setLocale(const Locale('en'));
  return s;
}

void main() {
  setUpAll(loadAppFonts);
  final l = lookupAppLocalizations(const Locale('en'));

  group('Tab order on desktop', () {
    for (final MapEntry(key: name, value: (section, step))
        in _screens.entries) {
      testWidgets(name, (t) async {
        final semantics = t.ensureSemantics();
        setSurface(t, const Size(1280, 800));
        final services = await _services(section);
        await t.pumpWidget(testApp(services, section: section));
        await settle(t, 10);
        await step?.call(t, l);
        await settle(t, 6);

        final forward = await _cycle(t);
        expect(forward.length, greaterThan(3), reason: '$forward');

        // Every stop has a name for screen readers.
        final unnamed = forward.where((s) => s.name.isEmpty).toList();
        expect(unnamed, isEmpty, reason: 'focusable without a label');

        // Shift+Tab walks the same stops in reverse.
        final backward = await _cycle(t, back: true);
        expect(
          backward.map((s) => s.node).toSet(),
          forward.map((s) => s.node).toSet(),
          reason: 'Shift+Tab does not visit the same stops as Tab',
        );

        // Reading order: the sidebar (when shown) before the page, the
        // sidebar from top to bottom.
        final sidebar = find.byKey(const ValueKey('nav-sidebar'));
        if (sidebar.evaluate().isNotEmpty) {
          final navRight = t.getTopLeft(sidebar).dx + 232;
          final nav = forward.where((s) => s.rect.center.dx < navRight);
          final navTops = nav.map((s) => s.rect.top).toList();
          expect(
            navTops,
            orderedEquals([...navTops]..sort()),
            reason: 'sidebar items out of order: ${nav.toList()}',
          );
          final firstPage = forward.indexWhere(
            (s) => s.rect.center.dx >= navRight,
          );
          final lastNav = forward.lastIndexWhere(
            (s) => s.rect.center.dx < navRight,
          );
          if (firstPage >= 0 && lastNav >= 0) {
            final pageFirst = firstPage < forward.indexOf(nav.first);
            // Either all the sidebar comes first, or (focus starting in the
            // page) the whole page comes before the sidebar.
            expect(
              pageFirst
                  ? forward
                        .skip(lastNav + 1)
                        .every((s) => s.rect.center.dx < navRight)
                  : forward
                        .skip(lastNav + 1)
                        .every((s) => s.rect.center.dx >= navRight),
              isTrue,
              reason: 'sidebar and page stops interleave: $forward',
            );
          }
        }
        services.music.pause();
        await t.pumpWidget(const SizedBox());
        await settle(t, 2);
        semantics.dispose();
      });
    }

    testWidgets('detail page: Back, then the actions in rows', (t) async {
      final semantics = t.ensureSemantics();
      setSurface(t, const Size(1280, 800));
      final services = await _services(Section.movies);
      await t.pumpWidget(testApp(services, section: Section.movies));
      await settle(t, 10);
      await _tap(t, find.text('The Quiet Orbit'));
      // Focus starts on the primary action (TV and keyboard users press
      // Enter to play).
      final start = _current(t);
      expect(start?.name, contains('Resume'));
      final stops = await _cycle(t);
      final names = stops.map((s) => s.name).toList();
      final resume = names.indexWhere((n) => n.contains('Resume'));
      final begin = names.indexWhere((n) => n.contains(l.detailFromBeginning));
      final fix = names.indexWhere((n) => n.contains(l.commonFixMatch));
      expect(resume, greaterThanOrEqualTo(0), reason: '$names');
      expect(begin, greaterThan(resume), reason: '$names');
      expect(fix, greaterThan(begin), reason: '$names');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
      semantics.dispose();
    });

    testWidgets('player controls are reachable with Tab', (t) async {
      final semantics = t.ensureSemantics();
      setSurface(t, const Size(1280, 800));
      final services = await _services(Section.movies);
      await t.pumpWidget(testApp(services, section: Section.movies));
      await settle(t, 10);
      await _tap(t, find.text('The Quiet Orbit'));
      await _tap(t, find.byIcon(Icons.play_arrow_rounded));
      final stops = await _cycle(t);
      final names = stops.map((s) => s.name).join(' | ');
      expect(stops.where((s) => s.name.isEmpty), isEmpty, reason: names);
      for (final control in ['Pause', 'Back']) {
        expect(names, contains(control), reason: names);
      }
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
      semantics.dispose();
    });
  });

  testWidgets('TV: the D-pad moves from the menu to the page and back', (
    t,
  ) async {
    setSurface(t, const Size(960, 540), dpr: 2);
    final services = await _services(Section.home);
    await t.pumpWidget(testApp(services, formFactor: FormFactor.tv));
    await settle(t, 10);
    final start = _current(t);
    expect(start, isNotNull, reason: 'nothing is focused on TV start');
    await pressKey(t, LogicalKeyboardKey.arrowDown);
    final down = _current(t)!;
    expect(down.node, isNot(same(start!.node)));
    expect(down.rect.top, greaterThan(start.rect.top - 1));
    await pressKey(t, LogicalKeyboardKey.arrowUp);
    // Back in the menu row (the item nearest to the page element).
    final up = _current(t)!;
    expect(up.rect.center.dy, closeTo(start.rect.center.dy, 4));
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });
}
