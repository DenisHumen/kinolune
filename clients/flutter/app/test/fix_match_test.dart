// SPDX-License-Identifier: MPL-2.0
//
// UI-012: «Исправить матч». On a work's review page: confidence and
// «Почему?» (the signals behind a score), a pasted TMDB / TheTVDB /
// MyAnimeList link or id, the hint that a choice covers every file and the
// folder, the report after it (files and the folder rule), the type a
// result of another kind switches to; «Исправить распознавание» from the
// home screen's cards. With KINOLUNE_SCREENSHOTS_DIR set the frames are
// written (in Russian) for a phone, a desktop and a TV:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-012 \
//     flutter test test/fix_match_test.dart

import 'dart:async';

import 'package:flutter/gestures.dart' show kSecondaryMouseButton;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_review_inbox.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/review/review_inbox_screen.dart';
import 'package:kinolune/screens/home_screen.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/cards.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

Finder _key(String k) => find.byKey(ValueKey(k));

Future<void> _tapKey(WidgetTester t, String key) async {
  final f = _key(key);
  await t.ensureVisible(f);
  await t.pump();
  await t.tap(f);
  await settle(t, 6);
}

const _tvdbLink = 'https://thetvdb.com/?tab=series&id=424536';

/// A season of 24 episodes nobody recognised; one weak candidate.
const _frieren = ReviewEntry(
  workId: 40,
  itemId: 's40',
  kind: 'anime',
  state: ReviewState.manual,
  title: 'Sousou no Frieren',
  files: 24,
  samplePath: 'Anime/Frieren/[Grp] Sousou no Frieren - 01 (1080p).mkv',
  candidates: [
    MatchCandidate(
      extKind: 'tv',
      extId: '60866',
      title: 'Хвост феи: Сто лет',
      originalTitle: 'FAIRY TAIL 100 YEARS QUEST',
      year: 2024,
      score: 0.61,
      signals: MatchSignals(
        title: 0.61,
        tier: 'fuzzy',
        fileKind: 'anime',
        kind: 'tv',
        year: 2024,
        episodesBad: 4,
      ),
    ),
  ],
);

/// The same files read as a movie.
const _movie = ReviewEntry(
  workId: 41,
  itemId: 'm41',
  state: ReviewState.manual,
  title: 'Sousou no Frieren',
  files: 1,
  samplePath: 'Movies/Sousou no Frieren.mkv',
);

FakeReviewInbox _inbox([ReviewEntry entry = _frieren]) => FakeReviewInbox(
  entries: [entry],
  ids: const {'424536': demoFrierenTmdb, '52991': demoFrierenAniList},
);

/// The review page of [entry] over the app.
Future<FakeReviewInbox> _page(
  WidgetTester t, {
  ReviewEntry entry = _frieren,
  FormFactor formFactor = FormFactor.desktop,
  Size size = const Size(1280, 900),
  double dpr = 1,
}) async {
  setSurface(t, size, dpr: dpr);
  final inbox = _inbox(entry);
  final s = await demoServices(reviewInbox: inbox);
  s.locale.value = const Locale('ru');
  await t.pumpWidget(
    testApp(s, section: Section.settings, formFactor: formFactor),
  );
  await settle(t);
  final context = t.element(find.byType(Scaffold).first);
  unawaited(
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => ReviewEntryScreen(inbox: inbox, entry: entry),
      ),
    ),
  );
  await settle(t, 6);
  return inbox;
}

/// Scrolls the page so [key] is near the top (screenshots).
Future<void> _align(WidgetTester t, String key) async {
  await t.runAsync(
    () => Scrollable.ensureVisible(t.element(_key(key)), alignment: 0.15),
  );
  await settle(t, 2);
}

/// Scrolls the home screen down until [target] is built and on screen.
Future<void> _scrollTo(WidgetTester t, Finder target, Finder scroll) async {
  for (var i = 0; i < 40 && target.hitTestable().evaluate().isEmpty; i++) {
    await t.drag(scroll, const Offset(0, -300));
    await settle(t, 2);
  }
  expect(target.hitTestable(), findsWidgets);
}

/// Types [text] into the paste field and looks it up: with «Найти», or
/// (the remote of a TV) the keyboard's done action.
Future<void> _paste(WidgetTester t, String text, {bool button = true}) async {
  await t.ensureVisible(_key('review-paste-field'));
  await t.enterText(_key('review-paste-field'), text);
  if (button) {
    await _tapKey(t, 'review-paste-find');
  } else {
    await t.testTextInput.receiveAction(TextInputAction.done);
    await settle(t, 6);
  }
}

void main() {
  setUpAll(loadAppFonts);

  test('«Почему?» lists the signals for and against', () {
    final s = reviewSignals(
      ru,
      const MatchSignals(
        title: 0.61,
        fileYear: 2023,
        year: 2024,
        episodesBad: 4,
        tied: true,
      ),
    );
    expect(s.map((x) => x.text), [
      ru.reviewWhyTitle(61),
      ru.reviewWhyYearSame(2024),
      ru.reviewWhyEpisodesMissing(4),
      ru.reviewWhyTied,
    ]);
    expect(s.map((x) => x.good), [false, true, false, false]);
    // A result found by its id says so.
    final id = reviewSignals(
      ru,
      const MatchSignals(provider: 'tvdb', tier: 'id', title: 1),
    );
    expect(id.single.text, ru.reviewWhyById('TheTVDB'));
    expect(
      reviewWhy(ru, const MatchSignals(provider: 'mal', tier: 'id')),
      ru.reviewWhy(ru.reviewWhyById('MyAnimeList')),
    );
  });

  test('the type a result switches the work to', () {
    const tv = MatchCandidate(extKind: 'tv', extId: '1', title: 'x');
    const movie = MatchCandidate(extKind: 'movie', extId: '1', title: 'x');
    const anilist = MatchCandidate(
      provider: 'anilist',
      extKind: '',
      extId: '1',
      title: 'x',
    );
    expect(reviewKindAfter('movie', tv), 'show');
    expect(reviewKindAfter('show', tv), isNull);
    expect(reviewKindAfter('anime', tv), isNull);
    expect(reviewKindAfter('show', movie), 'movie');
    expect(reviewKindAfter('show', anilist), 'anime');
    expect(reviewKindAfter('anime', anilist), isNull);
  });

  testWidgets('a pasted TheTVDB link fixes all 24 files and the folder', (
    t,
  ) async {
    final inbox = await _page(t);
    expect(find.text(ru.reviewApplyHint(24)), findsOneWidget);

    await _paste(t, _tvdbLink);
    expect(_key('review-pasted-tmdb-209867'), findsOneWidget);
    expect(
      find.text(ru.reviewWhy(ru.reviewWhyById('TheTVDB'))),
      findsOneWidget,
    );
    // Found by its id: no confidence, no type change for an anime.
    expect(_key('review-switch-kind-209867'), findsNothing);

    await _tapKey(t, 'review-pasted-tmdb-209867');
    expect(inbox.log, ['lookup:40:$_tvdbLink', 'choose:40:209867']);
    final snack = t.widget<Text>(
      find.descendant(of: _key('review-done'), matching: find.byType(Text)),
    );
    expect(snack.data, contains(ru.reviewDoneChosen(demoFrierenTmdb.title)));
    expect(snack.data, contains(ru.reviewDoneFiles(24)));
    expect(snack.data, contains(ru.reviewDoneRule('Anime/Frieren')));
  });

  testWidgets('MyAnimeList, Shikimori; unknown text and unknown ids', (
    t,
  ) async {
    final inbox = await _page(t);
    await _paste(t, 'https://myanimelist.net/anime/52991/Sousou_no_Frieren');
    expect(_key('review-pasted-anilist-154587'), findsOneWidget);
    expect(
      find.text(ru.reviewWhy(ru.reviewWhyById('MyAnimeList'))),
      findsOneWidget,
    );

    await _paste(t, 'Фрирен');
    expect(_key('review-paste-unknown'), findsOneWidget);
    expect(_key('review-pasted-anilist-154587'), findsNothing);

    await _paste(t, 'shikimori 1');
    expect(find.text(ru.reviewPasteNothing('Shikimori')), findsOneWidget);
    expect(inbox.log, hasLength(3));
  });

  testWidgets('the clipboard button pastes and looks up at once', (t) async {
    final inbox = await _page(t);
    t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async => call.method == 'Clipboard.getData'
          ? <String, dynamic>{'text': ' $_tvdbLink '}
          : null,
    );
    addTearDown(
      () => t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );
    await _tapKey(t, 'review-paste-clipboard');
    expect(inbox.log, ['lookup:40:$_tvdbLink']);
    expect(_key('review-pasted-tmdb-209867'), findsOneWidget);
  });

  testWidgets('confidence and «Почему?» on a candidate', (t) async {
    await _page(t);
    expect(find.textContaining(ru.reviewConfidence(61)), findsOneWidget);
    expect(_key('review-why-signals'), findsNothing);
    await _tapKey(t, 'review-why-60866');
    expect(_key('review-why-signals'), findsOneWidget);
    expect(
      find.descendant(
        of: _key('review-why-signals'),
        matching: find.text(ru.reviewWhyEpisodesMissing(4)),
      ),
      findsOneWidget,
    );
    await _tapKey(t, 'review-why-60866');
    expect(_key('review-why-signals'), findsNothing);
  });

  testWidgets('a series for a «movie» says the type changes', (t) async {
    final inbox = await _page(t, entry: _movie);
    await _paste(t, 'tvdb 424536');
    expect(find.text(ru.reviewSwitchKind(ru.reviewKindShow)), findsOneWidget);
    await _tapKey(t, 'review-pasted-tmdb-209867');
    expect(inbox.log.last, 'choose:41:209867');
  });

  testWidgets('«Исправить распознавание» from a home screen card', (t) async {
    setSurface(t, const Size(1280, 900));
    final homeScroll = find
        .descendant(
          of: find.byType(HomeScreen),
          matching: find.byType(Scrollable),
        )
        .first;
    // The first poster of the home screen.
    final first = await demoServices(reviewInbox: FakeReviewInbox());
    await t.pumpWidget(testApp(first, section: Section.home));
    await settle(t);
    await _scrollTo(t, find.byType(PosterCard), homeScroll);
    final item = t
        .widget<PosterCard>(find.byType(PosterCard).hitTestable().first)
        .item;

    final inbox = FakeReviewInbox(
      entries: [
        ReviewEntry(
          workId: 50,
          itemId: item.id,
          kind: item.kind == ItemKind.movie ? 'movie' : 'show',
          state: ReviewState.check,
          title: item.title,
          files: 1,
          samplePath: 'Movies/${item.title}.mkv',
        ),
      ],
    );
    final s = await demoServices(reviewInbox: inbox);
    s.locale.value = const Locale('ru');
    await t.pumpWidget(testApp(s, section: Section.home));
    await settle(t);
    final card = find.byWidgetPredicate(
      (w) => w is PosterCard && w.item.id == item.id,
    );
    await _scrollTo(t, card, homeScroll);
    await settle(t, 2);
    await t.tap(card.hitTestable().first, buttons: kSecondaryMouseButton);
    await settle(t, 4);
    expect(
      find.descendant(
        of: _key('item-menu-editMetadata'),
        matching: find.text(ru.commonFixMatch),
      ),
      findsOneWidget,
    );
    await _tapKey(t, 'item-menu-editMetadata');
    expect(find.byType(ReviewEntryScreen), findsOneWidget);
    expect(_key('review-paste-field'), findsOneWidget);
  });

  // docs/screenshots/progress/ui-012 (KINOLUNE_SCREENSHOTS_DIR), in
  // Russian: the page after a paste, with «Почему?» open, and the report.
  group('screenshots', () {
    for (final (name, ff, size, dpr) in [
      ('phone', FormFactor.phone, const Size(390, 844), 2.0),
      ('desktop', FormFactor.desktop, const Size(1280, 900), 1.0),
      ('tv', FormFactor.tv, const Size(960, 540), 2.0),
    ]) {
      testWidgets('fix match on $name', (t) async {
        await _page(t, formFactor: ff, size: size, dpr: dpr);
        await capture(t, 'ui-012-$name-page');
        await _tapKey(t, 'review-why-60866');
        await _align(t, 'review-why-60866');
        await settle(t, 2);
        await capture(t, 'ui-012-$name-why');
        await _paste(t, _tvdbLink, button: ff != FormFactor.tv);
        await _align(t, 'review-paste-field');
        await settle(t, 2);
        await capture(t, 'ui-012-$name-pasted');
        await _tapKey(t, 'review-pasted-tmdb-209867');
        await settle(t, 4);
        await capture(t, 'ui-012-$name-done');
      });
    }
  });
}
