// SPDX-License-Identifier: MPL-2.0
//
// TWI-006: live controller logic without libmpv — reports from the proxy,
// catch-up rate, "go to live", ads (never sped up or skipped); TWI-025:
// DVR seeks over the whole live window (never during an ad).
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

class _Telemetry implements LiveTelemetry {
  final List<Map<String, Object?>> next = [];
  final List<(double, double)> asked = [];
  int resets = 0;

  @override
  Future<LiveReport?> report({
    required double cacheAhead,
    required double rate,
  }) async {
    asked.add((cacheAhead, rate));
    return next.isEmpty ? null : LiveReport.fromJson(next.removeAt(0));
  }

  @override
  Future<void> resetPlayhead() async => resets++;

  @override
  Future<void> close() async {}
}

Map<String, Object?> _report({
  double? latency = 2.5,
  double rate = 1.0,
  Map<String, Object?>? ad,
  bool shouldJump = false,
}) => {
  'mode': 'low_latency',
  'latency': latency,
  'target': 3.0,
  'rate': rate,
  'should_jump': shouldJump,
  'ad': ad,
  'seek_allowed': ad == null,
  'low_latency': {'prefetch': 2, 'parts': 0},
  'totals': {'segments': 10, 'ad_segments': ad == null ? 0 : 2},
};

void main() {
  late FakePlayerEngine engine;
  late _Telemetry telemetry;
  late LivePlaybackController live;

  setUp(() {
    engine = FakePlayerEngine();
    telemetry = _Telemetry();
    live = LivePlaybackController(
      engine: engine,
      telemetry: telemetry,
      url: 'http://127.0.0.1:1/t/live.m3u8',
    );
  });

  tearDown(() => live.dispose());

  test('reports the cache ahead and exposes latency', () async {
    await live.start();
    engine.emit(
      (s) => s.copyWith(
        position: const Duration(seconds: 10),
        buffer: const Duration(milliseconds: 11500),
      ),
    );
    telemetry.next.add(_report(latency: 2.4));
    await live.tick();
    expect(telemetry.asked.single, (1.5, 1.0));
    final s = live.status.value;
    expect(s.latency, const Duration(milliseconds: 2400));
    expect(s.atLiveEdge, isTrue);
    expect(s.canGoLive, isFalse);
    expect(s.countsAsProgress, isTrue);
    expect(s.info['Live latency'], '2.4 s / 3.0 s');
    expect(s.info['Prefetch segments'], '2');
    // The stats overlay (UI-013) gets the rows through StatsEngine.
    expect(engine.extraStats['Live latency'], '2.4 s / 3.0 s');
    expect(engine.uris, ['http://127.0.0.1:1/t/live.m3u8']);
  });

  test('applies the catch-up rate and goes back to 1×', () async {
    await live.start();
    telemetry.next.add(_report(latency: 6, rate: 1.1));
    await live.tick();
    expect(engine.state.value.rate, 1.1);
    expect(live.status.value.catchingUp, isTrue);
    expect(live.status.value.info['Catch-up'], '1.10×');
    telemetry.next.add(_report(latency: 2.9, rate: 1.0));
    await live.tick();
    expect(engine.state.value.rate, 1.0);
    expect(live.status.value.catchingUp, isFalse);
  });

  test('catch-up off leaves the rate alone', () async {
    live.catchUp = false;
    await live.start();
    telemetry.next.add(_report(latency: 6, rate: 1.1));
    await live.tick();
    expect(engine.state.value.rate, 1.0);
  });

  test('an ad break: no seeking, no go-to-live, not progress', () async {
    await live.start();
    telemetry.next.add(
      _report(
        latency: 9,
        ad: {
          'id': 'stitched-ad-1',
          'remaining': 14.2,
          'pod_position': 0,
          'pod_length': 1,
        },
      ),
    );
    await live.tick();
    final s = live.status.value;
    expect(s.inAd, isTrue);
    expect(s.ad!.remaining, const Duration(milliseconds: 14200));
    expect(s.seekAllowed, isFalse);
    expect(s.canGoLive, isFalse);
    expect(s.countsAsProgress, isFalse);
    expect(s.info['Ad break'], '15 s');
    // The proxy's rate during an ad is 1.0: the ad is not sped up.
    expect(engine.state.value.rate, 1.0);
    final opens = engine.calls.where((c) => c.startsWith('open')).length;
    expect(await live.goToLive(), isFalse);
    expect(engine.calls.where((c) => c.startsWith('open')).length, opens);
    expect(telemetry.resets, 0);
  });

  test('go to live reloads at the edge', () async {
    await live.start();
    telemetry.next.add(_report(latency: 40, rate: 1.1, shouldJump: true));
    await live.tick();
    expect(live.status.value.shouldJump, isTrue);
    expect(live.status.value.canGoLive, isTrue);
    expect(await live.goToLive(), isTrue);
    expect(telemetry.resets, 1);
    expect(engine.state.value.rate, 1.0);
    expect(engine.calls.where((c) => c.startsWith('open')).length, 2);
    expect(live.status.value.latency, isNull);
  });

  test('unreachable proxy keeps the last status', () async {
    await live.start();
    await live.tick();
    expect(live.status.value, const LiveStatus());
  });

  test('mpv options per mode', () {
    final ll = liveMpvOptions(LiveLatencyMode.lowLatency);
    final stable = liveMpvOptions(LiveLatencyMode.stable);
    expect(ll['demuxer-lavf-o'], 'live_start_index=-2');
    expect(stable['demuxer-lavf-o'], 'live_start_index=-3');
    expect(ll.containsKey('speed'), isFalse);
  });

  // ---- TWI-025: live DVR ---------------------------------------------------

  group('live DVR', () {
    late _DvrTelemetry dvr;
    late LivePlaybackController c;

    setUp(() {
      dvr = _DvrTelemetry();
      c = LivePlaybackController(
        engine: engine,
        telemetry: dvr,
        url: 'http://127.0.0.1:1/t/live.m3u8',
        // Tests send the added-up seek themselves.
        seekDebounce: const Duration(hours: 1),
      );
    });

    tearDown(() => c.dispose());

    Map<String, Object?> window({
      double behind = 2.0,
      bool active = false,
      Map<String, Object?>? ad,
    }) => {
      ..._report(latency: behind + 0.5, ad: ad),
      'playhead_ms': _edge - (behind * 1000).round(),
      'edge_ms': _edge,
      'dvr': {
        'window_start_ms': _edge - 600000,
        'window_seconds': 600.0,
        'behind_seconds': behind,
        'active': active,
        'pending': false,
      },
    };

    test(
      'the window and the distance to the edge come from the proxy',
      () async {
        await c.start();
        dvr.next.add(window(behind: 75, active: true));
        await c.tick();
        final s = c.status.value;
        expect(s.dvr!.length, const Duration(minutes: 10));
        expect(
          s.dvr!.start,
          DateTime.fromMillisecondsSinceEpoch(_edge - 600000),
        );
        expect(s.behind, const Duration(seconds: 75));
        expect(s.playhead, DateTime.fromMillisecondsSinceEpoch(_edge - 75000));
        expect(s.rewound, isTrue);
        expect(s.canRewind, isTrue);
        // Rewound on purpose: not "at the edge", "В эфир" offered.
        expect(s.atLiveEdge, isFalse);
        expect(s.canGoLive, isTrue);
      },
    );

    test('quick presses add up and reload the stream once', () async {
      await c.start();
      dvr.next.add(window());
      await c.tick();
      final opens = engine.calls.where((e) => e.startsWith('open')).length;
      expect(c.seekBy(const Duration(seconds: -10)), isTrue);
      expect(c.seekBy(const Duration(seconds: -10)), isTrue);
      expect(c.seekBy(const Duration(seconds: -30)), isTrue);
      expect(c.pendingSeek.value, const Duration(seconds: -50));
      expect(dvr.seeks, isEmpty);
      dvr.answer = {'live': false, 'at_ms': _edge - 52000};
      final r = await c.flushSeek();
      expect(r!.ok, isTrue);
      expect(dvr.seeks, ['by:-50.000']);
      expect(c.pendingSeek.value, Duration.zero);
      expect(engine.calls.where((e) => e.startsWith('open')).length, opens + 1);
      expect(engine.uris, ['http://127.0.0.1:1/t/live.m3u8']);
      expect(c.status.value.behind, const Duration(seconds: 52));
      expect(c.status.value.rewound, isTrue);
    });

    test('60 s back on the seek bar, then back to live', () async {
      await c.start();
      dvr.next.add(window());
      await c.tick();
      final at = DateTime.fromMillisecondsSinceEpoch(_edge - 62000);
      dvr.answer = {'live': false, 'at_ms': at.millisecondsSinceEpoch};
      await c.seekTo(at);
      expect(dvr.seeks, ['at:${at.millisecondsSinceEpoch}']);
      dvr.next.add(window(behind: 62, active: true));
      await c.tick();
      expect(c.status.value.behind, const Duration(seconds: 62));
      expect(await c.goToLive(), isTrue);
      expect(dvr.resets, 1);
    });

    test('no seeking during an ad, and the proxy may refuse too', () async {
      await c.start();
      dvr.next.add(window(ad: {'id': 'stitched-ad-1', 'remaining': 9.0}));
      await c.tick();
      expect(c.status.value.canRewind, isFalse);
      expect(c.seekBy(const Duration(seconds: -10)), isFalse);
      expect(await c.seekTo(DateTime.now()), isNull);
      expect(c.pendingSeek.value, Duration.zero);
      expect(dvr.seeks, isEmpty);

      // The ad started between the report and the seek: the proxy says no.
      dvr.next.add(window());
      await c.tick();
      final opens = engine.calls.where((e) => e.startsWith('open')).length;
      dvr.answer = {'error': 'ad'};
      c.seekBy(const Duration(seconds: -10));
      final r = await c.flushSeek();
      expect(r!.ok, isFalse);
      expect(r.error, 'ad');
      expect(c.lastSeek.value!.error, 'ad');
      expect(engine.calls.where((e) => e.startsWith('open')).length, opens);
    });

    test('a forward jump that stops at an ad says so', () async {
      await c.start();
      dvr.next.add(window(behind: 120, active: true));
      await c.tick();
      dvr.answer = {
        'live': false,
        'at_ms': _edge - 90000,
        'stopped_at_ad': true,
      };
      c.seekBy(const Duration(seconds: 60));
      final r = await c.flushSeek();
      expect(r!.stoppedAtAd, isTrue);
      expect(c.lastSeek.value!.stoppedAtAd, isTrue);
    });

    test('without a seeker the controller does not seek', () async {
      await live.start();
      telemetry.next.add(_report());
      await live.tick();
      expect(live.canSeek, isFalse);
      expect(live.seekBy(const Duration(seconds: -10)), isFalse);
    });

    test('HTTP telemetry seeks next to stats.json', () {
      final t = HttpLiveTelemetry(
        stats: Uri.parse('http://127.0.0.1:9/tok/stats.json'),
        reset: Uri.parse('http://127.0.0.1:9/tok/reset'),
      );
      expect(t.seekUri, Uri.parse('http://127.0.0.1:9/tok/seek'));
      t.close();
    });
  });
}

const _edge = 1800000000000;

class _DvrTelemetry extends _Telemetry implements LiveSeeker {
  final List<String> seeks = [];
  Map<String, Object?> answer = const {'live': false};

  @override
  Future<LiveSeekResult> seek({Duration? by, DateTime? at}) async {
    seeks.add(
      by != null
          ? 'by:${(by.inMilliseconds / 1000).toStringAsFixed(3)}'
          : 'at:${at!.millisecondsSinceEpoch}',
    );
    return LiveSeekResult.fromJson(answer);
  }

  @override
  Future<LiveSeekResult> seekToFirst({DateTime? t0}) async {
    seeks.add('first:${t0?.millisecondsSinceEpoch}');
    return LiveSeekResult.fromJson(answer);
  }
}
