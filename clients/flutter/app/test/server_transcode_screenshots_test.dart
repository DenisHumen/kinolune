// SPDX-License-Identifier: MPL-2.0
//
// NET-013: the transcode choice of a long press on Play for a movie on a
// Plex server, in Russian with real fonts. With KINOLUNE_SCREENSHOTS_DIR set
// the frames are written to docs/screenshots/progress/net-013:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/net-013 \
//     flutter test test/server_transcode_screenshots_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/movie_detail_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/fake_jellyfin.dart';
import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));
final _now = DateTime(2026, 11, 18, 20);

void main() {
  setUpAll(loadAppFonts);

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('tv', FormFactor.tv, const Size(960, 540)),
    ('phone', FormFactor.phone, const Size(390, 844)),
  ]) {
    testWidgets('screenshots on $name', (t) async {
      setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
      final link = FakeJellyfinLink()..serverItems.add('m-plex');
      final services = AppServices(
        library: FakeLibrary.items([
          LibraryItem(
            id: 'm-plex',
            kind: ItemKind.movie,
            title: 'Big Buck Bunny',
            year: 2008,
            overview: 'Огромный кролик против трёх хулиганов.',
            filePath: '/Movies/Big Buck Bunny (2008)/Big Buck Bunny (2008).mkv',
            durationMs: 596000,
            addedAt: _now,
          ),
        ]),
        secureStore: MemorySecureStore(),
        playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
        jellyfin: link,
        clock: () => _now,
      );
      await services.init();
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.movies),
      );
      await settle(t, 10);
      await t.tap(find.text('Big Buck Bunny').first);
      await settle(t, 10);
      expect(find.byType(MovieDetailScreen), findsOneWidget);
      await t.longPress(find.byKey(detailPlayKey));
      await settle(t, 6);
      expect(find.text(ru.detailTranscodeTitle), findsOneWidget);
      await capture(t, '01-transcode-choice-$name');

      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
