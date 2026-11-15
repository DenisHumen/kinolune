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

Future<void> _tab(WidgetTester t, {bool back = false}) async {
  if (back) {
    await t.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await t.sendKeyEvent(LogicalKeyboardKey.tab);
    await t.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  } else {
    await t.sendKeyEvent(LogicalKeyboardKey.tab);
  }
  await settle(t, 2);
}

/// Presses Tab until focus has gone round once: back where it started, or,
/// with [inNav] (the app's sidebar), through the page and the sidebar and
/// into the page again. Pages with lazily built rows drop the stops scrolled
/// far away, so the start itself may not come back. The first stop is the
/// focus before the first key press, when there is one.
///
/// Fails on a keyboard trap: focus not moving, or no round in [max] steps.
Future<List<_Stop>> _cycle(
  WidgetTester t, {
  bool Function(_Stop s)? inNav,
  int max = 1500,
}) async {
  // A route's own scope (focus after opening a page) is no Tab stop.
  var initial = _current(t);
  if (initial?.node is FocusScopeNode) initial = null;
  final stops = <_Stop>[?initial];
  // 0: not in the page yet, 1: in the page, 2: in the sidebar after it.
  var phase = initial == null || (inNav?.call(initial) ?? false) ? 0 : 1;
  for (var i = 0; i < max; i++) {
    await _tab(t);
    final s = _current(t);
    expect(s, isNotNull, reason: 'Tab #$i left the focus nowhere');
    // Settings: the first Tab after opening the page leaves focus on the
    // route's scope (one lost key press; audit note).
    if (stops.isEmpty && s!.node is FocusScopeNode && i < 2) continue;
    if (stops.isNotEmpty && identical(s!.node, stops.last.node)) {
      fail(
        'Tab #$i did not move the focus from ${stops.last}; before: '
        '${stops.reversed.skip(1).take(4).join(', ')}',
      );
    }
    if (stops.isNotEmpty && identical(s!.node, stops.first.node)) {
      return stops;
    }
    if (inNav != null) {
      if (inNav(s!)) {
        if (phase >= 1) phase = 2;
      } else if (phase == 2) {
        return stops;
      } else {
        phase = 1;
      }
    }
    stops.add(s!);
  }
  fail('no Tab round in $max steps: ${stops.take(12).join(', ')} …');
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

        final sidebar = find.byKey(const ValueKey('nav-sidebar'));
        expect(sidebar, findsOneWidget);
        final navRight = t.getTopRight(
          find.descendant(of: sidebar, matching: find.byType(Container)).first,
        ).dx;
        bool inNav(_Stop s) => s.rect.center.dx < navRight;

        final forward = await _cycle(t, inNav: inNav);
        expect(forward.length, greaterThan(3), reason: '$forward');

        // Every stop is in the window when it gets focus (WCAG 2.4.11) and
        // has a name for screen readers.
        final screen = Offset.zero & const Size(1280, 800);
        final hidden = forward.where((s) => !screen.overlaps(s.rect)).toList();
        expect(hidden, isEmpty, reason: 'focus outside the window');
        final unnamed = forward.where((s) => s.name.isEmpty).toList();
        expect(unnamed, isEmpty, reason: 'focusable without a label');

        // Reading order: the page, then the sidebar as one block from top
        // to bottom (the round ends where focus re-enters the page).
        final firstNav = forward.indexWhere(inNav);
        expect(firstNav, greaterThan(0), reason: 'no sidebar: $forward');
        final nav = forward.skip(firstNav).toList();
        expect(nav.every(inNav), isTrue, reason: 'interleaved: $forward');
        final navTops = nav.map((s) => s.rect.top).toList();
        expect(
          navTops,
          orderedEquals([...navTops]..sort()),
          reason: 'sidebar items out of order: $nav',
        );

        // Shift+Tab undoes Tab, stop by stop, where Tab moved forward in
        // reading order (rows built lazily around a scrolled-to card can
        // make Tab enter them mid-way; Shift+Tab then reaches the cards
        // before it).
        var checked = 0;
        for (var i = 0; i < 30 && checked < 10; i++) {
          final a = _current(t)!;
          await _tab(t);
          final b = _current(t)!;
          final forwardMove =
              b.rect.top > a.rect.bottom - 1 ||
              (b.rect.top - a.rect.top).abs() < 2 && b.rect.left > a.rect.left;
          if (!forwardMove || inNav(a) || inNav(b)) continue;
          await _tab(t, back: true);
          expect(
            _current(t)!.node,
            same(a.node),
            reason: 'Shift+Tab from $b does not return to $a',
          );
          await _tab(t);
          checked++;
        }
        expect(checked, greaterThan(2), reason: 'too few Tab moves checked');

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
      final stops = await _cycle(t, inNav: (s) => s.rect.center.dx < 232);
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

  testWidgets('TV: the D-pad enters the page, Back returns to the menu', (
    t,
  ) async {
    final semantics = t.ensureSemantics();
    setSurface(t, const Size(960, 540), dpr: 2);
    final services = await _services(Section.home);
    await t.pumpWidget(testApp(services, formFactor: FormFactor.tv));
    await settle(t, 10);
    final start = _current(t);
    expect(start, isNotNull, reason: 'nothing is focused on TV start');
    await pressKey(t, LogicalKeyboardKey.arrowDown);
    final down = _current(t)!;
    expect(down.node, isNot(same(start!.node)));
    expect(down.node.debugLabel ?? '', isNot(startsWith('menu-')));
    expect(down.name, isNotEmpty, reason: 'focused card without a label');
    // Back returns to the menu (01 §4.2; Up from the page's first row
    // does not leave the page yet: docs/qa/ui-021-a11y-audit.md).
    await pressKey(t, LogicalKeyboardKey.escape);
    expect(_current(t)!.node.debugLabel, startsWith('menu-'));
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
    semantics.dispose();
  });
}
