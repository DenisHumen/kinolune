// SPDX-License-Identifier: MPL-2.0
//
// BUG-025: tapping the section that is already selected in the sidebar,
// the rail, the phone tabs or the TV menu goes back to the section's root
// page; at the root it scrolls the page to the top. Other sections keep
// their pages.

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/library_grid_screen.dart';
import 'package:kinolune/screens/movie_detail_screen.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

const _phone = Size(390, 844);
const _desktop = Size(1280, 800);
const _tablet = Size(820, 1180);
const _tv = Size(960, 540);

AppShellState _shell(WidgetTester t) =>
    t.state<AppShellState>(find.byType(AppShell));

/// The page scroll view of the visible library grid.
ScrollPosition _gridPosition(WidgetTester t) {
  final scrollable = find
      .descendant(
        of: find.byType(LibraryGridScreen).hitTestable(),
        matching: find.byType(Scrollable),
      )
      .evaluate()
      .map((e) => (e as StatefulElement).state as ScrollableState)
      .where((s) => s.position.axis == Axis.vertical)
      .reduce(
        (a, b) => a.position.viewportDimension >= b.position.viewportDimension
            ? a
            : b,
      );
  return scrollable.position;
}

Future<void> _openMovie(WidgetTester t) async {
  await t.tap(find.text('Harbor Lights').first);
  await settle(t);
  expect(find.byType(MovieDetailScreen), findsOneWidget);
}

void main() {
  testWidgets('desktop sidebar: re-tapping Movies pops a movie page', (
    t,
  ) async {
    setSurface(t, _desktop);
    await t.pumpWidget(testApp(await demoServices(), section: Section.movies));
    await settle(t);
    await _openMovie(t);

    await t.tap(find.text('Movies').first);
    await settle(t);
    expect(_shell(t).section, Section.movies);
    expect(find.byType(MovieDetailScreen), findsNothing);
    expect(find.text('14 titles'), findsOneWidget);
    expect(t.takeException(), isNull);
  });

  testWidgets('desktop sidebar: re-tapping at the root scrolls to the top', (
    t,
  ) async {
    setSurface(t, _desktop);
    await t.pumpWidget(testApp(await demoServices(), section: Section.movies));
    await settle(t);
    final pos = _gridPosition(t);
    expect(pos.maxScrollExtent, greaterThan(0));
    pos.jumpTo(pos.maxScrollExtent);
    await settle(t, 2);
    expect(pos.pixels, greaterThan(0));

    await t.tap(find.text('Movies').first);
    await settle(t, 8);
    expect(pos.pixels, pos.minScrollExtent);
    expect(_shell(t).section, Section.movies);
  });

  testWidgets('keyboard: Enter and Space on the focused item', (t) async {
    setSurface(t, _desktop);
    await t.pumpWidget(testApp(await demoServices(), section: Section.movies));
    await settle(t);
    await _openMovie(t);

    // The focus node of the sidebar item.
    final label = find.descendant(
      of: find.byKey(const ValueKey('nav-sidebar')),
      matching: find.text('Movies'),
    );
    Focus.of(t.element(label.first)).requestFocus();
    await settle(t, 2);
    await pressKey(t, LogicalKeyboardKey.enter);
    await settle(t);
    expect(find.byType(MovieDetailScreen), findsNothing);

    final pos = _gridPosition(t);
    pos.jumpTo(pos.maxScrollExtent);
    await settle(t, 2);
    await pressKey(t, LogicalKeyboardKey.space);
    await settle(t, 6);
    expect(pos.pixels, pos.minScrollExtent);
  });

  testWidgets('other sections keep their pages', (t) async {
    setSurface(t, _desktop);
    await t.pumpWidget(testApp(await demoServices(), section: Section.movies));
    await settle(t);
    await _openMovie(t);

    // TV Shows: open a show, then back to Movies — its movie page stays.
    await t.tap(find.text('TV Shows').first);
    await settle(t);
    expect(_shell(t).section, Section.shows);
    expect(_shell(t).navigatorOf(Section.movies)!.canPop(), isTrue);

    // Re-tapping TV Shows at its root does not touch the Movies stack.
    await t.tap(find.text('TV Shows').first);
    await settle(t);
    expect(_shell(t).navigatorOf(Section.movies)!.canPop(), isTrue);

    await t.tap(find.text('Movies').first);
    await settle(t);
    expect(_shell(t).section, Section.movies);
    expect(find.byType(MovieDetailScreen), findsOneWidget);
  });

  testWidgets('rail: re-tapping pops to the root', (t) async {
    setSurface(t, _tablet, dpr: 2);
    await t.pumpWidget(
      testApp(
        await demoServices(),
        formFactor: FormFactor.phone,
        section: Section.movies,
      ),
    );
    await settle(t);
    expect(find.byKey(const ValueKey('nav-rail')), findsOneWidget);
    await _openMovie(t);
    await t.tap(find.byIcon(Section.movies.selectedIcon).first);
    await settle(t);
    expect(find.byType(MovieDetailScreen), findsNothing);
  });

  testWidgets('phone tabs: re-tapping Library pops, then scrolls to top', (
    t,
  ) async {
    setSurface(t, _phone, dpr: 2);
    await t.pumpWidget(
      testApp(await demoServices(), formFactor: FormFactor.phone),
    );
    await settle(t);
    await t.tap(find.text('Library'));
    await settle(t);
    await _openMovie(t);

    await t.tap(find.text('Library'));
    await settle(t);
    expect(_shell(t).section, Section.movies);
    expect(find.byType(MovieDetailScreen), findsNothing);

    final pos = _gridPosition(t);
    expect(pos.maxScrollExtent, greaterThan(0));
    pos.jumpTo(pos.maxScrollExtent);
    await settle(t, 2);
    await t.tap(find.text('Library'));
    await settle(t, 8);
    expect(pos.pixels, pos.minScrollExtent);

    // Home keeps its own stack while Library is re-tapped.
    await t.tap(find.text('Home'));
    await settle(t);
    expect(_shell(t).section, Section.home);
  });

  testWidgets('TV menu: OK on the selected tab pops, then scrolls up', (
    t,
  ) async {
    setSurface(t, _tv, dpr: 2);
    await t.pumpWidget(
      testApp(
        await demoServices(),
        formFactor: FormFactor.tv,
        section: Section.movies,
      ),
    );
    await settle(t);
    final nav = _shell(t).navigatorOf(Section.movies)!;
    nav.push(
      MaterialPageRoute<void>(
        builder: (_) => const Scaffold(body: Text('pushed page')),
      ),
    );
    await settle(t);
    expect(find.text('pushed page'), findsOneWidget);
    final tab = find.descendant(
      of: find.byKey(const ValueKey('nav-top-menu')),
      matching: find.text('Movies'),
    );
    Focus.of(t.element(tab.first)).requestFocus();
    await settle(t, 2);
    await pressKey(t, LogicalKeyboardKey.select);
    await settle(t);
    expect(find.text('pushed page'), findsNothing);
    expect(_shell(t).section, Section.movies);

    final pos = _gridPosition(t);
    pos.jumpTo(pos.maxScrollExtent);
    await settle(t, 2);
    await pressKey(t, LogicalKeyboardKey.enter);
    await settle(t, 6);
    expect(pos.pixels, pos.minScrollExtent);
  });
}
