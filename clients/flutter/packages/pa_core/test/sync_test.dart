// SPDX-License-Identifier: MPL-2.0
// UI-019 through the Dart bindings: two libraries in one process pair by QR
// code over loopback, and a position saved on one shows up on the other.
import 'dart:async';
import 'dart:io';

import 'package:pa_core/pa_core.dart';
import 'package:test/test.dart';

import 'native_library.dart';

/// A fake media file with content unique to [seed] (same seed, same
/// fingerprint on both devices).
void _media(String root, String rel, int seed) {
  final f = File('$root/$rel')..createSync(recursive: true);
  f.writeAsBytesSync([for (var i = 0; i < 150000; i++) (i * 31 + seed) & 0xff]);
}

class _Device {
  _Device(this.dir, this.library, this.sync);
  final Directory dir;
  final CoreLibrary library;
  final CoreSync sync;

  static Future<_Device> open(String name, CoreDeviceClass cls) async {
    final dir = Directory.systemTemp.createTempSync('pa_sync_test_');
    final media = '${dir.path}/media';
    _media(media, 'Movies/The Matrix 1999.mkv', 2);
    final library = await CoreLibrary.open(path: '${dir.path}/library.db');
    await library.addSource(kind: 'local', name: 'Media', location: media);
    await library.scan().last;
    final sync = await CoreSync.open(
      library_: library,
      config: CoreSyncConfig(
        dataDir: '${dir.path}/sync',
        deviceName: name,
        deviceClass: cls,
        secretsDir: '${dir.path}/secrets',
        loopback: true,
      ),
    );
    return _Device(dir, library, sync);
  }

  Future<String> matrixId() async =>
      (await library.movies()).firstWhere((m) => m.title.contains('Matrix')).id;
}

Future<void> _until(String what, FutureOr<bool> Function() ok) async {
  final deadline = DateTime.now().add(const Duration(seconds: 15));
  while (!await ok()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out: $what');
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
}

void main() {
  setUpAll(() async {
    await RustLib.init(externalLibrary: openBuiltCoreLibrary());
  });

  tearDownAll(RustLib.dispose);

  test('pair by QR, then progress on A appears on B', () async {
    final pc = await _Device.open('Desktop', CoreDeviceClass.desktop);
    final phone = await _Device.open('Phone', CoreDeviceClass.mobile);
    addTearDown(() {
      pc.dir.deleteSync(recursive: true);
      phone.dir.deleteSync(recursive: true);
    });
    final events = <CoreSyncEventKind>[];
    final sub = phone.sync.events().listen((e) => events.add(e.kind));
    // Cancelling a flutter_rust_bridge stream can wait until the Rust side
    // sends its next event, which may never come (about half the runs hung
    // here for the 30 s test timeout). Do not await it.
    addTearDown(() => unawaited(sub.cancel()));

    expect(pc.sync.devices(), isEmpty);
    final invite = await pc.sync.invite();
    expect(invite.code, matches(RegExp(r'^\d{6}$')));
    await expectLater(
      phone.sync.join(input: '12'),
      throwsA(
        isA<CoreSyncError>().having((e) => e.code, 'code', 'invalid_code'),
      ),
    );
    final inviter = await phone.sync.join(input: invite.qr);
    expect(inviter.name, 'Desktop');
    expect(inviter.fingerprint, pc.sync.fingerprint());
    await _until('desktop runs', () => pc.sync.isRunning());
    expect(pc.sync.devices().map((d) => d.name), ['Desktop', 'Phone']);
    expect(phone.sync.devices().first.thisDevice, isTrue);

    pc.sync.peerUp(
      deviceId: phone.sync.deviceId(),
      deviceClass: CoreDeviceClass.mobile,
      addr: phone.sync.syncAddr()!,
    );
    phone.sync.peerUp(
      deviceId: pc.sync.deviceId(),
      deviceClass: CoreDeviceClass.desktop,
      addr: pc.sync.syncAddr()!,
    );
    await _until(
      'both online',
      () => phone.sync.devices().every((d) => d.online),
    );

    final now = DateTime.now().millisecondsSinceEpoch;
    expect(
      await pc.library.saveProgress(
        itemId: await pc.matrixId(),
        positionMs: 2471200,
        durationMs: 8160000,
        updatedAtMs: now,
        watched: false,
      ),
      isTrue,
    );
    final onPhone = await phone.matrixId();
    await _until('progress on the phone', () async {
      final p = await phone.library.progress(itemId: onPhone);
      return p?.positionMs == 2471200;
    });
    expect(events, contains(CoreSyncEventKind.libraryChanged));
    final cont = await phone.library.continueWatching(limit: 10);
    expect(cont.single.item.id, onPhone);

    await pc.sync.unpair(deviceId: phone.sync.deviceId());
    expect(pc.sync.devices().map((d) => d.name), ['Desktop']);
    expect(pc.sync.isPaired(), isFalse);
  });

  test('an SMB source with its password reaches the paired TV', () async {
    // SYNC-010 / UI-019: added on the phone, the source and its password
    // (sealed end-to-end) arrive on the TV; no call ever returns it.
    final phone = await _Device.open('Phone', CoreDeviceClass.mobile);
    final tv = await _Device.open('TV', CoreDeviceClass.tv);
    addTearDown(() {
      phone.dir.deleteSync(recursive: true);
      tv.dir.deleteSync(recursive: true);
    });
    final sources = <String, CoreSources>{};
    for (final (name, d) in [('phone', phone), ('tv', tv)]) {
      await d.library.connectSources(dataDir: d.dir.path);
      sources[name] = await CoreSources.open(
        dbPath: '${d.dir.path}/library.db',
        dataDir: d.dir.path,
      );
    }
    final invite = await tv.sync.invite();
    await phone.sync.join(input: invite.qr);
    await _until('tv runs', () => tv.sync.isRunning());
    tv.sync.peerUp(
      deviceId: phone.sync.deviceId(),
      deviceClass: CoreDeviceClass.mobile,
      addr: phone.sync.syncAddr()!,
    );
    phone.sync.peerUp(
      deviceId: tv.sync.deviceId(),
      deviceClass: CoreDeviceClass.tv,
      addr: tv.sync.syncAddr()!,
    );
    await _until(
      'both online',
      () => phone.sync.devices().every((d) => d.online),
    );

    final created = await sources['phone']!.create(
      input: const CoreSourceInput(
        kind: 'smb',
        name: 'NAS',
        address: 'smb://nas.local/media',
        username: 'anna',
        password: 'dart-side-secret',
        options: CoreSourceOptions(
          smbVersion: 'any',
          smbSigningRequired: false,
          smbEncryption: 'auto',
          jellyfinAltAddresses: [],
        ),
        scanIntoLibrary: true,
        autoScan: true,
        enabled: true,
        sync_: 'password',
      ),
    );
    expect(created.sync_, 'password');
    await _until('source on the tv', () async {
      final list = await sources['tv']!.list();
      return list.any((s) => s.name == 'NAS' && s.hasPassword);
    });
    final onTv = (await sources['tv']!.list()).singleWhere(
      (s) => s.name == 'NAS',
    );
    expect(onTv.uri, 'smb://nas.local/media');
    expect(onTv.username, 'anna');
    expect(onTv.sync_, 'password');

    await sources['phone']!.delete(id: created.id);
    await _until(
      'deleted on the tv',
      () async => !(await sources['tv']!.list()).any((s) => s.name == 'NAS'),
    );
  });

  test('typing on the TV from the phone: text, password, cancel', () async {
    // UI-027: a field focused on the TV opens input on the phone; what is
    // typed there (a password too, sealed on the wire) reaches the TV, and
    // a cancel on the TV closes the phone's input.
    final tv = await _Device.open('TV', CoreDeviceClass.tv);
    final phone = await _Device.open('Phone', CoreDeviceClass.mobile);
    addTearDown(() {
      tv.dir.deleteSync(recursive: true);
      phone.dir.deleteSync(recursive: true);
    });
    final onTv = <CoreRemoteInput>[];
    final onPhone = <CoreRemoteInput>[];
    final tvSub = tv.sync.events().listen((e) {
      if (e.input case final i?) onTv.add(i);
    });
    final phoneSub = phone.sync.events().listen((e) {
      if (e.input case final i?) onPhone.add(i);
    });
    // Do not await: see the first test.
    addTearDown(() {
      unawaited(tvSub.cancel());
      unawaited(phoneSub.cancel());
    });

    final invite = await tv.sync.invite();
    await phone.sync.join(input: invite.qr);
    await _until('tv runs', () => tv.sync.isRunning());
    tv.sync.peerUp(
      deviceId: phone.sync.deviceId(),
      deviceClass: CoreDeviceClass.mobile,
      addr: phone.sync.syncAddr()!,
    );
    phone.sync.peerUp(
      deviceId: tv.sync.deviceId(),
      deviceClass: CoreDeviceClass.tv,
      addr: tv.sync.syncAddr()!,
    );
    await _until(
      'both online',
      () => phone.sync.devices().every((d) => d.online),
    );

    // The TV's search field takes the focus.
    final search = await tv.sync.inputRequest(
      field: CoreInputField.search,
      label: 'Search',
      secret: false,
      text: '',
    );
    expect(search, isNotNull);
    await _until('request on the phone', () => onPhone.isNotEmpty);
    final req = onPhone.single;
    expect(req.session, search);
    expect(req.action, CoreInputAction.request);
    expect(req.field, CoreInputField.search);
    expect(req.deviceName, 'TV');
    final tvId = tv.sync.deviceId();
    await phone.sync.inputSend(
      session: search!,
      to: tvId,
      action: CoreInputAction.open,
      secret: false,
    );
    for (final text in ['ma', 'matrix']) {
      await phone.sync.inputSend(
        session: search,
        to: tvId,
        action: CoreInputAction.text,
        text: text,
        secret: false,
      );
    }
    await _until('text on the tv', () => onTv.any((i) => i.text == 'matrix'));
    expect(
      onTv.firstWhere((i) => i.action == CoreInputAction.open).deviceName,
      'Phone',
    );

    // The password field: sealed on the wire, opened on the TV.
    final password = await tv.sync.inputRequest(
      field: CoreInputField.password,
      label: 'Password',
      secret: true,
      text: '',
    );
    await _until(
      'password request on the phone',
      () => onPhone.any((i) => i.session == password),
    );
    expect(onPhone.last.secret, isTrue);
    await phone.sync.inputSend(
      session: password!,
      to: tvId,
      action: CoreInputAction.submit,
      text: 'dart-side-secret',
      secret: true,
    );
    await _until(
      'password on the tv',
      () => onTv.any(
        (i) => i.session == password && i.text == 'dart-side-secret',
      ),
    );

    // The TV cancels (the field lost focus): the phone closes its input.
    final pin = await tv.sync.inputRequest(
      field: CoreInputField.pin,
      secret: true,
      text: '',
    );
    await _until(
      'pin request on the phone',
      () => onPhone.any((i) => i.session == pin),
    );
    await tv.sync.inputSend(
      session: pin!,
      action: CoreInputAction.cancel,
      secret: false,
    );
    await _until(
      'cancel on the phone',
      () => onPhone.any(
        (i) => i.session == pin && i.action == CoreInputAction.cancel,
      ),
    );
  });
}
