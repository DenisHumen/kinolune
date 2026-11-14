// SPDX-License-Identifier: MPL-2.0
//
// TWI-006: live controller logic without libmpv — reports from the proxy,
// catch-up rate, "go to live", ads (never sped up or skipped).
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
}
