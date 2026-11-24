// SPDX-License-Identifier: MPL-2.0
//
// SYNC-011: the player's scrobbles to Trakt and the periodic sync.

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/trakt/trakt_link.dart';

class _FakeTrakt implements TraktLink {
  _FakeTrakt({String? account}) : _account = ValueNotifier(account);

  final ValueNotifier<String?> _account;
  final calls = <String>[];
  int syncs = 0;

  @override
  ValueListenable<String?> get account => _account;

  set signedIn(String? name) => _account.value = name;

  @override
  bool get configured => true;

  @override
  bool get ownClient => false;

  @override
  Future<void> setOwnClient(String? clientId, String? clientSecret) async {}

  @override
  Future<TraktDeviceLogin> startLogin() async => const TraktDeviceLogin(
    userCode: 'ABCD1234',
    verificationUrl: 'https://trakt.tv/activate',
    qrUrl: 'https://trakt.tv/activate/ABCD1234',
    expiresIn: Duration(minutes: 10),
    interval: Duration(seconds: 5),
  );

  @override
  Future<String?> pollLogin() async => 'tester';

  @override
  void cancelLogin() {}

  @override
  Future<void> signOut() async => _account.value = null;

  @override
  Future<bool> scrobble(String itemId, String action, double progress) async {
    calls.add('$action $itemId ${progress.toStringAsFixed(1)}');
    return true;
  }

  @override
  Future<TraktSyncResult> sync() async {
    syncs++;
    return const TraktSyncResult(watchedImported: 1);
  }

  @override
  Future<bool> setWatched(String itemId, bool watched) async => true;

  @override
  Future<bool> setRating(String itemId, int? stars) async => true;

  @override
  Future<List<String>> watchlistItems({bool refresh = false}) async => [];

  @override
  Future<bool> inWatchlist(String itemId) async => false;

  @override
  Future<bool> setWatchlisted(String itemId, bool listed) async => true;
}

void main() {
  test('start, pause after silence, start again, stop on close', () async {
    final trakt = _FakeTrakt(account: 'tester');
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
    final trakt = _FakeTrakt(account: 'tester');
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
    final trakt = _FakeTrakt();
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
    final trakt = _FakeTrakt();
    final auto = TraktAutoSync(trakt);
    expect(await auto.run(), isNull);
    trakt.signedIn = 'tester';
    final r = await auto.run();
    expect(r?.watchedImported, 1);
    expect(trakt.syncs, 1);
    auto.dispose();
  });
}
