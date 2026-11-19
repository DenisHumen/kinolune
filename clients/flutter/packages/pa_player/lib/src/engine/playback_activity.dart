// SPDX-License-Identifier: MPL-2.0
//
// Whether anything plays right now (PKG-024): silent app updates wait for
// it. Engines report themselves; the app only listens.

import 'package:flutter/foundation.dart';

/// Whether the app is busy playing: a track, a book or a stream plays, or
/// a video is open (even paused: restarting the app would close it).
/// [MediaKitEngine] reports itself; other engines may call [report] too.
abstract final class PlaybackActivity {
  static final Map<Object, (bool, bool)> _engines = {};
  static final ValueNotifier<bool> _busy = ValueNotifier(false);

  /// True while something plays or a video is open.
  static ValueListenable<bool> get busy => _busy;

  /// The state of [engine]: [playing], and [holding] a video that is open
  /// on the screen.
  static void report(
    Object engine, {
    required bool playing,
    bool holding = false,
  }) {
    final now = (playing, holding);
    if (_engines[engine] == now) return;
    _engines[engine] = now;
    _update();
  }

  /// [engine] is gone.
  static void forget(Object engine) {
    if (_engines.remove(engine) != null) _update();
  }

  static void _update() {
    _busy.value = _engines.values.any((e) => e.$1 || e.$2);
  }

  /// Forgets every engine (tests).
  @visibleForTesting
  static void reset() {
    _engines.clear();
    _busy.value = false;
  }
}
