// SPDX-License-Identifier: MPL-2.0
//
// [LiveEventSource] on the Rust core (TWI-009): `CoreTwitchLive` runs the
// pa-twitch-live monitor on its own thread while the app runs.

import 'package:pa_core/pa_core.dart' as core;

import 'live_notifications.dart';

core.CoreLiveChannel coreLiveChannel(LiveWatch c) => core.CoreLiveChannel(
  userId: c.userId,
  login: c.login,
  displayName: c.displayName,
  notify: c.notify,
);

LiveAlertInfo liveAlertOf(core.CoreLiveAlert a) => LiveAlertInfo(
  userId: a.userId,
  login: a.login,
  displayName: a.displayName,
  streamId: a.streamId,
  title: a.title,
  gameName: a.gameName,
  startedAt: DateTime.fromMillisecondsSinceEpoch(a.startedAtMs),
  detectedAt: DateTime.fromMillisecondsSinceEpoch(a.detectedAtMs),
  source: a.source,
);

core.CoreLivePower _power(LivePower p) => switch (p) {
  LivePower.plugged => core.CoreLivePower.plugged,
  LivePower.battery => core.CoreLivePower.battery,
  LivePower.saver => core.CoreLivePower.saver,
};

class CoreLiveSource implements LiveEventSource {
  CoreLiveSource(this._live);

  /// The made-up Twitch of the core (demo mode, emulator checks): the first
  /// watched channel with a bell goes live after [firstAfter].
  factory CoreLiveSource.demo({
    Duration firstAfter = const Duration(seconds: 20),
  }) => CoreLiveSource(
    core.CoreTwitchLive.demo(firstAfterSecs: firstAfter.inSeconds),
  );

  final core.CoreTwitchLive _live;

  @override
  Stream<LiveEvent> start(List<LiveWatch> channels, LivePower power) => _live
      .events(
        channels: [for (final c in channels) coreLiveChannel(c)],
        power: _power(power),
      )
      .expand((e) sync* {
        if (e.alert case final a?) yield LiveEvent.alert(liveAlertOf(a));
        if (e.status case final s?) {
          yield LiveEvent.status(
            LiveStatusInfo(
              eventsub: s.eventsub,
              eventsubChannels: s.eventsubChannels,
              polledOnly: s.polledOnly,
              nextPollSecs: s.nextPollSecs,
              budgetHit: s.budgetHit,
              pollErrors: s.pollErrors,
              lastError: s.lastError,
            ),
          );
        }
      });

  @override
  void update(List<LiveWatch> channels) => _live.setChannels(
    channels: [for (final c in channels) coreLiveChannel(c)],
  );

  @override
  void setPower(LivePower power) => _live.setPower(power: _power(power));

  @override
  void pollNow() => _live.pollNow();

  @override
  void stop() => _live.stop();
}
