// SPDX-License-Identifier: MPL-2.0
//
// NET-012: adding a Plex server with a code, in Russian with real fonts.
// With KINOLUNE_SCREENSHOTS_DIR set the frames are written to
// docs/screenshots/progress/net-012:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/net-012 \
//     flutter test test/plex_screenshots_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/fake_plex.dart';
import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

void main() {
  setUpAll(loadAppFonts);

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('phone', FormFactor.phone, const Size(390, 844)),
  ]) {
    testWidgets('screenshots on $name', (t) async {
      setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
      final link = FakePlexLink(pollsUntilLinked: 2);
      final services = await demoServices(plex: link);
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t);
      await t.tap(find.text(ru.addSourceTitle).first);
      await settle(t, 6);
      await t.tap(find.text(ru.sourcesTypePlex));
      await settle(t, 6);
      expect(find.text('K7Q2'), findsOneWidget);
      await capture(t, '01-code-$name');

      // Entered at plex.tv/link on the second check.
      await settle(t, 45);
      expect(find.text(ru.sourcesPlexWhoIsWatching), findsOneWidget);
      await t.tap(find.byKey(const ValueKey('plex-user-kid')));
      await settle(t, 4);
      await capture(t, '02-who-is-watching-$name');

      await t.enterText(find.byKey(const ValueKey('plex-pin')), '1234');
      await t.tap(find.byKey(const ValueKey('plex-pin-continue')));
      await settle(t, 6);
      expect(find.text(ru.sourcesPlexChooseServer), findsOneWidget);
      await capture(t, '03-server-$name');

      await t.tap(find.byKey(const ValueKey('plex-cancel')));
      await settle(t, 4);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
