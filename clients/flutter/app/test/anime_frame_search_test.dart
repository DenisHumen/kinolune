// SPDX-License-Identifier: MPL-2.0
//
// META-031: «Распознавать аниме по кадрам» in Settings — off by default
// (frames of the user's files go to trace.moe only with consent),
// remembered, handed to the library; and the «почему» line of a candidate
// the frames named. With KINOLUNE_SCREENSHOTS_DIR set the frame is
// written (in Russian):
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/meta-031 \
//     flutter test test/anime_frame_search_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:intl/date_symbol_data_local.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/review/review_inbox_screen.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));
final en = lookupAppLocalizations(const Locale('en'));

void main() {
  setUpAll(() async {
    await loadAppFonts();
    await initializeDateFormatting();
  });

  test('the «почему» line of a frame candidate', () {
    const s = MatchSignals(provider: 'trace_moe', title: 0.9, tier: 'frames');
    expect(reviewWhy(ru, s), 'Почему: совпало 90 % кадров (trace.moe)');
    expect(reviewWhy(en, s), contains('90% of the frames match'));
  });

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('phone', FormFactor.phone, const Size(390, 844)),
  ]) {
    testWidgets('the switch is off by default and remembered ($name)', (
      t,
    ) async {
      setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
      final services = await demoServices();
      expect(services.animeFrameSearch.value, isFalse);
      expect(await services.preferences.read(Preferences.animeFrameSearch), isNull);
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t);
      final tile = find.byKey(const ValueKey('settings-anime-frame-search'));
      await t.ensureVisible(tile);
      await settle(t, 2);
      Scrollable.ensureVisible(
        t.element(tile),
        alignment: name == 'phone' ? 0.5 : 0.6,
      );
      await settle(t, 4);
      expect(find.text(ru.settingsAnimeFrameSearch), findsOneWidget);
      expect(find.text(ru.settingsAnimeFrameSearchSubtitle), findsOneWidget);
      final library = services.library;
      if (library is LibraryAnimeDb) {
        expect((library as LibraryAnimeDb).animeFrameSearch, isFalse);
      }

      await t.tap(tile);
      await settle(t, 2);
      expect(services.animeFrameSearch.value, isTrue);
      expect(
        await services.preferences.read(Preferences.animeFrameSearch),
        'true',
      );
      if (library is LibraryAnimeDb) {
        expect((library as LibraryAnimeDb).animeFrameSearch, isTrue);
      }
      await capture(t, '0${name == 'desktop' ? 1 : 2}-settings-$name');

      await t.tap(tile);
      await settle(t, 2);
      expect(services.animeFrameSearch.value, isFalse);
      expect(
        await services.preferences.read(Preferences.animeFrameSearch),
        isNull,
      );
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
