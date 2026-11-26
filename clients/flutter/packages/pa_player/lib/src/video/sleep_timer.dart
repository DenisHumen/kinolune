// SPDX-License-Identifier: MPL-2.0
import 'dart:async';

import 'package:flutter/foundation.dart';

/// The choices of the sleep timer (PLAY-022).
enum SleepTimerOption {
  off,

  /// Pauses at the end of the playing episode or movie; Up Next does not
  /// start. Not offered on Twitch (live or VOD).
  endOfItem,
  minutes15,
  minutes30,
  minutes60,
  minutes90;

  /// How long a timed choice runs; null for [off] and [endOfItem].
  Duration? get duration => switch (this) {
    minutes15 => const Duration(minutes: 15),
    minutes30 => const Duration(minutes: 30),
    minutes60 => const Duration(minutes: 60),
    minutes90 => const Duration(minutes: 90),
    off || endOfItem => null,
  };
}

/// The player the sleep timer acts on: the video player (also the native
/// Twitch player) or the Twitch embed.
abstract interface class SleepTimerHost {
  /// The timer ran out or nobody answered «Вы ещё смотрите?»: pause, save
  /// the progress, and do not start Up Next.
  void sleepPause();

  /// The fade before the pause: [factor] of the volume the user set, from
  /// 1 down to 0; 1 again puts that volume back.
  void sleepFade(double factor);
}

/// The sleep timer and «Вы ещё смотрите?» (PLAY-022): one controller for
/// the app, shared by every player, so its state is never kept twice.
///
/// A timed choice counts wall time from when it was set; «в конце серии»
/// follows the playing item, which the host reports with [itemProgress]
/// and [itemEnded]. In the last [fade] before the pause the volume fades
/// to zero ([volume], pushed to the host) and [warning] asks the player to
/// show «Пауза через 30 с — Продлить»; [extend] adds [extension]. At the
/// deadline the host pauses ([SleepTimerHost.sleepPause]) and the timer
/// turns off.
///
/// After [stillWatchingAfter] automatic Up Next transitions in a row with
/// no input ([autoAdvanced], [userInput]) the controller [asking] «Вы ещё
/// смотрите?»; without an answer it pauses after [stillWatchingTimeout].
/// 0 turns the question off; a change goes to [onStillWatchingAfterChanged]
/// (the app stores it).
class SleepTimerController extends ChangeNotifier {
  SleepTimerController({
    DateTime Function()? clock,
    int stillWatchingAfter = kDefaultStillWatchingAfter,
    this.onStillWatchingAfterChanged,
    this.fade = const Duration(seconds: 30),
    this.extension = const Duration(minutes: 15),
    this.stillWatchingTimeout = const Duration(seconds: 60),
    this.tick = const Duration(milliseconds: 250),
  }) : _clock = clock ?? DateTime.now,
       _stillWatchingAfter = stillWatchingAfter.clamp(
         0,
         kMaxStillWatchingAfter,
       );

  /// Episodes in a row before «Вы ещё смотрите?» by default.
  static const kDefaultStillWatchingAfter = 3;

  /// The largest value offered.
  static const kMaxStillWatchingAfter = 10;

  final DateTime Function() _clock;
  final Duration fade;
  final Duration extension;
  final Duration stillWatchingTimeout;

  /// How often the timer looks at the clock (the fade's steps).
  final Duration tick;
  final ValueChanged<int>? onStillWatchingAfterChanged;

  SleepTimerHost? _host;
  SleepTimerOption _option = SleepTimerOption.off;
  DateTime? _deadline;
  Duration? _itemLeft;
  bool _itemPlaying = false;
  double _volume = 1.0;
  int _stillWatchingAfter;
  int _autoInRow = 0;
  DateTime? _askDeadline;
  Timer? _ticker;

  /// What was chosen; [SleepTimerOption.off] when the timer is not set.
  SleepTimerOption get option => _option;
  bool get active => _option != SleepTimerOption.off;

  /// Time to the pause: to the deadline of a timed choice, or to the end
  /// of the item with «в конце серии» (null until the host reports it).
  Duration? get remaining {
    switch (_option) {
      case SleepTimerOption.off:
        return null;
      case SleepTimerOption.endOfItem:
        return _itemLeft;
      default:
        final left = _deadline!.difference(_clock());
        return left.isNegative ? Duration.zero : left;
    }
  }

  /// Within [fade] of the pause: the player shows «Пауза через N с —
  /// Продлить». «В конце серии» only while the item plays.
  bool get warning {
    final left = remaining;
    if (left == null || left > fade) return false;
    return _option != SleepTimerOption.endOfItem || _itemPlaying;
  }

  /// The share of the user's volume now: 1, falling to 0 over [fade].
  double get volume => _volume;

  /// The host must not start Up Next by itself: the timer pauses at the
  /// end of this item.
  bool get holdsUpNext => _option == SleepTimerOption.endOfItem;

  /// Episodes in a row before «Вы ещё смотрите?»; 0: never asks.
  int get stillWatchingAfter => _stillWatchingAfter;
  set stillWatchingAfter(int value) {
    final v = value.clamp(0, kMaxStillWatchingAfter);
    if (v == _stillWatchingAfter) return;
    _stillWatchingAfter = v;
    if (v == 0) _stopAsking();
    onStillWatchingAfterChanged?.call(v);
    notifyListeners();
  }

  /// Sets the value read from the settings (no [onStillWatchingAfterChanged]).
  void restoreStillWatchingAfter(int value) {
    _stillWatchingAfter = value.clamp(0, kMaxStillWatchingAfter);
    notifyListeners();
  }

  /// Automatic transitions in a row with no input.
  int get autoplaysInRow => _autoInRow;

  /// «Вы ещё смотрите?» is on screen.
  bool get asking => _askDeadline != null;

  /// Time to the pause while [asking].
  Duration? get askRemaining {
    final at = _askDeadline;
    if (at == null) return null;
    final left = at.difference(_clock());
    return left.isNegative ? Duration.zero : left;
  }

  // ---- host ----------------------------------------------------------------

  /// The player on screen; it gets the pause and the fade. A timer set
  /// before (a Twitch player switching streams) goes on.
  void attach(SleepTimerHost host) {
    _host = host;
    if (_volume < 1.0) host.sleepFade(_volume);
  }

  void detach(SleepTimerHost host) {
    if (identical(_host, host)) _host = null;
  }

  // ---- choices ---------------------------------------------------------------

  /// Sets the timer; [SleepTimerOption.off] cancels it and puts the volume
  /// back.
  void set(SleepTimerOption option) {
    _option = option;
    final d = option.duration;
    _deadline = d == null ? null : _clock().add(d);
    if (option != SleepTimerOption.endOfItem) _itemLeft = null;
    _setVolume(1.0);
    _sync();
    notifyListeners();
  }

  void cancel() => set(SleepTimerOption.off);

  /// «Продлить»: [extension] more from the deadline (at least from now);
  /// «в конце серии» becomes a timer of [extension] from now. The volume
  /// comes back.
  void extend() {
    if (!active) return;
    final now = _clock();
    if (_option == SleepTimerOption.endOfItem) {
      _option = SleepTimerOption.minutes15;
      _deadline = now.add(extension);
      _itemLeft = null;
    } else {
      final from = _deadline!.isAfter(now) ? _deadline! : now;
      _deadline = from.add(extension);
    }
    _setVolume(1.0);
    _sync();
    notifyListeners();
  }

  /// The host's item: [left] to its end at the playing speed (null: not
  /// known, a live stream) and whether it plays. Drives «в конце серии».
  void itemProgress(Duration? left, {required bool playing}) {
    _itemPlaying = playing;
    if (_option != SleepTimerOption.endOfItem) return;
    final was = _itemLeft;
    _itemLeft = left;
    _applyFade();
    if (was?.inSeconds != left?.inSeconds) notifyListeners();
  }

  /// The host's item reached its end. True when the timer takes it («в
  /// конце серии»): the host has been paused and must not start Up Next.
  bool itemEnded() {
    if (_option != SleepTimerOption.endOfItem) return false;
    _fire();
    return true;
  }

  // ---- «Вы ещё смотрите?» --------------------------------------------------

  /// A key, a click, a tap or a remote button in the player: the count of
  /// automatic episodes starts again and the question is answered.
  void userInput() {
    _autoInRow = 0;
    if (asking) {
      _stopAsking();
      notifyListeners();
    }
  }

  /// «Да, смотрю».
  void stillWatching() => userInput();

  /// Up Next went on to the next item by itself (the countdown ran out, the
  /// credits were skipped). After [stillWatchingAfter] in a row it asks.
  void autoAdvanced() {
    if (_stillWatchingAfter == 0) return;
    _autoInRow++;
    if (_autoInRow < _stillWatchingAfter || asking) return;
    _askDeadline = _clock().add(stillWatchingTimeout);
    _sync();
    notifyListeners();
  }

  // ---- ticking ---------------------------------------------------------------

  void _sync() {
    final timed = _option.duration != null;
    if (timed || asking) {
      _ticker ??= Timer.periodic(tick, (_) => _onTick());
    } else {
      _ticker?.cancel();
      _ticker = null;
    }
  }

  int? _lastSecond;

  void _onTick() {
    final ask = _askDeadline;
    if (ask != null && !_clock().isBefore(ask)) {
      _stopAsking();
      _autoInRow = 0;
      _host?.sleepPause();
      notifyListeners();
    }
    if (_option.duration != null) {
      final left = remaining!;
      if (left <= Duration.zero) {
        _fire();
        return;
      }
      _applyFade();
    }
    final second = (remaining ?? askRemaining)?.inSeconds;
    if (second != _lastSecond) {
      _lastSecond = second;
      notifyListeners();
    }
  }

  void _applyFade() {
    final left = remaining;
    if (!warning || left == null) {
      _setVolume(1.0);
      return;
    }
    _setVolume(
      (left.inMilliseconds / fade.inMilliseconds).clamp(0.0, 1.0).toDouble(),
    );
  }

  void _setVolume(double v) {
    if (v == _volume) return;
    _volume = v;
    _host?.sleepFade(v);
  }

  /// The deadline: pause, then the volume back for the next play.
  void _fire() {
    _host?.sleepPause();
    _option = SleepTimerOption.off;
    _deadline = null;
    _itemLeft = null;
    _setVolume(1.0);
    _sync();
    notifyListeners();
  }

  void _stopAsking() {
    _askDeadline = null;
    _sync();
  }

  @override
  void dispose() {
    _ticker?.cancel();
    _ticker = null;
    super.dispose();
  }
}
