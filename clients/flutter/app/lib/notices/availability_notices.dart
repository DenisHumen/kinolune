// SPDX-License-Identifier: MPL-2.0
//
// The banner «Нет связи с NAS» (UI-053, ADR-0157 §8) over the in-app
// notices of UI-026. Only when a source or server that worked in this run
// goes down and stays down for more than two minutes, and at most once an
// hour for each of them: a NAS that is simply off, or a phone of the
// family that left home, never nags. The dot and the line on the screens
// stay for the rest (widgets/availability_status.dart).

import 'dart:async';

import '../screens/connection_status_screen.dart';
import '../sources/availability.dart';
import '../widgets/availability_status.dart';
import 'in_app_notices.dart';

/// How long a target is down before the banner.
const availabilityBannerAfter = Duration(minutes: 2);

/// The banner of one target comes at most this often.
const availabilityBannerEvery = Duration(hours: 1);

/// Decides which targets get a banner; pure, with the time passed in.
class AvailabilityBannerPolicy {
  AvailabilityBannerPolicy({
    this.after = availabilityBannerAfter,
    this.every = availabilityBannerEvery,
  });

  final Duration after;
  final Duration every;

  /// Targets seen up in this run: only their going down is a transition.
  final Set<String> _wasUp = {};

  /// The outage (its start) each target was already decided for.
  final Map<String, DateTime> _handled = {};

  /// When each target's banner was last shown.
  final Map<String, DateTime> _shown = {};

  /// Peers come and go (a phone leaves home): they get no banner.
  static bool watched(TargetAvailability t) => !t.isPeer;

  /// The targets whose banner is due at [now].
  List<TargetAvailability> due(
    Iterable<TargetAvailability> targets,
    DateTime now,
  ) {
    final out = <TargetAvailability>[];
    for (final t in targets) {
      if (!watched(t)) continue;
      if (t.status.isUp) {
        _wasUp.add(t.id);
        _handled.remove(t.id);
        continue;
      }
      if (t.status != Availability.unavailable) continue;
      final since = t.since;
      if (since == null || !_wasUp.contains(t.id)) continue;
      if (now.difference(since) < after) continue;
      if (_handled[t.id] == since) continue;
      _handled[t.id] = since;
      final last = _shown[t.id];
      if (last != null && now.difference(last) < every) continue;
      _shown[t.id] = now;
      out.add(t);
    }
    return out;
  }

  /// When [due] may have something next; null: nothing is pending.
  DateTime? next(Iterable<TargetAvailability> targets) {
    DateTime? at;
    for (final t in targets) {
      final since = t.since;
      if (!watched(t) ||
          t.status != Availability.unavailable ||
          since == null ||
          !_wasUp.contains(t.id) ||
          _handled[t.id] == since) {
        continue;
      }
      final when = since.add(after);
      if (at == null || when.isBefore(at)) at = when;
    }
    return at;
  }
}

/// Follows [availability] and posts the banners into [notices].
class AvailabilityNotices {
  AvailabilityNotices({
    required this.availability,
    required this.post,
    DateTime Function()? now,
    AvailabilityBannerPolicy? policy,
  }) : now = now ?? DateTime.now,
       policy = policy ?? AvailabilityBannerPolicy();

  final AvailabilityService availability;

  /// Posts a notice (the feed's filter: onboarding, disposed).
  final void Function(InAppNotice notice) post;
  final DateTime Function() now;
  final AvailabilityBannerPolicy policy;

  Timer? _timer;
  bool _started = false;

  void start() {
    if (_started) return;
    _started = true;
    availability.addListener(check);
    check();
  }

  void dispose() {
    _timer?.cancel();
    if (_started) availability.removeListener(check);
  }

  /// Posts what is due and waits for the next one.
  void check() {
    final at = now();
    final targets = availability.targets;
    for (final t in policy.due(targets, at)) {
      post(notice(t));
    }
    _timer?.cancel();
    _timer = null;
    final next = policy.next(targets);
    if (next != null) {
      final wait = next.difference(at);
      _timer = Timer(
        wait.isNegative ? Duration.zero : wait + const Duration(seconds: 1),
        check,
      );
    }
  }

  /// The banner of [t]: «Нет связи с NAS с 14:05», why, «Подробнее».
  static InAppNotice notice(TargetAvailability t) => InAppNotice(
    id: 'availability-${t.id}',
    kind: InAppNoticeKind.availability,
    title: (l) => l.availabilityBannerTitle(t.name),
    body: (l) {
      final reason = availabilityReason(t);
      return [
        if (reason != null) availabilityReasonText(l, reason),
        if (t.syncChannel) l.availabilityBannerSync,
      ].join(' ');
    },
    actionLabel: (l) => l.availabilityBannerDetails,
    onAction: openConnectionStatus,
  );
}
