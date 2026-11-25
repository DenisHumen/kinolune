// SPDX-License-Identifier: MPL-2.0
//
// BUG-021: a movie of two parts (`part1` + `part2`, 2 minutes each) plays
// as one 4-minute timeline (LIB-020), and the core now reports the sum of
// the parts as the item, version and tech-info duration. The movie card,
// the movie page, «Техническая информация» and the remaining time show
// those 4 minutes, not the 2 of the first part.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/screens/movie_detail_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final _now = DateTime(2026, 11, 21, 20);

const _part1 = '/media/Movies/Charge (2022)/Charge (2022) - part1.mkv';
const _min = 60000;

// What the bridge returns for the two-part movie: the item points at part 1
// and carries the runtime of both parts; there is no provider runtime.
final _charge = LibraryItem(
  id: 'm-charge',
  kind: ItemKind.movie,
  title: 'Charge',
  year: 2022,
  filePath: _part1,
  durationMs: 4 * _min,
  addedAt: _now,
  matchConfidence: .95,
);

const _details = ItemDetails(
  tech: TechInfo(
    container: 'matroska,webm',
    sizeBytes: 230000,
    durationMs: 4 * _min,
    width: 1920,
    height: 1080,
    filePath: _part1,
  ),
);

Future<AppServices> _services({List<Progress> progress = const []}) async {
  final s = AppServices(
    library: FakeLibrary.items(
      [_charge],
      progress: progress,
      details: {'m-charge': _details},
    ),
    secureStore: MemorySecureStore(),
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    clock: () => _now,
  );
  await s.init();
  return s;
}

Finder _inDetail(Finder f) =>
    find.descendant(of: find.byType(MovieDetailScreen), matching: f);

void main() {
  testWidgets('card, page and tech info show the runtime of both parts', (
    t,
  ) async {
    final s = await _services();
    setSurface(t, const Size(1280, 800));
    // Home: the top shelf of the movie, «2022 · 4 мин».
    await t.pumpWidget(testApp(s));
    await settle(t, 10);
    expect(find.textContaining('2022  ·  4 min'), findsWidgets);
    expect(find.textContaining('2 min'), findsNothing);
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);

    await t.pumpWidget(testApp(s, section: Section.movies));
    await settle(t, 10);
    await t.tap(find.text('Charge').first);
    await settle(t, 10);
    expect(find.byType(MovieDetailScreen), findsOneWidget);
    // The movie page: «2022 · 4 мин».
    expect(_inDetail(find.textContaining('2022  ·  4 min')), findsOneWidget);

    // «Техническая информация» → «Длительность 4 мин».
    await t.scrollUntilVisible(
      find.byKey(const ValueKey('detail-tech')),
      300,
      scrollable: _inDetail(find.byType(Scrollable)).first,
    );
    await settle(t, 4);
    final tech = find.byKey(const ValueKey('detail-tech'));
    expect(
      find.descendant(of: tech, matching: find.text('Duration')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: tech, matching: find.text('4 min')),
      findsOneWidget,
    );
    expect(_inDetail(find.textContaining('2 min')), findsNothing);

    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  testWidgets('the remaining time counts the second part too', (t) async {
    // Stopped at 1:00 of the 4:00 timeline: 3 minutes left, not 1.
    final s = await _services(
      progress: [
        Progress(
          itemId: 'm-charge',
          positionMs: 1 * _min,
          durationMs: 4 * _min,
          updatedAt: _now.subtract(const Duration(minutes: 5)),
        ),
      ],
    );
    setSurface(t, const Size(1280, 800));
    await t.pumpWidget(testApp(s, section: Section.movies));
    await settle(t, 10);
    await t.tap(find.text('Charge').first);
    await settle(t, 10);
    expect(_inDetail(find.textContaining('3 min left')), findsOneWidget);

    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });
}
