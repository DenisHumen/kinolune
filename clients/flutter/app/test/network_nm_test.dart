// SPDX-License-Identifier: MPL-2.0
//
// DIST-032: NmBackend against a fake NetworkManager on a private D-Bus
// (test/support/fake_network_manager.dart): the list with signal and
// lock, joining with and without a password, a wrong password, a hidden
// network, "Forget", auto-connect, Wi-Fi off, the cable, the address, the
// internet check and a captive portal; nothing without NetworkManager or
// without a Wi-Fi adapter. Passwords reach NetworkManager only.

import 'dart:io';

import 'package:dbus/dbus.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/network/network_backend.dart';
import 'package:kinolune/network/network_controller.dart';
import 'package:kinolune/network/nm_backend.dart';

import 'support/fake_network_manager.dart';

void main() {
  late FakeNetworkManager nm;
  late NmBackend backend;

  setUp(() async {
    nm = await FakeNetworkManager.start();
    await nm.addAp(FakeAp('Home', strength: 80, password: 'kinolune123'));
    await nm.addAp(FakeAp('Cafe', strength: 55, portal: true));
    await nm.addAp(
      FakeAp('Modern', strength: 35, password: 'sae-secret', wpa3: true),
    );
    await nm.addAp(FakeAp('Office', strength: 20, enterprise: true));
    await nm.addAp(FakeAp('Attic', password: 'attic-secret', hidden: true));
    backend = NmBackend(nm.client(), connectTimeout: const Duration(seconds: 5));
  });

  tearDown(() async {
    await backend.close();
    await nm.close();
  });

  test('nothing without NetworkManager on the bus', () async {
    final dir = await Directory.systemTemp.createTemp('kinolune-bus-');
    final server = DBusServer();
    final address = await server.listenAddress(DBusAddress.unix(dir: dir));
    final b = NmBackend(DBusClient(address));
    expect((await b.read()).available, isFalse);
    await b.close();
    await server.close();
    await dir.delete(recursive: true);
  });

  test('nothing without a Wi-Fi adapter', () async {
    nm.wifiAdapter = false;
    expect((await backend.read()).available, isFalse);
  });

  test('lists networks with signal and security, hidden ones left out', () async {
    final s = await backend.read();
    expect(s.available, isTrue);
    expect(s.wifiEnabled, isTrue);
    expect(s.networks.map((n) => n.ssid), ['Home', 'Cafe', 'Modern', 'Office']);
    final by = {for (final n in s.networks) n.ssid: n};
    expect(by['Home']!.security, WifiSecurity.wpaPersonal);
    expect(by['Home']!.strength, 80);
    expect(by['Cafe']!.security, WifiSecurity.open);
    expect(by['Modern']!.security, WifiSecurity.wpa3);
    expect(by['Office']!.security, WifiSecurity.enterprise);
    expect(s.active, isNull);
    expect(s.connectivity, NetConnectivity.none);
    expect(s.wired.present, isTrue);
    expect(s.wired.cable, isTrue);
    expect(s.wired.connected, isFalse);
    expect(s.portalUrl, 'http://check.example/hotspot.txt');
  });

  test('joins an open network without a password', () async {
    final s0 = await backend.read();
    final cafe = s0.networks.firstWhere((n) => n.ssid == 'Cafe');
    expect(await backend.connect(cafe), ConnectResult.ok);
    final s = await backend.read();
    expect(s.active?.ssid, 'Cafe');
    expect(s.active?.saved, isTrue);
    expect(s.wifiAddress, '192.168.50.23');
    // A café with a sign-in page.
    expect(s.connectivity, NetConnectivity.portal);
    expect(await backend.checkInternet(), NetConnectivity.portal);
    expect(nm.secretsReceived, isEmpty);
  });

  test('a wrong password is refused and not kept; the right one joins', () async {
    final home = (await backend.read()).networks.first;
    expect(
      await backend.connect(home, password: 'wrong-password'),
      ConnectResult.wrongPassword,
    );
    expect(nm.savedSsids, isEmpty, reason: 'failed profile removed');
    expect((await backend.read()).active, isNull);

    expect(
      await backend.connect(home, password: 'kinolune123'),
      ConnectResult.ok,
    );
    final s = await backend.read();
    expect(s.active?.ssid, 'Home');
    expect(s.connectivity, NetConnectivity.full);
    expect(nm.savedSsids, {'Home'});
    expect(nm.secretsReceived, ['wrong-password', 'kinolune123']);
  });

  test('a too short WPA password is a wrong password', () async {
    final home = (await backend.read()).networks.first;
    expect(
      await backend.connect(home, password: 'short'),
      ConnectResult.wrongPassword,
    );
  });

  test('a secured network asks a password before anything is sent', () async {
    final home = (await backend.read()).networks.first;
    expect(await backend.connect(home), ConnectResult.wrongPassword);
    expect(nm.calls, isNot(contains('NM.AddAndActivateConnection')));
  });

  test('a saved network joins again without a password', () async {
    final home = (await backend.read()).networks.first;
    await backend.connect(home, password: 'kinolune123');
    nm.dropWifi();
    expect((await backend.read()).active, isNull);
    final again = (await backend.read()).networks.first;
    expect(again.saved, isTrue);
    nm.calls.clear();
    expect(await backend.connect(again), ConnectResult.ok);
    expect(nm.calls, contains('NM.ActivateConnection'));
    expect(nm.calls, isNot(contains('NM.AddAndActivateConnection')));
    expect((await backend.read()).active?.ssid, 'Home');
  });

  test('WPA3 networks get SAE', () async {
    final modern = (await backend.read()).networks.firstWhere(
      (n) => n.ssid == 'Modern',
    );
    expect(
      await backend.connect(modern, password: 'sae-secret'),
      ConnectResult.ok,
    );
  });

  test('802.1X networks are left to the system settings', () async {
    final office = (await backend.read()).networks.firstWhere(
      (n) => n.ssid == 'Office',
    );
    expect(await backend.connect(office), ConnectResult.unsupported);
  });

  test('a hidden network joins by its name', () async {
    expect(
      await backend.connectHidden(
        'Nowhere',
        security: WifiSecurity.wpaPersonal,
        password: 'whatever1',
      ),
      ConnectResult.notFound,
    );
    expect(nm.savedSsids, isEmpty);
    expect(
      await backend.connectHidden(
        'Attic',
        security: WifiSecurity.wpaPersonal,
        password: 'attic-secret',
      ),
      ConnectResult.ok,
    );
    expect(nm.activeSsid, 'Attic');
  });

  test('forget removes the profile and drops the link', () async {
    final home = (await backend.read()).networks.first;
    await backend.connect(home, password: 'kinolune123');
    await backend.forget('Home');
    expect(nm.savedSsids, isEmpty);
    final s = await backend.read();
    expect(s.active, isNull);
    expect(s.connectivity, NetConnectivity.none);
    expect(s.networks.first.saved, isFalse);
  });

  test('auto-connect is switched off and on, the password stays', () async {
    final home = (await backend.read()).networks.first;
    await backend.connect(home, password: 'kinolune123');
    await backend.setAutoconnect('Home', false);
    expect((await backend.read()).networks.first.autoconnect, isFalse);
    await backend.setAutoconnect('Home', true);
    expect((await backend.read()).networks.first.autoconnect, isTrue);
    // NetworkManager still has the password: a new join needs none.
    nm.dropWifi();
    final again = (await backend.read()).networks.first;
    expect(await backend.connect(again), ConnectResult.ok);
    expect(nm.secretsReceived, ['kinolune123']);
  });

  test('scan asks the adapter', () async {
    await backend.scan();
    expect(nm.scans, 1);
  });

  test('Wi-Fi off hides the list', () async {
    await backend.setWifiEnabled(false);
    final s = await backend.read();
    expect(s.wifiEnabled, isFalse);
    expect(s.networks, isEmpty);
    await backend.setWifiEnabled(true);
    expect((await backend.read()).networks, isNotEmpty);
  });

  test('the cable: no cable, then connected with its address', () async {
    nm.carrier = false;
    var s = await backend.read();
    expect(s.wired.cable, isFalse);
    nm
      ..carrier = true
      ..wiredUp = true;
    s = await backend.read();
    expect(s.wired.connected, isTrue);
    expect(s.wired.address, '10.0.0.5');
    expect(s.address, '10.0.0.5');
    expect(s.connectivity, NetConnectivity.full);
  });

  test('the controller follows the backend and hides without Wi-Fi', () async {
    final c = NetworkController(
      backend,
      scanSettle: Duration.zero,
      poll: const Duration(hours: 1),
    );
    await c.start();
    expect(c.visible, isTrue);
    final home = c.snapshot.networks.first;
    expect(
      await c.connect(home, password: 'kinolune123'),
      ConnectResult.ok,
    );
    expect(c.snapshot.active?.ssid, 'Home');
    nm.wifiAdapter = false;
    await c.refresh();
    expect(c.visible, isFalse);
    // dispose closes the backend; tearDown closes it again harmlessly.
    c.dispose();
    backend = NmBackend(nm.client());
  });
}
