// SPDX-License-Identifier: MPL-2.0
//
// QA-006: cold-start probe. With KINOLUNE_STARTUP_PROBE=1 (or `exit`) in
// the environment, or as a dart-define, the app prints once how long after
// the process started the first frame, and the first frame with library
// artwork in it (a poster, a card, the Home hero), finished rasterizing:
//
//   KINOLUNE_STARTUP first_frame_ms=412 library_visible_ms=655
//
// With `exit` it quits right after (tool/perf_bench.sh starts it in a
// loop). Linux and Android take the process start from /proc and the frame
// ends from the engine's frame timings (same monotonic clock); elsewhere
// the clock starts at main(). Off by default: nothing is registered.

import 'dart:async';
import 'dart:developer' show Timeline;
import 'dart:io';
import 'dart:ui' show FramePhase;

import 'package:flutter/scheduler.dart';
import 'package:material_ui/material_ui.dart';

import '../widgets/artwork.dart';

const _defined = String.fromEnvironment('KINOLUNE_STARTUP_PROBE');

/// Arms the probe when the environment asks for it; call before runApp.
void armStartupProbe() {
  // The environment (desktop) or a dart-define (Android, where an app gets
  // no environment from `am start`).
  final env = Platform.environment['KINOLUNE_STARTUP_PROBE'];
  final mode = env == null || env.isEmpty ? _defined : env;
  if (mode.isEmpty || mode == '0') return;
  _StartupProbe(_processStartMicros(), exitAfter: mode == 'exit');
}

class _StartupProbe {
  _StartupProbe(this.startUs, {required this.exitAfter}) {
    final b = SchedulerBinding.instance;
    b.addTimingsCallback(_onTimings);
    b.addPersistentFrameCallback((_) {
      if (_posterUs == null) b.addPostFrameCallback(_check);
    });
  }

  /// Process start on the monotonic clock of [Timeline.now] and the
  /// engine's frame timestamps.
  final int startUs;
  final bool exitAfter;

  int? _firstRasterUs;
  int? _posterUs;
  bool _done = false;
  Timer? _flush;
  final _timings = <FrameTiming>[];

  void _check(Duration _) {
    if (_posterUs != null || !_hasPoster()) return;
    _posterUs = Timeline.now;
    // The engine reports frame timings in batches, on later frames: keep
    // frames coming until the one with the poster is reported.
    _flush = Timer.periodic(
      const Duration(milliseconds: 100),
      (_) => SchedulerBinding.instance.scheduleFrame(),
    );
  }

  void _onTimings(List<FrameTiming> timings) {
    if (_done) return;
    _timings.addAll(timings);
    _firstRasterUs ??= timings.first.timestampInMicroseconds(
      FramePhase.rasterFinish,
    );
    final poster = _posterUs;
    if (poster == null) return;
    FrameTiming? frame;
    for (final t in _timings) {
      if (t.timestampInMicroseconds(FramePhase.buildStart) <= poster &&
          poster <= t.timestampInMicroseconds(FramePhase.rasterFinish)) {
        frame = t;
        break;
      }
    }
    final later = _timings.any(
      (t) => t.timestampInMicroseconds(FramePhase.buildStart) > poster,
    );
    if (frame == null && !later) return;
    _done = true;
    _flush?.cancel();
    SchedulerBinding.instance.removeTimingsCallback(_onTimings);
    final visibleUs =
        frame?.timestampInMicroseconds(FramePhase.rasterFinish) ?? poster;
    int ms(int us) => ((us - startUs) / 1000).round();
    stdout.writeln(
      'KINOLUNE_STARTUP first_frame_ms=${ms(_firstRasterUs!)} '
      'library_visible_ms=${ms(visibleUs)}',
    );
    if (exitAfter) unawaited(stdout.flush().whenComplete(() => exit(0)));
  }
}

/// True when library [Artwork] is in the tree: the Home hero, a card of a
/// row or the grid (on a phone the poster rows start below the fold).
bool _hasPoster() {
  var found = false;
  void visit(Element e) {
    if (found) return;
    if (e.widget is Artwork) {
      found = true;
      return;
    }
    e.visitChildren(visit);
  }

  WidgetsBinding.instance.rootElement?.visitChildren(visit);
  return found;
}

/// The process start in [Timeline.now] microseconds (monotonic clock); now
/// when it cannot be read.
int _processStartMicros() {
  final now = Timeline.now;
  try {
    if (Platform.isLinux || Platform.isAndroid) {
      // Field 22 of /proc/self/stat: the start in clock ticks after boot
      // (USER_HZ = 100 on Linux and Android). The command name (field 2)
      // may hold spaces: split after its closing parenthesis.
      final stat = File('/proc/self/stat').readAsStringSync();
      final fields = stat.substring(stat.lastIndexOf(')') + 2).split(' ');
      final startMs = int.parse(fields[22 - 3]) * 10;
      // /proc/uptime is the boot clock; shift onto the monotonic one.
      final uptimeMs =
          double.parse(
            File('/proc/uptime').readAsStringSync().split(' ').first,
          ) *
          1000;
      return now - ((uptimeMs - startMs) * 1000).round();
    }
  } catch (_) {
    // Not readable: count from here.
  }
  return now;
}
