// SPDX-License-Identifier: MPL-2.0
//
// META-016: the review inbox «На проверку» — the row with a badge in
// Settings → Медиатека, the list, a work's candidates with the «почему»
// line, «Верно», another candidate, manual search, «Это не фильм», and
// «Исправить распознавание» opening the same page. With
// KINOLUNE_SCREENSHOTS_DIR set the frames are written (in Russian):
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/meta-016 \
//     flutter test test/review_inbox_test.dart

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_review_inbox.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/review/review_inbox_screen.dart';
import 'package:kinolune/screens/fix_match_dialog.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

Finder _key(String k) => find.byKey(ValueKey(k));

Future<void> _tapKey(WidgetTester t, String key) async {
  final f = _key(key);
  await t.ensureVisible(f);
  await t.pump();
  await t.tap(f);
  await settle(t, 6);
}

Future<(AppServices, FakeReviewInbox)> _open(
  WidgetTester t, {
  FakeReviewInbox? inbox,
  Locale locale = const Locale('ru'),
  Size size = const Size(1280, 900),
}) async {
  setSurface(t, size);
  final review = inbox ?? FakeReviewInbox.demo();
  final s = await demoServices(reviewInbox: review);
  s.locale.value = locale;
  await t.pumpWidget(testApp(s, section: Section.settings));
  await settle(t);
  return (s, review);
}

Future<void> _openInbox(WidgetTester t) async {
  await _tapKey(t, 'settings-review-inbox');
  await settle(t, 4);
}

void main() {
  setUpAll(loadAppFonts);

  test('the «почему» line names the strongest signals', () {
    const s = MatchSignals(
      title: 0.85,
      fileYear: 2013,
      year: 2013,
      lostMarker: 'ova',
      episodesOk: 3,
      durationError: 0.03,
      language: true,
      tied: true,
    );
    final why = reviewWhy(ru, s);
    expect(why, startsWith('Почему: '));
    for (final part in [
      ru.reviewWhyTitle(85),
      ru.reviewWhyYearSame(2013),
      ru.reviewWhyLostMarker('OVA'),
      ru.reviewWhyEpisodesFit,
      ru.reviewWhyDurationFits,
      ru.reviewWhyLanguage,
      ru.reviewWhyTied,
    ]) {
      expect(why, contains(part));
    }
    expect(
      reviewWhy(en, const MatchSignals(fileYear: 2001, year: 2009)),
      contains(en.reviewWhyYearDiffers(2001, 2009)),
    );
    expect(
      reviewWhy(en, const MatchSignals(episodesBad: 2, episodesOk: 5)),
      contains(en.reviewWhyEpisodesMissing(2)),
    );
    expect(reviewWhy(en, const MatchSignals()), isEmpty);
  });

  testWidgets('settings row with a badge, the list and «Верно»', (t) async {
    final (_, inbox) = await _open(t);
    expect(_key('settings-review-inbox'), findsOneWidget);
    expect(
      find.descendant(of: _key('review-inbox-badge'), matching: find.text('3')),
      findsOneWidget,
    );
    expect(find.text(ru.reviewInboxSubtitle(1, 2)), findsOneWidget);
    await t.runAsync(
      () => Scrollable.ensureVisible(
        t.element(_key('settings-review-inbox')),
        alignment: 0.4,
      ),
    );
    await settle(t, 4);
    await capture(t, 'meta-016-settings-row');

    await _openInbox(t);
    expect(_key('review-entry-901'), findsOneWidget);
    expect(_key('review-entry-902'), findsOneWidget);
    expect(_key('review-entry-903'), findsOneWidget);
    // Shown results to check first.
    expect(
      t.getTopLeft(_key('review-entry-901')).dy,
      lessThan(t.getTopLeft(_key('review-entry-902')).dy),
    );
    expect(find.text(ru.reviewStateCheck), findsOneWidget);
    expect(find.text(ru.reviewStateManual), findsNWidgets(2));
    await capture(t, 'meta-016-inbox');

    await _tapKey(t, 'review-entry-901');
    expect(find.text(ru.reviewShownNow), findsOneWidget);
    expect(find.textContaining(ru.reviewWhyLostMarker('OVA')), findsWidgets);
    expect(_key('review-choose-60866'), findsOneWidget);
    await capture(t, 'meta-016-check');
    await _tapKey(t, 'review-confirm');
    expect(inbox.log, ['confirm:901']);
    // Back on the list, which has reloaded.
    expect(_key('review-entry-901'), findsNothing);
    expect(_key('review-entry-902'), findsOneWidget);
  });

  testWidgets('another candidate, manual search and «Это не фильм»', (t) async {
    final (_, inbox) = await _open(t);
    await _openInbox(t);

    // A weak candidate of the manual queue.
    await _tapKey(t, 'review-entry-902');
    expect(find.text(ru.reviewShownNow), findsNothing);
    expect(find.textContaining(ru.reviewWhyYearSame(2013)), findsOneWidget);
    await capture(t, 'meta-016-manual');
    await _tapKey(t, 'review-choose-212162');
    expect(inbox.log, ['choose:902:212162']);

    // No candidates: search by title and year.
    await _tapKey(t, 'review-entry-903');
    expect(find.text(ru.reviewNoCandidates), findsOneWidget);
    expect(_key('review-sample-path'), findsOneWidget);
    await t.enterText(_key('review-search-title'), 'Unknown thing');
    await _tapKey(t, 'review-search');
    expect(_key('review-search-nothing'), findsOneWidget);
    await t.enterText(_key('review-search-title'), 'inception');
    await t.enterText(_key('review-search-year'), '2010');
    await _tapKey(t, 'review-search');
    expect(_key('review-found-27205'), findsOneWidget);
    // The snack bar of the choice above goes first.
    await t.pump(const Duration(seconds: 5));
    await settle(t, 4);
    await capture(t, 'meta-016-search');
    expect(inbox.log.where((e) => e.startsWith('search:903:')).length, 2);

    // Not a movie after all.
    await _tapKey(t, 'review-not-media');
    expect(inbox.log.last, 'not_media:903');
    expect(_key('review-entry-903'), findsNothing);
    expect(_key('review-entry-901'), findsOneWidget);
  });

  testWidgets('an empty inbox says so; no row without an inbox', (t) async {
    await _open(t, inbox: FakeReviewInbox(), locale: const Locale('en'));
    expect(find.text(en.reviewInboxSubtitleEmpty), findsOneWidget);
    expect(_key('review-inbox-badge'), findsNothing);
    await _openInbox(t);
    expect(_key('review-inbox-empty'), findsOneWidget);
  });

  testWidgets('«Исправить распознавание» opens the review page', (t) async {
    setSurface(t, const Size(1280, 900));
    final s = await demoServices();
    final movie = (await s.library.movies().first).first;
    final inbox = FakeReviewInbox(
      entries: [
        ReviewEntry(
          workId: 7,
          itemId: movie.id,
          state: ReviewState.manual,
          title: movie.title,
          samplePath: movie.filePath,
        ),
      ],
    );
    final withInbox = await demoServices(reviewInbox: inbox);
    await t.pumpWidget(testApp(withInbox, section: Section.settings));
    await settle(t);
    final context = t.element(find.byType(Scaffold).first);
    unawaited(showFixMatchDialog(context, movie));
    await settle(t, 6);
    expect(_key('review-search-title'), findsOneWidget);
    expect(_key('review-not-media'), findsOneWidget);
  });

  testWidgets('without an inbox the plain fix dialog stays', (t) async {
    setSurface(t, const Size(1280, 900));
    final plain = await demoServices();
    final movie = (await plain.library.movies().first).first;
    await t.pumpWidget(testApp(plain, section: Section.settings));
    await settle(t);
    final c2 = t.element(find.byType(Scaffold).first);
    unawaited(showFixMatchDialog(c2, movie));
    await settle(t, 6);
    expect(find.byType(AlertDialog), findsOneWidget);
    expect(_key('review-search-title'), findsNothing);
  });
}
