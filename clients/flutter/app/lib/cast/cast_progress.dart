// SPDX-License-Identifier: MPL-2.0
//
// PLAY-012 (ADR-0153): the receiver's position goes into the same progress
// path as the app's own player (PLAY-005, SYNC-014): the library save,
// the handoff heartbeat (SYNC-003) and the media server's report
// (NET-013/014). «Продолжить» and the sync to other devices then work the
// same whether the title played here or on the TV.

import 'dart:async';

import 'package:flutter/foundation.dart';

/// The receiver's player state (the Cast `MediaStatus` and AVPlayer's
/// `timeControlStatus` both map to it).
enum CastPlayerState { idle, loading, buffering, playing, paused }

/// Why a receiver went idle.
enum CastIdleReason { none, finished, cancelled, interrupted, error }

/// One status of the receiver.
@immutable
class CastStatus {
  const CastStatus({
    required this.state,
    this.position = Duration.zero,
    this.duration = Duration.zero,
    this.idleReason = CastIdleReason.none,
  });

  /// From the platform channel map (`state`, `positionMs`, `durationMs`,
  /// `idleReason`); unknown values count as idle / none.
  factory CastStatus.fromMap(Map<Object?, Object?> m) {
    T pick<T extends Enum>(List<T> values, Object? name, T fallback) =>
        values.where((v) => v.name == name).firstOrNull ?? fallback;
    int ms(Object? v) => v is num && v.isFinite && v > 0 ? v.toInt() : 0;
    return CastStatus(
      state: pick(CastPlayerState.values, m['state'], CastPlayerState.idle),
      position: Duration(milliseconds: ms(m['positionMs'])),
      duration: Duration(milliseconds: ms(m['durationMs'])),
      idleReason: pick(
        CastIdleReason.values,
        m['idleReason'],
        CastIdleReason.none,
      ),
    );
  }

  final CastPlayerState state;
  final Duration position;
  final Duration duration;
  final CastIdleReason idleReason;

  bool get finished =>
      state == CastPlayerState.idle && idleReason == CastIdleReason.finished;

  /// The receiver stopped playing this item for good.
  bool get ended =>
      state == CastPlayerState.idle &&
      (idleReason == CastIdleReason.finished ||
          idleReason == CastIdleReason.cancelled ||
          idleReason == CastIdleReason.error ||
          idleReason == CastIdleReason.interrupted);

  @override
  bool operator ==(Object other) =>
      other is CastStatus &&
      other.state == state &&
      other.position == position &&
      other.duration == duration &&
      other.idleReason == idleReason;

  @override
  int get hashCode => Object.hash(state, position, duration, idleReason);

  @override
  String toString() =>
      'CastStatus($state ${position.inMilliseconds}/'
      '${duration.inMilliseconds} $idleReason)';
}

/// The progress save for [status], as `(positionMs, durationMs)`, or null
/// when it says nothing new about the position:
///
/// - finished: the end (`position = duration`), so the title counts as
///   watched and Up Next/«Продолжить» move on;
/// - loading, or buffering at 0 before the first frame: the receiver has
///   not reached the start position yet, and saving 0 would lose the
///   resume point;
/// - any other idle (stopped, error): nothing — the last save stays;
/// - otherwise the position. A receiver without a duration (some HLS
///   transcodes) gets the library's [knownDurationMs].
(int, int)? castProgressOf(
  CastStatus status, {
  required int knownDurationMs,
  required bool started,
}) {
  final dur = status.duration > Duration.zero
      ? status.duration.inMilliseconds
      : knownDurationMs;
  if (status.finished) return dur > 0 ? (dur, dur) : null;
  switch (status.state) {
    case CastPlayerState.idle || CastPlayerState.loading:
      return null;
    case CastPlayerState.buffering when !started:
      return null;
    case _:
      if (dur <= 0) return null;
      final pos = status.position.inMilliseconds.clamp(0, dur);
      return (pos, dur);
  }
}

/// Polls a receiver while a cast session plays and saves its position:
/// every [interval] while it changes, at once on pause and at the end.
/// [onEnded] runs once when the receiver stops this item (finished, stopped
/// on the TV, error) or [stop] is called.
class CastProgressSync {
  CastProgressSync({
    required this.poll,
    required this.onProgress,
    required this.onEnded,
    required this.knownDurationMs,
    this.interval = const Duration(seconds: 1),
    this.onStatus,
  });

  final Future<CastStatus?> Function() poll;
  final void Function(int positionMs, int durationMs) onProgress;
  final VoidCallback onEnded;
  final ValueChanged<CastStatus>? onStatus;
  final int knownDurationMs;
  final Duration interval;

  Timer? _timer;
  bool _busy = false;
  bool _started = false;
  bool _done = false;
  (int, int)? _last;

  /// The last position saved.
  (int, int)? get last => _last;

  void start() {
    _timer ??= Timer.periodic(interval, (_) => tick());
  }

  /// One poll (the timer's; tests call it directly).
  Future<void> tick() async {
    if (_busy || _done) return;
    _busy = true;
    try {
      final status = await poll();
      if (_done) return;
      if (status == null) return;
      onStatus?.call(status);
      if (status.state == CastPlayerState.playing ||
          status.state == CastPlayerState.paused) {
        _started = true;
      }
      final p = castProgressOf(
        status,
        knownDurationMs: knownDurationMs,
        started: _started,
      );
      if (p != null && p != _last) {
        _last = p;
        onProgress(p.$1, p.$2);
      }
      // An idle receiver before the first frame is still loading the
      // item (the Default Media Receiver reports idle in between).
      if (status.ended && (_started || status.finished)) _end();
    } catch (e) {
      debugPrint('Cast status not read: $e');
    } finally {
      _busy = false;
    }
  }

  /// The session ended here (the user stopped casting, the device left).
  void stop() => _end();

  void _end() {
    if (_done) return;
    _done = true;
    _timer?.cancel();
    _timer = null;
    onEnded();
  }
}
