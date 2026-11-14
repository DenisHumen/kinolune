// SPDX-License-Identifier: MPL-2.0
//
// Wrong-PIN backoff (UI-015). The core counts and enforces it for every
// PIN check (ADR-0055 amendment, kept in the library database); the app's
// limiter mirrors that state ([restore]) so every PIN pad shows the same
// wait. One limiter for every PIN prompt of the app, so switching between
// prompts (Settings, another profile, App Lock) does not give fresh
// attempts. The counting rules here ([fail], [succeed], Preferences) are
// the same as the core's; the in-memory profiles of the demo use them.

import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../integration/library.dart' show PinAttempts;
import '../services/preferences.dart';

class PinAttemptLimiter extends ChangeNotifier {
  PinAttemptLimiter({
    Preferences? preferences,
    DateTime Function()? clock,
    this.freeAttempts = 5,
    this.baseDelay = const Duration(seconds: 30),
    this.maxDelay = const Duration(hours: 1),
  }) : _prefs = preferences,
       _clock = clock ?? DateTime.now;

  final Preferences? _prefs;
  final DateTime Function() _clock;

  /// Wrong PINs in a row before the first wait.
  final int freeAttempts;

  /// The first wait; every further wrong PIN doubles it up to [maxDelay].
  final Duration baseDelay;
  final Duration maxDelay;

  int _failures = 0;
  DateTime? _lockedUntil;

  /// Wrong PINs since the last right one.
  int get failures => _failures;

  /// Until when no PIN is checked; null when attempts are allowed.
  DateTime? get lockedUntil {
    final until = _lockedUntil;
    if (until == null || !_clock().isBefore(until)) return null;
    return until;
  }

  bool get locked => lockedUntil != null;

  /// Time left until the next attempt; zero when allowed now.
  Duration get remaining {
    final until = lockedUntil;
    return until == null ? Duration.zero : until.difference(_clock());
  }

  /// Wrong PINs left before a wait (0 while waiting).
  int get attemptsBeforeWait =>
      locked ? 0 : (freeAttempts - _failures).clamp(0, freeAttempts);

  /// The wait after the [failures]-th wrong PIN in a row.
  Duration delayAfter(int failures) {
    if (failures < freeAttempts) return Duration.zero;
    final doublings = failures - freeAttempts;
    // 2^20 × 30 s is far beyond any sane cap; stop doubling early.
    final factor = 1 << doublings.clamp(0, 20);
    final delay = baseDelay * factor;
    return delay > maxDelay ? maxDelay : delay;
  }

  /// Reads the saved state.
  Future<void> load() async {
    final raw = await _prefs?.read(Preferences.profilesPinAttempts);
    if (raw == null) return;
    try {
      final data = jsonDecode(raw);
      if (data is! Map) return;
      final failures = data['failures'];
      final until = data['until'];
      _failures = failures is int && failures > 0 ? failures : 0;
      _lockedUntil = until is int
          ? DateTime.fromMillisecondsSinceEpoch(until)
          : null;
      // A clock set back must not make the wait longer than the cap.
      final max = _clock().add(maxDelay);
      if (_lockedUntil != null && _lockedUntil!.isAfter(max)) {
        _lockedUntil = max;
      }
    } on FormatException {
      _failures = 0;
      _lockedUntil = null;
    }
    notifyListeners();
  }

  /// Takes the state of the core's backoff. [PinAttempts.lockedUntil] is
  /// wall-clock time; the wait left is kept on this limiter's clock.
  void restore(PinAttempts attempts) {
    _failures = attempts.failures < 0 ? 0 : attempts.failures;
    final until = attempts.lockedUntil;
    if (until == null) {
      _lockedUntil = null;
    } else {
      var left = until.difference(DateTime.now());
      if (left > maxDelay) left = maxDelay;
      _lockedUntil = left <= Duration.zero ? null : _clock().add(left);
    }
    notifyListeners();
  }

  /// The state as the library reports it ([restore] reads it back).
  PinAttempts get attempts => PinAttempts(
    failures: _failures,
    lockedUntil: switch (lockedUntil) {
      final until? => DateTime.now().add(until.difference(_clock())),
      null => null,
    },
  );

  /// A wrong PIN: counts it and starts the wait it earns.
  Future<void> fail() async {
    _failures++;
    final delay = delayAfter(_failures);
    _lockedUntil = delay == Duration.zero ? null : _clock().add(delay);
    notifyListeners();
    await _save();
  }

  /// A right PIN: the count starts over.
  Future<void> succeed() async {
    if (_failures == 0 && _lockedUntil == null) return;
    _failures = 0;
    _lockedUntil = null;
    notifyListeners();
    await _save();
  }

  Future<void> _save() async => _prefs?.write(
    Preferences.profilesPinAttempts,
    _failures == 0
        ? null
        : jsonEncode({
            'failures': _failures,
            if (_lockedUntil != null)
              'until': _lockedUntil!.millisecondsSinceEpoch,
          }),
  );
}
