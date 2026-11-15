// SPDX-License-Identifier: MPL-2.0
//
// ADR-NNNN (TWI-019 follow-up): record rules belong to the household, not
// to the viewer on screen. A kid profile keeps the live watcher running
// for the rules (the core's AutoRecord queues the recording on
// `stream.online`, see pa-frb `a_kid_profile_keeps_auto_record_queuing`),
// but gets no "X is live" notification, no recordings view and cannot
// change a rule without the parent token. Made-up channels only.
//
//   flutter test test/twitch_autorecord_profiles_test.dart
//   flutter test test/twitch_autorecord_profiles_test.dart \
//     --dart-define=KINOLUNE_FLAVOR=full
// Screenshots with KINOLUNE_SCREENSHOTS_DIR set.

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_profiles.dart';
import 'package:kinolune/fakes/fake_twitch.dart';
import 'package:kinolune/fakes/fake_twitch_archive.dart';
import 'package:kinolune/flavor.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/profiles/profile_controller.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/live_notifications.dart';
import 'package:kinolune/twitch/twitch_archive.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_core/pa_core.dart' as core;
import 'package:pa_library/pa_library.dart'
    show LibraryProfile, ProfileAccessDenied;

import 'support/test_app.dart';

class _FakeSource implements LiveEventSource {
  final events = StreamController<LiveEvent>.broadcast();
  List<LiveWatch>? started;
  final updates = <List<LiveWatch>>[];
  var starts = 0;
  var stops = 0;

  List<LiveWatch> get watched => updates.isEmpty ? started! : updates.last;

  @override
  Stream<LiveEvent> start(List<LiveWatch> channels, LivePower power) {
    starts++;
    started = channels;
    updates.clear();
    return events.stream;
  }

  @override
  void update(List<LiveWatch> channels) => updates.add(channels);

  @override
  void setPower(LivePower power) {}

  @override
  void pollNow() {}

  @override
  void stop() => stops++;
}

class _Recorder implements LiveNotifier {
  final shown = <String>[];

  @override
  Future<bool> init() async => true;

  @override
  Future<void> show(
    LiveAlertInfo alert, {
    required String title,
    String? body,
  }) async => shown.add(alert.userId);
}

LiveAlertInfo _online(String userId, String name) => LiveAlertInfo(
  userId: userId,
  login: name.toLowerCase(),
  displayName: name,
  streamId: 's-$userId',
  startedAt: demoNow,
  detectedAt: demoNow.add(const Duration(seconds: 3)),
  source: 'eventsub',
);

/// The core queue faked at the binding: like pa-frb, a rule change is
/// `denied` unless the parent token comes with it.
class _GuardedQueue implements core.CoreTwitchDownloads {
  _GuardedQueue(this.token);

  final String token;
  final calls = <String>[];

  Future<void> _check(String what, String? parentToken) async {
    if (parentToken != token) {
      throw const core.CoreDownloadError(
        code: 'denied',
        message: 'the parent PIN is needed',
      );
    }
    calls.add(what);
  }

  @override
  Future<void> setRecordRule({
    required int profileId,
    required core.CoreRecordRule rule,
    String? parentToken,
  }) => _check('set ${rule.userId}', parentToken);

  @override
  Future<void> deleteRecordRule({
    required int profileId,
    required String userId,
    String? parentToken,
  }) => _check('delete $userId', parentToken);

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  setUpAll(loadAppFonts);

  /// A parent (PIN 1234) with a record rule on the first made-up channel
  /// and a bell on the second; the kid profile Mia.
  Future<
    ({
      AppServices services,
      FakeProfiles profiles,
      _FakeSource source,
      _Recorder notifier,
      FakeTwitchBackend backend,
    })
  >
  household() async {
    final profiles = FakeProfiles.demo();
    final source = _FakeSource();
    final notifier = _Recorder();
    final backend = FakeTwitchBackend.demo(now: demoNow);
    final archive = FakeTwitchArchive.demo(demoNow);
    final ruled = backend.channels.first;
    await archive.setRule(
      1,
      TwitchRecordRule(userId: ruled.userId!, enabled: true),
    );
    final services = AppServices(
      library: FakeLibrary.demo(scanStepDelay: Duration.zero),
      secureStore: MemorySecureStore(),
      clock: () => demoNow,
      // No start picker over the shell.
      preferences: MemoryPreferences()
        ..values[Preferences.profilesPickerOnStart] = 'false',
      twitchBackend: backend,
      twitchArchive: archive,
      profileStore: profiles,
      liveSources: (_) => source,
      liveNotifier: notifier,
    );
    await services.init();
    await services.twitch.addFavorite(backend.channels[1]);
    await pumpEventQueue();
    return (
      services: services,
      profiles: profiles,
      source: source,
      notifier: notifier,
      backend: backend,
    );
  }

  LibraryProfile kid(AppServices s) =>
      s.profileControl.all.firstWhere((p) => p.isKid);

  test('a kid profile keeps the watcher for the rules, without alerts', () async {
    final h = await household();
    final s = h.services;
    final ruled = h.backend.channels.first;
    final belled = h.backend.channels[1];
    expect(s.liveNotifications.running, isTrue);
    expect(h.source.watched.map((w) => w.userId), [
      belled.userId,
      ruled.userId,
    ]);

    // The parent hands the device to the kid.
    expect(await s.profileControl.switchTo(kid(s)), PinCheck.ok);
    await pumpEventQueue();
    expect(s.profileControl.restricted, isTrue);
    // Not stopped, not restarted: the core's AutoRecord keeps hearing
    // `stream.online` of the household's channel; the kid's bells are off.
    expect(h.source.stops, 0);
    expect(h.source.starts, 1);
    expect(s.liveNotifications.running, isTrue);
    expect(h.source.watched.map((w) => w.userId), [ruled.userId]);
    expect(h.source.watched.single.notify, isFalse);
    expect(s.liveNotifications.recordingChannels, {ruled.userId});

    // A broadcast starts: nothing is shown to the kid.
    h.source.events
      ..add(LiveEvent.alert(_online(ruled.userId!, ruled.displayName)))
      ..add(LiveEvent.alert(_online(belled.userId!, belled.displayName)));
    await pumpEventQueue();
    expect(h.notifier.shown, isEmpty);

    // Back to the parent (PIN): the bells again, on the same watcher.
    expect(await s.profileControl.verifyParent('1234'), PinCheck.ok);
    final parent = s.profileControl.all.firstWhere((p) => p.id == 1);
    expect(await s.profileControl.switchTo(parent, pin: '1234'), PinCheck.ok);
    await pumpEventQueue();
    expect(h.source.stops, 0);
    expect(h.source.watched.map((w) => w.userId), contains(belled.userId));
    h.source.events.add(
      LiveEvent.alert(_online(belled.userId!, belled.displayName)),
    );
    await pumpEventQueue();
    expect(h.notifier.shown, [belled.userId]);
  });

  test('the recordings stay hidden from a kid profile', () async {
    final h = await household();
    final s = h.services;
    expect(s.visibleArchive, same(s.twitchArchive));
    // Even when a parent shows the Streams section to the kid.
    await s.profileControl.setHideTwitch(kid(s), false);
    await s.profileControl.switchTo(kid(s));
    await pumpEventQueue();
    expect(s.profileControl.twitchHidden, isFalse);
    expect(s.visibleArchive, isNull);
    expect(s.twitchArchive, isNotNull, reason: 'the rules still record');
  });

  test('rule changes are denied without the parent token', () async {
    final queue = _GuardedQueue('made-up-parent-token');
    String? token;
    final archive = CoreTwitchArchive(queue, parentToken: () => token);
    const rule = TwitchRecordRule(userId: '81000001', enabled: false);
    await expectLater(
      archive.setRule(1, rule),
      throwsA(isA<ProfileAccessDenied>()),
    );
    await expectLater(
      archive.deleteRule(1, '81000001'),
      throwsA(isA<ProfileAccessDenied>()),
    );
    expect(queue.calls, isEmpty);
    // After the parent PIN (the library holds the token).
    token = 'made-up-parent-token';
    await archive.setRule(1, rule);
    await archive.deleteRule(1, '81000001');
    expect(queue.calls, ['set 81000001', 'delete 81000001']);
  });

  testWidgets('Streams of a kid profile: no recordings entry', (t) async {
    setSurface(t, const Size(1280, 800));
    final h = (await t.runAsync(household))!;
    final s = h.services;
    await s.setShowTwitch(true);
    await s.profileControl.setHideTwitch(kid(s), false);
    await t.pumpWidget(testApp(s, section: Section.twitch));
    await settle(t);
    final card = find.byKey(const ValueKey('twitch-archive-card'));
    if (AppFlavor.twitchArchive) {
      await t.scrollUntilVisible(
        card,
        300,
        scrollable: find.byType(Scrollable).first,
      );
      expect(card, findsOneWidget);
      await capture(t, 'twi-019-kid/01-parent-streams-with-recordings');
    }

    await s.profileControl.switchTo(kid(s));
    await settle(t, 8);
    t.state<AppShellState>(find.byType(AppShell)).select(Section.twitch);
    await settle(t, 6);
    expect(find.byKey(const ValueKey('twitch-archive-card')), findsNothing);
    expect(find.byKey(const ValueKey('twitch-archive')), findsNothing);
    // Not even the "Downloaded and recorded" row.
    expect(
      find.text(lookupAppLocalizations(const Locale('en')).twitchDownloaded),
      findsNothing,
    );
    final categories = find.text(
      lookupAppLocalizations(const Locale('en')).twitchCategories,
    );
    if (categories.evaluate().isNotEmpty) {
      await t.ensureVisible(categories.first);
    }
    await settle(t, 2);
    await capture(t, 'twi-019-kid/02-kid-streams-without-recordings');
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });
}
