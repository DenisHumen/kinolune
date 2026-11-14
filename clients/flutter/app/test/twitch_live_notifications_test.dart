// SPDX-License-Identifier: MPL-2.0
//
// "X is live" (TWI-009), app side: which favourites the core watches, the
// per-channel bell, the global switch, notification texts and the honest
// platform hints. The core's watcher is faked here; its own tests (EventSub,
// polling, budget, timing) are in core/crates/pa-twitch-live.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_twitch.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/screens/settings_screen.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/live_notifications.dart';
import 'package:kinolune/twitch/live_notifications_platform.dart'
    show liveNotificationId;
import 'package:kinolune/twitch/twitch_models.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

class _FakeSource implements LiveEventSource {
  final events = StreamController<LiveEvent>.broadcast();
  List<LiveWatch>? started;
  final updates = <List<LiveWatch>>[];
  final powers = <LivePower>[];
  var polls = 0;
  var stopped = false;

  @override
  Stream<LiveEvent> start(List<LiveWatch> channels, LivePower power) {
    started = channels;
    powers.add(power);
    return events.stream;
  }

  @override
  void update(List<LiveWatch> channels) => updates.add(channels);

  @override
  void setPower(LivePower power) => powers.add(power);

  @override
  void pollNow() => polls++;

  @override
  void stop() => stopped = true;
}

class _Recorder implements LiveNotifier {
  final shown = <({String id, String title, String? body})>[];

  @override
  Future<bool> init() async => true;

  @override
  Future<void> show(
    LiveAlertInfo alert, {
    required String title,
    String? body,
  }) async => shown.add((id: alert.streamId, title: title, body: body));
}

LiveAlertInfo _alert(String userId, String name, {String stream = 's1'}) =>
    LiveAlertInfo(
      userId: userId,
      login: name.toLowerCase(),
      displayName: name,
      streamId: stream,
      title: 'Made-up title',
      gameName: 'Invented Game',
      startedAt: demoNow,
      detectedAt: demoNow.add(const Duration(seconds: 3)),
      source: 'eventsub',
    );

void main() {
  final en = lookupAppLocalizations(const Locale('en'));

  Future<
    ({
      TwitchService twitch,
      LiveNotificationController live,
      _FakeSource source,
      _Recorder notifier,
      MemoryPreferences prefs,
      FakeTwitchBackend backend,
    })
  >
  setUpLive({bool signedIn = true, MemoryPreferences? prefs}) async {
    final backend = FakeTwitchBackend.demo(now: demoNow, signedIn: signedIn);
    final preferences = prefs ?? MemoryPreferences();
    final twitch = TwitchService(
      backend: backend,
      preferences: preferences,
      clock: () => demoNow,
    );
    await twitch.init();
    final source = _FakeSource();
    final notifier = _Recorder();
    final live = LiveNotificationController(
      twitch: twitch,
      preferences: preferences,
      notifier: notifier,
      sourceFor: () => twitch.account == null ? null : source,
      texts: (a) => liveTexts(en, a),
    );
    await live.init();
    return (
      twitch: twitch,
      live: live,
      source: source,
      notifier: notifier,
      prefs: preferences,
      backend: backend,
    );
  }

  test('favourite JSON keeps the bell; on by default', () {
    final f = TwitchFavorite(login: 'made_up', addedAt: DateTime(2026, 11, 1));
    expect(f.notify, isTrue);
    expect(f.toJson().containsKey('notify'), isFalse);
    final off = f.copyWith(notify: false);
    expect(TwitchFavorite.fromJson(off.toJson())!.notify, isFalse);
    expect(TwitchFavorite.fromJson(f.toJson())!.notify, isTrue);
  });

  test('starts with the resolved favourites and shows alerts', () async {
    final h = await setUpLive();
    expect(h.source.started, isNull, reason: 'no favourites yet');
    final channels = h.backend.channels.take(2).toList();
    for (final c in channels) {
      await h.twitch.addFavorite(c);
    }
    await pumpEventQueue();
    expect(h.live.running, isTrue);
    // Started with the first favourite; the second arrives as an update.
    final watched = h.source.updates.isEmpty
        ? h.source.started!
        : h.source.updates.last;
    expect(watched.map((w) => w.userId), [for (final c in channels) c.userId]);
    expect(watched.every((w) => w.notify), isTrue);

    final c = channels.first;
    h.source.events.add(LiveEvent.alert(_alert(c.userId!, c.displayName)));
    await pumpEventQueue();
    expect(h.notifier.shown.single.title, '${c.displayName} is live');
    expect(h.notifier.shown.single.body, 'Made-up title · Invented Game');
    expect(h.live.shown, hasLength(1));
  });

  test('the bell: mirrored, sent to the core, alerts silenced', () async {
    final h = await setUpLive();
    final channels = h.backend.channels.take(2).toList();
    for (final c in channels) {
      await h.twitch.addFavorite(c);
    }
    await pumpEventQueue();
    final quiet = channels.first;
    await h.twitch.setNotify(quiet.login, false);
    await pumpEventQueue();
    expect(h.twitch.notifyOf(quiet.login), isFalse);
    expect(h.backend.syncedNotify[quiet.userId], isFalse);
    final last = h.source.updates.last;
    expect(last.firstWhere((w) => w.userId == quiet.userId).notify, isFalse);

    h.source.events.add(
      LiveEvent.alert(_alert(quiet.userId!, quiet.displayName)),
    );
    await pumpEventQueue();
    expect(h.notifier.shown, isEmpty);

    // All bells off: the watcher stops.
    await h.twitch.setNotify(channels.last.login, false);
    await pumpEventQueue();
    expect(h.source.stopped, isTrue);
    expect(h.live.running, isFalse);
  });

  test('global switch: off stops and is remembered', () async {
    final h = await setUpLive();
    await h.twitch.addFavorite(h.backend.channels.first);
    await pumpEventQueue();
    expect(h.live.running, isTrue);
    await h.live.setEnabled(false);
    expect(h.source.stopped, isTrue);
    expect(await h.prefs.read(Preferences.twitchLiveNotifications), 'false');

    final again = await setUpLive(prefs: h.prefs);
    await pumpEventQueue();
    expect(again.live.enabled, isFalse);
    expect(again.source.started, isNull);
  });

  test('signed out: nothing to run', () async {
    final h = await setUpLive(signedIn: false);
    await h.twitch.addFavorite(h.backend.channels.first);
    await pumpEventQueue();
    expect(h.live.running, isFalse);
  });

  test('power, resume and status reach the core', () async {
    final h = await setUpLive();
    h.live.setPower(LivePower.battery);
    await h.twitch.addFavorite(h.backend.channels.first);
    await pumpEventQueue();
    expect(h.source.powers, [LivePower.battery]);
    h.live.setPower(LivePower.saver);
    h.live.resumed();
    expect(h.source.powers.last, LivePower.saver);
    expect(h.source.polls, 1);
    h.source.events.add(
      const LiveEvent.status(
        LiveStatusInfo(
          eventsub: 'connected',
          eventsubChannels: 1,
          polledOnly: 0,
          nextPollSecs: 120,
        ),
      ),
    );
    await pumpEventQueue();
    expect(h.live.status?.eventsub, 'connected');
  });

  test('notification ids are stable per broadcast', () {
    expect(liveNotificationId('123'), liveNotificationId('123'));
    expect(liveNotificationId('123'), isNot(liveNotificationId('124')));
    expect(liveNotificationId('x' * 64), lessThan(1 << 31));
  });

  testWidgets('channel page bell and the honest settings hint', (t) async {
    setSurface(t, const Size(1280, 800));
    final services = await demoServices();
    await services.setShowTwitch(true);
    final channel = services.twitch.backend is FakeTwitchBackend
        ? (services.twitch.backend as FakeTwitchBackend).channels.first
        : throw StateError('fake backend expected');
    await services.twitch.addFavorite(channel);
    await t.pumpWidget(testApp(services, section: Section.twitch));
    await settle(t);
    final card = find.byKey(ValueKey('twitch-favorite-${channel.login}'));
    await t.ensureVisible(card.first);
    await settle(t, 4);
    await t.tap(card.first);
    await settle(t, 10);
    final bell = find.byKey(const ValueKey('twitch-channel-notify'));
    expect(bell, findsOneWidget);
    expect(find.text('Live alerts on'), findsOneWidget);
    await t.ensureVisible(bell);
    await t.tap(bell);
    await settle(t, 4);
    expect(services.twitch.notifyOf(channel.login), isFalse);
    expect(find.text('Live alerts off'), findsOneWidget);
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);

    // Desktop and Android hints while signed in.
    final tile = find.byKey(
      const ValueKey('settings-twitch-live-notifications'),
    );
    Future<void> openSettings(TargetPlatform platform) async {
      debugDefaultTargetPlatformOverride = platform;
      await t.pumpWidget(testApp(services, section: Section.settings));
      await settle(t, 8);
      final page = find
          .descendant(
            of: find.byType(SettingsScreen),
            matching: find.byType(Scrollable),
          )
          .first;
      await t.scrollUntilVisible(tile, 200, scrollable: page);
      await settle(t, 3);
    }

    await openSettings(TargetPlatform.linux);
    expect(
      find.descendant(of: tile, matching: find.textContaining('2 minutes')),
      findsOneWidget,
    );
    await openSettings(TargetPlatform.android);
    expect(
      find.descendant(
        of: tile,
        matching: find.textContaining('every 15 minutes'),
      ),
      findsOneWidget,
    );
    debugDefaultTargetPlatformOverride = null;
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });
}
