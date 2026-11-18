// SPDX-License-Identifier: MPL-2.0
//
// NET-018: a DLNA server found on the network and an M3U playlist added as
// sources, in Russian with real fonts. With KINOLUNE_SCREENSHOTS_DIR set
// the frames are written to docs/screenshots/progress/net-018:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/net-018 \
//     flutter test test/dlna_playlist_screenshots_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/discovery.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

const _found = [
  DiscoveredSource(
    type: SourceType.smb,
    name: 'nas',
    host: 'nas.local',
    url: 'smb://192.168.1.20/',
    secure: false,
  ),
  DiscoveredSource(
    type: SourceType.dlna,
    name: 'Домашний NAS (DLNA)',
    host: '192.168.1.20',
    url: 'http://192.168.1.20:8200/rootDesc.xml',
    secure: false,
  ),
];

void main() {
  setUpAll(loadAppFonts);

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('phone', FormFactor.phone, const Size(390, 844)),
  ]) {
    testWidgets('screenshots on $name', (t) async {
      setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
      final services = await demoServices(
        discovery: const StaticSourceDiscovery(_found),
      );
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t);

      // The DLNA server is in «В этой сети» next to the SMB share.
      await t.tap(find.text(ru.addSourceTitle).first);
      await settle(t, 6);
      expect(find.text('Домашний NAS (DLNA)'), findsOneWidget);
      await capture(t, '01-on-this-network-$name');

      await t.tap(find.text('Домашний NAS (DLNA)'));
      await settle(t, 6);
      expect(find.byKey(const ValueKey('source-password')), findsNothing);
      await capture(t, '02-dlna-form-$name');
      await t.ensureVisible(find.byKey(const ValueKey('source-submit')));
      await t.tap(find.byKey(const ValueKey('source-submit')));
      await settle(t, 8);

      // An M3U playlist by link.
      await t.tap(find.text(ru.addSourceTitle).first);
      await settle(t, 6);
      await t.ensureVisible(find.text(ru.sourcesTypePlaylist));
      await settle(t, 2);
      if (name == 'desktop') await capture(t, '03-kinds-$name');
      await t.tap(find.text(ru.sourcesTypePlaylist));
      await settle(t, 6);
      await t.enterText(
        find.byKey(const ValueKey('source-url')),
        'https://media.example/lists/films.m3u8',
      );
      await t.enterText(
        find.byKey(const ValueKey('source-name')),
        'Фильмы по ссылкам',
      );
      await settle(t, 4);
      await capture(t, '04-playlist-form-$name');
      await t.ensureVisible(find.byKey(const ValueKey('source-submit')));
      await t.tap(find.byKey(const ValueKey('source-submit')));
      await settle(t, 8);

      // Both in the list of sources.
      expect(find.text('Фильмы по ссылкам'), findsWidgets);
      await capture(t, '05-sources-$name');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
