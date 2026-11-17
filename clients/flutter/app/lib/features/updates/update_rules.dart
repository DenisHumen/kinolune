// SPDX-License-Identifier: MPL-2.0
//
// The rules behind the app's own update checks (PKG-022, ADR-NNNN): when the
// scheduler asks, what a check nobody asked for must leave alone, and what
// "automatically" means. Kept apart from the controller and the platforms so
// they can be tested on their own. They follow the owner's other project
// (Universal-Video-Downloader, `updater-rules.ts`).

/// How the app treats a new release.
enum UpdateMode {
  /// Desktop installs that update themselves (Windows installer, macOS app,
  /// AppImage) download and verify a new release in the background; it is
  /// applied by «Перезапустить» (or, on Windows, when the app quits).
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
