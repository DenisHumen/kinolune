// SPDX-License-Identifier: MPL-2.0
//
// UI-019: Settings → Devices. Pairing by code and by a scanned QR payload
// between two fake devices on one fake LAN, errors, unpairing, and remote
// changes refreshing the library.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/screens/devices_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sync/device_sync.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final _en = lookupAppLocalizations(const Locale('en'));

Future<void> _openDevices(WidgetTester t) async {
  await t.ensureVisible(find.text(_en.devicesTitle).last);
  await settle(t, 4);
  await t.tap(find.text(_en.devicesTitle).last);
  await settle(t, 10);
}

void main() {
  setUpAll(loadAppFonts);

  testWidgets('the demo family is listed with online state', (t) async {
    setSurface(t, const Size(1280, 900));
    final services = await demoServices();
    await t.pumpWidget(testApp(services, section: Section.settings));
    await settle(t);
    expect(find.text(_en.settingsDevicesCount(2)), findsOneWidget);
    await _openDevices(t);
    expect(find.text('Living room TV'), findsOneWidget);
    expect(find.text('Pixel 8'), findsOneWidget);
    expect(find.text(_en.devicesOnline), findsOneWidget);
    expect(find.text(_en.devicesOffline), findsOneWidget);
    expect(find.text(_en.devicesThisDeviceTag), findsOneWidget);
  });

  testWidgets('show a code, the other device types it and pairs', (t) async {
    setSurface(t, const Size(1280, 900));
    final lan = FakeSyncLan();
    final tv = FakeDeviceSync(
      id: 'tv',
      name: 'TV',
      kind: SyncDeviceKind.tv,
      lan: lan,
    );
    final phone = FakeDeviceSync(
      id: 'phone',
      name: 'Phone',
      kind: SyncDeviceKind.phone,
      lan: lan,
    );
    final services = await demoServices(sync: tv);
    await t.pumpWidget(testApp(services, section: Section.settings));
    await settle(t);
    await _openDevices(t);
    expect(find.text(_en.devicesNone), findsOneWidget);

    await t.tap(find.text(_en.devicesShowCode));
    await settle(t, 6);
    final invite = tv.openInvite!;
    expect(
      find.text('${invite.code.substring(0, 3)} ${invite.code.substring(3)}'),
      findsOneWidget,
    );
    expect(find.byKey(const ValueKey('devices-invite-code')), findsOneWidget);

    // The phone (another app) types a wrong code, then the right one.
    await expectLater(
      phone.join(invite.code == '123456' ? '654321' : '123456'),
      throwsA(
        isA<SyncException>().having(
          (e) => e.code,
          'code',
          SyncErrorCode.wrongCode,
        ),
      ),
    );
    final host = await phone.join(invite.code);
    expect(host.name, 'TV');
    await settle(t, 6);
    expect(find.text(_en.devicesPaired('Phone')), findsOneWidget);
    expect(find.text('Phone'), findsOneWidget);
    expect(find.byKey(const ValueKey('devices-invite-code')), findsNothing);
    expect(tv.paired, isTrue);
    expect(phone.devices().map((d) => d.name), ['Phone', 'TV']);

    // Unpair the phone from the TV.
    await t.tap(find.text(_en.devicesUnpair).first);
    await settle(t, 6);
    expect(find.text(_en.devicesUnpairTitle('Phone')), findsOneWidget);
    await t.tap(find.text(_en.devicesUnpair).last);
    await settle(t, 6);
    expect(find.text('Phone'), findsNothing);
    expect(tv.paired, isFalse);
  });

  testWidgets('join by typed code and by scanned QR; errors are shown', (
    t,
  ) async {
    setSurface(t, const Size(1280, 900));
    final lan = FakeSyncLan();
    final desktop = FakeDeviceSync(id: 'desk', name: 'Desk', lan: lan);
    final phone = FakeDeviceSync(
      id: 'phone',
      name: 'Phone',
      kind: SyncDeviceKind.phone,
      lan: lan,
    );
    final services = await demoServices(sync: phone);
    String? scanned;
    await t.pumpWidget(testApp(services, section: Section.settings));
    await settle(t);
    final nav = t.state<NavigatorState>(find.byType(Navigator).last);
    nav.push(
      MaterialPageRoute<void>(
        builder: (_) => DevicesScreen(scanQr: (_) async => scanned),
      ),
    );
    await settle(t, 10);

    // Too short, then nobody shows a code.
    await t.enterText(find.byKey(const ValueKey('devices-code')), '12');
    await t.tap(find.text(_en.devicesConnect));
    await settle(t, 6);
    expect(find.text(_en.devicesErrorInvalidCode), findsOneWidget);
    await t.enterText(find.byKey(const ValueKey('devices-code')), '123456');
    await t.tap(find.text(_en.devicesConnect));
    await settle(t, 6);
    expect(find.text(_en.devicesErrorNotFound), findsOneWidget);

    // The desktop shows a code; the phone scans its QR code.
    scanned = (await desktop.invite()).qr;
    await t.tap(find.text(_en.devicesScanQr));
    await settle(t, 6);
    expect(find.text(_en.devicesJoined('Desk')), findsOneWidget);
    expect(find.text('Desk'), findsOneWidget);
    expect(desktop.devices().map((d) => d.name), ['Desk', 'Phone']);
  });

  testWidgets('the folder picks itself: one status line, no choice to make', (
    t,
  ) async {
    setSurface(t, const Size(1280, 1400));
    final lan = FakeSyncLan();
    final sync = FakeDeviceSync(id: 'pc', name: 'PC', lan: lan);
    FakeDeviceSync(id: 'tv', name: 'TV', lan: lan);
    FakeDeviceSync(id: 'phone', name: 'Phone', lan: lan);
    final services = await demoServices(sync: sync);
    sync.clock = services.clock;
    final folders = (await services.sourceManager.list())
        .where((s) => s.type.folderSyncable)
        .toList();
    expect(folders, isNotEmpty);
    // The round that runs on its own found the NAS.
    sync.autoSourceId = folders.first.id;
    await sync.folderSyncNow();
    await t.pumpWidget(testApp(services, section: Section.settings));
    await settle(t);
    await _openDevices(t);
    await t.ensureVisible(find.byKey(const ValueKey('devices-folder-status')));
    await settle(t, 4);
    expect(
      find.text(
        _en.devicesFolderVia(
          folders.first.name,
          _en.devicesFolderDeviceCount(3),
          _en.devicesFolderJustNow,
        ),
      ),
      findsOneWidget,
    );
    expect(find.text(_en.devicesFolderPickedAuto), findsOneWidget);
    expect(find.byKey(const ValueKey('devices-folder-change')), findsOneWidget);
    expect(find.byKey(const ValueKey('devices-folder-now')), findsOneWidget);
    expect(find.byType(DropdownButton<String?>), findsNothing);
  });

  testWidgets('no network folder yet: it will be picked by itself', (t) async {
    setSurface(t, const Size(1280, 1400));
    final sync = FakeDeviceSync(id: 'pc', name: 'PC');
    final services = await demoServices(sync: sync);
    await t.pumpWidget(testApp(services, section: Section.settings));
    await settle(t);
    await _openDevices(t);
    await t.ensureVisible(find.byKey(const ValueKey('devices-folder-status')));
    await settle(t, 4);
    expect(find.text(_en.devicesFolderSearching), findsOneWidget);
    expect(find.text(_en.devicesFolderPickedAuto), findsNothing);
    // "Sync now" looks again.
    sync.autoSourceId = (await services.sourceManager.list())
        .firstWhere((s) => s.type.folderSyncable)
        .id;
    await t.tap(find.byKey(const ValueKey('devices-folder-now')));
    await settle(t, 4);
    expect(find.text(_en.devicesFolderPickedAuto), findsOneWidget);
  });

  testWidgets('Change: another folder by hand, read-only, away, off, auto', (
    t,
  ) async {
    setSurface(t, const Size(1280, 1400));
    final lan = FakeSyncLan();
    final sync = FakeDeviceSync(id: 'pc', name: 'PC', lan: lan);
    FakeDeviceSync(id: 'tv', name: 'TV', lan: lan);
    final services = await demoServices(sync: sync);
    final folders = (await services.sourceManager.list())
        .where((s) => s.type.folderSyncable)
        .toList();
    final mine = folders.last;
    sync.readOnlySources.add(mine.id);
    sync.autoSourceId = folders.first.id;
    await sync.folderSyncNow();
    // The last round was five minutes ago.
    sync.clock = () => services.clock().subtract(const Duration(minutes: 5));
    await t.pumpWidget(testApp(services, section: Section.settings));
    await settle(t);
    await _openDevices(t);
    await t.ensureVisible(find.byKey(const ValueKey('devices-folder-change')));
    await settle(t, 4);

    // Pick a folder by hand: synced at once, kept as the user's choice.
    await t.tap(find.byKey(const ValueKey('devices-folder-change')));
    await settle(t, 6);
    expect(find.text(_en.devicesFolderChangeTitle), findsOneWidget);
    expect(find.text(_en.devicesFolderAutoHint), findsOneWidget);
    await t.tap(find.byKey(ValueKey('devices-folder-option-source:${mine.id}')));
    await settle(t, 6);
    expect(sync.folder.mode, FolderSyncMode.manual);
    expect(sync.folder.sourceId, mine.id);
    expect(
      find.text(
        _en.devicesFolderVia(
          mine.name,
          _en.devicesFolderDeviceCount(2),
          _en.devicesFolderMinutesAgo(5),
        ),
      ),
      findsOneWidget,
    );
    expect(find.text(_en.devicesFolderPickedByYou), findsOneWidget);
    expect(find.text(_en.devicesFolderReadOnly), findsOneWidget);

    // An unreachable folder says so.
    sync.folder = FolderSyncState(
      mode: FolderSyncMode.manual,
      sourceId: mine.id,
      error: FolderSyncError.unavailable,
    );
    await t.tap(find.byKey(const ValueKey('devices-folder-now')));
    await settle(t, 4);
    expect(find.text(_en.devicesFolderErrorUnavailable), findsOneWidget);

    // Off: said so, nothing to sync now.
    await t.tap(find.byKey(const ValueKey('devices-folder-change')));
    await settle(t, 6);
    await t.tap(find.byKey(const ValueKey('devices-folder-option-off')));
    await settle(t, 6);
    expect(sync.folder.mode, FolderSyncMode.off);
    expect(find.text(_en.devicesFolderOffStatus), findsOneWidget);
    expect(find.byKey(const ValueKey('devices-folder-now')), findsNothing);
    expect(find.byKey(const ValueKey('devices-folder-error')), findsNothing);

    // Automatically again.
    await t.tap(find.byKey(const ValueKey('devices-folder-change')));
    await settle(t, 6);
    await t.tap(find.byKey(const ValueKey('devices-folder-option-auto')));
    await settle(t, 6);
    expect(sync.folder.mode, FolderSyncMode.auto);
    expect(sync.folder.sourceId, folders.first.id);
    expect(find.text(_en.devicesFolderPickedAuto), findsOneWidget);
  });

  test('remote changes refresh the library and start sync', () async {
    final base = await demoServices();
    final sync = FakeDeviceSync.demo();
    var refreshed = 0;
    final services = AppServices(
      library: base.library,
      secureStore: base.secureStore,
      sync: sync,
      onRemoteLibraryChange: () => refreshed++,
    );
    await services.init();
    await Future<void>.delayed(Duration.zero);
    expect(sync.starts, 1);
    expect(sync.running, isTrue);
    sync.remoteChange();
    await Future<void>.delayed(Duration.zero);
    expect(refreshed, 1);
    final played = <SyncEvent>[];
    final sub = services.playbackElsewhere.listen(played.add);
    sync.remoteChange();
    await Future<void>.delayed(Duration.zero);
    expect(played, isEmpty);
    await sub.cancel();
    expect(refreshed, 2);
  });
}
