// SPDX-License-Identifier: MPL-2.0
//
// SYNC-011: the player's scrobbles to Trakt and the periodic sync.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/trakt/trakt_link.dart';

import 'support/fake_trakt.dart';

void main() {
  test('start, pause after silence, start again, stop on close', () async {
    final trakt = FakeTrakt(account: 'tester');
    var now = DateTime(2026, 11, 24, 20);
    final s = TraktScrobbler(trakt, clock: () => now, timer: false);
    s.report('m1', 60000, 600000);
    for (var i = 1; i <= 3; i++) {
      now = now.add(const Duration(seconds: 1));
      s.report('m1', 60000 + i * 1000, 600000);
      s.tick();
    }
    // The player paused: no reports for longer than [idle].
    now = now.add(const Duration(seconds: 5));
    s.tick();
    s.tick();
    now = now.add(const Duration(seconds: 30));
    s.report('m1', 63000, 600000);
    now = now.add(const Duration(seconds: 1));
    s.report('m1', 540000, 600000);
    s.close();
    await Future<void>.delayed(Duration.zero);
    expect(trakt.calls, [
      'start m1 10.0',
      'pause m1 10.5',
      'start m1 10.5',
      'stop m1 90.0',
    ]);
  });

  test('Up Next stops the previous item and starts the next one', () async {
    final trakt = FakeTrakt(account: 'tester');
    final s = TraktScrobbler(trakt, timer: false);
    s.report('e1', 1150000, 1200000);
    s.report('e2', 0, 1200000);
    s.close();
    await Future<void>.delayed(Duration.zero);
    expect(trakt.calls, [
      'start e1 95.8',
      'stop e1 95.8',
      'start e2 0.0',
      'stop e2 0.0',
    ]);
  });

  test('nothing is sent when signed out or without Trakt', () async {
    final trakt = FakeTrakt();
    final s = TraktScrobbler(trakt, timer: false);
    s.report('m1', 1000, 10000);
    s.close();
    TraktScrobbler(null, timer: false)
      ..report('m1', 1000, 10000)
      ..close();
    await Future<void>.delayed(Duration.zero);
    expect(trakt.calls, isEmpty);
  });

  test('the periodic sync runs only while signed in', () async {
    final trakt = FakeTrakt();
    final auto = TraktAutoSync(trakt);
    expect(await auto.run(), isNull);
    trakt.signedIn = 'tester';
    final r = await auto.run();
    expect(r?.watchedImported, 1);
    expect(trakt.syncs, 1);
    auto.dispose();
  });
}
