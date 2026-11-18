// SPDX-License-Identifier: MPL-2.0
//
// QA-007, tests on a bad network: MediaKitEngine on real libmpv plays a
// file from an HTTP server behind a real throttled link (Linux netem:
// delay, loss, bandwidth limit, a dropout), the way a media server or the
// core's share proxy feeds it over Wi-Fi.
//
// - With 2× the stream bitrate the player never stops to load, on a clean
//   link, a far one (high latency) and a lossy Wi-Fi one.
// - The Infuse complaint (research 01 §3 п.4: "buffering on a slow and
//   unstable network since 2022", the forum asks to test the playback
//   stack against a throttled network): a throttled lossy link that drops
//   out for 4 s mid-film does not stop playback either.
// - The indicator «сеть против битрейта» is right: the network speed of
//   the stats is what the link really delivers (measured by a plain
//   download over the same link), and the verdict follows; a link slower
//   than the stream shows «не хватает», the buffer goes to its biggest
//   level and a media server gets the transcode hint.
//
// The link is shaped inside a throwaway container, never on the host:
// tools/qa/netem/run.sh starts it (nginx + tc, `--cap-add NET_ADMIN`),
// runs this file and removes it. Skipped unless that script set
// PA_NETEM_CONTAINER, PA_NETEM_URL and PA_NETEM_MEDIA (and PA_TEST_FFMPEG,
// PA_LIBMPV_TESTS=1 like the other libmpv tests).
import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:media_kit/media_kit.dart' as mk;
import 'package:pa_player/pa_player.dart';

final _env = Platform.environment;
final _container = _env['PA_NETEM_CONTAINER'];
final _baseUrl = _env['PA_NETEM_URL'];
final _mediaDir = _env['PA_NETEM_MEDIA'];
final _ffmpeg = _env['PA_TEST_FFMPEG'];
final _enabled =
    _env['PA_LIBMPV_TESTS'] == '1' &&
    _container != null &&
    _baseUrl != null &&
    _mediaDir != null &&
    _ffmpeg != null;

/// Length of the test film. Long enough that the cache is still filling
/// when the dropout comes, short enough to keep the job fast.
const _filmSecs = 40;

/// How long each profile plays.
const _playSecs = 24;

/// 40 s of noisy test pattern at a steady ~2 Mbps (mpeg4, constrained
/// rate) with stereo AAC: a constant bitrate makes "2× the bitrate" exact.
Future<File> _film(String dir) async {
  final file = File('$dir/QA-007.Netem.mkv');
  if (file.existsSync()) return file;
  final r = await Process.run(_ffmpeg!, [
    '-v',
    'error',
    '-y',
    '-f',
    'lavfi',
    '-i',
    'testsrc2=size=640x360:rate=24:duration=$_filmSecs,'
        'noise=alls=40:allf=t+u',
    '-f',
    'lavfi',
    '-i',
    'sine=frequency=440:duration=$_filmSecs:sample_rate=48000',
    '-ac',
    '2',
    '-c:v',
    'mpeg4',
    '-b:v',
    '2M',
    '-minrate',
    '2M',
    '-maxrate',
    '2M',
    '-bufsize',
    '1M',
    '-c:a',
    'aac',
    '-b:a',
    '128k',
    file.path,
  ]);
  if (r.exitCode != 0) fail('ffmpeg: ${r.stderr}');
  return file;
}

/// Replaces the shaping of the server's outgoing traffic (what the player
/// downloads) inside the test container.
Future<void> _netem(String args) async {
  final r = await Process.run('docker', [
    'exec',
    _container!,
    'tc',
    'qdisc',
    'replace',
    'dev',
    'eth0',
    'root',
    'netem',
    ...args.split(' '),
  ]);
  if (r.exitCode != 0) fail('tc netem $args: ${r.stderr}');
}

/// The link's real goodput, bits per second: a plain download of [url]
/// for [secs] over the shaped link, the same bytes the player would get.
Future<int> _goodput(String url, {int secs = 5}) async {
  final client = HttpClient();
  try {
    final req = await client.getUrl(Uri.parse(url));
    final res = await req.close();
    final sw = Stopwatch()..start();
    var bytes = 0;
    var first = true;
    await for (final chunk in res) {
      // From the first byte: connection setup is not bandwidth.
      if (first) {
        first = false;
        sw.reset();
        continue;
      }
      bytes += chunk.length;
      if (sw.elapsedMilliseconds >= secs * 1000) break;
    }
    return (bytes * 8 * 1000 / sw.elapsedMilliseconds).round();
  } finally {
    client.close(force: true);
  }
}

MediaKitEngine _engine() => MediaKitEngine(
  // No VideoController (no texture in a test): decode to vo=null.
  video: false,
  mpvOptions: const {'vid': 'auto', 'vo': 'null', 'ao': 'null'},
);

Future<String> _prop(MediaKitEngine e, String name) =>
    (e.player.platform as mk.NativePlayer).getProperty(name);

Future<void> _until(
  bool Function() done,
  String what, {
  Duration timeout = const Duration(seconds: 30),
}) async {
  final deadline = DateTime.now().add(timeout);
  while (!done()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out: $what');
    await Future<void>.delayed(const Duration(milliseconds: 50));
  }
}

Future<PlaybackStats> _stats(MediaKitEngine e) async {
  await e.setStatsActive(true);
  final s = e.state.value.playbackStats!;
  await e.setStatsActive(false);
  return s;
}

/// What one run of playback saw.
class _Run {
  _Run(this.stats, this.lag, this.pausedForCache, this.position);

  final PlaybackStats stats;

  /// Wall time minus playback time since the first frame: how long the
  /// picture stood still.
  final Duration lag;

  /// Times libmpv paused to refill its cache after the first frame.
  final int pausedForCache;
  final Duration position;

  @override
  String toString() =>
      'lag ${lag.inMilliseconds} ms, paused-for-cache $pausedForCache, '
      'position ${position.inMilliseconds} ms, '
      'network ${stats.networkBps}, bitrate ${stats.bitrateBps}, '
      'headroom ${stats.headroom?.toStringAsFixed(2)}, '
      'fit ${stats.networkFit?.name}, buffer ${stats.bufferTargetMs}, '
      'stalls ${stats.stalls}';
}

/// Opens [url], plays for [secs] and watches for stops: libmpv's
/// `paused-for-cache` (polled), the engine's own stall count and the
/// playback clock against the wall clock. [during] runs at the given
/// second of playback (the dropout).
Future<_Run> _play(
  String url, {
  int secs = _playSecs,
  (int, Future<void> Function())? during,
}) async {
  final e = _engine();
  try {
    await e.open([url], autoLoadSubtitles: false);
    await _until(
      () =>
          e.state.value.error != null ||
          e.state.value.position > const Duration(milliseconds: 500),
      'first frame of $url',
    );
    expect(e.state.value.error, isNull);
    final wall = Stopwatch()..start();
    final start = e.state.value.position;
    var paused = 0;
    var wasPaused = false;
    var fired = false;
    while (wall.elapsed < Duration(seconds: secs)) {
      final p = (await _prop(e, 'paused-for-cache')).trim() == 'yes';
      if (p && !wasPaused) paused++;
      wasPaused = p;
      if (during != null && !fired && wall.elapsed.inSeconds >= during.$1) {
        fired = true;
        unawaited(during.$2());
      }
      await Future<void>.delayed(const Duration(milliseconds: 200));
    }
    final played = e.state.value.position - start;
    final s = await _stats(e);
    return _Run(s, wall.elapsed - played, paused, e.state.value.position);
  } finally {
    await e.dispose();
  }
}

void main() {
  final skip = _enabled
      ? false
      : 'run through tools/qa/netem/run.sh (needs Docker, libmpv, ffmpeg)';
  late String url;
  late int bitrate;

  setUpAll(() async {
    if (skip != false) return;
    mk.MediaKit.ensureInitialized(libmpv: _env['LIBMPV_LIBRARY_PATH']);
    final film = await _film(_mediaDir!);
    bitrate = (film.lengthSync() * 8 / _filmSecs).round();
    url = '$_baseUrl${film.uri.pathSegments.last}';
    // ignore: avoid_print
    print('QA-007: film ${film.lengthSync()} bytes, $bitrate bps');
  });

  tearDownAll(() async {
    if (skip != false) return;
    await _netem('delay 0ms');
  });

  /// netem rate for [ratio] times the film's bitrate.
  String rate(double ratio) => 'rate ${(bitrate * ratio).round()}bit';

  /// A queue of ~0.3 s at the profile's rate (Wi-Fi-like, no bufferbloat).
  const limit = 'limit 100';

  // The profiles of QA-007 at 2× the stream bitrate: (name, netem).
  final twice = [
    ('bandwidth 2×', () => 'delay 10ms ${rate(2)} $limit'),
    ('latency 150 ms, 2×', () => 'delay 150ms ${rate(2)} $limit'),
    (
      'Wi-Fi: jitter and 0.5 % loss, 2×',
      () => 'delay 30ms 10ms loss 0.5% ${rate(2)} $limit',
    ),
  ];

  for (final (name, netem) in twice) {
    test('$name: no stops, the indicator is the link speed', () async {
      await _netem(netem());
      final link = await _goodput(url);
      final run = await _play(url);
      // ignore: avoid_print
      print('QA-007 $name: link $link bps; $run');
      final s = run.stats;
      expect(s.source, PlaybackSource.server);
      // The player never stood still to load.
      expect(run.pausedForCache, 0, reason: '$run');
      expect(s.stalls, 0, reason: '$run');
      expect(run.lag, lessThan(const Duration(milliseconds: 1500)));
      // The indicator: the speed the link really has (±25 %), never more
      // than netem lets through, and its verdict.
      expect(s.networkBps, isNotNull);
      expect(s.networkBps, inInclusiveRange(link * 0.75, link * 1.25));
      expect(s.networkBps, lessThanOrEqualTo(bitrate * 2 * 1.05));
      final h = s.headroom!;
      expect(h, inInclusiveRange(link / bitrate * 0.7, link / bitrate * 1.3));
      expect(s.networkFit, NetworkFit.of(h));
      expect(s.check(), isNull);
      expect(s.transcodeAdvised, isFalse, reason: 'it keeps up');
      if (link >= bitrate * 1.8) {
        // A clean 2× link: enough, the buffer stays at its normal level.
        expect(s.networkFit, NetworkFit.enough, reason: '$run');
        expect(s.bufferTargetMs, BufferLevel.normal.readAheadSecs * 1000);
      }
    }, skip: skip);
  }

  test('the Infuse complaint: a throttled lossy link that drops out for 4 s '
      'does not stop playback', () async {
    const wifi = 'delay 30ms 10ms loss 0.5%';
    await _netem('$wifi ${rate(2)} $limit');
    final run = await _play(
      url,
      secs: 28,
      during: (
        12,
        () async {
          // The Wi-Fi drops out: nothing gets through for 4 s.
          await _netem('delay 30ms 10ms loss 100% ${rate(2)} $limit');
          await Future<void>.delayed(const Duration(seconds: 4));
          await _netem('$wifi ${rate(2)} $limit');
        },
      ),
    );
    // ignore: avoid_print
    print('QA-007 dropout: $run');
    expect(run.pausedForCache, 0, reason: '$run');
    expect(run.stats.stalls, 0, reason: '$run');
    expect(run.lag, lessThan(const Duration(milliseconds: 1500)));
  }, skip: skip);

  test('a link slower than the stream: «не хватает», the biggest buffer, '
      'the transcode hint', () async {
    await _netem('delay 10ms ${rate(0.8)} $limit');
    final link = await _goodput(url);
    final e = _engine();
    try {
      await e.open([url], autoLoadSubtitles: false);
      var resume = 0.0;
      await _until(() {
        unawaited(
          _prop(
            e,
            'cache-pause-wait',
          ).then((v) => resume = double.tryParse(v) ?? 0),
        );
        return resume >= BufferLevel.lossy.resumeWaitSecs;
      }, 'the lossy level on a slow link');
      // A few more samples for the smoothed speed.
      await Future<void>.delayed(const Duration(seconds: 6));
      final s = await _stats(e);
      // ignore: avoid_print
      print(
        'QA-007 slow: link $link bps; network ${s.networkBps}, '
        'bitrate ${s.bitrateBps}, headroom ${s.headroom}, '
        'stalls ${s.stalls}',
      );
      expect(s.networkFit, NetworkFit.short);
      expect(s.networkBps, inInclusiveRange(link * 0.75, link * 1.25));
      expect(s.bufferTargetMs, BufferLevel.lossy.readAheadSecs * 1000);
      expect(s.transcodeAdvised, isTrue);
      expect(
        s.rows().map((r) => r.$1),
        contains(networkVersusBitrateRow),
      );
      expect(s.check(), isNull);
    } finally {
      await e.dispose();
    }
  }, skip: skip);
}
