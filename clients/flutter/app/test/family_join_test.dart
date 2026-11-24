// SPDX-License-Identifier: MPL-2.0
//
// SYNC-015: a new device joins the family in the LAN without a code. The
// new TV sees «Pixel» nearby, asks with one tap and shows a short code;
// the phone shows «“Living room TV” wants to join» with the same code,
// «Отклонить» holds the remote's focus, and one «Разрешить» lets the TV
// in. Declining or a withdrawn request shares nothing. The core part
// (keys pinned, profiles and progress over sync) is tested in
// core/crates/pa-sync/tests/service.rs and
// core/crates/pa-catalog/tests/family_join.rs.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/image_cache_control.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/sources/discovery.dart';
import 'package:kinolune/sync/device_sync.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));

Future<AppServices> _services(DeviceSync sync, {bool firstRun = false}) async {
  final lib = FakeLibrary(scanStepDelay: Duration.zero);
  final prefs = MemoryPreferences();
  if (!firstRun) prefs.values[Preferences.onboardingDone] = 'true';
  final s = AppServices(
    library: lib,
    secureStore: MemorySecureStore(),
    preferences: prefs,
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
    ),
  );
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

final _dialog = find.byKey(const ValueKey('family-join-dialog'));

void main() {
  setUpAll(loadAppFonts);

  testWidgets('a trusted device lets a new TV in with one tap', (t) async {
    final (:tv, :phone) = _lan();
    final services = await _services(phone);
    setSurface(t, const Size(1280, 900));
    await t.pumpWidget(testApp(services));
    await settle(t);
    expect(_dialog, findsNothing);

    final ask = await tv.joinAsk(phone.offeredFamilyId);
    await settle(t);
    expect(_dialog, findsOneWidget);
    expect(
      find.text(en.familyJoinRequestTitle('Living room TV')),
      findsOneWidget,
    );
    expect(find.text(ask.shortCode), findsOneWidget);
    expect(find.text(en.familyJoinRequestDuplicate), findsNothing);
    await capture(t, 'sync-015-request');

    await t.tap(find.byKey(const ValueKey('family-join-allow')));
    await settle(t);
    expect(_dialog, findsNothing);
    expect(tv.paired && phone.paired, isTrue);
    expect(phone.allowed, ['tv']);
  });

  testWidgets('declined or withdrawn: nothing is shared', (t) async {
    final (:tv, :phone) = _lan();
    final services = await _services(phone);
    setSurface(t, const Size(1280, 900));
    await t.pumpWidget(testApp(services));
    await settle(t);

    await tv.joinAsk(phone.offeredFamilyId);
    await settle(t);
    await t.tap(find.byKey(const ValueKey('family-join-decline')));
    await settle(t);
    expect(_dialog, findsNothing);
    expect(tv.paired || phone.paired, isFalse);
    expect(phone.declined, ['tv']);
    // Declined: the same device asking again is not shown.
    await tv.joinAsk(phone.offeredFamilyId);
    await settle(t);
    expect(_dialog, findsNothing);

    // Another device asks and withdraws: the dialog goes away by itself.
    final pc = FakeDeviceSync(id: 'pc', name: 'PC', lan: phone.lan);
    await pc.joinAsk(phone.offeredFamilyId);
    await settle(t);
    expect(_dialog, findsOneWidget);
    await pc.joinAskCancel();
    await settle(t);
    expect(_dialog, findsNothing);
    expect(pc.paired || phone.paired, isFalse);
  });

  testWidgets('on a TV the remote sits on «Отклонить»', (t) async {
    final (:tv, :phone) = _lan();
    final services = await _services(tv);
    setSurface(t, const Size(960, 540));
    await t.pumpWidget(testApp(services, formFactor: FormFactor.tv));
    await settle(t);
    await phone.joinAsk(tv.offeredFamilyId);
    await settle(t);
    expect(_dialog, findsOneWidget);
    expect(_focused(find.byKey(const ValueKey('family-join-decline'))), isTrue);
    await capture(t, 'sync-015-request-tv');
  });

  testWidgets('a new device sees the family, asks and is let in', (t) async {
    final (:tv, :phone) = _lan();
    final services = await _services(tv, firstRun: true);
    setSurface(t, const Size(1280, 1400));
    await t.pumpWidget(testApp(services));
    await settle(t);

    final join = find.byKey(ValueKey('family-join-${phone.offeredFamilyId}'));
    expect(join, findsOneWidget);
    expect(find.text(en.familyJoinOfferTitle), findsOneWidget);
    expect(find.text(en.familyJoinOfferBody('«Pixel»')), findsOneWidget);
    await t.tap(join);
    await settle(t);
    final ask = tv.asking!;
    expect(find.text(en.familyJoinCode(ask.shortCode)), findsOneWidget);
    expect(phone.requests.single.shortCode, ask.shortCode);
    await capture(t, 'sync-015-offer-waiting');

    await phone.joinAllow(phone.requests.single.id);
    await settle(t, 20);
    expect(tv.paired, isTrue);
    expect(find.byKey(const ValueKey('family-join-done')), findsOneWidget);
    expect(find.text(en.familyJoinDoneTitle('Pixel')), findsOneWidget);
  });
}
