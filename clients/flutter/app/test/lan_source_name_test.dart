// SPDX-License-Identifier: MPL-2.0
//
// BUG-004: a server picked in «В этой сети» is saved by the IP that
// answered together with its stable name (mDNS host, computer name, DLNA
// UDN), so the core finds it again after DHCP gives it another IP; the
// sources screen shows the name, not the IP.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/core_discovery.dart';
import 'package:kinolune/sources/discovery.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_core/pa_core.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));

FakeSourceManager _fake(AppServices s) => s.sourceManager as FakeSourceManager;

const _nas = StaticSourceDiscovery([
  DiscoveredSource(
    type: SourceType.smb,
    name: 'Home NAS',
    host: 'nas.local',
    url: 'smb://192.168.1.10/',
    stableName: 'nas.local',
  ),
]);

Future<AppServices> _open(WidgetTester t) async {
  setSurface(t, const Size(1280, 900));
  final services = await demoServices(discovery: _nas);
  await t.pumpWidget(testApp(services, section: Section.settings));
  await settle(t);
  return services;
}

Future<void> _tapKey(WidgetTester t, String key) async {
  final f = find.byKey(ValueKey(key));
  await t.ensureVisible(f);
  await t.pump();
  await t.tap(f);
  await settle(t, 6);
}

Future<void> _enter(WidgetTester t, String key, String text) async {
  final f = find.byKey(ValueKey(key));
  await t.ensureVisible(f);
  await t.enterText(f, text);
  await t.pump();
}

Future<void> _startSmb(WidgetTester t) async {
  await t.ensureVisible(find.text(en.addSourceTitle).first);
  await t.tap(find.text(en.addSourceTitle).first);
  await settle(t, 6);
  await t.tap(find.text(en.sourcesTypeSmb));
  await settle(t, 6);
}

Future<void> _signInAndPickMedia(WidgetTester t) async {
  await _enter(t, 'smb-user', 'alex');
  await _enter(t, 'smb-password', 'secret');
  await _tapKey(t, 'smb-connect');
  await _tapKey(t, 'smb-share-Media');
  await _tapKey(t, 'smb-use-folder');
}

void main() {
  setUpAll(loadAppFonts);

  group('the stable name of what discovery found', () {
    test('mDNS: the host name, not an IP', () {
      CoreDiscoveredService mdns(String host) => CoreDiscoveredService(
        kind: CoreServiceKind.smb,
        instance: 'Home NAS',
        host: host,
        port: 445,
        addresses: const ['192.168.1.10'],
        url: 'smb://192.168.1.10/',
      );
      expect(
        discoveredSourceFromCore(mdns('nas.local'))!.stableName,
        'nas.local',
      );
      expect(discoveredSourceFromCore(mdns(''))!.stableName, isNull);
      expect(
        discoveredSourceFromCore(mdns('192.168.1.10'))!.stableName,
        isNull,
      );
    });

    test('NetBIOS / WS-Discovery: the computer name of a share', () {
      CoreLanService lan(CoreLanKind kind, String name, String url) =>
          CoreLanService(
            kind: kind,
            name: name,
            host: Uri.parse(url).host,
            url: url,
          );
      expect(
        discoveredSourceFromLan(
          lan(CoreLanKind.smb, 'OFFICE-PC', 'smb://192.168.1.31/'),
        ).stableName,
        'OFFICE-PC',
      );
      // A computer that gave no name is listed by its IP.
      expect(
        discoveredSourceFromLan(
          lan(CoreLanKind.smb, '192.168.1.31', 'smb://192.168.1.31/'),
        ).stableName,
        isNull,
      );
      // Media servers keep their server id from the sign-in.
      expect(
        discoveredSourceFromLan(
          lan(CoreLanKind.jellyfin, 'Home', 'http://192.168.1.40:8096'),
        ).stableName,
        isNull,
      );
    });

    test('the address for people shows the name', () {
      const m = ManagedSource(
        id: '1',
        type: SourceType.smb,
        name: 'Movies',
        address: 'smb://192.168.1.10/Media/Movies',
        lanName: 'nas.local',
      );
      expect(sourceDisplayAddress(m), 'smb://nas.local/Media/Movies');
      const old = ManagedSource(
        id: '2',
        type: SourceType.smb,
        name: 'Movies',
        address: 'smb://192.168.1.10/Media/Movies',
      );
      expect(sourceDisplayAddress(old), 'smb://192.168.1.10/Media/Movies');
      const dlna = ManagedSource(
        id: '3',
        type: SourceType.dlna,
        name: 'TV',
        address: 'http://192.168.1.12:8200/rootDesc.xml',
        lanName: 'uuid:4d696e69-444c-164e-9d41-001122334455',
      );
      expect(sourceDisplayAddress(dlna), dlna.address);
    });
  });

  group('SMB flow', () {
    testWidgets('a server found on the network is saved with its name', (
      t,
    ) async {
      final services = await _open(t);
      await _startSmb(t);
      await _tapKey(t, 'smb-server-192.168.1.10');
      await _signInAndPickMedia(t);
      final d = _fake(services).drafts.single;
      // Connect by IP (fast, Android); find again by name.
      expect(d.address, 'smb://192.168.1.10/Media');
      expect(d.lanName, 'nas.local');

      // The sources screen shows the name, not the IP.
      await t.ensureVisible(find.text(en.sourcesManage));
      await t.tap(find.text(en.sourcesManage));
      await settle(t, 8);
      expect(find.textContaining('smb://nas.local/Media'), findsWidgets);
      expect(find.textContaining('smb://192.168.1.10/Media'), findsNothing);
    });

    testWidgets('a typed server has no name to keep', (t) async {
      final services = await _open(t);
      await _startSmb(t);
      await _enter(t, 'smb-server', '192.168.1.10');
      await _tapKey(t, 'smb-next');
      await _signInAndPickMedia(t);
      final d = _fake(services).drafts.single;
      expect(d.address, 'smb://192.168.1.10/Media');
      expect(d.lanName, isNull);
    });
  });
}
