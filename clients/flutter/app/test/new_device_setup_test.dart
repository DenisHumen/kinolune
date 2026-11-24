// SPDX-License-Identifier: MPL-2.0
//
// UI-046: a new device is set up from a phone by QR. The first run of a
// new TV shows the QR code «Перенести настройки с телефона» as the main
// way in, without taking the remote's focus; a phone of the family scans
// it (or types the 6 digits) on Settings → Devices → «Настроить новое
// устройство», confirms, and the TV is in its family with its sources and
// settings: «Смотреть» takes the focus. Only known settings are taken
// over. The core part (sealed bundle, passwords in the secret store only,
// profiles with their PIN hashes) is tested in
// core/crates/pa-catalog/tests/new_device_setup.rs.

import 'package:flutter/services.dart' show LogicalKeyboardKey;
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/devices_screen.dart';
import 'package:kinolune/screens/new_device_screen.dart';
import 'package:kinolune/screens/onboarding_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/image_cache_control.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/discovery.dart';
import 'package:kinolune/sync/device_sync.dart';
import 'package:kinolune/sync/new_device_setup.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));

/// A fresh install (no sources, setup not done) syncing through [sync].
Future<AppServices> _fresh(DeviceSync sync, {Preferences? prefs}) async {
  final lib = FakeLibrary(scanStepDelay: Duration.zero);
  final s = AppServices(
    library: lib,
    secureStore: MemorySecureStore(),
    preferences: prefs ?? MemoryPreferences(),
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    sourceManager: FakeSourceManager(lib),
    discovery: const NoSourceDiscovery(),
    sync: sync,
    clock: () => demoNow,
    imageCache: demoImageCache(),
  );
  await s.init();
  return s;
}

bool _focused(Finder f) {
  final ctx = FocusManager.instance.primaryFocus?.context;
  if (ctx == null || f.evaluate().isEmpty) return false;
  final el = f.evaluate().single;
  if (ctx == el) return true;
  var hit = false;
  ctx.visitAncestorElements((e) {
    hit = e == el;
    return !hit;
  });
  return hit;
}

({FakeDeviceSync tv, FakeDeviceSync phone}) _lan() {
  final lan = FakeSyncLan();
  return (
    tv: FakeDeviceSync(
      id: 'tv',
      name: 'Living room TV',
      kind: SyncDeviceKind.tv,
      lan: lan,
    ),
    phone: FakeDeviceSync(
      id: 'phone',
      name: 'Pixel',
      kind: SyncDeviceKind.phone,
      lan: lan,
    )..setupSources = 2,
  );
}

void main() {
  setUpAll(loadAppFonts);

  test('setup codes: QR payload and typed digits', () {
    final code = SetupCode.parse(
      'kinolune://setup?v=1&c=042917&n=Living%20room%20TV',
    )!;
    expect(code.code, '042917');
    expect(code.deviceName, 'Living room TV');
    expect(SetupCode.parse(' 042-917 ')!.code, '042917');
    expect(SetupCode.parse('042 917')!.deviceName, isNull);
    for (final bad in [
      '',
      '12345',
      'abcdef',
      'kinolune://setup?v=2&c=123456',
      'kinolune://setup?v=1',
      'projectalpha://pair?v=1&c=123456&k=00',
    ]) {
      expect(SetupCode.parse(bad), isNull, reason: bad);
    }
    expect(SetupCode.isSetupQr('kinolune://setup?v=1&c=123456'), isTrue);
    expect(SetupCode.isSetupQr('projectalpha://pair?v=1&c=123456'), isFalse);
    expect(code.toString(), isNot(contains('042917')));
  });

  test('only known settings travel and are applied', () async {
    final phone = MemoryPreferences()
      ..values[Preferences.skipSeconds] = '30'
      ..values[Preferences.locale] = 'ru'
      ..values[Preferences.searchHistory] = '["private"]'
      ..values[Preferences.imageCacheMaxBytes] = '1'
      ..values[Preferences.animeFrameSearch] = 'true';
    final sent = await collectSetupSettings(phone);
    expect(sent, {Preferences.skipSeconds: '30', Preferences.locale: 'ru'});
    final tv = MemoryPreferences();
    final written = await applySetupSettings(tv, {
      ...sent,
      Preferences.searchHistory: '["x"]',
      'anything': 'else',
    });
    expect(written.toSet(), {Preferences.skipSeconds, Preferences.locale});
    expect(tv.values[Preferences.skipSeconds], '30');
    expect(tv.values.containsKey(Preferences.searchHistory), isFalse);
    expect(tv.values.containsKey('anything'), isFalse);
  });

  testWidgets('new TV: the QR code is the main way in, the phone transfers, '
      '«Смотреть» takes the remote', (t) async {
    final (:tv, :phone) = _lan();
    final prefs = MemoryPreferences();
    final services = await _fresh(tv, prefs: prefs);
    expect(services.onboardingPending, isTrue);
    setSurface(t, const Size(960, 540));
    await t.pumpWidget(testApp(services, formFactor: FormFactor.tv));
    await settle(t);
    expect(find.byType(OnboardingScreen), findsOneWidget);

    // The QR code and the code for a phone without a camera; the remote
    // stays on «Начать» (setting up by hand is one OK away).
    expect(find.byKey(const ValueKey('new-device-qr')), findsOneWidget);
    expect(find.text(en.newDeviceQrTitle), findsOneWidget);
    final offer = tv.setupOpen!;
    expect(
      find.text(
        en.newDeviceQrCode(
          '${offer.code.substring(0, 3)} ${offer.code.substring(3)}',
        ),
      ),
      findsOneWidget,
    );
    expect(offer.qr, startsWith(SetupCode.prefix));
    expect(SetupCode.parse(offer.qr)!.deviceName, 'Living room TV');
    expect(_focused(find.byKey(const ValueKey('onboarding-next'))), isTrue);

    // The phone scans it.
    await phone.setupSend(offer.qr, {
      Preferences.skipSeconds: '30',
      'not_a_setting': 'x',
    });
    await settle(t, 10);
    expect(tv.paired, isTrue);
    expect(phone.paired, isTrue);
    expect(find.byKey(const ValueKey('new-device-done')), findsOneWidget);
    expect(find.text(en.newDeviceDoneTitle('Pixel')), findsOneWidget);
    expect(find.text(en.newDeviceDoneSources(2)), findsOneWidget);
    expect(prefs.values[Preferences.skipSeconds], '30');
    expect(prefs.values.containsKey('not_a_setting'), isFalse);
    final open = find.byKey(const ValueKey('new-device-open'));
    expect(_focused(open), isTrue, reason: 'the remote is not on «Смотреть»');

    // One OK and the first run is over.
    await pressKey(t, LogicalKeyboardKey.select);
    await settle(t, 6);
    expect(find.byType(AppShell), findsOneWidget);
    expect(prefs.values[Preferences.onboardingDone], 'true');
  });

  testWidgets('no QR code where the device is already in a family', (t) async {
    final services = await _fresh(FakeDeviceSync.demo());
    setSurface(t, const Size(960, 540));
    await t.pumpWidget(testApp(services, formFactor: FormFactor.tv));
    await settle(t);
    expect(find.byType(OnboardingScreen), findsOneWidget);
    expect(find.byKey(const ValueKey('new-device-qr')), findsNothing);
  });

  testWidgets('leaving the first run withdraws the offer', (t) async {
    final (:tv, phone: _) = _lan();
    final services = await _fresh(tv);
    setSurface(t, const Size(1280, 900));
    await t.pumpWidget(testApp(services));
    await settle(t);
    expect(tv.setupOpen, isNotNull);
    await t.tap(find.byKey(const ValueKey('onboarding-skip')));
    await settle(t);
    expect(find.byType(AppShell), findsOneWidget);
    expect(tv.setupOpen, isNull);
  });

  testWidgets('phone without a camera: types the code, transfers, done', (
    t,
  ) async {
    final (:tv, :phone) = _lan();
    final offer = await tv.setupOffer();
    final received = tv.setupWait(const Duration(seconds: 4));
    final prefs = MemoryPreferences()
      ..values[Preferences.skipSeconds] = '15'
      ..values[Preferences.searchHistory] = '["private"]';
    final services = await demoServices(sync: phone, preferences: prefs);
    setSurface(t, const Size(390, 844), dpr: 2);
    await t.pumpWidget(
      testApp(
        services,
        formFactor: FormFactor.phone,
        section: Section.settings,
      ),
    );
    await settle(t);
    final nav = t.state<NavigatorState>(find.byType(Navigator).last);
    nav.push(MaterialPageRoute<void>(builder: (_) => const DevicesScreen()));
    await settle(t, 10);
    await t.ensureVisible(find.byKey(const ValueKey('devices-setup-new')));
    await settle(t, 4);
    await t.tap(find.byKey(const ValueKey('devices-setup-new')));
    await settle(t, 8);

    // No camera here: the code field, no scan button.
    expect(find.byKey(const ValueKey('send-setup-scan')), findsNothing);
    await t.enterText(find.byKey(const ValueKey('send-setup-field')), '12');
    await t.tap(find.byKey(const ValueKey('send-setup-next')));
    await settle(t, 4);
    expect(find.text(en.devicesErrorInvalidCode), findsOneWidget);
    final spaced = '${offer.code.substring(0, 3)} ${offer.code.substring(3)}';
    await t.enterText(find.byKey(const ValueKey('send-setup-field')), spaced);
    await t.tap(find.byKey(const ValueKey('send-setup-next')));
    await settle(t, 4);
    expect(find.text(en.newDeviceSendTargetUnnamed), findsOneWidget);
    expect(find.text(en.newDeviceSendWhat), findsOneWidget);

    await t.tap(find.byKey(const ValueKey('send-setup-send')));
    await settle(t, 8);
    expect(find.text(en.newDeviceSendDone('Living room TV')), findsOneWidget);
    final got = (await received)!;
    expect(got.deviceName, 'Pixel');
    expect(got.settings[Preferences.skipSeconds], '15');
    expect(got.settings.containsKey(Preferences.searchHistory), isFalse);
    expect(phone.devices().map((d) => d.name), ['Pixel', 'Living room TV']);

    await t.tap(find.byKey(const ValueKey('send-setup-close')));
    await settle(t, 6);
    expect(find.byType(SendSetupScreen), findsNothing);
    expect(find.byType(DevicesScreen), findsOneWidget);
  });

  testWidgets('a scanned setup QR on Devices opens the transfer, not a join', (
    t,
  ) async {
    final (:tv, :phone) = _lan();
    final offer = await tv.setupOffer();
    final received = tv.setupWait(const Duration(seconds: 4));
    final services = await demoServices(sync: phone);
    setSurface(t, const Size(1280, 900));
    await t.pumpWidget(testApp(services, section: Section.settings));
    await settle(t);
    final nav = t.state<NavigatorState>(find.byType(Navigator).last);
    nav.push(
      MaterialPageRoute<void>(
        builder: (_) => DevicesScreen(scanQr: (_) async => offer.qr),
      ),
    );
    await settle(t, 10);
    await t.ensureVisible(find.text(en.devicesScanQr));
    await settle(t, 4);
    await t.tap(find.text(en.devicesScanQr));
    await settle(t, 8);
    expect(find.byType(SendSetupScreen), findsOneWidget);
    expect(find.text(en.newDeviceSendTarget('Living room TV')), findsOneWidget);
    // Nothing joined yet: the user confirms first.
    expect(phone.paired, isFalse);
    await t.tap(find.byKey(const ValueKey('send-setup-send')));
    await settle(t, 8);
    expect(find.byKey(const ValueKey('send-setup-done')), findsOneWidget);
    expect((await received)!.sourcesAdded, 2);
    expect(tv.paired && phone.paired, isTrue);
  });
}
