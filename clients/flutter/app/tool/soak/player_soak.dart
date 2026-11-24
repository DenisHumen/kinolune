// SPDX-License-Identifier: MPL-2.0
//
// BUG-001: memory soak of the player in the REAL app, buildable as a
// release (or profile) bundle — no test framework, no SMB, no network:
//
//   flutter build linux --release -t tool/soak/player_soak.dart
//   KINOLUNE_SOAK_DIR=/abs/films KINOLUNE_SOAK_CYCLES=200 \
//     build/linux/x64/release/bundle/kinolune
//
// The app starts as usual (Rust core, libmpv, the real services) on the
// profile its XDG_* dirs point at, adds KINOLUNE_SOAK_DIR as a local folder,
// scans it and then runs cycles of open → play 6 s → seek → play 4 s →
// close, cycling over the films found. After every cycle one line:
//
//   E2E SOAK cycle 12 ok <title> 14s rss=812MB threads=41 heap=… inuse=… free=…
//
// the same line the live e2e soak prints, so tools/qa/e2e/soak_memory.py
// gives the verdict (tools/qa/e2e/player_soak.sh runs all of it under Xvfb).
// The process exits when done: 0, or 1 when cycles failed.
//
// KINOLUNE_SOAK_MODE:
//   app     (default) what «Смотреть» does: playVideo with the library item
//           (trickplay, skip segments, progress, handoff, Up Next)
//   player  the bare PlayerScreen on the file, nothing else of the app
//   url     the bare PlayerScreen on KINOLUNE_SOAK_URL (comma-separated
//           URLs, e.g. a local HTTP server: the network path of mpv)
// KINOLUNE_SOAK_SETTLE_MS  wait after each close before sampling (2000)

import 'dart:async';
import 'dart:ffi' as ffi;
import 'dart:io';

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:kinolune/integration/library.dart'
    show LibraryItem, MediaSource, SourceKind;
import 'package:kinolune/main.dart' as app;
import 'package:kinolune/screens/playback.dart' show playVideo;
import 'package:kinolune/services/app_services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart'
    show EngineState, PlayRequest, PlayerScreen, PlayerScreenState;

final _env = Platform.environment;
final _dir = _env['KINOLUNE_SOAK_DIR'] ?? '';
final _cycles = int.tryParse(_env['KINOLUNE_SOAK_CYCLES'] ?? '') ?? 40;
final _mode = _env['KINOLUNE_SOAK_MODE'] ?? 'app';
final _urls = (_env['KINOLUNE_SOAK_URL'] ?? '')
    .split(',')
    .where((u) => u.isNotEmpty)
    .toList();
final _settle = Duration(
  milliseconds: int.tryParse(_env['KINOLUNE_SOAK_SETTLE_MS'] ?? '') ?? 2000,
);

void _log(String line) {
  // ignore: avoid_print
  print('E2E $line');
}

Future<void> main() async {
  unawaited(
    runZonedGuarded(_soak, (e, s) {
      _log('FAIL soak: $e\n$s');
      exit(2);
    }),
  );
}

Future<void> _soak() async {
  _log('NOTE soak start mode=$_mode cycles=$_cycles rss=${_rssMb()}MB');
  unawaited(app.main(const []));
  final nav = await _until(_rootNavigator, 'the app', secs: 120);
  final services = Services.read(nav.context);

  final targets = <({String title, LibraryItem? item, String uri})>[];
  if (_mode == 'url') {
    for (final u in _urls) {
      targets.add((title: u.split('/').last, item: null, uri: u));
    }
  } else {
    if (_dir.isEmpty) throw StateError('KINOLUNE_SOAK_DIR is not set');
    await services.addSource(
      MediaSource(
        id: '',
        kind: SourceKind.localFolder,
        name: 'soak',
        path: _dir,
      ),
    );
    await services.scan();
    await _until(() => services.scanning ? null : true, 'the scan', secs: 600);
    final movies = await services.library.movies().first;
    for (final m in movies) {
      targets.add((title: m.title, item: m, uri: m.playbackUri));
    }
  }
  if (targets.isEmpty) throw StateError('nothing to play');
  _log('NOTE soak targets: ${targets.map((t) => t.title).join(', ')}');

  var failed = 0;
  final sw = Stopwatch()..start();
  for (var i = 1; i <= _cycles; i++) {
    final t = targets[i % targets.length];
    final c = Stopwatch()..start();
    var ok = false;
    try {
      if (_mode == 'app') {
        unawaited(playVideo(nav.context, t.item!, fromStart: true));
      } else {
        unawaited(
          nav.push(
            MaterialPageRoute<void>(
              fullscreenDialog: true,
              builder: (_) => PlayerScreen(
                request: PlayRequest(uri: t.uri, title: t.title),
                onProgress: (_, _) {},
                engineFactory: services.playerEngineFactory,
              ),
            ),
          ),
        );
      }
      final player = await _until(_player, 'the player', secs: 30);
      final engine = player.engine;
      final a = await _advance(engine.state, const Duration(seconds: 6));
      final dur = engine.state.value.duration;
      final target = Duration(
        milliseconds: (dur.inMilliseconds * (0.2 + (i % 7) / 10)).round(),
      );
      await engine.seek(target);
      final b = await _advance(engine.state, const Duration(seconds: 4));
      ok = a > const Duration(seconds: 3) && b > const Duration(seconds: 2);
      nav.pop();
      await _until(() => _player() == null ? true : null, 'close', secs: 15);
    } catch (e) {
      _log('NOTE soak cycle $i: $e');
      if (_player() != null) nav.pop();
    }
    await Future<void>.delayed(_settle);
    if (!ok) failed++;
    _log(
      'SOAK cycle $i ${ok ? 'ok' : 'FAIL'} ${t.title.replaceAll(' ', '_')} '
      '${c.elapsed.inSeconds}s rss=${_rssMb()}MB ${_nativeMem()}',
    );
  }
  _log(
    'SOAK done cycles=$_cycles failed=$failed '
    'in ${sw.elapsed.inMinutes} min',
  );
  exit(failed == 0 ? 0 : 1);
}

/// How far the clock of [state] moved during [wall] of playback.
Future<Duration> _advance(
  ValueListenable<EngineState> state,
  Duration wall,
) async {
  final start = state.value.position;
  await Future<void>.delayed(wall);
  return state.value.position - start;
}

/// Polls [probe] every 200 ms until it gives a value.
Future<T> _until<T extends Object>(
  T? Function() probe,
  String what, {
  required int secs,
}) async {
  final deadline = DateTime.now().add(Duration(seconds: secs));
  while (true) {
    final v = probe();
    if (v != null) return v;
    if (DateTime.now().isAfter(deadline)) {
      throw TimeoutException('no $what in $secs s');
    }
    await Future<void>.delayed(const Duration(milliseconds: 200));
  }
}

T? _findState<T extends State>() {
  T? found;
  void visit(Element e) {
    if (found != null) return;
    if (e is StatefulElement && e.state is T) {
      found = e.state as T;
      return;
    }
    e.visitChildren(visit);
  }

  WidgetsBinding.instance.rootElement?.visitChildren(visit);
  return found;
}

NavigatorState? _rootNavigator() => _findState<NavigatorState>();

PlayerScreenState? _player() => _findState<PlayerScreenState>();

int _rssMb() => ProcessInfo.currentRss ~/ (1024 * 1024);

/// Threads of the process and glibc malloc (`mallinfo2`): a leak grows
/// «in use», fragmentation only «free» (BUG-001).
String _nativeMem() {
  final parts = <String>[];
  try {
    final status = File('/proc/self/status').readAsLinesSync();
    final threads = status.firstWhere((l) => l.startsWith('Threads:'));
    parts.add('threads=${threads.split(RegExp(r'\s+')).last}');
  } catch (_) {}
  final info = _mallinfo2;
  if (info != null) {
    final m = info();
    const mb = 1024 * 1024;
    parts
      ..add('heap=${(m.arena + m.hblkhd) ~/ mb}MB')
      ..add('inuse=${(m.uordblks + m.hblkhd) ~/ mb}MB')
      ..add('free=${m.fordblks ~/ mb}MB');
  }
  return parts.join(' ');
}

final class _MallInfo2 extends ffi.Struct {
  @ffi.Size()
  external int arena;
  @ffi.Size()
  external int ordblks;
  @ffi.Size()
  external int smblks;
  @ffi.Size()
  external int hblks;
  @ffi.Size()
  external int hblkhd;
  @ffi.Size()
  external int usmblks;
  @ffi.Size()
  external int fsmblks;
  @ffi.Size()
  external int uordblks;
  @ffi.Size()
  external int fordblks;
  @ffi.Size()
  external int keepcost;
}

final _MallInfo2 Function()? _mallinfo2 = () {
  if (!Platform.isLinux) return null;
  try {
    return ffi.DynamicLibrary.process()
        .lookupFunction<_MallInfo2 Function(), _MallInfo2 Function()>(
          'mallinfo2',
        );
  } catch (_) {
    return null;
  }
}();
