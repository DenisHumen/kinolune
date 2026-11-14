// SPDX-License-Identifier: MPL-2.0
//
// LIB-020 "done when": a movie split over two files plays as one timeline on
// real libmpv through media_kit, from local paths and from HTTP URLs (the
// loopback stream proxy of network sources, ADR-0038): the duration is the
// sum of the parts, and a seek into part 2 lands at the right position on
// the right frame.
//
// Each test movie is 12 s of solid colour blocks, 3 s each: red, green
// (part 1), blue, white (part 2). Two layouts:
//   - `cd1`/`cd2`: two AVI files (MPEG-4 Part 2), each starting at 0;
//   - DVD: one MPEG-PS stream cut by bytes on a 2048-byte sector into
//     `VTS_01_1.VOB` and `VTS_01_2.VOB`, as on a disc: part 2 starts mid-
//     stream with timestamps that continue part 1.
//
// LIB-021 "done when": the subtitles of the parts (`Heat.1995.cd1.srt`,
// `.cd2.srt`) loaded as one track through MediaKitEngine.addSubtitle show
// at the right moment of the common timeline: a cd2 line at 0:01.5 of part
// 2 shows at 6 + 1.5 s, also when only cd2 has subtitles and over HTTP.
//
// Needs libmpv and an ffmpeg to make the media, so it is skipped unless
// PA_LIBMPV_TESTS=1 and PA_TEST_FFMPEG are set:
//   PA_LIBMPV_TESTS=1 PA_TEST_FFMPEG=<ffmpeg> \
//     LIBMPV_LIBRARY_PATH=<dir>/libmpv.so.2 \
//     flutter test test/edl_libmpv_test.dart
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:media_kit/media_kit.dart' as mk;
import 'package:pa_player/pa_player.dart';

final _enabled = Platform.environment['PA_LIBMPV_TESTS'] == '1';
final _ffmpeg = Platform.environment['PA_TEST_FFMPEG'];

/// Colour blocks of the movie, 3 s each, as ffmpeg names and as BGR.
const _blocks = ['red', 'lime', 'blue', 'white'];
const _block = 3.0;
const _total = 12.0;

Future<void> _ffmpegRun(List<String> args) async {
  final r = await Process.run(_ffmpeg!, ['-v', 'error', '-y', ...args]);
  if (r.exitCode != 0) fail('ffmpeg ${args.last}: ${r.stderr}');
}

/// ffmpeg inputs and filter for colour blocks [from]..[to) of the movie.
List<String> _colours(int from, int to) => [
  for (var i = from; i < to; i++) ...[
    '-f',
    'lavfi',
    '-i',
    'color=c=${_blocks[i]}:s=160x120:r=10:d=$_block',
  ],
  '-filter_complex',
  '${[for (var i = 0; i < to - from; i++) '[$i:v]'].join()}'
      // Noise: a solid colour compresses to a few KiB, too small to cut
      // like a VOB; the centre pixel keeps its colour.
      'concat=n=${to - from}:v=1:a=0,noise=alls=12:allf=t+u[v]',
  '-map',
  '[v]',
];

/// `cd1`/`cd2`: two files, two blocks each.
Future<List<String>> _cdParts(Directory dir) async {
  final out = <String>[];
  for (final (no, from) in [(1, 0), (2, 2)]) {
    final path = '${dir.path}/Heat.1995.cd$no.avi';
    await _ffmpegRun([
      ..._colours(from, from + 2),
      '-c:v',
      'mpeg4',
      '-q:v',
      '3',
      '-g',
      '10',
      path,
    ]);
    out.add(path);
  }
  return out;
}

/// DVD: one MPEG-PS of all blocks, cut in two on a sector boundary near the
/// middle (VOBs are split by size, not on frames).
Future<List<String>> _vobParts(Directory dir) async {
  final whole = '${dir.path}/whole.mpg';
  await _ffmpegRun([
    ..._colours(0, 4),
    '-c:v',
    'mpeg2video',
    '-q:v',
    '3',
    '-g',
    '10',
    '-f',
    'vob',
    whole,
  ]);
  final bytes = File(whole).readAsBytesSync();
  final cut = (bytes.length ~/ 2) ~/ 2048 * 2048;
  final one = '${dir.path}/VTS_01_1.VOB';
  final two = '${dir.path}/VTS_01_2.VOB';
  File(one).writeAsBytesSync(Uint8List.sublistView(bytes, 0, cut));
  File(two).writeAsBytesSync(Uint8List.sublistView(bytes, cut));
  return [one, two];
}

/// A minimal HTTP file server with single `Range` requests, like the
/// loopback stream proxy.
Future<HttpServer> _serve(Directory dir) async {
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
        await res.addStream(file.openRead(start, end + 1));
      }
      await res.close();
    } catch (_) {
      // The player closed the connection (seek): nothing to do.
    }
  });
  return server;
}

/// The colour block a frame shows: the index in [_blocks] whose colour
/// matches the centre pixel (BGR0), or -1.
int _blockOf(Uint8List raw, int width, int height) {
  final stride = raw.length ~/ height;
  final i = (height ~/ 2) * stride + (width ~/ 2) * 4;
  final (b, g, r) = (raw[i], raw[i + 1], raw[i + 2]);
  bool hi(int v) => v > 180;
  bool lo(int v) => v < 80;
  if (hi(r) && lo(g) && lo(b)) return 0;
  if (lo(r) && hi(g) && lo(b)) return 1;
  if (lo(r) && lo(g) && hi(b)) return 2;
  if (hi(r) && hi(g) && hi(b)) return 3;
  return -1;
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

/// Opens [uri] paused on libmpv through media_kit and checks the common
/// timeline: duration, and seeks into each block of both parts.
Future<void> _checkTimeline(String uri) async {
  final engine = MediaKitEngine(
    // No VideoController (no texture in a test): decode to vo=null and
    // read frames with screenshot-raw.
    video: false,
    mpvOptions: const {
      'vid': 'auto',
      'vo': 'null',
      'ao': 'null',
      'hr-seek': 'yes',
    },
  );
  try {
    await engine.open([uri], play: false, autoLoadSubtitles: false);
    final state = engine.state;
    await _until(
      () =>
          state.value.error != null ||
          state.value.duration > Duration.zero &&
              (state.value.videoWidth ?? 0) > 0,
      'open $uri',
    );
    expect(state.value.error, isNull);
    final duration = state.value.duration.inMilliseconds / 1000;
    expect(duration, closeTo(_total, 0.6), reason: 'one timeline of 2 parts');

    final player = engine.player;
    // Part 2 first (the point of the test), then back into part 1.
    for (final at in [10.5, 7.5, 4.5, 1.5, 11.6]) {
      await engine.seek(Duration(milliseconds: (at * 1000).round()));
      final want = (at / _block).floor();
      var got = -1;
      var pos = -1.0;
      final deadline = DateTime.now().add(const Duration(seconds: 10));
      while (DateTime.now().isBefore(deadline)) {
        await Future<void>.delayed(const Duration(milliseconds: 150));
        pos = player.state.position.inMilliseconds / 1000;
        final raw = await player.screenshot(format: null);
        if (raw == null || raw.isEmpty) continue;
        got = _blockOf(raw, state.value.videoWidth!, state.value.videoHeight!);
        if (got == want && (pos - at).abs() < 0.5) break;
      }
      expect(pos, closeTo(at, 0.5), reason: 'position after seek to $at');
      expect(
        got,
        want,
        reason:
            'frame at $at s: ${_blocks[want]}, '
            'not ${got < 0 ? '?' : _blocks[got]}',
      );
    }
  } finally {
    await engine.dispose();
  }
}

/// LIB-021: subtitles of the parts, each with a line at 0:01–0:02 of its
/// own part, loaded through [MediaKitEngine.addSubtitle] onto the timeline
/// of [uri] (two parts of 6 s): the cd2 line must show at 6 + 1.5 s.
Future<void> _checkPartSubtitles(
  String uri,
  List<String?> subtitles, {
  required Map<double, String> want,
}) async {
  final engine = MediaKitEngine(
    video: false,
    mpvOptions: const {'vid': 'auto', 'vo': 'null', 'ao': 'null'},
  );
  try {
    await engine.open([uri], play: false, autoLoadSubtitles: false);
    final state = engine.state;
    await _until(
      () => state.value.error != null || state.value.duration > Duration.zero,
      'open $uri',
    );
    await engine.addSubtitle(
      ExternalSubtitle(
        uri: subtitles.firstWhere((s) => s != null)!,
        title: 'Heat parts',
        language: 'rus',
        parts: subtitles,
      ),
      select: true,
    );
    final p = engine.player.platform as mk.NativePlayer;
    await _until(
      () => state.value.subtitleTracks.any((t) => t.title == 'Heat parts'),
      'subtitle track listed',
    );
    for (final MapEntry(key: at, value: text) in want.entries) {
      await engine.seek(Duration(milliseconds: (at * 1000).round()));
      var got = '';
      final deadline = DateTime.now().add(const Duration(seconds: 10));
      while (DateTime.now().isBefore(deadline)) {
        await Future<void>.delayed(const Duration(milliseconds: 150));
        final pos = engine.player.state.position.inMilliseconds / 1000;
        got = (await p.getProperty('sub-text')).trim();
        if ((pos - at).abs() < 0.3 && got == text) break;
      }
      expect(got, text, reason: 'subtitle at $at s');
    }
  } finally {
    await engine.dispose();
  }
}

/// SubRip with one line at 0:01–0:02 (and one at 0:05.5–0:06.5 that
/// crosses the end of a 6 s part: it is cut at the part's end).
void _srt(String path, List<int> line, {List<int>? late}) {
  File(path).writeAsBytesSync([
    ...ascii.encode('1\r\n00:00:01,000 --> 00:00:02,000\r\n'),
    ...line,
    ...ascii.encode('\r\n\r\n'),
    if (late != null) ...[
      ...ascii.encode('2\r\n00:00:05,500 --> 00:00:06,500\r\n'),
      ...late,
      ...ascii.encode('\r\n\r\n'),
    ],
  ]);
}

void main() {
  late Directory tmp;

  setUpAll(() {
    if (!_enabled) return;
    ensurePaPlayerInitialized();
  });
  setUp(() => tmp = Directory.systemTemp.createTempSync('pa_edl_'));
  tearDown(() => tmp.deleteSync(recursive: true));

  final skip = _enabled && _ffmpeg != null
      ? null
      : 'needs libmpv and ffmpeg: PA_LIBMPV_TESTS=1 PA_TEST_FFMPEG=<ffmpeg> '
            '(see the file header)';

  for (final (name, make) in [
    ('cd1/cd2 movie', _cdParts),
    ('DVD title set of two VOBs', _vobParts),
  ]) {
    test('$name plays as one timeline from local files', () async {
      final parts = await make(tmp);
      final uri = partsUri(parts);
      expect(uri, startsWith(edlScheme));
      await _checkTimeline(uri);
    }, skip: skip);

    test('$name plays as one timeline from HTTP URLs', () async {
      final parts = await make(tmp);
      final server = await _serve(tmp);
      try {
        final base = 'http://127.0.0.1:${server.port}';
        await _checkTimeline(
          partsUri([
            for (final p in parts)
              '$base/${Uri.encodeComponent(p.split('/').last)}',
          ]),
        );
      } finally {
        await server.close(force: true);
      }
    }, skip: skip);
  }

  group('LIB-021: subtitles of the parts on the common timeline', () {
    const one = 'first part';
    const two = 'вторая часть';
    Future<(List<String>, String, String)> media() async {
      final parts = await _cdParts(tmp);
      final cd1 = '${tmp.path}/Heat.1995.cd1.srt';
      final cd2 = '${tmp.path}/Heat.1995.cd2.srt';
      _srt(cd1, utf8.encode(one), late: utf8.encode('end of part one'));
      _srt(cd2, utf8.encode(two));
      return (parts, cd1, cd2);
    }

    test('cd1 and cd2 subtitles from local files', () async {
      final (parts, cd1, cd2) = await media();
      await _checkPartSubtitles(
        partsUri(parts),
        [cd1, cd2],
        want: {
          1.5: one,
          4.5: '',
          5.8: 'end of part one',
          // Part 1's line is cut where part 2 begins.
          6.3: '',
          7.5: two,
          9.0: '',
          1.4: one,
        },
      );
    }, skip: skip);

    test('only cd2 has subtitles', () async {
      final (parts, _, cd2) = await media();
      await _checkPartSubtitles(
        partsUri(parts),
        [null, cd2],
        want: {1.5: '', 7.5: two, 3.0: ''},
      );
    }, skip: skip);

    test('cd1 and cd2 subtitles over HTTP', () async {
      final (parts, cd1, cd2) = await media();
      final server = await _serve(tmp);
      String url(String p) =>
          'http://127.0.0.1:${server.port}/'
          '${Uri.encodeComponent(p.split('/').last)}';
      try {
        await _checkPartSubtitles(
          partsUri([for (final p in parts) url(p)]),
          [url(cd1), url(cd2)],
          want: {1.5: one, 7.5: two},
        );
      } finally {
        await server.close(force: true);
      }
    }, skip: skip);
  });
}
