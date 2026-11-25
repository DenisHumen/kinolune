// SPDX-License-Identifier: MPL-2.0
//
// UI-056: total runtime and time left of a show, a season and a movie.
// The show page says «3 сезона · … · всего 2 ч · осталось 38 мин», the
// season tab «2 серии · 48 мин · осталось 38 мин», «просмотрено» once every
// episode is watched; marking a season watched (UI-049) updates the lines
// without leaving the page. A started movie: «всего 4 мин · осталось 3 мин»
// on its page and in «Продолжить просмотр». Desktop and phone, ru/en/uk.
// The core sums a show in one query (core/crates/pa-catalog
// tests/runtime_totals.rs); the demo library adds up the episodes the same
// way (ShowRuntime.of), which is checked here too.

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/movie_detail_screen.dart';
import 'package:kinolune/screens/show_detail_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/format.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final _now = DateTime(2026, 11, 25, 20);
const _min = 60000;
const _showId = 's-kaiju';

final _show = LibraryItem(
  id: _showId,
  kind: ItemKind.show,
  title: 'Kaiju Patrol',
  year: 2024,
  filePath: '/media/Shows/Kaiju Patrol',
  addedAt: _now,
  matchConfidence: .96,
);

LibraryItem _ep(int s, int e, {int? minutes = 24}) => LibraryItem(
  id: 'e-$s-$e',
  kind: ItemKind.episode,
  title: 'Episode $s.$e',
  showId: _showId,
  season: s,
  episode: e,
  filePath:
      '/media/Shows/Kaiju Patrol/Season $s/Kaiju.Patrol.S0${s}E0$e.1080p.mkv',
  durationMs: minutes == null ? null : minutes * _min,
  addedAt: _now,
);

/// S1: 3 × 24 min, all watched. S2: 2 × 24 min, E1 at 10 min. A special
/// of 24 min, not in the total of the show.
final _episodes = [
  _ep(1, 1),
  _ep(1, 2),
  _ep(1, 3),
  _ep(2, 1),
  _ep(2, 2),
  _ep(0, 1),
];

Progress _watched(String id) => Progress(
  itemId: id,
  positionMs: 24 * _min,
  durationMs: 24 * _min,
  updatedAt: _now.subtract(const Duration(days: 1)),
  watched: true,
);

final _progress = [
  _watched('e-1-1'),
  _watched('e-1-2'),
  _watched('e-1-3'),
  Progress(
    itemId: 'e-2-1',
    positionMs: 10 * _min,
    durationMs: 24 * _min,
    updatedAt: _now.subtract(const Duration(minutes: 5)),
  ),
];

final _movie = LibraryItem(
  id: 'm-charge',
  kind: ItemKind.movie,
  title: 'Charge',
  year: 2022,
  filePath: '/media/Movies/Charge (2022)/Charge (2022).mkv',
  durationMs: 148 * _min,
  addedAt: _now,
  matchConfidence: .95,
);

Future<(AppServices, FakeLibrary)> _services(Locale locale) async {
  final lib = FakeLibrary.items(
    [_show, ..._episodes, _movie],
    progress: [
      ..._progress,
      Progress(
        itemId: 'm-charge',
        positionMs: 108 * _min,
        durationMs: 148 * _min,
        updatedAt: _now.subtract(const Duration(minutes: 2)),
      ),
    ],
  );
  final s = AppServices(
    library: lib,
    secureStore: MemorySecureStore(),
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    clock: () => _now,
  );
  await s.init();
  s.locale.value = locale;
  return (s, lib);
}

String _text(WidgetTester t, String key) =>
    t.widget<Text>(find.byKey(ValueKey(key))).data!;

/// Expected lines per locale: (show meta part, season 2, season 1 watched,
/// movie).
const _expected = {
  'ru': (
    'всего 2 ч · осталось 38 мин',
    '2 серии · 48 мин · осталось 38 мин',
    '3 серии · 1 ч 12 мин · просмотрено',
    'всего 2 ч 28 мин · осталось 40 мин',
  ),
  'en': (
    '2 h total · 38 min left',
    '2 episodes · 48 min · 38 min left',
    '3 episodes · 1 h 12 min · watched',
    '2 h 28 min total · 40 min left',
  ),
  'uk': (
    'усього 2 год · залишилося 38 хв',
    '2 серії · 48 хв · залишилося 38 хв',
    '3 серії · 1 год 12 хв · переглянуто',
    'усього 2 год 28 хв · залишилося 40 хв',
  ),
};

void main() {
  group('format', () {
    final ru = lookupAppLocalizations(const Locale('ru'));
    final en = lookupAppLocalizations(const Locale('en'));

    test('spans: minutes, hours and minutes, whole hours, from 100 h', () {
      expect(formatSpan(ru, 45 * _min), '45 мин');
      expect(formatSpan(ru, 65 * _min), '1 ч 5 мин');
      expect(formatSpan(ru, 120 * _min), '2 ч');
      expect(formatSpan(ru, (130 * 60 + 25) * _min), '130 ч');
      expect(formatSpan(en, 30 * 1000), '1 min');
      expect(formatSpan(ru, 45 * _min, approx: true), '≈45 мин');
    });

    test('show line: total, left, watched, nothing started', () {
      const t = RuntimeTotal(
        episodes: 30,
        watched: 12,
        totalMs: (21 * 60 + 40) * _min,
        remainingMs: (12 * 60 + 5) * _min,
      );
      expect(formatShowRuntime(ru, t), 'всего 21 ч 40 мин · осталось 12 ч 5 мин');
      const done = RuntimeTotal(
        episodes: 2,
        watched: 2,
        totalMs: 50 * _min,
        estimated: true,
      );
      expect(formatShowRuntime(ru, done), 'всего ≈50 мин · просмотрено');
      const fresh = RuntimeTotal(
        episodes: 2,
        totalMs: 50 * _min,
        remainingMs: 50 * _min,
      );
      expect(formatShowRuntime(ru, fresh), 'всего 50 мин');
      expect(formatShowRuntime(ru, RuntimeTotal.empty), '');
    });

    test('the demo library sums like the core', () {
      final eps = orderEpisodes(
        [..._episodes, _ep(2, 3, minutes: null)],
        progress: {for (final p in _progress) p.itemId: p},
      );
      final r = eps.runtime;
      expect(
        r.show,
        const RuntimeTotal(
          episodes: 6,
          watched: 3,
          totalMs: 120 * _min,
          remainingMs: 38 * _min,
          estimated: true,
          remainingEstimated: true,
        ),
      );
      expect(r.season(1).allWatched, isTrue);
      expect(r.season(0).totalMs, 24 * _min);
      expect(r.season(2).remainingMs, 38 * _min);
    });
  });

  for (final (name, ff, size) in [
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('phone', FormFactor.phone, const Size(390, 844)),
  ]) {
    for (final locale in ['ru', 'en', 'uk']) {
      final (show, season2, season1, movie) = _expected[locale]!;

      testWidgets('show page and season tab on $name ($locale)', (t) async {
        final (s, lib) = await _services(Locale(locale));
        setSurface(t, size);
        await t.pumpWidget(testApp(s, formFactor: ff, section: Section.shows));
        await settle(t, 10);
        await t.tap(find.text('Kaiju Patrol').first);
        await settle(t, 10);
        expect(find.byType(ShowDetailScreen), findsOneWidget);

        expect(_text(t, 'show-meta'), contains(show));
        // The tab of the episode to continue: season 2.
        expect(_text(t, 'show-season-runtime'), season2);

        await t.ensureVisible(find.byKey(const ValueKey('show-season-1')));
        await settle(t, 2);
        await t.tap(find.byKey(const ValueKey('show-season-1')));
        await settle(t, 6);
        expect(_text(t, 'show-season-runtime'), season1);

        // «Отметить сезон просмотренным» (UI-049): season 2 too; the page
        // updates without being opened again.
        await lib.setWatchedMany(['e-2-1', 'e-2-2'], watched: true);
        await settle(t, 6);
        final l = lookupAppLocalizations(Locale(locale));
        expect(_text(t, 'show-meta'), contains(l.formatAllWatched));
        expect(_text(t, 'show-meta'), isNot(contains(l.formatLeft(''))));
        if (locale == 'ru') await capture(t, 'ui-056-show-$name');

        await t.pumpWidget(const SizedBox());
        await settle(t, 2);
      });

      testWidgets('started movie on $name ($locale)', (t) async {
        final (s, _) = await _services(Locale(locale));
        setSurface(t, size);
        // «Продолжить просмотр» on Home.
        await t.pumpWidget(testApp(s, formFactor: ff));
        await settle(t, 10);
        expect(find.textContaining(movie), findsWidgets);
        await t.pumpWidget(const SizedBox());
        await settle(t, 2);

        await t.pumpWidget(
          testApp(s, formFactor: ff, section: Section.movies),
        );
        await settle(t, 10);
        await t.tap(find.text('Charge').first);
        await settle(t, 10);
        expect(find.byType(MovieDetailScreen), findsOneWidget);
        expect(_text(t, 'detail-meta'), contains(movie));
        if (locale == 'ru') await capture(t, 'ui-056-movie-$name');

        await t.pumpWidget(const SizedBox());
        await settle(t, 2);
      });
    }
  }
}
