// SPDX-License-Identifier: MPL-2.0
//
// BUG-030: the remote config is checked soon after start and then
// periodically; a newer document changes the Twitch route without a restart.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/twitch/remote_config_refresh.dart';
import 'package:kinolune/twitch/twitch_backend.dart';
import 'package:kinolune/twitch/twitch_service.dart';

void main() {
  testWidgets('first check after the delay, then on the timer', (tester) async {
    var checks = 0;
    var updates = 0;
    final r = RemoteConfigRefresher(
      check: () async {
        checks++;
        return checks == 1;
      },
      onUpdated: () => updates++,
      firstDelay: const Duration(seconds: 10),
      every: const Duration(minutes: 30),
      watchLifecycle: false,
    )..start();
    addTearDown(r.dispose);

    await tester.pump(const Duration(seconds: 9));
    expect(checks, 0, reason: 'not on the start path');
    await tester.pump(const Duration(seconds: 1));
    expect(checks, 1);
    expect(updates, 1);

    await tester.pump(const Duration(minutes: 30));
    expect(checks, 2);
    expect(updates, 1, reason: 'unchanged: nothing to apply');
  });

  testWidgets('a failed check is swallowed and retried later', (tester) async {
    var checks = 0;
    final r = RemoteConfigRefresher(
      check: () async {
        checks++;
        if (checks == 1) throw StateError('offline');
        return false;
      },
      onUpdated: () => fail('nothing new'),
      firstDelay: Duration.zero,
      every: const Duration(minutes: 1),
      watchLifecycle: false,
    )..start();
    addTearDown(r.dispose);
    await tester.pump();
    expect(checks, 1);
    await tester.pump(const Duration(minutes: 1));
    expect(checks, 2);
  });

  test('checks do not overlap', () async {
    var checks = 0;
    final r = RemoteConfigRefresher(
      check: () async {
        checks++;
        await Future<void>.delayed(const Duration(milliseconds: 10));
        return false;
      },
      onUpdated: () {},
      watchLifecycle: false,
    );
    await Future.wait([r.run(), r.run(), r.run()]);
    expect(checks, 1);
    await r.run();
    expect(checks, 2);
  });

  test('a newer config switches the route to native without a restart', () {
    Future<void> noop(BuildContext _, TwitchPlayTarget _) async {}
    final twitch = TwitchService(
      backend: const LocalTwitchBackend(),
      preferences: MemoryPreferences(),
      fullBuild: true,
    )..embedPlayer = noop;
    var notified = 0;
    twitch.addListener(() => notified++);
    expect(twitch.playRoute, TwitchPlayRoute.embed);

    // What main.dart does on `updated`: register the player, then apply.
    twitch
      ..nativePlayer = noop
      ..applyNativeConfig(allowed: true, killed: false);
    expect(twitch.playRoute, TwitchPlayRoute.native);
    expect(notified, 1);

    twitch.applyNativeConfig(allowed: true, killed: false);
    expect(notified, 1, reason: 'no change, no rebuild');

    // A later kill-switch sends the next player back to the embed.
    twitch.applyNativeConfig(allowed: true, killed: true);
    expect(twitch.playRoute, TwitchPlayRoute.embed);
  });

  test('store builds stay off the native route', () {
    Future<void> noop(BuildContext _, TwitchPlayTarget _) async {}
    final twitch =
        TwitchService(
            backend: const LocalTwitchBackend(),
            preferences: MemoryPreferences(),
          )
          ..embedPlayer = noop
          ..nativePlayer = noop
          ..applyNativeConfig(allowed: true, killed: false);
    expect(twitch.playRoute, TwitchPlayRoute.embed);
  });
}
