// SPDX-License-Identifier: MPL-2.0
//
// NET-024: the availability monitor's statuses reach the UI — the source
// card follows the monitor between checks and scans. The full indicator
// is UI-053.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/screens/sources_screen.dart';
import 'package:kinolune/sources/availability.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final _en = lookupAppLocalizations(const Locale('en'));

TargetAvailability _source(String id, Availability status) =>
    TargetAvailability(
      id: 'source:$id',
      kind: 'smb',
      name: 'NAS',
      status: status,
    );

void main() {
  test('statuses parse from the core names', () {
    expect(Availability.parse('available'), Availability.available);
    expect(Availability.parse('slow'), Availability.slow);
    expect(Availability.parse('no-access'), Availability.noAccess);
    expect(Availability.parse('unavailable'), Availability.unavailable);
    expect(Availability.parse('no-network'), Availability.noNetwork);
    expect(Availability.parse('???'), Availability.unknown);
    expect(Availability.slow.isUp, isTrue);
    expect(Availability.noAccess.isUp, isFalse);
  });

  test('a source target is found by the source id', () {
    final a = FakeAvailability([
      _source('7', Availability.available),
      const TargetAvailability(
        id: 'peer:tv',
        kind: 'sync-peer',
        name: 'TV',
        status: Availability.unavailable,
      ),
    ]);
    expect(a.ofSource('7')?.status, Availability.available);
    expect(a.ofSource('8'), isNull);
    expect(a.targets.last.sourceId, isNull);
    a.retry('source:7');
    expect(a.retried, ['source:7']);
  });

  testWidgets('the source card follows the monitor', (t) async {
    setSurface(t, const Size(1280, 900));
    final monitor = FakeAvailability();
    final s = await demoServices(availability: monitor);
    final id = s.managedSources.first.id;
    monitor.targets = [_source(id, Availability.unavailable)];
    await t.pumpWidget(testApp(s));
    await settle(t, 4);
    final nav = t.state<NavigatorState>(find.byType(Navigator).first);
    // ignore: unawaited_futures
    nav.push(
      MaterialPageRoute<void>(
        builder: (_) => const SourcesScreen(checkOnOpen: false),
      ),
    );
    await settle(t, 6);
    expect(find.text(_en.sourcesStatusOffline), findsWidgets);

    monitor.targets = [
      for (final m in s.managedSources) _source(m.id, Availability.available),
    ];
    await settle(t, 2);
    expect(find.text(_en.sourcesStatusOffline), findsNothing);
    expect(find.text(_en.sourcesStatusOnline), findsWidgets);
  });
}
