// SPDX-License-Identifier: MPL-2.0
//
// NET-015: adding an Emby server, in Russian with real fonts. With
// KINOLUNE_SCREENSHOTS_DIR set the frames are written to
// docs/screenshots/progress/net-015:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/net-015 \
//     flutter test test/emby_screenshots_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/fake_jellyfin.dart';
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
      final services = await demoServices(jellyfin: FakeJellyfinLink());
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t);
      await t.tap(find.text(ru.addSourceTitle).first);
      await settle(t, 6);
      expect(find.text(ru.sourcesTypeEmby), findsOneWidget);
      if (name == 'desktop') await capture(t, '01-kinds-$name');

      await t.tap(find.text(ru.sourcesTypeEmby));
      await settle(t, 6);
      await t.enterText(
        find.byKey(const ValueKey('jellyfin-address')),
        '192.168.1.20:8096',
      );
      await t.enterText(
        find.byKey(const ValueKey('jellyfin-alt-addresses')),
        'https://media.example.org',
      );
      await t.enterText(find.byKey(const ValueKey('jellyfin-user')), 'anna');
      await t.enterText(
        find.byKey(const ValueKey('jellyfin-password')),
        'password',
      );
      await settle(t, 4);
      await capture(t, '0${name == 'desktop' ? 2 : 3}-sign-in-$name');

      // A wrong password: the error under the form.
      await t.ensureVisible(find.byKey(const ValueKey('jellyfin-submit')));
      await t.tap(find.byKey(const ValueKey('jellyfin-submit')));
      await settle(t, 6);
      expect(find.text(ru.sourcesErrorAuth), findsOneWidget);
      if (name == 'desktop') await capture(t, '04-wrong-password-$name');
      await t.tap(find.text(ru.commonCancel).last);
      await settle(t, 4);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
