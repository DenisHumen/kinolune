// SPDX-License-Identifier: MPL-2.0
//
// TWI-006 "done when" on real libmpv: a Twitch-style low-latency live stream
// (made-up, local) plays through the live proxy of the Rust core with
// [liveMpvOptions]; the latency stays under 5 s, and the ad break is played
// in full: every ad segment reaches the player, the playhead moves through
// the ad at normal speed and the controller labels it as an ad.
//
// The stream: 48 s cut into 2 s MPEG-TS segments by ffmpeg (mpeg2video +
// mp2, no GPL encoder needed), seconds 16–22 are the "ad" (colour bars).
// `live_lab` (pa-twitch example, full flavor) serves them in real time as a
// fake origin with prefetch segments and a `twitch-stitched-ad` DATERANGE,
// behind the live proxy.
//
// Ground truth: segments are encoded with timestamps starting at 10 s and
// mpv runs with `rebase-start-time=no`, so the program date-time on screen
// is `t0 + (time-pos − 10 s)`, and latency = now − that.
//
// Skipped unless PA_LIBMPV_TESTS=1 and PA_TEST_FFMPEG and
// PA_TWITCH_LIVE_LAB are set:
//   cargo build -p pa-twitch --example live_lab \
//     --features testing,pa-flavor/full
//   PA_LIBMPV_TESTS=1 PA_TEST_FFMPEG=<ffmpeg> \
//     PA_TWITCH_LIVE_LAB=<target>/debug/examples/live_lab \
//     LIBMPV_LIBRARY_PATH=<dir>/libmpv.so.2 \
//     flutter test test/live_libmpv_test.dart
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:media_kit/media_kit.dart' as mk;
import 'package:pa_player/pa_player.dart';

final _enabled = Platform.environment['PA_LIBMPV_TESTS'] == '1';
final _ffmpeg = Platform.environment['PA_TEST_FFMPEG'];
final _lab = Platform.environment['PA_TWITCH_LIVE_LAB'];

const _segment = 2.0;
const _ptsOffset = 10.0;
const _adFrom = 16.0, _adTo = 22.0;
const _adSegments = [8, 9, 10];

Future<void> _makeSegments(Directory dir) async {
  final r = await Process.run(_ffmpeg!, [
    '-v', 'error', '-y', //
    '-f', 'lavfi', '-i', 'testsrc2=s=640x360:r=30:d=$_adFrom',
    '-f', 'lavfi', '-i', 'smptebars=s=640x360:r=30:d=${_adTo - _adFrom}',
    '-f', 'lavfi', '-i', 'testsrc2=s=640x360:r=30:d=${48 - _adTo}',
    '-f', 'lavfi', '-i', 'sine=f=440:d=48',
    '-filter_complex', '[0:v][1:v][2:v]concat=n=3:v=1:a=0[v]',
    '-map', '[v]', '-map', '3:a',
    '-c:v', 'mpeg2video', '-q:v', '4', '-bf', '0',
    '-g', '60', '-keyint_min', '60', '-sc_threshold', '0',
    '-c:a', 'mp2', '-b:a', '128k',
    '-output_ts_offset', '$_ptsOffset', '-muxdelay', '0', '-muxpreload', '0',
    '-f', 'segment', '-segment_time', '$_segment',
    '-segment_format', 'mpegts', '-reset_timestamps', '0',
    '${dir.path}/seg_%03d.ts',
  ]);
  if (r.exitCode != 0) fail('ffmpeg: ${r.stderr}');
}

class _Sample {
  _Sample(this.media, this.truth, this.estimate, this.rate, this.inAd);

  /// Media time on screen (seconds from the start of the stream).
  final double media;
  final double truth;
  final double? estimate;
  final double rate;
  final bool inAd;
}

/// Starts `live_lab` on [dir]; returns the process, its stdout lines and
/// the info line.
Future<(Process, Stream<String>, Map<String, Object?>)> _startLab(
  Directory dir,
) async {
  final lab = await Process.start(_lab!, [
    dir.path,
    '--segment-secs',
    '$_segment',
    '--ads',
    _adSegments.join(','),
  ]);
  final lines = lab.stdout
      .transform(utf8.decoder)
      .transform(const LineSplitter())
      .asBroadcastStream();
  unawaited(lab.stderr.transform(utf8.decoder).forEach(stderr.write));
  final info =
      jsonDecode(await lines.first.timeout(const Duration(seconds: 20)))
          as Map<String, Object?>;
  return (lab, lines, info);
}

MediaKitEngine _engine() => MediaKitEngine(
  video: false,
  mpvOptions: {
    ...liveMpvOptions(LiveLatencyMode.lowLatency),
    'vid': 'auto',
    'vo': 'null',
    'ao': 'null',
    'rebase-start-time': 'no',
  },
);

LivePlaybackController _controller(
  MediaKitEngine engine,
  Map<String, Object?> info,
) => LivePlaybackController(
  engine: engine,
  telemetry: HttpLiveTelemetry(
    stats: Uri.parse(info['stats']! as String),
    reset: Uri.parse(info['reset']! as String),
  ),
  url: info['proxy']! as String,
);

/// Real latency in seconds (see the file comment), or null before the
/// first frame.
Future<double?> _truth(MediaKitEngine engine, int t0) async {
  final mpv = engine.player.platform! as mk.NativePlayer;
  final pos = double.tryParse(await mpv.getProperty('time-pos'));
  if (pos == null) return null;
  final now = DateTime.now().millisecondsSinceEpoch;
  return (now - (t0 + (pos - _ptsOffset) * 1000)) / 1000;
}

double _median(List<double> v) {
  final s = [...v]..sort();
  return s[s.length ~/ 2];
}

void main() {
  setUpAll(() {
    if (_enabled) ensurePaPlayerInitialized();
  });

  test(
    'low-latency live on libmpv: < 5 s behind, ads played in full',
    () async {
      final dir = await Directory.systemTemp.createTemp('pa-live-');
      Process? lab;
      MediaKitEngine? engine;
      LivePlaybackController? live;
      try {
        await _makeSegments(dir);
        final (process, lines, info) = await _startLab(dir);
        lab = process;
        final t0 = info['t0_ms']! as int;
        final first = info['first_sequence']! as int;

        engine = _engine();
        live = _controller(engine, info);
        await live.start();
        final mpv = engine.player.platform! as mk.NativePlayer;

        final samples = <_Sample>[];
        final started = DateTime.now();
        while (DateTime.now().difference(started) <
            const Duration(seconds: 34)) {
          await Future<void>.delayed(const Duration(milliseconds: 500));
          final pos = double.tryParse(await mpv.getProperty('time-pos'));
          if (pos == null) continue;
          final now = DateTime.now().millisecondsSinceEpoch;
          final media = pos - _ptsOffset;
          final s = live.status.value;
          samples.add(
            _Sample(
              media,
              (now - (t0 + media * 1000)) / 1000,
              s.latency == null ? null : s.latency!.inMilliseconds / 1000,
              engine.state.value.rate,
              s.inAd,
            ),
          );
        }
        // ignore: avoid_print
        print(
          [
            for (final s in samples)
              '${s.media.toStringAsFixed(1)}: truth '
                  '${s.truth.toStringAsFixed(2)} est '
                  '${s.estimate?.toStringAsFixed(2)} rate ${s.rate}'
                  '${s.inAd ? ' AD' : ''}',
          ].join('\n'),
        );

        // Latency, after the first 4 s of start-up.
        final steady = samples.skip(8).toList();
        expect(steady.length, greaterThan(40));
        final truths = [for (final s in steady) s.truth];
        expect(
          truths.reduce((a, b) => a > b ? a : b),
          lessThan(5.0),
          reason: 'low-latency live must stay under 5 s',
        );
        // The latency shown in the OSD matches the real one.
        final errors = [
          for (final s in steady)
            if (s.estimate != null) (s.estimate! - s.truth).abs(),
        ];
        expect(errors.length, greaterThan(30));
        expect(_median(errors), lessThan(1.0));

        // The ad break was played through at normal speed and labelled.
        final inBreak = samples
            .where((s) => s.media > _adFrom + 0.5 && s.media < _adTo - 0.5)
            .toList();
        expect(
          inBreak.length,
          greaterThanOrEqualTo(8),
          reason: 'the playhead must run through the 6 s ad',
        );
        expect(inBreak.every((s) => s.inAd), isTrue);
        expect(inBreak.every((s) => (s.rate - 1.0).abs() < 1e-3), isTrue);
        expect(
          samples
              .where((s) => s.media < _adFrom - 1 || s.media > _adTo + 1)
              .any((s) => s.inAd),
          isFalse,
        );
        expect(samples.last.media, greaterThan(_adTo + 2));

        // Every ad segment reached the player in full.
        lab.stdin.writeln('served');
        final served =
            jsonDecode(await lines.first.timeout(const Duration(seconds: 5)))
                as List<Object?>;
        final adServed = {
          for (final s in served.cast<Map<String, Object?>>())
            if (s['ad'] == true) s['sequence'],
        };
        expect(adServed, {for (final k in _adSegments) first + k});
        final report = await HttpLiveTelemetry(
          stats: Uri.parse(info['stats']! as String),
          reset: Uri.parse(info['reset']! as String),
        ).report(cacheAhead: 0, rate: 1);
        expect(report!.adSegments, _adSegments.length);
      } finally {
        await live?.dispose();
        await engine?.dispose();
        lab?.kill();
        await dir.delete(recursive: true);
      }
    },
    skip: _enabled && _ffmpeg != null && _lab != null
        ? false
        : 'needs PA_LIBMPV_TESTS=1, PA_TEST_FFMPEG and PA_TWITCH_LIVE_LAB',
    timeout: const Timeout(Duration(minutes: 2)),
  );

  test(
    'go to live after a pause brings the latency back under 5 s',
    () async {
      final dir = await Directory.systemTemp.createTemp('pa-live-');
      Process? lab;
      MediaKitEngine? engine;
      LivePlaybackController? live;
      try {
        await _makeSegments(dir);
        final (process, _, info) = await _startLab(dir);
        lab = process;
        final t0 = info['t0_ms']! as int;
        engine = _engine();
        live = _controller(engine, info);
        await live.start();
        await Future<void>.delayed(const Duration(seconds: 4));
        await engine.pause();
        await Future<void>.delayed(const Duration(seconds: 7));
        await engine.play();
        await Future<void>.delayed(const Duration(seconds: 2));
        final behind = (await _truth(engine, t0))!;
        expect(behind, greaterThan(6.0), reason: 'paused for 7 s');
        // The controller sees it too, and offers to go live.
        await live.tick();
        expect(
          live.status.value.latency!.inMilliseconds / 1000,
          greaterThan(5),
        );
        expect(live.status.value.canGoLive, isTrue);
        expect(await live.goToLive(), isTrue);
        await Future<void>.delayed(const Duration(seconds: 5));
        final after = (await _truth(engine, t0))!;
        // ignore: avoid_print
        print('behind $behind s, after go-to-live $after s');
        expect(after, lessThan(5.0));
      } finally {
        await live?.dispose();
        await engine?.dispose();
        lab?.kill();
        await dir.delete(recursive: true);
      }
    },
    skip: _enabled && _ffmpeg != null && _lab != null
        ? false
        : 'needs PA_LIBMPV_TESTS=1, PA_TEST_FFMPEG and PA_TWITCH_LIVE_LAB',
    timeout: const Timeout(Duration(minutes: 2)),
  );
}
