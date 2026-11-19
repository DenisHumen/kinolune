// SPDX-License-Identifier: MPL-2.0
//
// UI-043: quiet recognition. A sure match asks nothing: no «?» on its
// poster, no line on the home screen, nothing under the title of its card.
// An unsure one gets a small «?» on the poster and one line on the home
// screen («1 фильм стоит проверить»); fixing it on the card clears both.
// With KINOLUNE_SCREENSHOTS_DIR set the frames are written to
// docs/screenshots/progress/ui-043:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-043 \
//     flutter test test/quiet_recognition_test.dart

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_review_inbox.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/review/review_inbox_screen.dart';
import 'package:kinolune/screens/movie_detail_screen.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/watched_art.dart' show ReviewMark;
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

Finder _key(String k) => find.byKey(ValueKey(k));

ReviewEntry _unsure(LibraryItem m, {int workId = 1, String kind = 'movie'}) =>
    ReviewEntry(
      workId: workId,
      itemId: m.id,
      kind: kind,
      state: ReviewState.check,
      title: m.title,
      year: m.year,
      confidence: .62,
      candidates: [
        MatchCandidate(
          extKind: 'movie',
          extId: '1',
          title: m.title,
          year: m.year,
          score: .62,
          current: true,
        ),
        const MatchCandidate(
          extKind: 'movie',
          extId: '2',
          title: 'Paper Lanterns Returns', // l10n-ignore
          year: 2019,
          score: .55,
        ),
      ],
    );

Future<void> _pushCard(WidgetTester t, LibraryItem movie) async {
  final context = t.element(find.byType(Scaffold).first);
  unawaited(
    Navigator.of(context).push(
      MaterialPageRoute<void>(builder: (_) => MovieDetailScreen(item: movie)),
    ),
  );
  await settle(t, 8);
}

void main() {
  setUpAll(loadAppFonts);

  group('the home line', () {
    test('names movies, shows or titles', () {
      const m = ReviewEntry(
        workId: 1,
        itemId: 'm1',
        state: ReviewState.check,
        title: 'A',
      );
      const s = ReviewEntry(
        workId: 2,
        itemId: 's2',
        kind: 'show',
        state: ReviewState.check,
        title: 'B',
      );
      const animeFilm = ReviewEntry(
        workId: 3,
        itemId: 'm3',
        kind: 'anime',
        state: ReviewState.manual,
        title: 'C',
      );
      const animeShow = ReviewEntry(
        workId: 4,
        itemId: 's4',
        kind: 'anime',
        state: ReviewState.check,
        title: 'D',
      );
      expect(reviewHomeKind([m, animeFilm]), ReviewHomeKind.movies);
      expect(reviewHomeKind([s, animeShow]), ReviewHomeKind.shows);
      expect(reviewHomeKind([m, s]), ReviewHomeKind.titles);
    });

    test('reads like a sentence in Russian and English', () {
      expect(
        reviewHomeLine(ru, ReviewHomeKind.movies, 3),
        '3 фильма стоит проверить',
      );
      expect(
        reviewHomeLine(ru, ReviewHomeKind.movies, 5),
        '5 фильмов стоит проверить',
      );
      expect(
        reviewHomeLine(ru, ReviewHomeKind.shows, 1),
        '1 сериал стоит проверить',
      );
      expect(
        reviewHomeLine(ru, ReviewHomeKind.titles, 2),
        '2 названия стоит проверить',
      );
      expect(
        reviewHomeLine(en, ReviewHomeKind.movies, 1),
        '1 movie to check',
      );
      expect(
        reviewHomeLine(en, ReviewHomeKind.titles, 4),
        '4 titles to check',
      );
    });
  });

  testWidgets('sure matches: no mark, no home line, nothing on the card', (
    t,
  ) async {
    setSurface(t, const Size(1280, 900));
    final s = await demoServices(reviewInbox: FakeReviewInbox());
    s.locale.value = const Locale('en');
    final movies = await s.library.movies().first;

    await t.pumpWidget(testApp(s, section: Section.home));
    await settle(t);
    expect(_key('home-review-banner'), findsNothing);
    expect(find.byType(ReviewMark, skipOffstage: false), findsNothing);
    expect(find.byType(SnackBar), findsNothing);

    await t.pumpWidget(testApp(s, section: Section.movies));
    await settle(t);
    expect(find.byType(ReviewMark, skipOffstage: false), findsNothing);

    // The demo's 62 % movie too: the inbox did not list it, so its card
    // stays quiet (no «Распознано неточно» line, no strip).
    final low = movies.firstWhere((m) => (m.matchConfidence ?? 1) < .75);
    await _pushCard(t, low);
    expect(find.byType(MovieDetailScreen), findsOneWidget);
    expect(_key('review-strip'), findsNothing);
    expect(find.text(en.detailMatchUncertain(low.matchConfidence!)), findsNothing);
    expect(find.text(en.detailMatched(low.matchConfidence!)), findsNothing);
  });

  testWidgets('an unsure match: a mark and one home line; the card fixes it', (
    t,
  ) async {
    setSurface(t, const Size(1280, 900));
    final lib = FakeLibrary.demo(scanStepDelay: Duration.zero);
    final movie = (await lib.movies().first).first;
    final inbox = FakeReviewInbox(entries: [_unsure(movie)]);
    final s = await demoServices(library: lib, reviewInbox: inbox);
    s.locale.value = const Locale('en');

    await t.pumpWidget(testApp(s, section: Section.home));
    await settle(t);
    expect(_key('home-review-banner'), findsOneWidget);
    expect(find.text(en.reviewHomeLineMovies(1)), findsOneWidget);
    // The line opens the list; the list is not in the navigation.
    await t.tap(_key('home-review-banner'));
    await settle(t, 6);
    expect(find.byType(ReviewInboxScreen), findsOneWidget);
    await t.pageBack();
    await settle(t, 6);

    await t.pumpWidget(testApp(s, section: Section.movies));
    await settle(t);
    expect(_key('badge-review-${movie.id}'), findsOneWidget);

    // Fixed on its card: «Верно» under the title.
    await _pushCard(t, movie);
    expect(_key('review-strip'), findsOneWidget);
    await t.tap(_key('review-strip-confirm'));
    await settle(t, 6);
    expect(inbox.log, contains('confirm:1'));
    expect(_key('review-strip'), findsNothing);
    await t.pageBack();
    await settle(t, 6);
    expect(_key('badge-review-${movie.id}'), findsNothing);

    await t.pumpWidget(testApp(s, section: Section.home));
    await settle(t);
    expect(_key('home-review-banner'), findsNothing);
  });

  testWidgets('the line counts every unsure title and names them', (t) async {
    setSurface(t, const Size(1280, 900));
    final lib = FakeLibrary.demo(scanStepDelay: Duration.zero);
    final movies = await lib.movies().first;
    final shows = await lib.shows().first;
    final inbox = FakeReviewInbox(
      entries: [
        _unsure(movies[0]),
        _unsure(movies[1], workId: 2),
        _unsure(shows[0], workId: 3, kind: 'show'),
      ],
    );
    final s = await demoServices(library: lib, reviewInbox: inbox);
    s.locale.value = const Locale('ru');
    await t.pumpWidget(testApp(s, section: Section.home));
    await settle(t);
    expect(find.text('3 названия стоит проверить'), findsOneWidget);
    // One line, not one per title.
    expect(_key('home-review-banner'), findsOneWidget);
  });

  // Screenshots: the home line, the «?» on a poster and the fix on the card,
  // in Russian on a phone, a desktop and a TV.
  for (final (name, ff, size, dpr) in <(String, FormFactor, Size, double)>[
    ('phone', FormFactor.phone, const Size(390, 844), 2),
    ('desktop', FormFactor.desktop, const Size(1280, 800), 1),
    ('tv', FormFactor.tv, const Size(960, 540), 2),
  ]) {
    testWidgets('screens on $name', (t) async {
      setSurface(t, size, dpr: dpr);
      final lib = FakeLibrary.demo(scanStepDelay: Duration.zero);
      final movies = await lib.movies().first;
      final unsure = movies.firstWhere(
        (m) => m.title == 'Paper Lanterns', // l10n-ignore
        orElse: () => movies.last,
      );
      final inbox = FakeReviewInbox(
        entries: [
          _unsure(unsure),
          _unsure(movies.first, workId: 2),
          _unsure(movies[2], workId: 3),
        ],
      );
      final s = await demoServices(library: lib, reviewInbox: inbox);
      s.locale.value = const Locale('ru');

      await t.pumpWidget(testApp(s, formFactor: ff));
      await settle(t);
      expect(find.text('3 фильма стоит проверить'), findsOneWidget);
      await t.ensureVisible(_key('home-review-banner'));
      await settle(t, 4);
      await capture(t, '01-home-line-$name');

      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
      await t.pumpWidget(
        testApp(s, formFactor: ff, section: Section.movies),
      );
      await settle(t);
      await capture(t, '02-movies-mark-$name');

      await _pushCard(t, unsure);
      expect(_key('review-strip'), findsOneWidget);
      await t.ensureVisible(_key('review-strip'));
      await settle(t, 4);
      await capture(t, '03-card-fix-$name');

      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
