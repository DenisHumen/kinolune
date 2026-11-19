// SPDX-License-Identifier: MPL-2.0
//
// BUG-003 on real libmpv: MediaKitEngine sets `error` for a file that
// cannot be opened or is not media, clears it on the next open, and a
// playing file with the hardware decoder probe on (no GPU on the build
// server: mpv logs "Could not open codec" and falls back to software)
// has no error.
//
// Needs libmpv and a video: one made by ffmpeg (PA_TEST_FFMPEG) or any
// short video file (PA_TEST_MOVIE), so it is skipped unless
// PA_LIBMPV_TESTS=1 and one of them are set:
//   PA_LIBMPV_TESTS=1 PA_TEST_FFMPEG=<ffmpeg> (or PA_TEST_MOVIE=<file>) \
//     LIBMPV_LIBRARY_PATH=<dir>/libmpv.so.2 \
//     flutter test test/playback_error_libmpv_test.dart
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:media_kit/media_kit.dart' as mk;
import 'package:pa_player/pa_player.dart';

final _enabled = Platform.environment['PA_LIBMPV_TESTS'] == '1';
final _ffmpeg = Platform.environment['PA_TEST_FFMPEG'];
final _testMovie = Platform.environment['PA_TEST_MOVIE'];

/// 10 s of test pattern with a sine tone.
Future<String> _movie(Directory dir) async {
  final given = _testMovie;
  if (given != null && given.isNotEmpty) return given;
  final path = '${dir.path}/Error.Test.2026.mkv';
  final r = await Process.run(_ffmpeg!, [
    '-v',
    'error',
    '-y',
    '-f',
    'lavfi',
    '-i',
    'testsrc2=size=320x240:rate=24:duration=10',
    '-f',
    'lavfi',
    '-i',
    'sine=frequency=440:duration=10',
    '-c:v',
    'mpeg4',
    '-c:a',
    'aac',
    path,
  ]);
  if (r.exitCode != 0) fail('ffmpeg: ${r.stderr}');
  return path;
}

Future<void> _until(
  bool Function() done,
  String what, {
  Duration timeout = const Duration(seconds: 20),
}) async {
  final deadline = DateTime.now().add(timeout);
  while (!done()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out: $what');
    await Future<void>.delayed(const Duration(milliseconds: 50));
  }
}

MediaKitEngine _engine() => MediaKitEngine(
  // No VideoController (no texture in a test): decode to vo=null.
  video: false,
  // The hardware decoder probe as on a real device.
  mpvOptions: const {
    'vid': 'auto',
    'vo': 'null',
    'ao': 'null',
    'hwdec': 'auto-copy',
  },
);

void main() {
  final skip = _enabled && (_ffmpeg != null || _testMovie != null)
      ? false
      : 'set PA_LIBMPV_TESTS=1 and PA_TEST_FFMPEG or PA_TEST_MOVIE';
  late Directory tmp;

  setUpAll(() {
    if (skip != false) return;
    mk.MediaKit.ensureInitialized(
      libmpv: Platform.environment['LIBMPV_LIBRARY_PATH'],
    );
  });
  setUp(() async {
    if (skip != false) return;
    tmp = await Directory.systemTemp.createTemp('pa_bug003_');
  });
  tearDown(() async {
    if (skip != false) return;
    await tmp.delete(recursive: true);
  });

  test('missing file: error; then a good file: no error', () async {
    final e = _engine();
    final errors = <String>[];
    final sub = e.player.stream.log
        .where((l) => l.level == 'error')
        .listen((l) => errors.add('${l.prefix}: ${l.text}'));
    try {
      await e.open(['${tmp.path}/gone.mkv'], autoLoadSubtitles: false);
      await _until(() => e.state.value.error != null, 'error');
      expect(e.state.value.error!.code, PlaybackErrorCode.openFailed);

      await e.open([await _movie(tmp)], autoLoadSubtitles: false);
      expect(e.state.value.error, isNull);
      await _until(
        () => e.state.value.position > const Duration(seconds: 2),
        'play',
      );
      // The hwdec probe and anything else harmless did not stick.
      expect(e.state.value.error, isNull);
      // ignore: avoid_print
      print('mpv errors while playing: $errors');
    } finally {
      await sub.cancel();
      await e.dispose();
    }
  }, skip: skip);

  test('not a media file: error', () async {
    final e = _engine();
    try {
      final junk = File('${tmp.path}/junk.mkv')
        ..writeAsBytesSync(List.generate(64 * 1024, (i) => (i * 7919) & 0xff));
      await e.open([junk.path], autoLoadSubtitles: false);
      await _until(() => e.state.value.error != null, 'error');
    } finally {
      await e.dispose();
    }
  }, skip: skip);
}
