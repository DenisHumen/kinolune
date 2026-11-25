// SPDX-License-Identifier: MPL-2.0
//
// BUG-001: a closed player frees hundreds of megabytes (libmpv, FFmpeg,
// their ~100 threads) that glibc keeps inside its heap; after a while the
// next player reuses them, but the process looks bigger than it is and the
// free space fragments. After the last player of a burst is gone, give the
// free pages back to the system (Linux, glibc `malloc_trim`).

import 'dart:async';
import 'dart:ffi';
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';

/// Returns freed native memory to the system some time after a player is
/// disposed: media_kit destroys the mpv handle 5 s after `dispose()`.
class NativeHeap {
  NativeHeap._();

  /// Delay after the last [afterPlayerDisposed] before trimming.
  static const delay = Duration(seconds: 7);

  static Timer? _timer;

  /// Trims made so far (tests, diagnostics).
  @visibleForTesting
  static int trims = 0;

  /// Replaces `malloc_trim` (tests).
  @visibleForTesting
  static void Function()? trimOverride;

  /// Schedules a trim; another dispose within [delay] postpones it.
  static void afterPlayerDisposed() {
    if (trimOverride == null && _trim == null) return;
    _timer?.cancel();
    _timer = Timer(delay, trimNow);
  }

  /// Gives free heap pages back to the system now.
  static void trimNow() {
    _timer?.cancel();
    _timer = null;
    final f = trimOverride ?? _trim;
    if (f == null) return;
    f();
    trims++;
  }

  static final void Function()? _trim = () {
    if (kIsWeb || !Platform.isLinux) return null;
    try {
      final fn = DynamicLibrary.process()
          .lookupFunction<Int32 Function(Size), int Function(int)>(
            'malloc_trim',
          );
      return () => fn(0);
    } catch (_) {
      // Not glibc (musl): nothing to trim.
      return null;
    }
  }();
}
