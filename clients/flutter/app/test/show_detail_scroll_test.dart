// SPDX-License-Identifier: MPL-2.0
//
// BUG-019: a show with a long season. The page scrolls down to its last
// episode on a phone, a tablet, a desktop (drag and mouse wheel) and a TV
// (D-pad focus walks the cards and keeps the focused one in view); the
// episodes are built lazily; a show with many seasons reaches its last tab.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/platform/form_factor.dart';
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

Future<AppServices> _services() async {
  final lib = FakeLibrary.items([
    _show,
    for (var e = 1; e <= _long; e++) _ep(1, e),
    for (var s = 2; s <= _seasons; s++) _ep(s, 1),
  ]);
  final s = AppServices(
    library: lib,
    secureStore: MemorySecureStore(),
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    clock: () => _now,
  );
  await s.init();
  return s;
}

Future<void> _open(WidgetTester t, FormFactor ff, Size size) async {
  setSurface(t, size);
  await t.pumpWidget(
    testApp(await _services(), formFactor: ff, section: Section.shows),
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

Finder _card(int e) => find.byKey(showEpisodeKey('e-1-$e'));

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

const _sizes = <(String, FormFactor, Size)>[
  ('phone', FormFactor.phone, Size(390, 844)),
  ('tablet', FormFactor.phone, Size(820, 1180)),
  ('desktop', FormFactor.desktop, Size(1280, 800)),
  ('tv', FormFactor.tv, Size(960, 540)),
];

void main() {
  for (final (name, ff, size) in _sizes) {
    testWidgets('the last of $_long episodes is reached and played on $name', (
      t,
    ) async {
      await _open(t, ff, size);
      expect(_card(1), findsOneWidget);
      // Built on demand: the end of the season is not there yet.
      expect(_card(_long), findsNothing);
      await t.scrollUntilVisible(_card(_long), 400, scrollable: _page);
      await settle(t, 4);
      await t.ensureVisible(_card(_long));
      await settle(t, 4);
      _expectInView(t, _card(_long), size);
      await t.tap(_card(_long));
      await settle(t, 8);
      final player = t.widget<PlayerScreen>(find.byType(PlayerScreen));
      expect(player.request.uri, contains('Long.Voyage.S1E$_long.mkv'));
      await _close(t);
    });
  }

  testWidgets('the mouse wheel scrolls the page to the last episode', (
    t,
  ) async {
    const size = Size(1280, 800);
    await _open(t, FormFactor.desktop, size);
    final mouse = TestPointer(1, PointerDeviceKind.mouse);
    // Over the episode grid, where a row of cards used to take the wheel.
    final at = t.getCenter(_card(1));
    t.binding.handlePointerEvent(mouse.hover(at));
    for (var i = 0; i < 200 && _card(_long).evaluate().isEmpty; i++) {
      t.binding.handlePointerEvent(mouse.scroll(const Offset(0, 300)));
      await t.pump(const Duration(milliseconds: 50));
    }
    // The rest of the way: the end of the page.
    for (var i = 0; i < 20; i++) {
      t.binding.handlePointerEvent(mouse.scroll(const Offset(0, 300)));
      await t.pump(const Duration(milliseconds: 50));
    }
    await settle(t, 4);
    _expectInView(t, _card(_long), size);
    await _close(t);
  });

  testWidgets('the D-pad walks to the last episode and keeps it in view', (
    t,
  ) async {
    const size = Size(960, 540);
    await _open(t, FormFactor.tv, size);
    String? last;
    var stuck = 0;
    for (var i = 0; i < 400 && _focusedEpisode() != 'e-1-$_long'; i++) {
      await t.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await t.pump(const Duration(milliseconds: 80));
      await t.pump(const Duration(milliseconds: 80));
      final now = _focusedEpisode();
      if (now != null && now == last) {
        // The last row: across to the end of it.
        if (++stuck > 1) {
          await pressKey(t, LogicalKeyboardKey.arrowRight, times: 4);
        }
      }
      last = _focusedEpisode();
    }
    await settle(t, 6);
    expect(_focusedEpisode(), 'e-1-$_long');
    _expectInView(t, _card(_long), size);
    await _close(t);
  });

  for (final (name, ff, size) in _sizes) {
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
