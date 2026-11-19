// SPDX-License-Identifier: MPL-2.0
import 'dart:async';

import '../engine/player_engine.dart';

typedef ProgressCallback = void Function(int positionMs, int durationMs);

/// Calls [onProgress] every [interval] while playing, and on pause, seek and
/// exit. SYNC-014 (ADR-0137): every second, so a kill or a power cut loses
/// at most a second and another device continues from the same second.
/// Ticks and seeks go out at most once per [minGap] (a seek right after a
/// report waits for the end of the gap, with the position of that moment);
/// pause, exit and going to the background ([report]) go out at once.
class ProgressReporter {
  ProgressReporter({
    required this.engine,
    required this.onProgress,
    this.interval = const Duration(seconds: 1),
    this.minGap = const Duration(seconds: 1),
  }) {
    _wasPlaying = engine.state.value.playing;
    engine.state.addListener(_onState);
    _timer = Timer.periodic(interval, (_) {
      if (engine.state.value.playing) _paced();
    });
  }

  final PlayerEngine engine;
  final ProgressCallback onProgress;
  final Duration interval;
  final Duration minGap;
  Timer? _timer;
  late bool _wasPlaying;
  int? _lastPos;
  int? _lastDur;

  /// Running while the last report is younger than [minGap].
  Timer? _gap;

  /// A paced report waits for the end of [_gap].
  bool _waiting = false;
  bool _closed = false;

  void _onState() {
    final playing = engine.state.value.playing;
    if (_wasPlaying && !playing) report();
    _wasPlaying = playing;
  }

  /// A tick or a seek: now, or at the end of the gap after the last report.
  void _paced() {
    if (_gap != null) {
      _waiting = true;
      return;
    }
    _send();
  }

  /// Reports the current position now (no-op until the duration is known).
  void report({Duration? position}) => _send(position: position);

  void _send({Duration? position}) {
    final s = engine.state.value;
    final dur = s.duration.inMilliseconds;
    if (dur <= 0) return;
    final pos = (position ?? s.position).inMilliseconds.clamp(0, dur);
    _waiting = false;
    if (pos == _lastPos && dur == _lastDur) return;
    _lastPos = pos;
    _lastDur = dur;
    onProgress(pos, dur);
    if (_closed) return;
    _gap?.cancel();
    _gap = Timer(minGap, () {
      _gap = null;
      if (_waiting && !_closed) _send();
    });
  }

  /// Call after a seek with the target position.
  void onSeek(Duration target) {
    if (_gap == null) {
      _send(position: target);
    } else {
      _waiting = true;
    }
  }

  /// Final report on exit; stops the timers.
  void close() {
    _closed = true;
    _timer?.cancel();
    _timer = null;
    _gap?.cancel();
    _gap = null;
    engine.state.removeListener(_onState);
    report();
  }
}
