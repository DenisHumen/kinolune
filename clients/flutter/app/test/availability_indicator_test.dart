// SPDX-License-Identifier: MPL-2.0
//
// UI-053: the availability indicator over the monitor of NET-024 — the
// status line with its reason and one action on a phone, a computer and a
// TV in ru/en/uk; the plate «Синхронизация через … на паузе» with the
// changes waiting on this device; the banner after two minutes, at most
// once an hour; «Состояние подключений».

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/notices/availability_notices.dart';
import 'package:kinolune/notices/in_app_notices.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/connection_status_screen.dart';
import 'package:kinolune/screens/devices_screen.dart';
import 'package:kinolune/screens/sources_screen.dart';
import 'package:kinolune/shell/app_shell.dart' show Section;
import 'package:kinolune/sources/availability.dart';
import 'package:kinolune/sync/device_sync.dart';
import 'package:kinolune/widgets/availability_status.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final _en = lookupAppLocalizations(const Locale('en'));

TargetAvailability _t(
  String id,
  Availability status, {
  String name = 'NAS',
  String kind = 'smb',
  DateTime? since,
  String? reason,
  bool syncChannel = false,
  DateTime? lastOk,
  int? rttMs,
}) => TargetAvailability(
  id: id,
  kind: kind,
  name: name,
  status: status,
  since: since,
  reason: reason,
  syncChannel: syncChannel,
  lastOk: lastOk,
  rttMs: rttMs,
);

/// Every status once: two sources (a refused password, an address that is
/// gone), the own server down, a slow hub, a device of the family online,
/// an endpoint not checked yet and one with no network.
List<TargetAvailability> _all(List<String> sources) => [
  _t(
    'source:${sources[0]}',
    Availability.noAccess,
    name: 'Attic NAS',
    since: demoNow.subtract(const Duration(minutes: 55)),
    reason: 'authentication failed: bad password',
  ),
  _t(
    'source:${sources[1]}',
    Availability.unavailable,
    name: 'Study PC',
    since: demoNow.subtract(const Duration(minutes: 20)),
    reason: 'host not found: no such host',
  ),
  _t(
    'endpoint:server',
    Availability.unavailable,
    name: 'Home server',
    kind: 'kinolune',
    since: demoNow.subtract(const Duration(minutes: 5)),
    reason: '192.0.2.1:8487: connection refused',
  ),
  _t('endpoint:hub', Availability.slow, name: 'Hub', kind: 'sync-hub'),
  _t(
    'peer:tv',
    Availability.available,
    name: 'Living room TV',
    kind: 'sync-peer',
    rttMs: 12,
  ),
  _t('endpoint:new', Availability.unknown, name: 'Box', kind: 'kinolune'),
  _t('endpoint:far', Availability.noNetwork, name: 'Far', kind: 'kinolune'),
];

Future<void> _push(WidgetTester t, Widget page) async {
  final nav = t.state<NavigatorState>(find.byType(Navigator).first);
  // ignore: unawaited_futures
  nav.push(MaterialPageRoute<void>(builder: (_) => page));
  await settle(t, 8);
}

/// Whether the focused node is [finder] or inside it.
bool _focusedIn(Finder finder) {
  final ctx = FocusManager.instance.primaryFocus?.context;
  if (ctx == null) return false;
  final target = finder.evaluate().single;
  if (ctx == target) return true;
  var found = false;
  ctx.visitAncestorElements((a) {
    if (a == target) {
      found = true;
      return false;
    }
    return true;
  });
  return found;
}

void main() {
  group('reasons and actions', () {
    test('a refused password is fixed in the form', () {
      final t = _t(
        'source:1',
        Availability.noAccess,
        reason: 'authentication failed: x',
      );
      expect(availabilityReason(t), AvailabilityReason.password);
      expect(availabilityFix(t), AvailabilityFix.password);
    });

    test('rights and the secret store', () {
      expect(
        availabilityReason(
          _t('source:1', Availability.noAccess, reason: 'permission denied'),
        ),
        AvailabilityReason.rights,
      );
      expect(
        availabilityReason(
          _t('source:1', Availability.noAccess, reason: 'secret store: locked'),
        ),
        AvailabilityReason.secretStore,
      );
    });

    test('an address that is gone is changed, anything else retried', () {
      final gone = _t(
        'source:1',
        Availability.unavailable,
        reason: 'host not found: nas.local',
      );
      expect(availabilityFix(gone), AvailabilityFix.address);
      final off = _t(
        'source:1',
        Availability.unavailable,
        reason: '10.0.0.2:445: connection timed out',
      );
      expect(availabilityReason(off), AvailabilityReason.timeout);
      expect(availabilityFix(off), AvailabilityFix.retry);
      // A server that is no source only retries.
      final server = _t(
        'endpoint:s',
        Availability.noAccess,
        reason: 'HTTP 401',
      );
      expect(availabilityFix(server), AvailabilityFix.retry);
      expect(
        availabilityReason(
          _t(
            'peer:x',
            Availability.unavailable,
            reason: 'not seen on the network',
          ),
        ),
        AvailabilityReason.notSeen,
      );
      expect(availabilityReason(_t('x', Availability.available)), isNull);
    });

    test('sync is paused only through a channel that cannot be used', () {
      expect(
        _t('s', Availability.unavailable, syncChannel: true).syncPaused,
        isTrue,
      );
      expect(
        _t('s', Availability.noAccess, syncChannel: true).syncPaused,
        isTrue,
      );
      expect(_t('s', Availability.slow, syncChannel: true).syncPaused, isFalse);
      expect(
        _t('s', Availability.unknown, syncChannel: true).syncPaused,
        isFalse,
      );
      expect(_t('s', Availability.unavailable).syncPaused, isFalse);
    });
  });

  group('every status on a phone, a computer and a TV', () {
    const forms = [
      ('phone', FormFactor.phone, Size(390, 844)),
      ('desktop', FormFactor.desktop, Size(1280, 900)),
      ('tv', FormFactor.tv, Size(1920, 1080)),
    ];
    for (final locale in const [Locale('ru'), Locale('en'), Locale('uk')]) {
      for (final (name, ff, size) in forms) {
        testWidgets('${locale.languageCode} $name', (t) async {
          setSurface(t, size);
          final l = lookupAppLocalizations(locale);
          final monitor = FakeAvailability();
          final s = await demoServices(availability: monitor);
          await s.setLocale(locale);
          final ids = [for (final m in s.managedSources) m.id];
          monitor.targets = _all(ids);
          await t.pumpWidget(testApp(s, formFactor: ff));
          await settle(t, 4);
          await _push(t, const ConnectionStatusScreen());

          expect(find.text(l.availabilityScreenTitle), findsOneWidget);
          // Lines: the time follows the name, so the line without it is
          // its start.
          expect(
            find.textContaining(l.availabilityLineNoAccess('Attic NAS')),
            findsWidgets,
          );
          expect(
            find.textContaining(l.availabilityLineUnavailable('Study PC')),
            findsWidgets,
          );
          expect(
            find.textContaining(l.availabilityLineUnavailable('Home server')),
            findsWidgets,
          );
          expect(find.text(l.availabilityLineSlow('Hub')), findsOneWidget);
          expect(
            find.text(l.availabilityLineAvailable('Living room TV')),
            findsOneWidget,
          );
          expect(find.text(l.availabilityLineUnknown('Box')), findsOneWidget);
          expect(find.text(l.availabilityLineNoNetwork), findsOneWidget);
          // Reasons in plain words, never the core's text.
          expect(find.text(l.availabilityReasonPassword), findsOneWidget);
          expect(find.text(l.availabilityReasonHostNotFound), findsOneWidget);
          expect(find.text(l.availabilityReasonRefused), findsOneWidget);
          expect(find.textContaining('192.0.2.1'), findsNothing);
          // One action each.
          final password = find.byKey(
            ValueKey('availability-fix-source:${ids[0]}'),
          );
          final address = find.byKey(
            ValueKey('availability-fix-source:${ids[1]}'),
          );
          final retry = find.byKey(
            const ValueKey('availability-fix-endpoint:server'),
          );
          expect(
            find.descendant(
              of: password,
              matching: find.text(l.availabilityActionPassword),
            ),
            findsOneWidget,
          );
          expect(
            find.descendant(
              of: address,
              matching: find.text(l.availabilityActionAddress),
            ),
            findsOneWidget,
          );
          expect(
            find.descendant(
              of: retry,
              matching: find.text(l.availabilityActionRetry),
            ),
            findsOneWidget,
          );
          expect(t.takeException(), isNull);

          if (ff == FormFactor.tv) {
            // The remote reaches the action.
            for (var i = 0; i < 40 && !_focusedIn(retry); i++) {
              await t.sendKeyEvent(LogicalKeyboardKey.tab);
              await t.pump(const Duration(milliseconds: 80));
            }
            expect(_focusedIn(retry), isTrue);
            await t.sendKeyEvent(LogicalKeyboardKey.enter);
            await settle(t, 2);
            expect(monitor.retried, contains('endpoint:server'));
          } else {
            await t.ensureVisible(retry);
            await settle(t, 2);
            await t.tap(retry);
            await settle(t, 2);
            expect(monitor.retried, contains('endpoint:server'));
          }
        });
      }
    }
  });

  testWidgets('no access has its own words on the source card', (t) async {
    setSurface(t, const Size(1280, 900));
    final monitor = FakeAvailability();
    final s = await demoServices(availability: monitor);
    final id = s.managedSources.first.id;
    monitor.targets = [
      _t(
        'source:$id',
        Availability.noAccess,
        name: 'Attic NAS',
        since: demoNow.subtract(const Duration(minutes: 3)),
        reason: 'authentication failed: bad password',
      ),
    ];
    await t.pumpWidget(testApp(s));
    await settle(t, 4);
    await _push(t, const SourcesScreen(checkOnOpen: false));
    expect(find.text(_en.sourcesStatusNoAccess), findsWidgets);
    expect(
      find.textContaining(_en.availabilityLineNoAccess('Attic NAS')),
      findsOneWidget,
    );
    expect(find.text(_en.availabilityActionPassword), findsOneWidget);
  });

  testWidgets(
    'a stopped sync folder shows the waiting changes until it is back',
    (t) async {
      setSurface(t, const Size(1280, 900));
      final monitor = FakeAvailability();
      final sync = FakeDeviceSync.demo();
      final s = await demoServices(availability: monitor, sync: sync);
      final id = s.managedSources.first.id;
      final lastSync = demoNow.subtract(const Duration(minutes: 30));
      sync.folder = FolderSyncState(
        mode: FolderSyncMode.manual,
        sourceId: id,
        lastSync: lastSync,
        devices: 1,
        pending: 12,
      );
      sync.outbox = 3;
      final down = _t(
        'source:$id',
        Availability.unavailable,
        name: 'NAS',
        since: demoNow.subtract(const Duration(minutes: 10)),
        reason: 'connection timed out',
        syncChannel: true,
      );
      monitor.targets = [down];
      await t.pumpWidget(testApp(s));
      await settle(t, 4);
      await _push(t, const DevicesScreen());

      expect(
        find.byKey(const ValueKey('availability-sync-plate')),
        findsOneWidget,
      );
      expect(find.text(_en.availabilitySyncPaused('NAS', 12)), findsOneWidget);
      expect(
        find.text(
          _en.availabilitySyncLast('NAS', _en.devicesFolderMinutesAgo(30)),
        ),
        findsOneWidget,
      );

      // The folder is back: the round empties the journal.
      sync.folder = FolderSyncState(
        mode: FolderSyncMode.manual,
        sourceId: id,
        lastSync: demoNow,
        devices: 1,
      );
      monitor.targets = [
        _t(
          'source:$id',
          Availability.available,
          name: 'NAS',
          syncChannel: true,
          lastOk: demoNow,
        ),
      ];
      await settle(t, 4);
      expect(
        find.byKey(const ValueKey('availability-sync-plate')),
        findsNothing,
      );
      expect(
        find.textContaining(_en.availabilitySyncPaused('NAS', 12)),
        findsNothing,
      );

      await _push(t, const ConnectionStatusScreen());
      expect(find.text(_en.availabilityFolderOutbox(0)), findsOneWidget);
      expect(find.text(_en.availabilityOutbox(3)), findsOneWidget);
    },
  );

  testWidgets('the plate takes the remote focus on a TV', (t) async {
    setSurface(t, const Size(1920, 1080));
    final monitor = FakeAvailability();
    final sync = FakeDeviceSync.demo()..outbox = 4;
    final s = await demoServices(availability: monitor, sync: sync);
    monitor.targets = [
      _t(
        'endpoint:hub',
        Availability.unavailable,
        name: 'Hub',
        kind: 'sync-hub',
        since: demoNow.subtract(const Duration(minutes: 10)),
        syncChannel: true,
      ),
    ];
    await t.pumpWidget(testApp(s, formFactor: FormFactor.tv));
    await settle(t, 4);
    await _push(t, const DevicesScreen());
    final plate = find.byKey(const ValueKey('availability-sync-plate'));
    expect(plate, findsOneWidget);
    expect(find.text(_en.availabilitySyncPaused('Hub', 4)), findsOneWidget);
    for (var i = 0; i < 20 && !_focusedIn(plate); i++) {
      await t.sendKeyEvent(LogicalKeyboardKey.tab);
      await t.pump(const Duration(milliseconds: 80));
    }
    expect(_focusedIn(plate), isTrue);
    await t.sendKeyEvent(LogicalKeyboardKey.enter);
    await settle(t, 8);
    expect(find.byType(ConnectionStatusScreen), findsOneWidget);
  });

  group('banner', () {
    final t0 = DateTime(2026, 11, 25, 14, 5);
    TargetAvailability nas(Availability s, DateTime? since) =>
        _t('source:1', s, since: since);

    test('only after two minutes down, once an hour', () {
      final p = AvailabilityBannerPolicy();
      expect(p.due([nas(Availability.available, t0)], t0), isEmpty);
      final down = nas(Availability.unavailable, t0);
      expect(p.due([down], t0.add(const Duration(minutes: 1))), isEmpty);
      expect(p.next([down]), t0.add(availabilityBannerAfter));
      final shown = t0.add(const Duration(minutes: 2, seconds: 1));
      expect(p.due([down], shown), hasLength(1));
      // The same outage is not announced again.
      expect(p.due([down], t0.add(const Duration(minutes: 30))), isEmpty);
      expect(p.next([down]), isNull);
      // Back, then down again within the hour: no second banner.
      p.due([
        nas(Availability.available, t0),
      ], t0.add(const Duration(minutes: 31)));
      final again = nas(
        Availability.unavailable,
        t0.add(const Duration(minutes: 40)),
      );
      expect(p.due([again], t0.add(const Duration(minutes: 45))), isEmpty);
      // An hour after the last banner it may come again.
      p.due([
        nas(Availability.available, t0),
      ], t0.add(const Duration(minutes: 50)));
      final later = nas(
        Availability.unavailable,
        t0.add(const Duration(minutes: 70)),
      );
      expect(p.due([later], t0.add(const Duration(minutes: 73))), hasLength(1));
    });

    test('not for what was never up, devices of the family or no access', () {
      final p = AvailabilityBannerPolicy();
      final late = t0.add(const Duration(minutes: 10));
      expect(p.due([nas(Availability.unavailable, t0)], late), isEmpty);
      final peer = _t('peer:phone', Availability.available, kind: 'sync-peer');
      p.due([peer], t0);
      expect(
        p.due([_t('peer:phone', Availability.unavailable, since: t0)], late),
        isEmpty,
      );
      p.due([nas(Availability.available, t0)], t0);
      expect(p.due([nas(Availability.noAccess, t0)], late), isEmpty);
    });

    test('posts one notice into the queue', () {
      final monitor = FakeAvailability([nas(Availability.available, t0)]);
      final notices = InAppNotices();
      var now = t0;
      final feed = AvailabilityNotices(
        availability: monitor,
        post: notices.post,
        now: () => now,
      )..start();
      addTearDown(feed.dispose);
      monitor.targets = [nas(Availability.unavailable, t0)];
      expect(notices.current, isNull);
      now = t0.add(const Duration(minutes: 3));
      feed.check();
      expect(notices.current?.id, 'availability-source:1');
      expect(notices.current?.kind, InAppNoticeKind.availability);
      expect(notices.current?.title(_en), _en.availabilityBannerTitle('NAS'));
      notices.dismiss();
      now = t0.add(const Duration(minutes: 20));
      feed.check();
      expect(notices.current, isNull);
    });
  });

  testWidgets('advanced settings open «Состояние подключений»', (t) async {
    setSurface(t, const Size(1280, 900));
    final monitor = FakeAvailability([
      _t(
        'endpoint:server',
        Availability.unavailable,
        name: 'Home server',
        kind: 'kinolune',
        since: demoNow.subtract(const Duration(minutes: 5)),
      ),
    ]);
    final s = await demoServices(availability: monitor, advancedSettings: true);
    await t.pumpWidget(testApp(s, section: Section.settings));
    await settle(t, 6);
    final tile = find.byKey(const ValueKey('settings-connection-status'));
    await t.scrollUntilVisible(
      tile,
      300,
      scrollable: find.byType(Scrollable).first,
    );
    await settle(t, 2);
    // The own server, as a row of the section.
    expect(
      find.textContaining(_en.availabilityLineUnavailable('Home server')),
      findsWidgets,
    );
    await t.tap(tile);
    await settle(t, 8);
    expect(find.byType(ConnectionStatusScreen), findsOneWidget);
  });
}
