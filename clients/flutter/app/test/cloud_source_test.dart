// SPDX-License-Identifier: MPL-2.0
//
// NET-017 in the app: clouds in the «Добавить» list under their own
// header; Google Drive signs in through the browser (the form waits, then
// asks for an optional folder); a cloud without a client id in this build
// takes the user's own; a declined sign-in says so; MEGA asks for the 2FA
// code when the account has it; a MEGA link is added as it is.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/app_links.dart';
import 'package:kinolune/screens/add_source_dialog.dart';
import 'package:kinolune/screens/cloud_source_form.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';

import 'support/fake_cloud.dart';
import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));

/// Opens the form of [type] in a dialog; [opened] collects browser pages.
Future<void> _openForm(
  WidgetTester tester,
  AppServices services,
  SourceType type,
  List<Uri> opened,
) async {
  setSurface(tester, const Size(1280, 900));
  await tester.pumpWidget(testApp(services, section: Section.settings));
  await settle(tester);
  final context = tester.element(find.byType(AppShell));
  showDialog<ManagedSource>(
    context: context,
    builder: (_) => Dialog(
      child: CloudSourceForm(
        type: type,
        openUrl: (u) async {
          opened.add(u);
          return true;
        },
      ),
    ),
  );
  await settle(tester, 4);
}

// The add dialog's own list: the settings screen behind it scrolls too.
final _dialogScrollable = find
    .descendant(
      of: find.byType(AddSourceDialog),
      matching: find.byType(Scrollable),
    )
    .first;

void main() {
  setUpAll(loadAppFonts);

  test('clouds are kinds the core adds by signing in', () {
    for (final t in [
      SourceType.mega,
      SourceType.gdrive,
      SourceType.onedrive,
      SourceType.dropbox,
    ]) {
      expect(SourceType.available, contains(t));
      expect(SourceType.fromCoreKind(t.coreKind), t);
      expect(t.cloud, isTrue);
      expect(t.indexable, isTrue);
    }
    expect(SourceType.smb.cloud, isFalse);
  });

  test('the sign-in answer link goes to the cloud form', () {
    final link = AppLink.parse('kinolune://oauth?code=c&state=s');
    expect(link, isA<OAuthLink>());
    expect((link! as OAuthLink).link, 'kinolune://oauth?code=c&state=s');
  });

  testWidgets('the add list shows the clouds under their header', (
    tester,
  ) async {
    setSurface(tester, const Size(1280, 1400));
    final services = await demoServices(cloud: FakeCloudLink());
    await tester.pumpWidget(testApp(services, section: Section.settings));
    await settle(tester);
    await tester.tap(find.text(en.addSourceTitle).first);
    await settle(tester, 4);
    await tester.scrollUntilVisible(
      find.text('Dropbox'),
      200,
      scrollable: _dialogScrollable,
    );
    expect(find.text(en.sourcesAddClouds), findsOneWidget);
    for (final name in ['MEGA', 'Google Drive', 'OneDrive', 'Dropbox']) {
      final tile = tester.widget<ListTile>(
        find.ancestor(of: find.text(name), matching: find.byType(ListTile)),
      );
      expect(tile.enabled, isTrue, reason: name);
    }
    await tester.tap(find.text('Google Drive'));
    await settle(tester, 4);
    expect(find.byType(CloudSourceForm), findsOneWidget);
    expect(find.text(en.sourcesCloudIntro('Google Drive')), findsOneWidget);
  });

  testWidgets('without the core the clouds are not offered', (tester) async {
    setSurface(tester, const Size(1280, 1400));
    final services = await demoServices();
    await tester.pumpWidget(testApp(services, section: Section.settings));
    await settle(tester);
    await tester.tap(find.text(en.addSourceTitle).first);
    await settle(tester, 4);
    await tester.scrollUntilVisible(
      find.text('MEGA'),
      200,
      scrollable: _dialogScrollable,
    );
    final tile = tester.widget<ListTile>(
      find.ancestor(of: find.text('MEGA'), matching: find.byType(ListTile)),
    );
    expect(tile.enabled, isFalse);
  });

  testWidgets('Google Drive: browser, then an optional folder', (tester) async {
    final link = FakeCloudLink();
    final opened = <Uri>[];
    final services = await demoServices(cloud: link);
    await _openForm(tester, services, SourceType.gdrive, opened);
    expect(find.text(en.sourcesCloudReadOnly), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('cloud-sign-in')));
    await settle(tester, 4);
    expect(opened.single.host, 'accounts.example');
    expect(find.text(en.sourcesCloudWaiting), findsOneWidget);

    link.browser.complete();
    await settle(tester, 4);
    expect(
      find.text(en.sourcesCloudSignedIn('anna@example.org')),
      findsOneWidget,
    );
    await tester.enterText(
      find.byKey(const ValueKey('cloud-folder')),
      'Movies',
    );
    await tester.tap(find.byKey(const ValueKey('cloud-add')));
    await settle(tester, 4);
    expect(link.calls, ['start gdrive -', 'wait', 'finish Movies']);
    expect(find.byType(CloudSourceForm), findsNothing);
  });

  testWidgets('a cloud without a build key takes the own client id', (
    tester,
  ) async {
    final link = FakeCloudLink();
    final services = await demoServices(cloud: link);
    await _openForm(tester, services, SourceType.onedrive, []);
    expect(find.byKey(const ValueKey('cloud-no-client')), findsOneWidget);
    FilledButton button() => tester.widget<FilledButton>(
      find.byKey(const ValueKey('cloud-sign-in')),
    );
    expect(button().onPressed, isNull);
    await tester.enterText(
      find.byKey(const ValueKey('cloud-client-id')),
      'my-app-id',
    );
    await settle(tester, 2);
    expect(button().onPressed, isNotNull);
    await tester.tap(find.byKey(const ValueKey('cloud-sign-in')));
    await settle(tester, 4);
    expect(link.calls.first, 'start onedrive my-app-id');
  });

  testWidgets('a declined sign-in says so', (tester) async {
    final link = FakeCloudLink(declineBrowser: true);
    final services = await demoServices(cloud: link);
    await _openForm(tester, services, SourceType.dropbox, []);
    await tester.tap(find.byKey(const ValueKey('cloud-sign-in')));
    await settle(tester, 4);
    link.browser.complete();
    await settle(tester, 4);
    expect(find.byKey(const ValueKey('cloud-cancelled')), findsOneWidget);
    expect(find.byKey(const ValueKey('cloud-sign-in')), findsOneWidget);
  });

  testWidgets('MEGA asks for the two-factor code', (tester) async {
    final link = FakeCloudLink();
    final services = await demoServices(cloud: link);
    await _openForm(tester, services, SourceType.mega, []);
    expect(find.text(en.sourcesCloudMegaNote), findsOneWidget);
    await tester.enterText(
      find.byKey(const ValueKey('cloud-mega-email')),
      'anna@example.org',
    );
    await tester.enterText(
      find.byKey(const ValueKey('cloud-mega-password')),
      'secret',
    );
    await settle(tester, 2);
    await tester.tap(find.byKey(const ValueKey('cloud-mega-add')));
    await settle(tester, 4);
    expect(find.byKey(const ValueKey('cloud-mega-mfa')), findsOneWidget);
    await tester.enterText(
      find.byKey(const ValueKey('cloud-mega-mfa')),
      '123456',
    );
    await tester.tap(find.byKey(const ValueKey('cloud-mega-add')));
    await settle(tester, 4);
    expect(link.calls, [
      'mega anna@example.org -',
      'mega anna@example.org 123456',
    ]);
    expect(find.byType(CloudSourceForm), findsNothing);
  });

  testWidgets('MEGA link', (tester) async {
    final link = FakeCloudLink();
    final services = await demoServices(cloud: link);
    await _openForm(tester, services, SourceType.mega, []);
    await tester.tap(find.text(en.sourcesCloudMegaLink));
    await settle(tester, 2);
    await tester.enterText(
      find.byKey(const ValueKey('cloud-mega-link')),
      'https://mega.nz/folder/AbCd1234#AAAAAAAAAAAAAAAAAAAAAA',
    );
    await settle(tester, 2);
    await tester.tap(find.byKey(const ValueKey('cloud-mega-add')));
    await settle(tester, 4);
    expect(link.calls, ['mega-link']);
  });
}
