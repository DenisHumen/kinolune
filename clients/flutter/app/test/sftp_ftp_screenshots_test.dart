// SPDX-License-Identifier: MPL-2.0
//
// NET-006: adding SFTP and FTP sources and the "RSA keys" setting, in
// Russian with real fonts. With KINOLUNE_SCREENSHOTS_DIR set the frames are
// written to docs/screenshots/progress/net-006:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/net-006 \
//     flutter test test/sftp_ftp_screenshots_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

Future<void> _enter(WidgetTester t, String key, String text) async {
  await t.ensureVisible(find.byKey(ValueKey(key)));
  await t.enterText(find.byKey(ValueKey(key)), text);
  await t.pump();
}

void main() {
  setUpAll(loadAppFonts);

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('phone', FormFactor.phone, const Size(390, 844)),
  ]) {
    testWidgets('screenshots on $name', (t) async {
      setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
      final services = await demoServices(advancedSettings: true);
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t);
      if (name == 'desktop') {
        final rsa = find.byKey(const ValueKey('settings-ssh-rsa-keys'));
        await t.ensureVisible(rsa);
        await settle(t, 2);
        await capture(t, '01-rsa-setting-$name');
      }
      await t.ensureVisible(find.text(ru.addSourceTitle).first);
      await t.tap(find.text(ru.addSourceTitle).first);
      await settle(t, 6);
      if (name == 'desktop') await capture(t, '02-kinds-$name');

      await t.tap(find.text(ru.sourcesTypeSftp));
      await settle(t, 6);
      await _enter(t, 'source-url', 'sftp://nas.local/srv/media');
      await _enter(t, 'source-user', 'anna');
      if (name != 'desktop') {
        await t.ensureVisible(find.byKey(const ValueKey('source-url')));
        await settle(t, 2);
        await capture(t, '04-sftp-form-$name');
        return;
      }
      await t.ensureVisible(find.text(ru.sourcesFormAdvanced));
      await t.tap(find.text(ru.sourcesFormAdvanced));
      await settle(t, 4);
      await _enter(
        t,
        'source-ssh-host-key',
        'ssh-ed25519 SHA256:Xk1mE0bqN5yQm0RkVv7m3W9Q2pZt8sJdLfH4cA6uYe0',
      );
      await settle(t, 2);
      await capture(t, '03-sftp-form-$name');

      // The server now shows another key: refused with a clear message.
      (services.sourceManager as FakeSourceManager).nextError =
          const SourceException(
            SourceErrorCode.hostKeyChanged,
            'nas.local:22: the SSH host key changed (was ssh-ed25519 '
            'SHA256:Xk1m…, now ssh-ed25519 SHA256:q9Zr…)',
          );
      await t.ensureVisible(find.byKey(const ValueKey('source-submit')));
      await t.tap(find.byKey(const ValueKey('source-submit')));
      await settle(t, 6);
      expect(find.text(ru.sourcesErrorHostKeyChanged), findsOneWidget);
      await t.ensureVisible(find.text(ru.sourcesErrorHostKeyChanged));
      await settle(t, 2);
      await capture(t, '05-host-key-changed-$name');

      await t.tap(find.text(ru.commonCancel));
      await settle(t, 4);
      await t.ensureVisible(find.text(ru.addSourceTitle).first);
      await t.tap(find.text(ru.addSourceTitle).first);
      await settle(t, 6);
      await t.tap(find.text(ru.sourcesTypeFtp));
      await settle(t, 6);
      await _enter(t, 'source-url', 'ftpes://nas.local/video');
      await _enter(t, 'source-user', 'anna');
      await settle(t, 2);
      await capture(t, '06-ftp-form-$name');
    });
  }
}
