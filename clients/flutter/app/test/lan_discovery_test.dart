// SPDX-License-Identifier: MPL-2.0
//
// NET-010 in the app: Windows shares, Jellyfin, Emby and Plex servers found
// on the network (WS-Discovery, NetBIOS, GDM, UDP auto-discovery of the
// core) join «В этой сети»; tapping one prefills its form, Plex is "soon"
// until its sign-in (NET-012). With KINOLUNE_SCREENSHOTS_DIR set the frames
// are written to docs/screenshots/progress/net-010:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/net-010 \
//     flutter test test/lan_discovery_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/core_discovery.dart';
import 'package:kinolune/sources/discovery.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_core/pa_core.dart';

import 'support/fake_jellyfin.dart';
import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

CoreLanService _lan(CoreLanKind kind, String name, String url) =>
    CoreLanService(
      kind: kind,
      name: name,
      host: Uri.parse(url).host,
      url: url,
    );

final _found = [
  for (final s in [
    _lan(CoreLanKind.smb, 'OFFICE-PC', 'smb://192.168.1.31/'),
    _lan(CoreLanKind.jellyfin, 'Home Jellyfin', 'http://192.168.1.40:8096'),
    _lan(CoreLanKind.plex, 'Living Room Plex', 'http://192.168.1.41:32400'),
  ])
    discoveredSourceFromLan(s),
];

Future<FakeJellyfinLink> _open(WidgetTester tester, {Locale? locale}) async {
  setSurface(tester, const Size(1280, 900));
  final link = FakeJellyfinLink();
  final services = await demoServices(
    discovery: StaticSourceDiscovery(_found),
    jellyfin: link,
  );
  if (locale != null) services.locale.value = locale;
  await tester.pumpWidget(testApp(services, section: Section.settings));
  await settle(tester);
  final l = locale == null ? en : ru;
  await tester.tap(find.text(l.addSourceTitle).first);
  await settle(tester, 4);
  return link;
}

void main() {
  setUpAll(loadAppFonts);

  test('LAN services of the core map to the add-flow kinds', () {
    final kinds = {
      for (final k in CoreLanKind.values)
        k: discoveredSourceFromLan(_lan(k, 'x', 'http://10.0.0.2:1')).type,
    };
    expect(kinds, {
      CoreLanKind.smb: SourceType.smb,
      CoreLanKind.jellyfin: SourceType.jellyfin,
      CoreLanKind.emby: SourceType.emby,
      CoreLanKind.plex: SourceType.plex,
    });
    final https = discoveredSourceFromLan(
      _lan(CoreLanKind.jellyfin, 'x', 'https://10.0.0.2:8920'),
    );
    expect(https.secure, isTrue);
    expect(https.host, '10.0.0.2');
  });

  test('one search failing leaves the others; all failing fails', () async {
    const a = DiscoveredSource(type: SourceType.dlna, name: 'a', host: 'h');
    Future<List<DiscoveredSource>> ok() async => [a];
    Future<List<DiscoveredSource>> bad() async => throw StateError('no net');
    expect(await coreNetworkSearch(searches: [ok, bad]), [a]);
    await expectLater(
      coreNetworkSearch(searches: [bad, bad]),
      throwsA(isA<StateError>()),
    );
    expect(await coreNetworkSearch(searches: const []), isEmpty);
  });

  test('a share found twice (mDNS and WS-Discovery) is listed once', () {
    const mdns = DiscoveredSource(
      type: SourceType.smb,
      name: 'nas',
      host: 'nas.local',
      url: 'smb://192.168.1.20/',
    );
    const wsd = DiscoveredSource(
      type: SourceType.smb,
      name: 'NAS',
      host: '192.168.1.20',
      url: 'smb://192.168.1.20/',
    );
    const dlna = DiscoveredSource(
      type: SourceType.dlna,
      name: 'nas',
      host: '192.168.1.20',
      url: 'http://192.168.1.20:8200/rootDesc.xml',
    );
    expect(withoutDuplicates([mdns, wsd, dlna]), [mdns, dlna]);
  });

  testWidgets('found servers are listed; Plex is soon', (tester) async {
    await _open(tester);
    expect(find.text('OFFICE-PC'), findsOneWidget);
    expect(find.text('${en.sourcesTypeSmb} · 192.168.1.31'), findsOneWidget);
    expect(find.text('Home Jellyfin'), findsOneWidget);
    final plex = tester.widget<ListTile>(
      find.ancestor(
        of: find.text('Living Room Plex'),
        matching: find.byType(ListTile),
      ),
    );
    expect(plex.enabled, isFalse);
    expect(plex.onTap, isNull);
  });

  testWidgets('a Jellyfin server found on the network prefills its form', (
    tester,
  ) async {
    final link = await _open(tester);
    await tester.tap(find.text('Home Jellyfin'));
    await settle(tester, 4);
    final address = tester.widget<TextField>(
      find.byKey(const ValueKey('jellyfin-address')),
    );
    expect(address.controller!.text, 'http://192.168.1.40:8096');
    final name = tester.widget<TextField>(
      find.byKey(const ValueKey('jellyfin-name')),
    );
    expect(name.controller!.text, 'Home Jellyfin');
    // Nothing is sent before the user signs in.
    expect(link.calls, isEmpty);
  });

  testWidgets('a Windows share opens the SMB flow at sign-in', (tester) async {
    await _open(tester);
    await tester.tap(find.text('OFFICE-PC'));
    await settle(tester, 4);
    // Step 2 of the SMB flow (UI-028): the server is known, sign in.
    expect(find.textContaining('192.168.1.31'), findsWidgets);
  });

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('phone', FormFactor.phone, const Size(390, 844)),
  ]) {
    testWidgets('screenshots on $name', (t) async {
      setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
      final services = await demoServices(
        discovery: StaticSourceDiscovery(_found),
        jellyfin: FakeJellyfinLink(),
      );
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t);
      await t.tap(find.text(ru.addSourceTitle).first);
      await settle(t, 6);
      await capture(t, '01-found-$name');
      await t.tap(find.text('Home Jellyfin'));
      await settle(t, 6);
      await capture(t, '02-jellyfin-prefilled-$name');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
