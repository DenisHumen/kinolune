// SPDX-License-Identifier: MPL-2.0
//
// META-034: an unsure or failed recognition is fixed on the media card
// (candidates under the title, one tap chooses, «Верно» keeps it) and the
// home screen links to the review list while there is something to check.

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_review_inbox.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/review/review_inbox_screen.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));

Finder _key(String k) => find.byKey(ValueKey(k));

ReviewEntry _entry(String itemId, ReviewState state) => ReviewEntry(
  workId: 7,
  itemId: itemId,
  state: state,
  title: 'Shown',
  candidates: const [
    MatchCandidate(
      extKind: 'movie',
      extId: '1',
      title: 'Shown',
      year: 2001,
      current: true,
    ),
    MatchCandidate(extKind: 'movie', extId: '2', title: 'Other', year: 2003),
  ],
);

Future<FakeReviewInbox> _strip(WidgetTester t, ReviewState state) async {
  setSurface(t, const Size(1280, 900));
  final inbox = FakeReviewInbox(entries: [_entry('m1', state)]);
  final s = await demoServices(reviewInbox: inbox);
  s.locale.value = const Locale('en');
  await t.pumpWidget(testApp(s, section: Section.settings));
  await settle(t);
  final context = t.element(find.byType(Scaffold).first);
  unawaited(
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => const Scaffold(body: ReviewStrip(itemId: 'm1')),
      ),
    ),
  );
  await settle(t, 6);
  return inbox;
}

void main() {
  setUpAll(loadAppFonts);

  testWidgets('an unsure card shows other candidates; a tap chooses', (
    t,
  ) async {
    final inbox = await _strip(t, ReviewState.check);
    expect(_key('review-strip'), findsOneWidget);
    expect(find.text(en.reviewStripCheck), findsOneWidget);
    expect(_key('review-strip-candidate-1'), findsNothing);
    await t.tap(_key('review-strip-candidate-2'));
    await settle(t, 6);
    expect(inbox.log, contains('choose:7:2'));
    expect(_key('review-strip'), findsNothing);
  });

  testWidgets('«Верно» keeps the shown result and hides the strip', (t) async {
    final inbox = await _strip(t, ReviewState.check);
    await t.tap(_key('review-strip-confirm'));
    await settle(t, 6);
    expect(inbox.log, contains('confirm:7'));
    expect(_key('review-strip'), findsNothing);
  });

  testWidgets('an unrecognized card asks to pick, without «Верно»', (t) async {
    await _strip(t, ReviewState.manual);
    expect(find.text(en.reviewStripManual), findsOneWidget);
    expect(_key('review-strip-confirm'), findsNothing);
    expect(_key('review-strip-more'), findsOneWidget);
  });

  testWidgets('a sure card shows nothing', (t) async {
    setSurface(t, const Size(1280, 900));
    final s = await demoServices(reviewInbox: FakeReviewInbox());
    await t.pumpWidget(testApp(s, section: Section.settings));
    await settle(t);
    final context = t.element(find.byType(Scaffold).first);
    unawaited(
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => const Scaffold(body: ReviewStrip(itemId: 'm1')),
        ),
      ),
    );
    await settle(t, 6);
    expect(_key('review-strip'), findsNothing);
  });

  testWidgets('home links to the list only while there is something', (
    t,
  ) async {
    setSurface(t, const Size(1280, 900));
    final s = await demoServices(reviewInbox: FakeReviewInbox.demo());
    await t.pumpWidget(testApp(s, section: Section.home));
    await settle(t);
    expect(_key('home-review-banner'), findsOneWidget);
    await t.tap(_key('home-review-banner'));
    await settle(t, 6);
    expect(find.byType(ReviewInboxScreen), findsOneWidget);

    final empty = await demoServices(reviewInbox: FakeReviewInbox());
    await t.pumpWidget(testApp(empty, section: Section.home));
    await settle(t);
    expect(_key('home-review-banner'), findsNothing);
  });
}
