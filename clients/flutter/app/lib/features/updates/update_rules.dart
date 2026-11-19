// SPDX-License-Identifier: MPL-2.0
//
// The rules behind the app's own update checks (PKG-022, ADR-0109): when the
// scheduler asks, what a check nobody asked for must leave alone, and what
// "automatically" means. Kept apart from the controller and the platforms so
// they can be tested on their own. They follow the owner's other project
// (Universal-Video-Downloader, `updater-rules.ts`).

/// How the app treats a new release.
enum UpdateMode {
  /// Silent updates (PKG-024): installs that update themselves download and
  /// verify a new release in the background, never while something plays.
  /// Desktops apply it at the next start (Windows: when the app quits) or
  /// restart into it once the app is idle; Android, whose system dialog
  /// cannot be skipped, offers «Установить» when nothing plays.
  auto,

  /// Only say that a release exists; download when asked.
  notify,
}

/// Timings of the scheduler.
abstract final class UpdateSchedule {
  /// The first look after launch.
  static const launchDelay = Duration(seconds: 5);

  /// How long an answer from GitHub lasts: four requests a day, far below
  /// the 60 an hour GitHub allows an address without an account.
  static const every = Duration(hours: 6);

  /// How soon to try again after a scheduled check that got no answer (a
  /// check at login often runs before the network is up).
  static const retry = Duration(minutes: 30);

  /// How often the scheduler asks whether a check is due. Free unless one
  /// is.
  static const tick = Duration(minutes: 10);

  /// The first look after the computer wakes up, once Wi-Fi has had a
  /// chance to come back.
  static const resumeDelay = Duration(seconds: 60);

  /// A gap between two ticks this much longer than [tick] means the
  /// computer slept (timers do not run in sleep).
  static const sleepGap = Duration(minutes: 20);

  /// PKG-024: a desktop restarts into a staged update after this long
  /// without a key press, a click or a touch, with nothing playing.
  static const idleApply = Duration(minutes: 10);

  /// How often a staged update looks whether the app is idle.
  static const idlePoll = Duration(seconds: 30);
}

/// Whether a silent update may download now (PKG-024): never while
/// something plays (it would take bandwidth from the stream), and on
/// Android not over a metered network.
bool mayDownloadQuietly({required bool playing, required bool metered}) =>
    !playing && !metered;

/// Whether a desktop may restart into a staged update by itself
/// (PKG-024): nothing plays, nobody touched the app for [idle] and the
/// window is in front (a window that pops up over other work would
/// interrupt). Never on a TV: there it waits for the next start. A time of
/// the last input in the future (the clock moved back) counts as just now.
bool mayApplyWhenIdle({
  required DateTime now,
  required DateTime lastInput,
  required bool playing,
  required bool foreground,
  required bool tv,
  Duration idle = UpdateSchedule.idleApply,
}) {
  if (playing || tv || !foreground) return false;
  final quiet = now.difference(lastInput);
  return !quiet.isNegative && quiet >= idle;
}

/// Whether a scheduled check is due at [now].
///
/// [lastAnswered] is the last time any check, scheduled or asked for, heard
/// back from GitHub; [lastTried] the last scheduled attempt, answered or
/// not. Both are wall-clock times, so a laptop that slept through a check
/// makes it on the first tick after waking. A time in the future means the
/// clock was moved back and counts as long ago, rather than holding checks
/// off until the clock catches up.
bool checkDue(DateTime now, DateTime? lastAnswered, DateTime? lastTried) {
  bool recent(DateTime? at, Duration span) {
    if (at == null) return false;
    final age = now.difference(at);
    return !age.isNegative && age < span;
  }

  if (recent(lastAnswered, UpdateSchedule.every)) return false;
  if (recent(lastTried, UpdateSchedule.retry)) return false;
  return true;
}

/// Whether the computer slept between two scheduler ticks.
bool sleptBetween(DateTime previousTick, DateTime now) =>
    now.difference(previousTick) > UpdateSchedule.sleepGap;

/// The states a check nobody asked for leaves alone (see
/// [mayCheckInBackground]).
enum UpdateActivity {
  /// Nothing running, or a release on offer (checked again on purpose: a
  /// newer one may have come out since).
  idle,

  /// A check is running.
  checking,

  /// A download, its verification or the installation is running.
  downloading,

  /// The update is in place or staged and waits for «Перезапустить».
  readyToRestart,
}

/// Whether a scheduled check may run now. Never over a running download or
/// a pending «Перезапустить»: a fresh answer would replace that state with
/// "available" and take the restart button away.
bool mayCheckInBackground(UpdateActivity activity) =>
    activity == UpdateActivity.idle;
