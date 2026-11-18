// SPDX-License-Identifier: MPL-2.0
//
// NET-012 in the app: adding a Plex server with a code entered at
// plex.tv/link — the code is shown at once, then "who is watching" (a
// protected Home user asks for the PIN), then the server (a single one is
// added without asking); an expired code gives a new one; a server found
// on the network is preselected; editing renames.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/screens/add_source_dialog.dart';
import 'package:kinolune/screens/plex_source_form.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/discovery.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';

import 'support/fake_plex.dart';
import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));

/// Polls run every 2 s: 25 frames of 100 ms see one.
Future<void> _poll(WidgetTester tester) => settle(tester, 25);

Future<AppServices> _openPlexForm(
  WidgetTester tester,
  FakePlexLink link,
) async {
  setSurface(tester, const Size(1280, 900));
  final services = await demoServices(plex: link);
  await tester.pumpWidget(testApp(services, section: Section.settings));
  await settle(tester);
  await tester.tap(find.text(en.addSourceTitle).first);
  await settle(tester, 4);
  await tester.tap(find.text(en.sourcesTypePlex));
  await settle(tester, 4);
  return services;
}

void main() {
  setUpAll(loadAppFonts);

  test('Plex is a kind the core adds by signing in', () {
    expect(SourceType.available, contains(SourceType.plex));
    expect(SourceType.fromCoreKind('plex'), SourceType.plex);
    expect(SourceType.plex.plexServer, isTrue);
    expect(SourceType.plex.mediaServer, isFalse);
    expect(SourceType.plex.indexable, isTrue);
  });

  testWidgets('without the core Plex is not offered', (tester) async {
    setSurface(tester, const Size(1280, 900));
    final services = await demoServices();
    await tester.pumpWidget(testApp(services, section: Section.settings));
    await settle(tester);
    await tester.tap(find.text(en.addSourceTitle).first);
    await settle(tester, 4);
    final tile = tester.widget<ListTile>(
      find.ancestor(
        of: find.text(en.sourcesTypePlex),
        matching: find.byType(ListTile),
      ),
    );
    expect(tile.enabled, isFalse);
  });

  testWidgets('code, protected Home user with a PIN, then the server', (
    tester,
  ) async {
    final link = FakePlexLink();
    await _openPlexForm(tester, link);
    // The code is there at once, with where to enter it.
    expect(find.byKey(const ValueKey('plex-code')), findsOneWidget);
    expect(find.text('K7Q2'), findsOneWidget);
    expect(find.text(en.sourcesPlexLinkHow('plex.tv/link')), findsOneWidget);
    expect(find.text(en.sourcesPlexWaiting), findsOneWidget);
    expect(link.calls, ['start']);

    await _poll(tester);
    expect(find.text(en.sourcesPlexWhoIsWatching), findsOneWidget);
    expect(find.text('Anna'), findsOneWidget);
    expect(find.text(en.sourcesPlexOwner), findsOneWidget);
    expect(find.text(en.sourcesPlexManaged), findsOneWidget);

    // The kid is protected: the PIN first; a wrong one says so.
    await tester.tap(find.byKey(const ValueKey('plex-user-kid')));
    await settle(tester, 4);
    expect(find.byKey(const ValueKey('plex-pin')), findsOneWidget);
    await tester.enterText(find.byKey(const ValueKey('plex-pin')), '0000');
    await tester.tap(find.byKey(const ValueKey('plex-pin-continue')));
    await settle(tester, 4);
    expect(find.text(en.sourcesPlexWrongPin), findsOneWidget);
    await tester.enterText(find.byKey(const ValueKey('plex-pin')), '1234');
    await tester.tap(find.byKey(const ValueKey('plex-pin-continue')));
    await settle(tester, 4);
    expect(link.calls, contains('switch kid 1234'));

    // Two servers: the user picks one.
    expect(find.text(en.sourcesPlexChooseServer), findsOneWidget);
    expect(find.text("Max's Plex"), findsOneWidget);
    expect(
      find.textContaining(en.sourcesPlexServerShared),
      findsOneWidget,
    );
    await tester.tap(find.byKey(const ValueKey('plex-server-friend')));
    await settle(tester, 2);
    await tester.tap(find.byKey(const ValueKey('plex-add')));
    await settle(tester, 6);
    expect(link.calls.last, 'finish friend');
    // The dialog closed with the new source.
    expect(find.byType(PlexSourceForm), findsNothing);
  });

  testWidgets('one server and no Home: added right after the code', (
    tester,
  ) async {
    final link = FakePlexLink(home: false, twoServers: false);
    await _openPlexForm(tester, link);
    await _poll(tester);
    await settle(tester, 6);
    expect(link.calls, ['start', 'poll 1', 'finish living']);
    expect(find.byType(PlexSourceForm), findsNothing);
  });

  testWidgets('the owner needs no PIN in the app', (tester) async {
    final link = FakePlexLink(twoServers: false);
    await _openPlexForm(tester, link);
    await _poll(tester);
    await tester.tap(find.byKey(const ValueKey('plex-user-owner')));
    await settle(tester, 8);
    expect(link.calls, ['start', 'poll 1', 'switch owner -', 'finish living']);
  });

  testWidgets('an expired code gives a new one', (tester) async {
    final link = FakePlexLink(expireCode: true);
    await _openPlexForm(tester, link);
    await _poll(tester);
    expect(find.byKey(const ValueKey('plex-expired')), findsOneWidget);
    expect(find.text('K7Q2'), findsNothing);
    await tester.tap(find.byKey(const ValueKey('plex-new-code')));
    await settle(tester, 4);
    expect(find.text('M3XP'), findsOneWidget);
    // Cancel ends the sign-in at the core.
    await tester.tap(find.byKey(const ValueKey('plex-cancel')));
    await settle(tester, 4);
    expect(link.calls.last, 'cancel 2');
  });

  testWidgets('a server found on the network is preselected', (tester) async {
    setSurface(tester, const Size(1280, 900));
    final link = FakePlexLink(home: false);
    final services = await demoServices(plex: link);
    await tester.pumpWidget(testApp(services, section: Section.settings));
    await settle(tester);
    final context = tester.element(find.byType(Scaffold).first);
    showAddSourceDialog(
      context,
      scanAfter: false,
      discovered: const DiscoveredSource(
        type: SourceType.plex,
        name: 'Living Room',
        host: '192.168.1.41',
        url: 'http://192.168.1.41:32400',
      ),
    );
    await settle(tester, 4);
    expect(link.calls, ['start http://192.168.1.41:32400']);
    await _poll(tester);
    final living = tester.widget<ListTile>(
      find.byKey(const ValueKey('plex-server-living')),
    );
    expect(living.selected, isTrue);
  });

  testWidgets('editing renames, the sign-in stays', (tester) async {
    setSurface(tester, const Size(1280, 900));
    final link = FakePlexLink();
    final services = await demoServices(plex: link);
    await tester.pumpWidget(testApp(services, section: Section.settings));
    await settle(tester);
    final context = tester.element(find.byType(Scaffold).first);
    const source = ManagedSource(
      id: '77',
      type: SourceType.plex,
      name: 'Living Room',
      address: 'https://192-168-1-41.abc.plex.direct:32400',
      username: 'Anna',
      hasPassword: true,
    );
    showEditSourceDialog(context, source);
    await settle(tester, 4);
    expect(find.byKey(const ValueKey('plex-code')), findsNothing);
    expect(link.calls, isEmpty);
    final name = tester.widget<TextField>(
      find.byKey(const ValueKey('plex-name')),
    );
    expect(name.controller!.text, 'Living Room');
  });
}
