// SPDX-License-Identifier: MPL-2.0
//
// META-021: album and artist pages with MusicBrainz data, in Russian with
// real fonts. With KINOLUNE_SCREENSHOTS_DIR set the frames are written to
// docs/screenshots/progress/meta-021:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/meta-021 \
//     flutter test test/music_metadata_screenshots_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/focusable.dart' show KButton;
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

Future<void> _tap(WidgetTester t, Finder f) async {
  await t.tap(f.first);
  await settle(t, 10);
}

void main() {
  setUpAll(loadAppFonts);

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('phone', FormFactor.phone, const Size(390, 844)),
  ]) {
    testWidgets('screenshots on $name', (t) async {
      setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
      final services = await demoServices();
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.music),
      );
      await settle(t);
      await _tap(t, find.text('Nocturne Avenue'));
      expect(find.textContaining('Lantern Street Records'), findsOneWidget);
      expect(find.textContaining('Первый выпуск: 2019'), findsOneWidget);
      await capture(t, '0${name == 'desktop' ? 1 : 3}-album-$name');
      await _tap(t, find.widgetWithText(KButton, 'Velvet Static'));
      expect(find.textContaining('Группа'), findsOneWidget);
      expect(find.textContaining('С 2012 года'), findsOneWidget);
      await capture(t, '0${name == 'desktop' ? 2 : 4}-artist-$name');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
