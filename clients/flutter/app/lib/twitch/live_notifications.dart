// SPDX-License-Identifier: MPL-2.0
//
// "X is live" notifications without any cloud (TWI-009). The core
// (pa-twitch-live) watches the favourites: EventSub WebSocket for about ten
// channels (Twitch's cost budget), polling of GET /streams for the rest,
// one notification per broadcast. This file is the app side: which channels
// are watched (favourites with their bell on), the device's power state,
// and showing the notification. Platform plumbing (the notification plugin,
// Android WorkManager) is in live_notifications_platform.dart.
//
// What each platform can do (also said in Settings):
// - Desktop (Linux, Windows, macOS): a background thread while the app
//   runs, tray-less; ≤ 2 minutes after the start of a broadcast.
// - Android: the same while the app process lives; after Android stops it,
//   a WorkManager check at best every 15 minutes, later in Doze or battery
//   saver. No foreground service: a permanent notification for this is not
//   worth it.
// - iOS (later): only while the app is open, or BGAppRefresh when iOS
//   decides.

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../l10n/l10n.dart';
import '../services/preferences.dart';
import 'twitch_service.dart';

/// A channel the core watches.
@immutable
class LiveWatch {
  const LiveWatch({
    required this.userId,
    required this.login,
    required this.displayName,
    required this.notify,
  });

  final String userId;
  final String login;
  final String displayName;

  /// Its bell is on.
  final bool notify;

  @override
  bool operator ==(Object other) =>
      other is LiveWatch &&
      other.userId == userId &&
      other.login == login &&
      other.displayName == displayName &&
      other.notify == notify;

  @override
  int get hashCode => Object.hash(userId, login, displayName, notify);
}

/// Power state: sets the polling interval (60 s, 90 s, 5 min).
enum LivePower { plugged, battery, saver }

/// An "X is live" notification.
@immutable
class LiveAlertInfo {
  const LiveAlertInfo({
    required this.userId,
    required this.login,
    required this.displayName,
    required this.streamId,
    required this.startedAt,
    required this.detectedAt,
    required this.source,
    this.title,
    this.gameName,
  });

  final String userId;
  final String login;
  final String displayName;
  final String streamId;
  final String? title;
  final String? gameName;
  final DateTime startedAt;
  final DateTime detectedAt;

  /// `eventsub` or `polling`.
  final String source;

  /// Delay between the start and the notification.
  Duration get delay => detectedAt.difference(startedAt);
}

/// Health of the watcher (Settings).
@immutable
class LiveStatusInfo {
  const LiveStatusInfo({
    required this.eventsub,
    required this.eventsubChannels,
    required this.polledOnly,
    required this.nextPollSecs,
    this.budgetHit = false,
    this.pollErrors = 0,
    this.lastError,
  });

  /// `off`, `connecting`, `connected`, `reconnecting`, `stopped`.
  final String eventsub;
  final int eventsubChannels;
  final int polledOnly;
  final int nextPollSecs;
  final bool budgetHit;
  final int pollErrors;
  final String? lastError;
}

/// One event of a [LiveEventSource].
@immutable
class LiveEvent {
  const LiveEvent.alert(LiveAlertInfo this.alert) : status = null;
  const LiveEvent.status(LiveStatusInfo this.status) : alert = null;

  final LiveAlertInfo? alert;
  final LiveStatusInfo? status;
}

/// The core's watcher (or a fake in tests).
abstract interface class LiveEventSource {
  /// Starts watching; the stream ends after [stop].
  Stream<LiveEvent> start(List<LiveWatch> channels, LivePower power);
  void update(List<LiveWatch> channels);
  void setPower(LivePower power);
  void pollNow();
  void stop();
}

/// Shows notifications (the plugin, or a recorder in tests).
abstract interface class LiveNotifier {
  /// Prepares the plugin and asks for the permission where needed; false
  /// if notifications cannot be shown.
  Future<bool> init();

  Future<void> show(LiveAlertInfo alert, {required String title, String? body});
}

/// Shows nothing (tests, platforms without notifications).
class NoLiveNotifier implements LiveNotifier {
  const NoLiveNotifier();

  @override
  Future<bool> init() async => false;

  @override
  Future<void> show(
    LiveAlertInfo alert, {
    required String title,
    String? body,
  }) async {}
}

/// Texts of a notification in the UI language.
typedef LiveTexts = ({String title, String? body}) Function(LiveAlertInfo);

/// The texts of a notification in [l10n].
({String title, String? body}) liveTexts(
  AppLocalizations l10n,
  LiveAlertInfo a,
) {
  final title = l10n.twitchLiveNotificationTitle(a.displayName);
  final parts = [?a.title, ?a.gameName].where((s) => s.isNotEmpty).toList();
  return (title: title, body: parts.isEmpty ? null : parts.join(' · '));
}

/// Favourites with a known user id, as watched channels.
List<LiveWatch> watchedChannels(TwitchService twitch) => [
  for (final f in twitch.favorites)
    if (f.userId case final id?)
      LiveWatch(
        userId: id,
        login: f.login,
        displayName: f.displayName ?? f.login,
        notify: f.notify,
      ),
];

/// Runs the watcher while notifications are on and someone is watchable,
/// and turns its alerts into notifications.
class LiveNotificationController extends ChangeNotifier {
  LiveNotificationController({
    required this.twitch,
    required this.preferences,
    required this.notifier,
    required this.sourceFor,
    required this.texts,
  });

  final TwitchService twitch;
  final Preferences preferences;
  final LiveNotifier notifier;

  /// The watcher for the current sign-in; null when there is none (no
  /// client id, signed out).
  final LiveEventSource? Function() sourceFor;
  final LiveTexts texts;

  bool _enabled = true;
  bool _ready = false;
  bool _permitted = false;
  LivePower _power = LivePower.plugged;
  LiveEventSource? _source;
  StreamSubscription<LiveEvent>? _sub;
  List<LiveWatch> _channels = const [];
  LiveStatusInfo? _status;
  final List<LiveAlertInfo> _shown = [];
  void _listener() => unawaited(_sync());

  /// Notifications on (Settings); on by default.
  bool get enabled => _enabled;

  /// The watcher runs.
  bool get running => _source != null;

  /// The system lets us show notifications.
  bool get permitted => _permitted;

  LiveStatusInfo? get status => _status;

  /// Notifications shown in this session, newest last.
  List<LiveAlertInfo> get shown => List.unmodifiable(_shown);

  Future<void> init() async {
    _enabled =
        await preferences.read(Preferences.twitchLiveNotifications) != 'false';
    _ready = true;
    twitch.addListener(_listener);
    await _sync();
  }

  Future<void> setEnabled(bool on) async {
    _enabled = on;
    await preferences.write(
      Preferences.twitchLiveNotifications,
      on ? null : 'false',
    );
    await _sync();
    notifyListeners();
  }

  /// The device's power state changed.
  void setPower(LivePower power) {
    if (power == _power) return;
    _power = power;
    _source?.setPower(power);
  }

  /// The app came back to the foreground: check now.
  void resumed() => _source?.pollNow();

  /// The sign-in changed: a new watcher for the new token.
  Future<void> restart() async {
    _stop();
    await _sync();
  }

  Future<void> _sync() async {
    if (!_ready) return;
    final channels = watchedChannels(twitch);
    final want = _enabled && channels.any((c) => c.notify);
    if (!want) {
      if (_source != null) {
        _stop();
        notifyListeners();
      }
      return;
    }
    final source = _source;
    if (source != null) {
      if (!listEquals(channels, _channels)) {
        _channels = channels;
        source.update(channels);
      }
      return;
    }
    final next = sourceFor();
    if (next == null) return;
    _permitted = await notifier.init();
    _source = next;
    _channels = channels;
    _sub = next.start(channels, _power).listen(_onEvent);
    notifyListeners();
  }

  void _onEvent(LiveEvent e) {
    if (e.status case final s?) {
      _status = s;
      notifyListeners();
    }
    final alert = e.alert;
    if (alert == null) return;
    // The bell may have been turned off since the core last heard.
    final watched = _channels.where((c) => c.userId == alert.userId);
    if (watched.isNotEmpty && !watched.first.notify) return;
    _shown.add(alert);
    final t = texts(alert);
    unawaited(notifier.show(alert, title: t.title, body: t.body));
    notifyListeners();
  }

  void _stop() {
    _sub?.cancel();
    _sub = null;
    _source?.stop();
    _source = null;
    _status = null;
  }

  @override
  void dispose() {
    twitch.removeListener(_listener);
    _stop();
    super.dispose();
  }
}
