// SPDX-License-Identifier: MPL-2.0
//
// SYNC-011: Settings → Trakt — sign-in with a code and a QR code, the
// signed-in account — in Russian with real fonts. With
// KINOLUNE_SCREENSHOTS_DIR set the frames are written to
// docs/screenshots/progress/sync-011:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/sync-011 \
//     flutter test test/trakt_screenshots_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/fake_trakt.dart';
import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

void main() {
  setUpAll(loadAppFonts);

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('tv', FormFactor.tv, const Size(1920, 1080)),
  ]) {
    testWidgets('sign in to Trakt on $name', (t) async {
      setSurface(t, size);
      final trakt = FakeTrakt()..pollsUntilApproved = 1;
      final services = await demoServices(trakt: trakt);
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t);
      final signIn = find.byKey(const ValueKey('settings-trakt-sign-in'));
      await t.ensureVisible(signIn);
      await settle(t, 4);
      expect(find.text(ru.traktSignIn), findsWidgets);
      await capture(t, '01-settings-signed-out-$name');

      await t.tap(signIn);
      await settle(t, 6);
      expect(find.byKey(const ValueKey('trakt-sign-in')), findsOneWidget);
      expect(find.text('ABCD1234'), findsOneWidget);
      await capture(t, '02-sign-in-code-$name');

      // Two polls, five seconds apart: the second one is approved.
      await t.pump(const Duration(seconds: 5));
      await t.pump(const Duration(seconds: 5));
      await settle(t, 6);
      expect(find.byKey(const ValueKey('trakt-sign-in')), findsNothing);
      final account = find.byKey(const ValueKey('settings-trakt-account'));
      await t.ensureVisible(account);
      await settle(t, 4);
      expect(find.text(ru.traktSignedInAs('tester')), findsOneWidget);
      await capture(t, '03-settings-signed-in-$name');
      expect(trakt.syncs, 1);

      services.traktSync?.dispose();
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
