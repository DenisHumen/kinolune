// SPDX-License-Identifier: MPL-2.0
//
// PLAY-010 "done when" on real libmpv: MediaKitEngine fills the one stats
// model (the contract of every engine, `statsContractProblems`), from a
// local file and over HTTP like the loopback stream proxy of shares; and
// the adaptive buffer grows the read-ahead when the source is slower than
// the stream (a throttled server, like SMB over a weak Wi-Fi).
//
// Needs libmpv and an ffmpeg to make the media, so it is skipped unless
// PA_LIBMPV_TESTS=1 and PA_TEST_FFMPEG are set:
//   PA_LIBMPV_TESTS=1 PA_TEST_FFMPEG=<ffmpeg> \
//     LIBMPV_LIBRARY_PATH=<dir>/libmpv.so.2 \
//     flutter test test/stats_libmpv_test.dart
import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:media_kit/media_kit.dart' as mk;
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

final _enabled = Platform.environment['PA_LIBMPV_TESTS'] == '1';
final _ffmpeg = Platform.environment['PA_TEST_FFMPEG'];

/// 20 s of noisy test pattern (about 3 Mbps) with a stereo AAC track.
Future<String> _movie(Directory dir) async {
  final path = '${dir.path}/Stats.Test.2026.mkv';
  final r = await Process.run(_ffmpeg!, [
    '-v',
    'error',
    '-y',
    '-f',
    'lavfi',
    '-i',
    'testsrc2=size=640x360:rate=24:duration=20,noise=alls=40:allf=t+u',
    '-f',
    'lavfi',
    '-i',
    'sine=frequency=440:duration=20:sample_rate=48000',
    '-ac',
    '2',
    '-c:v',
    'mpeg4',
    '-b:v',
    '3M',
    '-c:a',
    'aac',
    path,
  ]);
  if (r.exitCode != 0) fail('ffmpeg: ${r.stderr}');
  return path;
}

/// A file server with `Range`, like the loopback stream proxy; with
/// [bytesPerSecond] it sends no faster than that.
Future<HttpServer> _serve(Directory dir, {int? bytesPerSecond}) async {
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  server.listen((req) async {
    final res = req.response;
    final file = File('${dir.path}${Uri.decodeComponent(req.uri.path)}');
    if (!file.existsSync()) {
      res.statusCode = HttpStatus.notFound;
      await res.close();
      return;
    }
    final size = file.lengthSync();
    var start = 0, end = size - 1;
    final range = RegExp(
      r'^bytes=(\d*)-(\d*)$',
    ).firstMatch(req.headers.value(HttpHeaders.rangeHeader) ?? '');
    res.headers
      ..set(HttpHeaders.acceptRangesHeader, 'bytes')
      ..contentType = ContentType.binary;
    if (range != null) {
      if (range[1]!.isNotEmpty) start = int.parse(range[1]!);
      if (range[2]!.isNotEmpty) end = int.parse(range[2]!);
      if (range[1]!.isEmpty && range[2]!.isNotEmpty) {
        start = size - int.parse(range[2]!);
        end = size - 1;
      }
      if (end >= size) end = size - 1;
      if (start >= size) {
        res.statusCode = HttpStatus.requestedRangeNotSatisfiable;
        res.headers.set(HttpHeaders.contentRangeHeader, 'bytes */$size');
        await res.close();
        return;
      }
      res.statusCode = HttpStatus.partialContent;
      res.headers.set(
        HttpHeaders.contentRangeHeader,
        'bytes $start-$end/$size',
      );
    }
    res.contentLength = end - start + 1;
    try {
      if (req.method != 'HEAD') {
        final rate = bytesPerSecond;
        if (rate == null) {
          await res.addStream(file.openRead(start, end + 1));
        } else {
          // Chunks of 1/10 s: a steady, slow link.
          final chunk = rate ~/ 10;
          final raf = await file.open();
          try {
            for (var at = start; at <= end; at += chunk) {
              await raf.setPosition(at);
              final n = (end + 1 - at) < chunk ? end + 1 - at : chunk;
              res.add(await raf.read(n));
              await res.flush();
              await Future<void>.delayed(const Duration(milliseconds: 100));
            }
          } finally {
            await raf.close();
          }
        }
      }
      await res.close();
    } catch (_) {
      // The player closed the connection (seek): nothing to do.
    }
  });
  return server;
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
  mpvOptions: const {'vid': 'auto', 'vo': 'null', 'ao': 'null'},
);

Future<String> _prop(MediaKitEngine e, String name) =>
    (e.player.platform as mk.NativePlayer).getProperty(name);

Future<void> _open(MediaKitEngine e, String uri) async {
  await e.open([uri], autoLoadSubtitles: false);
  await _until(
    () =>
        e.state.value.error != null ||
        e.state.value.duration > Duration.zero &&
            e.state.value.position > const Duration(milliseconds: 300),
    'play $uri',
  );
  expect(e.state.value.error, isNull);
}

Future<void> _second() => Future<void>.delayed(const Duration(seconds: 2));

void main() {
  final skip = _enabled && _ffmpeg != null
      ? false
      : 'set PA_LIBMPV_TESTS=1 and PA_TEST_FFMPEG';
  late Directory tmp;
  String? movie;

  setUpAll(() {
    if (skip != false) return;
    mk.MediaKit.ensureInitialized(
      libmpv: Platform.environment['LIBMPV_LIBRARY_PATH'],
    );
  });
  setUp(() async {
    if (skip != false) return;
    tmp = await Directory.systemTemp.createTemp('pa_stats_');
    movie = await _movie(tmp);
  });
  tearDown(() async {
    if (skip != false) return;
    await tmp.delete(recursive: true);
  });

  test('a local file: libmpv fills the model', () async {
    final e = _engine();
    try {
      await _open(e, movie!);
      expect(await statsContractProblems(e, _second), isEmpty);
      await e.setStatsActive(true);
      final s = e.state.value.playbackStats!;
      await e.setStatsActive(false);
      expect(s.videoCodec, 'mpeg4');
      expect(s.audioCodec, 'aac');
      expect((s.videoWidth, s.videoHeight), (640, 360));
      expect(s.videoFps, closeTo(24, 0.01));
      // mpv's `file-format`: "mkv" (older builds: "matroska,webm").
      expect(s.container, anyOf(contains('mkv'), contains('matroska')));
      expect(s.audioChannels, 2);
      expect(s.audioSampleRate, 48000);
      expect(s.audioPath, AudioPath.decoded);
      expect(s.hwDecoding, isFalse);
      expect(s.engine, startsWith('libmpv'));
      expect(s.source, PlaybackSource.local);
      expect(s.bufferTargetMs, isNull, reason: 'libmpv defaults for files');
      expect(s.networkBps, isNull);
      expect(s.bitrateBps, greaterThan(1000000));
    } finally {
      await e.dispose();
    }
  }, skip: skip);

  test('over HTTP (the share proxy): network source and read-ahead', () async {
    final server = await _serve(tmp);
    final e = _engine();
    try {
      await _open(
        e,
        'http://127.0.0.1:${server.port}/${Uri.encodeComponent(movie!.split('/').last)}',
      );
      expect(await statsContractProblems(e, _second), isEmpty);
      await e.setStatsActive(true);
      final s = e.state.value.playbackStats!;
      await e.setStatsActive(false);
      expect(s.source, PlaybackSource.network);
      expect(s.bufferTargetMs, 30000);
      expect(s.stalls, 0);
      expect(await _prop(e, 'cache'), 'yes');
      expect(double.parse(await _prop(e, 'demuxer-readahead-secs')), 30);
      expect(
        int.parse(await _prop(e, 'demuxer-max-bytes')),
        greaterThanOrEqualTo(32 << 20),
      );
    } finally {
      await e.dispose();
      await server.close(force: true);
    }
  }, skip: skip);

  test('a link slower than the stream: the buffer grows', () async {
    // ~1.2 Mbps for a ~3 Mbps stream: headroom 0.4.
    final server = await _serve(tmp, bytesPerSecond: 150000);
    final e = _engine();
    try {
      await e.open([
        'http://127.0.0.1:${server.port}/${Uri.encodeComponent(movie!.split('/').last)}',
      ], autoLoadSubtitles: false);
      await _until(
        () => e.state.value.duration > Duration.zero,
        'open the slow stream',
      );
      var resume = 0.0;
      await _until(
        () {
          unawaited(
            _prop(
              e,
              'cache-pause-wait',
            ).then((v) => resume = double.tryParse(v) ?? 0),
          );
          return resume >= BufferLevel.lossy.resumeWaitSecs;
        },
        'the lossy level on a slow link',
        timeout: const Duration(seconds: 30),
      );
      expect(double.parse(await _prop(e, 'demuxer-readahead-secs')), 120);
      await e.setStatsActive(true);
      final s = e.state.value.playbackStats!;
      await e.setStatsActive(false);
      expect(s.bufferTargetMs, 120000);
      expect(s.networkBps, isNotNull);
      expect(s.headroom, lessThan(AdaptiveBuffer.lossyHeadroom));
      expect(s.check(), isNull);
    } finally {
      await e.dispose();
      await server.close(force: true);
    }
  }, skip: skip);
}
