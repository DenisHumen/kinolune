// SPDX-License-Identifier: MPL-2.0
//
// LIB-024: Settings › Library shows the automatic check and recognition
// status, in Russian with real fonts. With KINOLUNE_SCREENSHOTS_DIR set the
// frames are written to docs/screenshots/progress/lib-024:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/lib-024 \
//     flutter test test/auto_recognition_screenshots_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart'
    show RecognitionStatus, ScanPhase, ScanProgress, ScanStep;
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/settings_screen.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

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
      final lib = FakeLibrary.demo(scanStepDelay: Duration.zero);
      final services = await demoServices(library: lib);
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t);
      final status = find.byKey(const ValueKey('settings-recognition-status'));
      if (status.evaluate().isEmpty) {
        await t.scrollUntilVisible(
          status,
          200,
          scrollable: find
              .descendant(
                of: find.byType(SettingsScreen),
                matching: find.byType(Scrollable),
              )
              .first,
        );
      }
      await settle(t, 2);

      lib.recognitionStatus = RecognitionStatus(
        lastCheck: DateTime.now().subtract(const Duration(minutes: 3)),
        running: true,
        worksWaiting: 42,
        progress: const ScanProgress(
          phase: ScanPhase.metadata,
          step: ScanStep.titles,
          processed: 158,
          total: 200,
        ),
      );
      await settle(t, 3);
      await t.ensureVisible(find.text(ru.settingsScanNow));
      await settle(t, 2);
      expect(t.widget<Text>(status).data, ru.settingsScanStepTitles(158, 200));
      await capture(t, '01-running-$name');

      lib.recognitionStatus = RecognitionStatus(
        lastCheck: DateTime.now(),
        worksWaiting: 12,
        worksFailed: 2,
      );
      await settle(t, 3);
      expect(t.widget<Text>(status).data, ru.settingsRecognitionWaiting(12));
      await capture(t, '02-waiting-$name');

      lib.recognitionStatus = RecognitionStatus(lastCheck: DateTime.now());
      await settle(t, 3);
      expect(t.widget<Text>(status).data, ru.settingsRecognitionDone);
      await capture(t, '03-done-$name');

      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
