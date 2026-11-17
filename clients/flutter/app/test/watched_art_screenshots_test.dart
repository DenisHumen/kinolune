// SPDX-License-Identifier: MPL-2.0
//
// UI-037: the watched state in the real screens, in Russian with real fonts:
// Home's Continue row, a show's episodes (watched, started and next, not
// watched; then Hide Spoilers) and the poster grids of shows and movies with
// the «?» of the review inbox. With KINOLUNE_SCREENSHOTS_DIR set the frames
// are written to docs/screenshots/progress/ui-037:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-037 \
//     flutter test test/watched_art_screenshots_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_review_inbox.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/show_detail_screen.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/watched_art.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

const _show = 'show-moonlit-bureau';

/// Season 1 and S2E1–E3 of Moonlit Bureau watched; the demo has S2E4
/// started at 67 %.
Future<void> _watchSome(FakeLibrary lib) async {
  final eps = await lib.episodes(_show).first;
  final watched = eps.where(
    (e) => e.season == 1 || (e.season == 2 && (e.episode ?? 0) < 4),
  );
  var hours = 200;
  for (final e in watched) {
    await lib.saveProgress(
      Progress(
        itemId: e.id,
        positionMs: e.durationMs!,
        durationMs: e.durationMs!,
        updatedAt: demoNow.subtract(Duration(hours: hours--)),
        watched: true,
      ),
    );
  }
}

void main() {
  setUpAll(loadAppFonts);

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('tv', FormFactor.tv, const Size(960, 540)),
  ]) {
    testWidgets('screens on $name', (t) async {
      setSurface(t, size, dpr: name == 'tv' ? 2 : 1);
      final lib = FakeLibrary.demo(scanStepDelay: Duration.zero);
      await _watchSome(lib);
      final movies = await lib.movies().first;
      final unsure = movies.firstWhere(
        (m) => m.title == 'Paper Lanterns', // l10n-ignore
        orElse: () => movies.last,
      );
      final services = await demoServices(
        library: lib,
        reviewInbox: FakeReviewInbox(
          entries: [
            ReviewEntry(
              workId: 1,
              itemId: unsure.id,
              state: ReviewState.check,
              title: unsure.title,
              confidence: .62,
            ),
          ],
        ),
      );
      services.locale.value = const Locale('ru');

      // Home: the Continue row splits its stills.
      await t.pumpWidget(testApp(services, formFactor: ff));
      await settle(t);
      expect(find.byKey(watchedBlurKey), findsWidgets);
      await capture(t, '01-home-continue-$name');

      // Shows: unwatched episode counts.
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.shows),
      );
      await settle(t);
      expect(find.byKey(const ValueKey('badge-left-$_show')), findsOneWidget);
      await capture(t, '02-shows-grid-$name');

      // A show: watched, started and next, not watched.
      await t.tap(find.text('Moonlit Bureau').first);
      await settle(t);
      expect(find.byType(ShowDetailScreen), findsOneWidget);
      final card = find.byKey(showEpisodeKey('$_show-s2e4'));
      final first = find.byKey(showEpisodeKey('$_show-s2e1'));
      // The episode row in the lower part of the frame, the hero above.
      await t.scrollUntilVisible(
        first,
        150,
        scrollable: find
            .descendant(
              of: find.byType(ShowDetailScreen),
              matching: find.byType(Scrollable),
            )
            .first,
      );
      await settle(t, 2);
      await Scrollable.ensureVisible(t.element(first), alignment: .7);
      await settle(t, 4);
      // From S2E2 (TV: S2E3): watched, the started next one, not watched.
      final step =
          t.getRect(find.byKey(showEpisodeKey('$_show-s2e2'))).left -
          t.getRect(first).left;
      t
          .state<ScrollableState>(
            find.ancestor(of: first, matching: find.byType(Scrollable)).first,
          )
          .position
          .jumpTo(ff == FormFactor.tv ? step * 2 : step);
      await settle(t, 4);
      expect(
        find.descendant(of: card, matching: find.byKey(watchedBlurKey)),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: card,
          matching: find.byKey(const ValueKey('watched-next')),
        ),
        findsOneWidget,
      );
      await capture(t, '03-show-episodes-$name');

      // Hide Spoilers: future episodes blurred whole, with their number.
      await services.setHideSpoilers(true);
      await settle(t, 4);
      expect(
        find.descendant(
          of: find.byKey(showEpisodeKey('$_show-s2e5')),
          matching: find.byKey(watchedHiddenKey),
        ),
        findsOneWidget,
      );
      await capture(t, '04-show-spoilers-$name');
      await services.setHideSpoilers(false);

      // Movies: bars, marks and «?».
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.movies),
      );
      await settle(t);
      expect(find.byKey(ValueKey('badge-review-${unsure.id}')), findsOneWidget);
      await capture(t, '05-movies-grid-$name');

      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
