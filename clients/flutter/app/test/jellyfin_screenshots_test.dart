// SPDX-License-Identifier: MPL-2.0
//
// NET-014: adding a Jellyfin server, in Russian with real fonts. With
// KINOLUNE_SCREENSHOTS_DIR set the frames are written to
// docs/screenshots/progress/net-014:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/net-014 \
//     flutter test test/jellyfin_screenshots_test.dart

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

  for (final (name, ff, size, first) in <(String, FormFactor, Size, int)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800), 1),
    ('phone', FormFactor.phone, const Size(390, 844), 4),
  ]) {
    testWidgets('screenshots on $name', (t) async {
      setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
      final link = FakeJellyfinLink()..pollsUntilApproved = 1000;
      final services = await demoServices(jellyfin: link);
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t);
      await t.tap(find.text(ru.addSourceTitle).first);
      await settle(t, 6);
      expect(find.text(ru.sourcesTypeJellyfin), findsOneWidget);
      if (name == 'desktop') await capture(t, '01-kinds-$name');

      await t.tap(find.text(ru.sourcesTypeJellyfin));
      await settle(t, 6);
      await t.enterText(
        find.byKey(const ValueKey('jellyfin-address')),
        '192.168.1.10:8096',
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
      await capture(t, '0${first + (name == 'desktop' ? 1 : 0)}-password-$name');

      await t.tap(find.text(ru.sourcesJellyfinModeQuickConnect));
      await settle(t, 4);
      await t.ensureVisible(find.byKey(const ValueKey('jellyfin-submit')));
      await t.tap(find.byKey(const ValueKey('jellyfin-submit')));
      await settle(t, 6);
      expect(find.text('123456'), findsOneWidget);
      await capture(t, '0${first + (name == 'desktop' ? 2 : 1)}-quick-connect-$name');
      await t.tap(find.text(ru.commonCancel).last);
      await settle(t, 4);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
