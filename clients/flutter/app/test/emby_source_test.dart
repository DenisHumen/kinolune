// SPDX-License-Identifier: MPL-2.0
//
// NET-015 in the app: adding an Emby server. The form is the Jellyfin one
// (NET-014) for kind `emby`: password sign-in only (no Quick Connect), a
// note about Emby Connect accounts, the sign-in goes to the core as Emby.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/screens/jellyfin_source_form.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';

import 'support/fake_jellyfin.dart';
import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));

Future<void> _openEmbyForm(WidgetTester tester, FakeJellyfinLink link) async {
  setSurface(tester, const Size(1280, 900));
  final services = await demoServices(jellyfin: link);
  await tester.pumpWidget(testApp(services, section: Section.settings));
  await settle(tester);
  await tester.tap(find.text(en.addSourceTitle).first);
  await settle(tester, 4);
  await tester.tap(find.text(en.sourcesTypeEmby));
  await settle(tester, 4);
}

Future<void> _enter(WidgetTester tester, String key, String text) async {
  await tester.ensureVisible(find.byKey(ValueKey(key)));
  await tester.enterText(find.byKey(ValueKey(key)), text);
  await tester.pump();
}

void main() {
  setUpAll(loadAppFonts);

  test('Emby is a media server the core adds', () {
    expect(SourceType.available, contains(SourceType.emby));
    expect(SourceType.emby.mediaServer, isTrue);
    expect(SourceType.emby.indexable, isTrue);
    expect(SourceType.fromCoreKind('emby'), SourceType.emby);
    expect(SourceType.emby.coreKind, 'emby');
  });

  testWidgets('signs in with a password, as Emby, without Quick Connect', (
    tester,
  ) async {
    final link = FakeJellyfinLink();
    await _openEmbyForm(tester, link);
    final form = tester.widget<JellyfinSourceForm>(
      find.byType(JellyfinSourceForm),
    );
    expect(form.server, SourceType.emby);
    expect(find.textContaining(en.sourcesTypeEmby), findsOneWidget);
    expect(find.byKey(const ValueKey('jellyfin-mode')), findsNothing);
    expect(find.text(en.sourcesJellyfinModeQuickConnect), findsNothing);
    expect(find.byKey(const ValueKey('emby-local-user-note')), findsOneWidget);
    expect(find.text(en.sourcesJellyfinTokenNote), findsOneWidget);

    await _enter(tester, 'jellyfin-address', '192.168.1.20:8096');
    await _enter(tester, 'jellyfin-user', 'anna');
    await _enter(tester, 'jellyfin-password', 'wrong');
    await tester.ensureVisible(find.byKey(const ValueKey('jellyfin-submit')));
    await tester.tap(find.byKey(const ValueKey('jellyfin-submit')));
    await settle(tester, 8);
    expect(link.calls.last, 'signIn emby http://192.168.1.20:8096 anna 5');
    expect(find.text(en.sourcesErrorAuth), findsOneWidget);

    await _enter(tester, 'jellyfin-password', 'secret');
    await tester.tap(find.byKey(const ValueKey('jellyfin-submit')));
    await settle(tester, 8);
    expect(find.byType(JellyfinSourceForm), findsNothing);
  });
}
