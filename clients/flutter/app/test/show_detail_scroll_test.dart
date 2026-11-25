// SPDX-License-Identifier: MPL-2.0
//
// BUG-019 and UI-055: a show with a long season. The episodes of the
// season tab are one horizontal row (UI-055) and every one of them is
// reachable and plays: a swipe that settles on a card on a phone, the ‹ ›
// buttons, Shift+wheel, a trackpad swipe, a mouse drag and the keys on a
// desktop, the D-pad on a TV (the row follows the focus, which stays at most
// the second card from an edge). A plain wheel over the row scrolls the
// page. The row opens on «Далее», range chips jump in a long season, a
// season of a thousand episodes builds only the cards in view, «Все серии»
// opens the old grid (BUG-019: the last episode reached there too) and a
// show with many seasons reaches its last tab. With KINOLUNE_SCREENSHOTS_DIR
// set it writes the row per form factor (docs/screenshots/progress/ui-055).

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/episode_row.dart';
import 'package:kinolune/screens/show_detail_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final _now = DateTime(2026, 11, 24, 12);

const _showId = 's-long';

/// Episodes of season 1.
const _long = 200;

/// Seasons 2.. with one episode each.
const _seasons = 30;

final _show = LibraryItem(
  id: _showId,
  kind: ItemKind.show,
  title: 'Long Voyage',
  year: 2001,
  filePath: '/media/Shows/Long Voyage',
  addedAt: _now,
  matchConfidence: .97,
);

LibraryItem _ep(int s, int e) => LibraryItem(
  id: 'e-$s-$e',
  kind: ItemKind.episode,
  title: 'Episode $s-$e',
  showId: _showId,
  season: s,
  episode: e,
  filePath: '/media/Shows/Long Voyage/Season $s/Long.Voyage.S${s}E$e.mkv',
  durationMs: 24 * 60000,
  addedAt: _now,
);

Progress _watched(int e) => Progress(
  itemId: 'e-1-$e',
  positionMs: 24 * 60000,
  durationMs: 24 * 60000,
  updatedAt: _now.subtract(Duration(minutes: 1000 - e)),
  watched: true,
);

Future<AppServices> _services({
  int long = _long,
  List<Progress> progress = const [],
}) async {
  final lib = FakeLibrary.items([
    _show,
    for (var e = 1; e <= long; e++) _ep(1, e),
    for (var s = 2; s <= _seasons; s++) _ep(s, 1),
  ], progress: progress);
  final s = AppServices(
    library: lib,
    secureStore: MemorySecureStore(),
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    clock: () => _now,
  );
  await s.init();
  return s;
}

Future<void> _open(
  WidgetTester t,
  FormFactor ff,
  Size size, {
  int long = _long,
  List<Progress> progress = const [],
}) async {
  setSurface(t, size);
  await t.pumpWidget(
    testApp(
      await _services(long: long, progress: progress),
      formFactor: ff,
      section: Section.shows,
    ),
  );
  await settle(t, 10);
  await t.tap(find.text('Long Voyage').first);
  await settle(t, 10);
  expect(find.byType(ShowDetailScreen), findsOneWidget);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

Finder get _page => find
    .descendant(
      of: find.byType(ShowDetailScreen),
      matching: find.byType(Scrollable),
    )
    .first;

Finder get _row => find.byKey(episodeRowKey);

ScrollPosition _pos(WidgetTester t, Finder scrollable) =>
    t.state<ScrollableState>(scrollable).position;

ScrollPosition _rowPos(WidgetTester t) => _pos(
  t,
  find.descendant(of: _row, matching: find.byType(Scrollable)).first,
);

Finder _card(int e, {int season = 1}) =>
    find.byKey(showEpisodeKey('e-$season-$e'));

/// Episode cards built now (in view or in the cache past the edges).
int _built(WidgetTester t) => t
    .widgetList(
      find.byWidgetPredicate(
        (w) =>
            w.key is ValueKey<String> &&
            (w.key! as ValueKey<String>).value.startsWith('show-episode-e-'),
        skipOffstage: false,
      ),
    )
    .length;

/// The card is wholly inside the window.
void _expectInView(WidgetTester t, Finder card, Size size) {
  final r = t.getRect(card);
  expect(r.top, greaterThanOrEqualTo(0), reason: '$r');
  expect(r.bottom, lessThanOrEqualTo(size.height), reason: '$r');
  expect(r.left, greaterThanOrEqualTo(0), reason: '$r');
  expect(r.right, lessThanOrEqualTo(size.width), reason: '$r');
}

/// Id of the episode card holding the focus, if any.
String? _focusedEpisode() {
  final ctx = FocusManager.instance.primaryFocus?.context;
  String? id;
  ctx?.visitAncestorElements((el) {
    final k = el.widget.key;
    if (k is ValueKey<String> && k.value.startsWith('show-episode-')) {
      id = k.value.substring('show-episode-'.length);
      return false;
    }
    return true;
  });
  return id;
}

int? _focusedNumber() {
  final id = _focusedEpisode();
  return id == null ? null : int.parse(id.split('-').last);
}

/// Key of the widget around the focus whose key starts with [prefix].
String? _focusedKey(String prefix) {
  final ctx = FocusManager.instance.primaryFocus?.context;
  String? key;
  ctx?.visitAncestorElements((el) {
    final k = el.widget.key;
    if (k is ValueKey<String> && k.value.startsWith(prefix)) {
      key = k.value;
      return false;
    }
    return true;
  });
  return key;
}

/// The ‹ or › button is shown.
bool _shown(WidgetTester t, Key key) =>
    t
        .widget<AnimatedOpacity>(
          find.descendant(
            of: find.byKey(key),
            matching: find.byType(AnimatedOpacity),
          ),
        )
        .opacity ==
    1;

/// Plays the card and checks the player got its file.
Future<void> _play(WidgetTester t, int e) async {
  await t.tap(_card(e));
  await settle(t, 8);
  final player = t.widget<PlayerScreen>(find.byType(PlayerScreen));
  expect(player.request.uri, contains('Long.Voyage.S1E$e.mkv'));
}

/// Brings the episode row into the window.
Future<void> _showRow(WidgetTester t) async {
  await t.ensureVisible(find.byKey(episodeRowKey, skipOffstage: false));
  await settle(t, 4);
}

/// A mouse over the middle of the row.
Future<TestGesture> _hover(WidgetTester t) async {
  final mouse = await t.createGesture(kind: PointerDeviceKind.mouse);
  await mouse.addPointer(location: t.getCenter(_row));
  await mouse.moveTo(t.getCenter(_row));
  await t.pump();
  return mouse;
}

/// A click of the hovering [mouse] on the widget with [key].
Future<void> _click(WidgetTester t, TestGesture mouse, Key key) async {
  final at = t.getCenter(find.byKey(key));
  await mouse.moveTo(at);
  await mouse.down(at);
  await mouse.up();
  await settle(t, 4);
}

const _desktop = Size(1280, 800);
const _tv = Size(960, 540);

void main() {
  for (final (name, size) in const [
    ('phone', Size(390, 844)),
    ('tablet', Size(820, 1180)),
  ]) {
    testWidgets('$name: a swipe reaches the last of $_long episodes, it '
        'settles on a card and plays', (t) async {
      await _open(t, FormFactor.phone, size);
      await _showRow(t);
      // Built on demand: the end of the season is not there yet.
      expect(_card(_long), findsNothing);
      final extent = t.getRect(_card(2)).left - t.getRect(_card(1)).left;
      await t.fling(_row, const Offset(-200, 0), 1200);
      await settle(t, 20);
      final at = _rowPos(t).pixels;
      expect(at, greaterThan(0));
      // Settled on a card.
      expect(at % extent, closeTo(0, .5));
      for (var i = 0; i < 300 && _rowPos(t).extentAfter > 0; i++) {
        await t.fling(_row, const Offset(-300, 0), 3000);
        await t.pump(const Duration(milliseconds: 50));
      }
      await settle(t, 10);
      _expectInView(t, _card(_long), size);
      await capture(t, 'ui-055-row-end-$name');
      await _play(t, _long);
      await _close(t);
    });
  }

  testWidgets('desktop: › pages to the last episode, ‹ and › hide at the '
      'ends', (t) async {
    await _open(t, FormFactor.desktop, _desktop);
    await _showRow(t);
    final mouse = await _hover(t);
    await settle(t, 2);
    expect(_shown(t, episodeRowForwardKey), isTrue);
    expect(_shown(t, episodeRowBackKey), isFalse);
    await capture(t, 'ui-055-row-desktop-hover');
    final page = _pos(t, _page).pixels;
    final viewport = _rowPos(t).viewportDimension;
    await _click(t, mouse, episodeRowForwardKey);
    // About the visible part at a time.
    expect(_rowPos(t).pixels, greaterThan(viewport * .6));
    expect(_rowPos(t).pixels, lessThanOrEqualTo(viewport));
    expect(_shown(t, episodeRowBackKey), isTrue);
    for (var i = 0; i < 100 && _shown(t, episodeRowForwardKey); i++) {
      await _click(t, mouse, episodeRowForwardKey);
    }
    expect(_shown(t, episodeRowForwardKey), isFalse);
    expect(_rowPos(t).extentAfter, 0);
    // The page itself did not move.
    expect(_pos(t, _page).pixels, page);
    _expectInView(t, _card(_long), _desktop);
    await capture(t, 'ui-055-row-end-desktop');
    await mouse.removePointer();
    await _play(t, _long);
    await _close(t);
  });

  testWidgets('desktop: Shift+wheel moves the row, a plain wheel the page', (
    t,
  ) async {
    await _open(t, FormFactor.desktop, _desktop);
    await _showRow(t);
    final mouse = TestPointer(1, PointerDeviceKind.mouse);
    t.binding.handlePointerEvent(mouse.hover(t.getCenter(_row)));
    await t.pump();
    // A plain wheel over the row: the page scrolls, the row stays.
    final page = _pos(t, _page).pixels;
    final up = page > 0;
    t.binding.handlePointerEvent(mouse.scroll(Offset(0, up ? -120 : 120)));
    await settle(t, 3);
    expect(
      _pos(t, _page).pixels,
      up ? lessThan(page) : greaterThan(page),
    );
    expect(_rowPos(t).pixels, 0);
    await _showRow(t);
    t.binding.handlePointerEvent(mouse.hover(t.getCenter(_row)));
    final pageNow = _pos(t, _page).pixels;
    // With Shift: the row, never the page.
    await t.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    for (var i = 0; i < 400 && _rowPos(t).extentAfter > 0; i++) {
      t.binding.handlePointerEvent(mouse.scroll(const Offset(0, 300)));
      await t.pump(const Duration(milliseconds: 50));
    }
    await settle(t, 3);
    expect(_pos(t, _page).pixels, pageNow);
    expect(_rowPos(t).extentAfter, 0);
    _expectInView(t, _card(_long), _desktop);
    // A platform that turns Shift+wheel into a horizontal delta itself.
    for (var i = 0; i < 3; i++) {
      t.binding.handlePointerEvent(mouse.scroll(const Offset(-300, 0)));
      await t.pump(const Duration(milliseconds: 50));
    }
    await settle(t, 3);
    expect(_rowPos(t).extentAfter, closeTo(900, 1));
    await t.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await _close(t);
  });

  testWidgets('desktop: a horizontal trackpad swipe and a mouse drag move '
      'the row', (t) async {
    await _open(t, FormFactor.desktop, _desktop);
    await _showRow(t);
    final pad = TestPointer(2, PointerDeviceKind.trackpad);
    final at = t.getCenter(_row);
    for (var i = 0; i < 300 && _rowPos(t).extentAfter > 0; i++) {
      t.binding.handlePointerEvent(pad.panZoomStart(at));
      for (var k = 1; k <= 6; k++) {
        t.binding.handlePointerEvent(
          pad.panZoomUpdate(at, pan: Offset(-100.0 * k, 0)),
        );
        await t.pump(const Duration(milliseconds: 16));
      }
      t.binding.handlePointerEvent(pad.panZoomEnd());
      await t.pump(const Duration(milliseconds: 50));
    }
    await settle(t, 6);
    expect(_rowPos(t).extentAfter, 0);
    _expectInView(t, _card(_long), _desktop);
    // A mouse drag back to the start.
    for (var i = 0; i < 200 && _rowPos(t).pixels > 0; i++) {
      await t.drag(_row, const Offset(600, 0), kind: PointerDeviceKind.mouse);
      await t.pump(const Duration(milliseconds: 50));
    }
    await settle(t, 6);
    expect(_rowPos(t).pixels, 0);
    _expectInView(t, _card(1), _desktop);
    await _close(t);
  });

  testWidgets('desktop: the keys walk the row, a held key speeds up, End '
      'goes to the last episode', (t) async {
    await _open(t, FormFactor.desktop, _desktop);
    await _showRow(t);
    Focus.of(
      t.element(
        find
            .descendant(of: _card(1), matching: find.byType(AnimatedScale))
            .first,
      ),
    ).requestFocus();
    await settle(t, 2);
    expect(_focusedNumber(), 1);
    await pressKey(t, LogicalKeyboardKey.arrowRight, times: 6);
    expect(_focusedNumber(), 7);
    _expectInView(t, _card(7), _desktop);
    // Held: more cards than key repeats.
    await t.sendKeyDownEvent(LogicalKeyboardKey.arrowRight);
    for (var i = 0; i < 30; i++) {
      await t.sendKeyRepeatEvent(LogicalKeyboardKey.arrowRight);
      await t.pump(const Duration(milliseconds: 30));
    }
    await t.sendKeyUpEvent(LogicalKeyboardKey.arrowRight);
    await settle(t, 4);
    expect(_focusedNumber(), greaterThan(8 + 30));
    _expectInView(t, _card(_focusedNumber()!), _desktop);
    // The ‹ › buttons are there while the keyboard is in use.
    expect(_shown(t, episodeRowBackKey), isTrue);
    await pressKey(t, LogicalKeyboardKey.end);
    expect(_focusedNumber(), _long);
    _expectInView(t, _card(_long), _desktop);
    await pressKey(t, LogicalKeyboardKey.enter);
    await settle(t, 6);
    final player = t.widget<PlayerScreen>(find.byType(PlayerScreen));
    expect(player.request.uri, contains('Long.Voyage.S1E$_long.mkv'));
    await _close(t);
  });

  testWidgets('TV: the D-pad walks to the last episode, the row follows the '
      'focus', (t) async {
    await _open(t, FormFactor.tv, _tv);
    for (var i = 0; i < 20 && _focusedEpisode() == null; i++) {
      await pressKey(t, LogicalKeyboardKey.arrowDown);
    }
    expect(_focusedNumber(), 1);
    final screen = t.getRect(_row);
    while (_focusedNumber()! < _long) {
      final was = _focusedNumber()!;
      await t.sendKeyEvent(LogicalKeyboardKey.arrowRight);
      await t.pump(const Duration(milliseconds: 80));
      await t.pump(const Duration(milliseconds: 300));
      final now = _focusedNumber()!;
      expect(now, was + 1);
      _expectInView(t, _card(now), _tv);
      // At most the second card from an edge: a neighbour on each side.
      if (now < _long) {
        expect(t.getRect(_card(now + 1)).right, lessThanOrEqualTo(screen.right));
      }
      expect(t.getRect(_card(now - 1)).left, greaterThanOrEqualTo(0));
    }
    await capture(t, 'ui-055-row-end-tv');
    // ↑ in a long season: the range chip of the focused card.
    await pressKey(t, LogicalKeyboardKey.arrowUp);
    expect(_focusedKey('show-range-'), 'show-range-151');
    // Back down and OK: the last episode plays.
    await pressKey(t, LogicalKeyboardKey.arrowDown);
    expect(_focusedEpisode(), isNotNull);
    await pressKey(t, LogicalKeyboardKey.end);
    await pressKey(t, LogicalKeyboardKey.select);
    await settle(t, 6);
    final player = t.widget<PlayerScreen>(find.byType(PlayerScreen));
    expect(player.request.uri, contains('Long.Voyage.S1E$_long.mkv'));
    await _close(t);
  });

  testWidgets('TV: a held → speeds up, ↑ in a short season goes to the '
      'season tab', (t) async {
    await _open(t, FormFactor.tv, _tv);
    for (var i = 0; i < 20 && _focusedEpisode() == null; i++) {
      await pressKey(t, LogicalKeyboardKey.arrowDown);
    }
    await t.sendKeyDownEvent(LogicalKeyboardKey.arrowRight);
    for (var i = 0; i < 40; i++) {
      await t.sendKeyRepeatEvent(LogicalKeyboardKey.arrowRight);
      await t.pump(const Duration(milliseconds: 30));
    }
    await t.sendKeyUpEvent(LogicalKeyboardKey.arrowRight);
    await settle(t, 4);
    expect(_focusedNumber(), greaterThan(41));
    _expectInView(t, _card(_focusedNumber()!), _tv);

    // Season 2 (one episode): ↓ to its card, ↑ back to its tab.
    final tab = find.byKey(const ValueKey('show-season-2'));
    await settle(t, 4);
    await t.ensureVisible(tab);
    await settle(t, 4);
    await t.tap(tab);
    await settle(t, 6);
    Focus.of(
      t.element(
        find
            .descendant(
              of: _card(1, season: 2),
              matching: find.byType(AnimatedScale),
            )
            .first,
      ),
    ).requestFocus();
    await settle(t, 2);
    await pressKey(t, LogicalKeyboardKey.arrowUp);
    expect(_focusedKey('show-season-'), 'show-season-2');
    await _close(t);
  });

  for (final (name, ff, size) in const [
    ('phone', FormFactor.phone, Size(390, 844)),
    ('desktop', FormFactor.desktop, _desktop),
    ('tv', FormFactor.tv, _tv),
  ]) {
    testWidgets('$name: the row opens on «Далее»', (t) async {
      await _open(
        t,
        ff,
        size,
        progress: [for (var e = 1; e <= 120; e++) _watched(e)],
      );
      await _showRow(t);
      _expectInView(t, _card(121), size);
      expect(
        find.descendant(
          of: _card(121),
          matching: find.byKey(const ValueKey('watched-next')),
        ),
        findsOneWidget,
      );
      expect(_card(1), findsNothing);
      await capture(t, 'ui-055-row-next-$name');
      await _close(t);
    });
  }

  testWidgets('range chips: «151–200» jumps to the 151st episode', (t) async {
    await _open(t, FormFactor.desktop, _desktop);
    await _showRow(t);
    for (final label in const ['1–50', '51–100', '101–150', '151–200']) {
      expect(find.text(label), findsOneWidget);
    }
    final chip = find.byKey(episodeRangeKey(151));
    await t.tap(chip);
    await settle(t, 6);
    _expectInView(t, _card(151), _desktop);
    // The 151st is the first card wholly in the row.
    expect(t.getRect(_card(150)).left, lessThan(0));
    final handle = t.ensureSemantics();
    await t.pump();
    expect(
      t.getSemantics(find.bySemanticsLabel('Episodes 151 to 200')),
      isSemantics(isSelected: true),
    );
    handle.dispose();
    await capture(t, 'ui-055-row-ranges-desktop');
    await _close(t);
  });

  for (final (name, ff, size) in const [
    ('phone', FormFactor.phone, Size(390, 844)),
    ('desktop', FormFactor.desktop, _desktop),
    ('tv', FormFactor.tv, _tv),
  ]) {
    testWidgets('$name: a season of 1000 episodes builds only the cards in '
        'view', (t) async {
      final watch = Stopwatch()..start();
      await _open(t, ff, size, long: 1000);
      await _showRow(t);
      expect(_built(t), lessThan(16));
      expect(find.text('951–1000'), findsOneWidget);
      await t.ensureVisible(find.byKey(episodeRangeKey(951)));
      await settle(t, 2);
      await t.tap(find.byKey(episodeRangeKey(951)));
      await settle(t, 6);
      expect(_card(951), findsOneWidget);
      expect(_built(t), lessThan(16));
      // Well under the minutes a thousand built cards would take.
      expect(watch.elapsed, lessThan(const Duration(seconds: 30)));
      await _close(t);
    });
  }

  for (final (name, ff, size) in const [
    ('phone', FormFactor.phone, Size(390, 844)),
    ('tablet', FormFactor.phone, Size(820, 1180)),
    ('desktop', FormFactor.desktop, _desktop),
    ('tv', FormFactor.tv, _tv),
  ]) {
    testWidgets('$name: «Все серии» opens the grid, its last episode is '
        'reached and played (BUG-019)', (t) async {
      await _open(t, ff, size);
      await t.ensureVisible(
        find.byKey(showAllEpisodesKey, skipOffstage: false),
      );
      await settle(t, 2);
      await t.tap(find.byKey(showAllEpisodesKey));
      await settle(t, 8);
      expect(find.byType(SeasonEpisodesScreen), findsOneWidget);
      expect(find.text('Long Voyage · Season 1'), findsOneWidget);
      final grid = find
          .descendant(
            of: find.byType(SeasonEpisodesScreen),
            matching: find.byType(Scrollable),
          )
          .first;
      await capture(t, 'ui-055-all-episodes-$name');
      expect(_card(_long), findsNothing);
      for (var i = 0; i < 400 && _card(_long).evaluate().isEmpty; i++) {
        await t.drag(grid, const Offset(0, -400));
        await t.pump(const Duration(milliseconds: 50));
      }
      await settle(t, 4);
      await t.ensureVisible(_card(_long));
      await settle(t, 4);
      _expectInView(t, _card(_long), size);
      await _play(t, _long);
      await _close(t);
    });
  }

  for (final (name, ff, size) in const [
    ('phone', FormFactor.phone, Size(390, 844)),
    ('tablet', FormFactor.phone, Size(820, 1180)),
    ('desktop', FormFactor.desktop, _desktop),
    ('tv', FormFactor.tv, _tv),
  ]) {
    testWidgets('the last of $_seasons season tabs is reached on $name', (
      t,
    ) async {
      await _open(t, ff, size);
      final tab = find.byKey(const ValueKey('show-season-$_seasons'));
      await t.scrollUntilVisible(tab, 200, scrollable: _page);
      await t.ensureVisible(tab);
      await settle(t, 2);
      _expectInView(t, tab, size);
      await t.tap(tab);
      await settle(t, 6);
      expect(find.byKey(showEpisodeKey('e-$_seasons-1')), findsOneWidget);
      expect(_card(1), findsNothing);
      await _close(t);
    });
  }
}
