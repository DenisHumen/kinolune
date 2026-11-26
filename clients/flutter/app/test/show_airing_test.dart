// SPDX-License-Identifier: MPL-2.0
//
// UI-057: ongoing shows say when the next episode airs. The show page:
// «Серия 4 — вт, 1 дек.», anime with a countdown to the hour, «Сериал
// завершён», offline «нет сети · данные от …». The row of the season ends
// with a card of the episode still to come: no frame, the date, it does not
// open (tap, OK), the remote can focus it and then shows the date. The
// library posters: «Новая серия во вт», «Сериал завершён». Phone, desktop
// and TV in ru/en/uk. The core side (TMDB, AniList, Shikimori answers,
// offline, a file arriving) is core/crates/pa-catalog/tests/airing.rs.
// With KINOLUNE_SCREENSHOTS_DIR set it writes docs/screenshots/progress/
// ui-057 pictures.

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:intl/intl.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/library/show_airing.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/episode_row.dart';
import 'package:kinolune/screens/show_detail_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

/// Thursday, 26 November 2026, noon (device time).
final _now = DateTime(2026, 11, 26, 12);

const _showId = 's-harbor';
const _animeId = 's-comet';
const _endedId = 's-quiet';

LibraryItem _show(String id, String title) => LibraryItem(
  id: id,
  kind: ItemKind.show,
  title: title,
  year: 2025,
  filePath: '/media/Shows/$title',
  addedAt: _now,
  matchConfidence: .97,
);

LibraryItem _ep(String show, String title, int s, int e) => LibraryItem(
  id: '$show-e-$s-$e',
  kind: ItemKind.episode,
  title: 'Episode $e',
  showId: show,
  season: s,
  episode: e,
  filePath: '/media/Shows/$title/Season $s/$title.S${s}E$e.mkv',
  durationMs: 42 * 60000,
  addedAt: _now,
);

/// Episode 4 on Tuesday 1 December (TMDB: the date only).
final _harbor = ShowAiring(
  showId: _showId,
  status: AiringStatus.airing,
  next: NextAiring(season: 1, episode: 4, date: DateTime(2026, 12, 1)),
  source: 'tmdb',
  checkedAt: _now.subtract(const Duration(hours: 2)),
);

/// Anime: episode 8 in 2 days 5 hours (AniList: the exact moment).
final _comet = ShowAiring(
  showId: _animeId,
  status: AiringStatus.airing,
  next: NextAiring(
    episode: 8,
    at: _now.add(const Duration(days: 2, hours: 5)).toUtc(),
  ),
  source: 'anilist',
  checkedAt: _now,
);

final _quiet = ShowAiring(
  showId: _endedId,
  status: AiringStatus.ended,
  source: 'tmdb',
  checkedAt: _now,
);

Future<(AppServices, FakeLibrary)> _services(Locale locale) async {
  final lib = FakeLibrary.items([
    _show(_showId, 'Harbor Lights'),
    for (var e = 1; e <= 3; e++) _ep(_showId, 'Harbor Lights', 1, e),
    _show(_animeId, 'Comet Hunters'),
    for (var e = 1; e <= 7; e++) _ep(_animeId, 'Comet Hunters', 1, e),
    _show(_endedId, 'Quiet Valley'),
    for (var e = 1; e <= 2; e++) _ep(_endedId, 'Quiet Valley', 1, e),
  ]);
  lib
    ..setAiring(_harbor)
    ..setAiring(_comet)
    ..setAiring(_quiet);
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

Future<(AppServices, FakeLibrary)> _openShow(
  WidgetTester t,
  FormFactor ff,
  Size size,
  Locale locale,
  String title,
) async {
  final (s, lib) = await _services(locale);
  setSurface(t, size);
  await t.pumpWidget(testApp(s, formFactor: ff, section: Section.shows));
  await settle(t, 10);
  await t.tap(find.text(title).first);
  await settle(t, 10);
  expect(find.byType(ShowDetailScreen), findsOneWidget);
  return (s, lib);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

String _text(WidgetTester t, Key key) => t.widget<Text>(find.byKey(key)).data!;

/// The primary focus is inside the card of the episode still to come.
bool _upcomingFocused() {
  final context = primaryFocus?.context;
  if (context == null) return false;
  var found = false;
  context.visitAncestorElements((e) {
    found = e.widget.key == showUpcomingKey;
    return !found;
  });
  return found;
}

Future<void> _showRowEnd(WidgetTester t) async {
  await t.ensureVisible(find.byKey(episodeRowKey, skipOffstage: false));
  await settle(t, 4);
  await t.scrollUntilVisible(
    find.byKey(showUpcomingKey),
    300,
    scrollable: find
        .descendant(
          of: find.byKey(episodeRowKey),
          matching: find.byType(Scrollable),
        )
        .first,
  );
  await settle(t, 4);
}

void main() {
  setUpAll(loadAppFonts);

  group('formatting', () {
    final l = lookupAppLocalizations(const Locale('ru'));
    test('a date, today, tomorrow; anime with time and countdown', () {
      expect(
        airingLine(l, 'ru', _harbor, _now),
        l.airingNextOn(4, DateFormat.MMMEd('ru').format(DateTime(2026, 12, 1))),
      );
      final today = ShowAiring(
        showId: 'x',
        status: AiringStatus.airing,
        next: NextAiring(episode: 2, date: DateTime(2026, 11, 26)),
        checkedAt: _now,
      );
      expect(airingLine(l, 'ru', today, _now), 'Серия 2 — сегодня');
      expect(airingBadge(l, 'ru', today, _now), 'Новая серия сегодня');
      expect(airingLine(l, 'ru', _comet, _now), endsWith('· через 2 д 5 ч'));
      expect(
        airingCountdown(l, _now.add(const Duration(minutes: 20)), _now),
        'меньше часа',
      );
      expect(
        airingCountdown(l, _now.add(const Duration(hours: 5)), _now),
        '5 ч',
      );
    });

    test('poster labels by weekday, a far date, the end', () {
      // Tuesday: «во вт».
      expect(airingBadge(l, 'ru', _harbor, _now), 'Новая серия во вт');
      final far = ShowAiring(
        showId: 'x',
        status: AiringStatus.airing,
        next: NextAiring(episode: 1, season: 2, date: DateTime(2027, 1, 9)),
        checkedAt: _now,
      );
      expect(airingBadge(l, 'ru', far, _now), startsWith('Новая серия 9'));
      expect(airingBadge(l, 'ru', _quiet, _now), 'Сериал завершён');
      // On a break, or the date passed: no label.
      final hiatus = ShowAiring(
        showId: 'x',
        status: AiringStatus.hiatus,
        checkedAt: _now,
      );
      expect(airingBadge(l, 'ru', hiatus, _now), isNull);
      expect(airingLine(l, 'ru', hiatus, _now), l.airingNoDate);
      expect(airingBadge(l, 'ru', _harbor, DateTime(2026, 12, 2, 9)), isNull);
    });

    test('offline: the old value, marked', () {
      final old = ShowAiring(
        showId: _showId,
        status: AiringStatus.airing,
        next: _harbor.next,
        checkedAt: DateTime(2026, 11, 24, 10),
        offline: true,
      );
      expect(airingLine(l, 'ru', old, _now), contains('нет сети · данные от'));
      expect(airingLine(l, 'ru', old, _now), startsWith('Серия 4 —'));
    });
  });

  for (final (name, ff, size) in [
    ('phone', FormFactor.phone, const Size(390, 844)),
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('tv', FormFactor.tv, const Size(960, 540)),
  ]) {
    for (final locale in ['ru', 'en', 'uk']) {
      testWidgets('show page and the card still to come on $name ($locale)', (
        t,
      ) async {
        final loc = Locale(locale);
        final l = lookupAppLocalizations(loc);
        final (_, lib) = await _openShow(t, ff, size, loc, 'Harbor Lights');
        // Asked from the providers when the page opened.
        expect(lib.airingRefreshes, contains(_showId));
        final date = DateFormat.MMMEd(locale).format(DateTime(2026, 12, 1));
        expect(_text(t, showAiringKey), l.airingNextOn(4, date));

        // The card is the last one of the row: no frame, the date.
        await _showRowEnd(t);
        final card = find.byKey(showUpcomingKey);
        expect(card, findsOneWidget);
        expect(
          find.descendant(of: card, matching: find.text(date)),
          findsOneWidget,
        );
        expect(
          find.descendant(of: card, matching: find.text(l.airingCardTitle)),
          findsOneWidget,
        );
        expect(
          find.descendant(of: card, matching: find.byType(Image)),
          findsNothing,
        );
        if (locale == 'ru') await capture(t, 'ui-057-show-$name');

        // It does not open.
        await t.tap(card);
        await settle(t, 6);
        expect(find.byType(PlayerScreen), findsNothing);
        expect(find.byType(ShowDetailScreen), findsOneWidget);

        if (ff != FormFactor.phone) {
          // The keyboard / the remote reaches it; OK does nothing; the
          // focused card shows the date.
          for (var i = 0; i < 20 && !_upcomingFocused(); i++) {
            await pressKey(t, LogicalKeyboardKey.arrowDown);
            if (_upcomingFocused()) break;
            await pressKey(t, LogicalKeyboardKey.end);
          }
          expect(_upcomingFocused(), isTrue);
          expect(
            find.descendant(
              of: card,
              matching: find.byKey(const ValueKey('show-upcoming-date')),
            ),
            findsOneWidget,
          );
          expect(_text(t, const ValueKey('show-upcoming-date')), date);
          expect(
            t.getSemantics(card).label,
            contains(l.airingCardLabel(l.showEpisodeCode(1, 4), date)),
          );
          if (locale == 'ru') await capture(t, 'ui-057-focus-$name');
          await pressKey(t, LogicalKeyboardKey.select);
          await pressKey(t, LogicalKeyboardKey.enter);
          await settle(t, 6);
          expect(find.byType(PlayerScreen), findsNothing);
          expect(_upcomingFocused(), isTrue);
        }
        await _close(t);
      });

      testWidgets('anime counts down, an ended show says so on $name '
          '($locale)', (t) async {
        final loc = Locale(locale);
        final l = lookupAppLocalizations(loc);
        await _openShow(t, ff, size, loc, 'Comet Hunters');
        expect(_text(t, showAiringKey), endsWith(l.airingLeftDaysHours(2, 5)));
        expect(
          _text(t, showAiringKey),
          startsWith(l.airingNextIn(8, '', '').split(' — ').first),
        );
        // Anime numbered in one run: the card ends the last season, «Э8».
        await _showRowEnd(t);
        expect(
          find.descendant(
            of: find.byKey(showUpcomingKey),
            matching: find.text(l.showEpisodeNumber(8)),
          ),
          findsOneWidget,
        );
        await _close(t);

        await _openShow(t, ff, size, loc, 'Quiet Valley');
        expect(_text(t, showAiringKey), l.airingEnded);
        expect(find.byKey(showUpcomingKey, skipOffstage: false), findsNothing);
        await _close(t);
      });

      testWidgets('poster labels in the library on $name ($locale)', (t) async {
        final loc = Locale(locale);
        final l = lookupAppLocalizations(loc);
        final (s, _) = await _services(loc);
        setSurface(t, size);
        await t.pumpWidget(testApp(s, formFactor: ff, section: Section.shows));
        await settle(t, 10);
        expect(_text(t, airingBadgeKey(_showId)), l.airingBadgeNext('tue'));
        expect(_text(t, airingBadgeKey(_endedId)), l.airingEnded);
        // Saturday 28 November: within the week too.
        expect(_text(t, airingBadgeKey(_animeId)), l.airingBadgeNext('sat'));
        if (locale == 'ru') await capture(t, 'ui-057-posters-$name');
        await _close(t);
      });
    }
  }

  testWidgets('offline on the page: the old value, marked', (t) async {
    final (s, lib) = await _services(const Locale('ru'));
    lib.setAiring(
      ShowAiring(
        showId: _showId,
        status: AiringStatus.airing,
        next: _harbor.next,
        source: 'tmdb',
        checkedAt: DateTime(2026, 11, 24, 10),
        offline: true,
      ),
    );
    setSurface(t, const Size(1280, 800));
    await t.pumpWidget(
      testApp(s, formFactor: FormFactor.desktop, section: Section.shows),
    );
    await settle(t, 10);
    await t.tap(find.text('Harbor Lights').first);
    await settle(t, 10);
    expect(_text(t, showAiringKey), contains('нет сети · данные от 24'));
    // The card stays with the old date.
    await _showRowEnd(t);
    expect(find.byKey(showUpcomingKey), findsOneWidget);
    await capture(t, 'ui-057-offline-desktop');
    await _close(t);
  });

  testWidgets('the episode arrives: its card gives way to the real one', (
    t,
  ) async {
    final (s, lib) = await _services(const Locale('ru'));
    setSurface(t, const Size(1280, 800));
    await t.pumpWidget(
      testApp(s, formFactor: FormFactor.desktop, section: Section.shows),
    );
    await settle(t, 10);
    await t.tap(find.text('Harbor Lights').first);
    await settle(t, 10);
    await _showRowEnd(t);
    expect(find.byKey(showUpcomingKey), findsOneWidget);
    // The core drops the next episode once its file is in the library.
    lib.setAiring(
      ShowAiring(
        showId: _showId,
        status: AiringStatus.airing,
        source: 'tmdb',
        checkedAt: _now,
      ),
    );
    lib.addItems([_ep(_showId, 'Harbor Lights', 1, 4)]);
    await settle(t, 10);
    expect(find.byKey(showUpcomingKey, skipOffstage: false), findsNothing);
    expect(
      find.byKey(showEpisodeKey('$_showId-e-1-4'), skipOffstage: false),
      findsOneWidget,
    );
    await _close(t);
  });
}
