// SPDX-License-Identifier: MPL-2.0
//
// UI-049: watched marks on the show page. One episode from its «⋯» and by
// right click, «Отметить все предыдущие как просмотренные», a whole season
// from its tab, the whole show from «Ещё», «Отменить» putting back the
// marks and positions, the server getting every change, and the remote:
// a held OK on an episode opens its menu and focus comes back to the card.

import 'package:flutter/gestures.dart' show kSecondaryButton;
import 'package:flutter/services.dart' show LogicalKeyboardKey;
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/episode_marks.dart';
import 'package:kinolune/screens/show_detail_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/focusable.dart' show kLongSelect;
import 'package:kinolune/widgets/watched_art.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/fake_jellyfin.dart';
import 'support/test_app.dart';

// Before any real clock: a mark made in the test is always the newest
// change.
final _now = DateTime(2024, 1, 10, 20);

const _showId = 's-harbor';
const _minute = 60000;

final _show = LibraryItem(
  id: _showId,
  kind: ItemKind.show,
  title: 'Harbor Lights',
  year: 2025,
  filePath: '/media/Shows/Harbor Lights',
  addedAt: _now,
  matchConfidence: .97,
);

LibraryItem _ep(int s, int e) => LibraryItem(
  id: 'e-$s-$e',
  kind: ItemKind.episode,
  title: 'Episode $s.$e',
  showId: _showId,
  season: s,
  episode: e,
  filePath: '/media/Shows/Harbor Lights/Season $s/Harbor.S0${s}E0$e.mkv',
  durationMs: 40 * _minute,
  addedAt: _now,
);

final _episodes = [
  _ep(1, 1),
  _ep(1, 2),
  _ep(1, 3),
  _ep(1, 4),
  _ep(2, 1),
  _ep(2, 2),
];

/// S1E1 watched, S1E2 started 10 minutes in, the rest not started.
final _progress = [
  Progress(
    itemId: 'e-1-1',
    positionMs: 40 * _minute,
    durationMs: 40 * _minute,
    updatedAt: _now.subtract(const Duration(days: 1)),
    watched: true,
  ),
  Progress(
    itemId: 'e-1-2',
    positionMs: 10 * _minute,
    durationMs: 40 * _minute,
    updatedAt: _now.subtract(const Duration(hours: 1)),
  ),
];

Future<(AppServices, FakeLibrary, FakeJellyfinLink)> _services() async {
  final lib = FakeLibrary.items([_show, ..._episodes], progress: _progress);
  final server = FakeJellyfinLink();
  final s = AppServices(
    library: lib,
    secureStore: MemorySecureStore(),
    jellyfin: server,
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    clock: () => _now,
  );
  await s.init();
  return (s, lib, server);
}

Future<void> _open(
  WidgetTester t,
  AppServices s, {
  FormFactor ff = FormFactor.desktop,
  Size size = const Size(1280, 800),
}) async {
  setSurface(t, size);
  await t.pumpWidget(testApp(s, formFactor: ff, section: Section.shows));
  await settle(t, 10);
  await t.tap(find.text('Harbor Lights').first);
  await settle(t, 10);
  expect(find.byType(ShowDetailScreen), findsOneWidget);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

Future<Progress?> _p(FakeLibrary lib, String id) => lib.progress(id);

Future<bool> _watched(FakeLibrary lib, String id) async =>
    (await _p(lib, id))?.watched ?? false;

/// The watch state the still of an episode card shows (UI-037).
WatchArt _still(WidgetTester t, String id) => t
    .widget<WatchedStill>(
      find.descendant(
        of: find.byKey(showEpisodeKey(id)),
        matching: find.byType(WatchedStill),
      ),
    )
    .state;

Future<void> _tapKey(WidgetTester t, Key key) async {
  await t.ensureVisible(find.byKey(key));
  await settle(t, 2);
  await t.tap(find.byKey(key));
  await settle(t, 6);
}

Future<void> _menuOf(WidgetTester t, String id) =>
    _tapKey(t, showEpisodeMenuKey(id));

Future<void> _choose(WidgetTester t, EpisodeMarkAction a) async {
  await t.tap(find.byKey(ValueKey('mark-menu-${a.name}')));
  await settle(t, 6);
}

void main() {
  testWidgets('one episode: «⋯» marks it watched, right click unmarks it', (
    t,
  ) async {
    final (s, lib, server) = await _services();
    await _open(t, s);
    expect(_still(t, 'e-1-3'), WatchArt.unwatched);

    await _menuOf(t, 'e-1-3');
    expect(find.byKey(const ValueKey('mark-menu-watched')), findsOneWidget);
    expect(find.byKey(const ValueKey('mark-menu-unwatched')), findsNothing);
    expect(find.byKey(const ValueKey('mark-menu-previous')), findsOneWidget);
    await _choose(t, EpisodeMarkAction.watched);
    expect(await _watched(lib, 'e-1-3'), isTrue);
    // The still follows at once; one episode needs no «Отменить».
    expect(_still(t, 'e-1-3'), WatchArt.watched);
    expect(find.byKey(const ValueKey('snack-undo')), findsNothing);
    expect(server.calls, contains('watched e-1-3 true'));

    await t.tap(find.byKey(showEpisodeKey('e-1-3')), buttons: kSecondaryButton);
    await settle(t, 6);
    expect(find.byKey(const ValueKey('mark-menu-watched')), findsNothing);
    await _choose(t, EpisodeMarkAction.unwatched);
    expect(await _watched(lib, 'e-1-3'), isFalse);
    expect(_still(t, 'e-1-3'), WatchArt.unwatched);
    expect(server.calls, contains('watched e-1-3 false'));
    await _close(t);
  });

  testWidgets('a started episode can be marked watched or cleared', (t) async {
    final (s, lib, _) = await _services();
    await _open(t, s);
    await _menuOf(t, 'e-1-2');
    expect(find.byKey(const ValueKey('mark-menu-watched')), findsOneWidget);
    expect(find.byKey(const ValueKey('mark-menu-unwatched')), findsOneWidget);
    await _choose(t, EpisodeMarkAction.unwatched);
    final p = await _p(lib, 'e-1-2');
    expect((p?.positionMs, p?.watched), (0, false));
    await _close(t);
  });

  testWidgets('«все предыдущие» marks the earlier ones and «Отменить» puts '
      'them back with their positions', (t) async {
    final (s, lib, server) = await _services();
    await _open(t, s);
    await _menuOf(t, 'e-1-4');
    await _choose(t, EpisodeMarkAction.previous);
    // S1E1 was watched already; S1E2 and S1E3 now are, S1E4 is not.
    for (final id in ['e-1-1', 'e-1-2', 'e-1-3']) {
      expect(await _watched(lib, id), isTrue, reason: id);
      expect(_still(t, id), WatchArt.watched, reason: id);
    }
    expect(await _watched(lib, 'e-1-4'), isFalse);
    expect(find.text('2 episodes marked as watched'), findsOneWidget);
    expect(server.calls, isNot(contains('watched e-1-1 true')));

    await t.tap(find.byKey(const ValueKey('snack-undo')));
    await settle(t, 6);
    final e2 = await _p(lib, 'e-1-2');
    expect((e2?.positionMs, e2?.watched), (10 * _minute, false));
    expect(_still(t, 'e-1-2'), WatchArt.started);
    expect(await _watched(lib, 'e-1-3'), isFalse);
    expect(await _watched(lib, 'e-1-1'), isTrue);
    expect(server.calls, contains('watched e-1-3 false'));
    await _close(t);
  });

  testWidgets('a whole season from its tab', (t) async {
    final (s, lib, _) = await _services();
    await _open(t, s);
    await t.ensureVisible(find.byKey(const ValueKey('show-season-2')));
    await settle(t, 2);
    await t.longPress(find.byKey(const ValueKey('show-season-2')));
    await settle(t, 6);
    expect(
      find.byKey(const ValueKey('mark-menu-seasonUnwatched')),
      findsNothing,
    );
    await _choose(t, EpisodeMarkAction.seasonWatched);
    expect(await _watched(lib, 'e-2-1'), isTrue);
    expect(await _watched(lib, 'e-2-2'), isTrue);
    // Season 1 is untouched.
    expect(await _watched(lib, 'e-1-3'), isFalse);
    expect(find.text('2 episodes marked as watched'), findsOneWidget);

    await t.tap(find.byKey(const ValueKey('snack-undo')));
    await settle(t, 6);
    expect(await _watched(lib, 'e-2-1'), isFalse);
    expect(await _watched(lib, 'e-2-2'), isFalse);
    await _close(t);
  });

  testWidgets('the whole show from «Ещё», unwatched and back', (t) async {
    final (s, lib, server) = await _services();
    await _open(t, s);
    await _tapKey(t, showMoreKey);
    expect(find.byKey(const ValueKey('mark-menu-showWatched')), findsOneWidget);
    await _choose(t, EpisodeMarkAction.showWatched);
    for (final e in _episodes) {
      expect(await _watched(lib, e.id), isTrue, reason: e.id);
    }
    expect(find.text('5 episodes marked as watched'), findsOneWidget);
    // The snack bar goes away by itself; the marks stay.
    await settle(t, 90);
    expect(find.byKey(const ValueKey('snack-undo')), findsNothing);

    await _tapKey(t, showMoreKey);
    expect(find.byKey(const ValueKey('mark-menu-showWatched')), findsNothing);
    await _choose(t, EpisodeMarkAction.showUnwatched);
    for (final e in _episodes) {
      final p = await _p(lib, e.id);
      expect((p?.positionMs, p?.watched), (0, false), reason: e.id);
    }
    expect(find.text('6 episodes marked as unwatched'), findsOneWidget);
    expect(server.calls, contains('watched e-2-2 false'));

    // «Отменить»: everything watched again.
    await t.tap(find.byKey(const ValueKey('snack-undo')));
    await settle(t, 6);
    for (final e in _episodes) {
      expect(await _watched(lib, e.id), isTrue, reason: e.id);
    }
    await _close(t);
  });

  testWidgets('TV: a held OK on an episode opens its menu, focus comes back '
      'to the card', (t) async {
    final (s, lib, _) = await _services();
    await _open(t, s, ff: FormFactor.tv, size: const Size(960, 540));
    final card = find.byKey(showEpisodeKey('e-1-3'));
    await t.ensureVisible(card);
    await settle(t, 2);
    // No «⋯» on TV: the remote holds OK.
    expect(find.byKey(showEpisodeMenuKey('e-1-3')), findsNothing);
    Focus.of(
      t.element(
        find.descendant(of: card, matching: find.byType(AnimatedScale)).first,
      ),
    ).requestFocus();
    await settle(t, 2);

    await t.sendKeyDownEvent(LogicalKeyboardKey.select);
    await t.pump(kLongSelect + const Duration(milliseconds: 100));
    await t.sendKeyUpEvent(LogicalKeyboardKey.select);
    await settle(t, 6);
    expect(find.byKey(const ValueKey('mark-menu-watched')), findsOneWidget);
    expect(find.byKey(const ValueKey('mark-menu-previous')), findsOneWidget);
    // Nothing played: the hold opened the menu instead.
    expect(await _watched(lib, 'e-1-3'), isFalse);

    await _choose(t, EpisodeMarkAction.watched);
    expect(await _watched(lib, 'e-1-3'), isTrue);
    final cardElement = t.element(card);
    var onCard = false;
    (FocusManager.instance.primaryFocus?.context as Element?)
        ?.visitAncestorElements((e) {
          if (e == cardElement) onCard = true;
          return !onCard;
        });
    expect(onCard, isTrue, reason: 'focus is back on the card');
    await _close(t);
  });
}
