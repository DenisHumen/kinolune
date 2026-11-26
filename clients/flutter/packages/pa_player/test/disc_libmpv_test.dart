// SPDX-License-Identifier: MPL-2.0
//
// PLAY-017 on real libmpv: a Blu-ray title is clips (`.m2ts`) played back
// to back through an EDL; the clips do not carry the languages of their
// streams (the disc's playlist does), so the app hands them to the engine
// by PID (`src-id`), and every audio track shows its language. Another
// angle is another list of clips: opening it at the current position
// shows that angle's picture.
//
// Each clip is 3 s of one colour with three AC-3 tracks on the PIDs of a
// Blu-ray (video 0x1011, audio 0x1100–0x1102), without languages. Angle 1
// is red then green, angle 2 red then blue.
//
// Needs libmpv and an ffmpeg to make the media, so it is skipped unless
// PA_LIBMPV_TESTS=1 and PA_TEST_FFMPEG are set:
//   PA_LIBMPV_TESTS=1 PA_TEST_FFMPEG=<ffmpeg> \
//     LIBMPV_LIBRARY_PATH=<dir>/libmpv.so.2 \
//     flutter test test/disc_libmpv_test.dart
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';

final _enabled = Platform.environment['PA_LIBMPV_TESTS'] == '1';
final _ffmpeg = Platform.environment['PA_TEST_FFMPEG'];

Future<String> _clip(Directory dir, String name, String colour) async {
  final path = '${dir.path}/$name.m2ts';
  final r = await Process.run(_ffmpeg!, [
    '-v', 'error', '-y', //
    '-f', 'lavfi', '-i', 'color=c=$colour:s=160x120:r=10:d=3',
    for (final f in [440, 550, 660]) ...['-f', 'lavfi', '-i', 'sine=f=$f:d=3'],
    '-map', '0:v', '-map', '1:a', '-map', '2:a', '-map', '3:a', //
    '-c:v', 'mpeg2video', '-q:v', '3', '-g', '10', //
    '-c:a', 'ac3', '-b:a', '96k', //
    '-streamid', '0:0x1011', '-streamid', '1:0x1100', //
    '-streamid', '2:0x1101', '-streamid', '3:0x1102', //
    '-mpegts_m2ts_mode', '1', '-f', 'mpegts', path,
  ]);
  if (r.exitCode != 0) fail('ffmpeg $name: ${r.stderr}');
  return path;
}

/// Colour of the centre pixel (BGR0): 0 red, 1 green, 2 blue, -1 other.
int _colourOf(Uint8List raw, int width, int height) {
  final stride = raw.length ~/ height;
  final i = (height ~/ 2) * stride + (width ~/ 2) * 4;
  final (b, g, r) = (raw[i], raw[i + 1], raw[i + 2]);
  bool hi(int v) => v > 180;
  bool lo(int v) => v < 80;
  if (hi(r) && lo(g) && lo(b)) return 0;
  if (lo(r) && hi(g) && lo(b)) return 1;
  if (lo(r) && lo(g) && hi(b)) return 2;
  return -1;
}

Future<void> _until(bool Function() done, String what) async {
  final deadline = DateTime.now().add(const Duration(seconds: 20));
  while (!done()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out: $what');
    await Future<void>.delayed(const Duration(milliseconds: 50));
  }
}

/// The colour shown at [at] after a seek.
Future<int> _colourAt(MediaKitEngine engine, Duration at) async {
  await engine.seek(at);
  final s = engine.state;
  var got = -1;
  final deadline = DateTime.now().add(const Duration(seconds: 10));
  while (DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 150));
    final raw = await engine.player.screenshot(format: null);
    if (raw == null || raw.isEmpty) continue;
    got = _colourOf(raw, s.value.videoWidth!, s.value.videoHeight!);
    if (got >= 0) break;
  }
  return got;
}

void main() {
  late Directory tmp;

  setUpAll(() {
    if (!_enabled) return;
    ensurePaPlayerInitialized();
  });
  setUp(() => tmp = Directory.systemTemp.createTempSync('pa_disc_'));
  tearDown(() => tmp.deleteSync(recursive: true));

  final skip = _enabled && _ffmpeg != null
      ? null
      : 'needs libmpv and ffmpeg: PA_LIBMPV_TESTS=1 PA_TEST_FFMPEG=<ffmpeg> '
            '(see the file header)';

  test(
    'a disc title: one timeline, languages by PID, angles',
    () async {
      final red = await _clip(tmp, '00001', 'red');
      final green = await _clip(tmp, '00002', 'lime');
      final blue = await _clip(tmp, '00003', 'blue');
      final angle1 = partsUri([red, green]);
      final angle2 = partsUri([red, blue]);

      final engine = MediaKitEngine(
        video: false,
        mpvOptions: const {
          'vid': 'auto',
          'vo': 'null',
          'ao': 'null',
          'hr-seek': 'yes',
        },
      );
      try {
        engine.setStreamLanguages(const {
          0x1100: 'eng',
          0x1101: 'rus',
          0x1102: 'ukr',
        });
        await engine.open([angle1], play: false, autoLoadSubtitles: false);
        final state = engine.state;
        await _until(
          () =>
              state.value.error != null ||
              state.value.duration > Duration.zero &&
                  (state.value.videoWidth ?? 0) > 0 &&
                  state.value.audioTracks.length == 3,
          'open angle 1',
        );
        expect(state.value.error, isNull);
        expect(
          state.value.duration.inMilliseconds / 1000,
          closeTo(6, 0.6),
          reason: 'two clips as one title',
        );
        // The engine reads `track-list` after the tracks arrive.
        await _until(
          () => state.value.audioTracks.every((t) => t.language != null),
          'languages of the tracks',
        );
        expect(
          [for (final t in state.value.audioTracks) t.language],
          ['eng', 'rus', 'ukr'],
        );
        expect(
          await _colourAt(engine, const Duration(milliseconds: 4500)),
          1,
          reason: 'angle 1 is green in its second clip',
        );

        // Another angle: the same title from the same place.
        await engine.open(
          [angle2],
          start: const Duration(milliseconds: 4500),
          play: false,
          autoLoadSubtitles: false,
        );
        await _until(() => state.value.audioTracks.length == 3, 'open angle 2');
        expect(
          await _colourAt(engine, const Duration(milliseconds: 4500)),
          2,
          reason: 'angle 2 is blue in its second clip',
        );
        expect(
          await _colourAt(engine, const Duration(milliseconds: 1500)),
          0,
          reason: 'both angles share the first clip',
        );
      } finally {
        await engine.dispose();
      }
    },
    skip: skip,
    timeout: const Timeout(Duration(minutes: 2)),
  );
}
