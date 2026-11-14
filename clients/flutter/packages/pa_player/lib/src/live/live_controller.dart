// SPDX-License-Identifier: MPL-2.0
//
// Live playback (TWI-006, research 06 §2.4, §6.4): latency, catch-up,
// "go to live" and ad labelling on top of a PlayerEngine.
//
// The loopback live proxy of the Rust core (`pa_twitch::playback::live`,
// full builds) knows the program date-time of what it handed to the player
// and the ad breaks of the playlist; the engine knows how many seconds it
// holds ahead of the playhead. Every tick the controller sends the latter
// to the proxy and gets back a report: latency, the rate to play at and
// the ad under the playhead.
//
// Ads stay in the stream as Twitch serves them: the controller never skips,
// seeks over or speeds up an ad. It only reports the break so the UI can
// show "Реклама · 0:15", block seeking and not count it as progress.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

import '../engine/player_engine.dart';
import 'live_options.dart';

/// The ad break under the playhead.
@immutable
class LiveAd {
  const LiveAd({this.id, this.remaining, this.podPosition, this.podLength});

  /// Ad marker id (`EXT-X-DATERANGE` `ID`), when announced by one.
  final String? id;

  /// Time left in the break, when known.
  final Duration? remaining;

  /// Position in the pod (0-based) and pod size, when announced.
  final int? podPosition;
  final int? podLength;

  @override
  bool operator ==(Object other) =>
      other is LiveAd &&
      other.id == id &&
      other.remaining == remaining &&
      other.podPosition == podPosition &&
      other.podLength == podLength;

  @override
  int get hashCode => Object.hash(id, remaining, podPosition, podLength);
}

Duration? _seconds(Object? v) => v is num
    ? Duration(microseconds: (v * Duration.microsecondsPerSecond).round())
    : null;

/// One report of the live proxy (`stats.json`).
@immutable
class LiveReport {
  const LiveReport({
    required this.mode,
    this.latency,
    required this.target,
    this.rate = 1.0,
    this.shouldJump = false,
    this.ad,
    this.seekAllowed = true,
    this.prefetch = 0,
    this.segments = 0,
    this.adSegments = 0,
  });

  factory LiveReport.fromJson(Map<String, Object?> json) {
    final ad = json['ad'];
    final ll = json['low_latency'];
    final totals = json['totals'];
    int count(Object? map, String key) =>
        map is Map && map[key] is num ? (map[key] as num).toInt() : 0;
    return LiveReport(
      mode: json['mode'] == 'stable'
          ? LiveLatencyMode.stable
          : LiveLatencyMode.lowLatency,
      latency: _seconds(json['latency']),
      target: _seconds(json['target']) ?? Duration.zero,
      rate: (json['rate'] as num?)?.toDouble() ?? 1.0,
      shouldJump: json['should_jump'] == true,
      ad: ad is Map
          ? LiveAd(
              id: ad['id'] as String?,
              remaining: _seconds(ad['remaining']),
              podPosition: (ad['pod_position'] as num?)?.toInt(),
              podLength: (ad['pod_length'] as num?)?.toInt(),
            )
          : null,
      seekAllowed: json['seek_allowed'] != false,
      prefetch: count(ll, 'prefetch'),
      segments: count(totals, 'segments'),
      adSegments: count(totals, 'ad_segments'),
    );
  }

  final LiveLatencyMode mode;

  /// Wall clock minus the program date-time on screen.
  final Duration? latency;

  /// Latency the catch-up aims for.
  final Duration target;

  /// Rate the player should play at now (1.0 during ads).
  final double rate;

  /// So far behind that "go to live" beats speeding up.
  final bool shouldJump;
  final LiveAd? ad;

  /// False during an ad break.
  final bool seekAllowed;

  /// Prefetch segments in the last playlist (Twitch low latency).
  final int prefetch;

  /// Segments handed to the player, and of them ad segments.
  final int segments;
  final int adSegments;
}

/// Where live reports come from.
abstract class LiveTelemetry {
  /// The report for a player holding [cacheAhead] seconds ahead of its
  /// playhead at playback [rate]; null when the proxy is unreachable.
  Future<LiveReport?> report({
    required double cacheAhead,
    required double rate,
  });

  /// Forgets the playhead before the player reloads at the live edge.
  Future<void> resetPlayhead();

  Future<void> close();
}

/// [LiveTelemetry] over the proxy's loopback HTTP endpoints.
class HttpLiveTelemetry implements LiveTelemetry {
  HttpLiveTelemetry({required this.stats, required this.reset});

  final Uri stats;
  final Uri reset;
  final HttpClient _client = HttpClient()
    ..connectionTimeout = const Duration(seconds: 2);

  Future<String?> _get(Uri uri) async {
    try {
      final req = await _client.getUrl(uri);
      final res = await req.close().timeout(const Duration(seconds: 2));
      final body = await res.transform(utf8.decoder).join();
      return res.statusCode == HttpStatus.ok ? body : null;
    } on Object {
      return null;
    }
  }

  @override
  Future<LiveReport?> report({
    required double cacheAhead,
    required double rate,
  }) async {
    final body = await _get(
      stats.replace(
        queryParameters: {
          'ahead': cacheAhead.toStringAsFixed(3),
          'rate': rate.toStringAsFixed(3),
        },
      ),
    );
    if (body == null) return null;
    final json = jsonDecode(body);
    return json is Map<String, Object?> ? LiveReport.fromJson(json) : null;
  }

  @override
  Future<void> resetPlayhead() => _get(reset);

  @override
  Future<void> close() async => _client.close(force: true);
}

/// What the live UI shows: latency indicator, "К эфиру", ad badge.
@immutable
class LiveStatus {
  const LiveStatus({
    this.mode = LiveLatencyMode.lowLatency,
    this.latency,
    this.target = Duration.zero,
    this.rate = 1.0,
    this.ad,
    this.seekAllowed = true,
    this.shouldJump = false,
    this.prefetch = 0,
  });

  final LiveLatencyMode mode;
  final Duration? latency;
  final Duration target;

  /// Rate the engine plays at (> 1 while catching up).
  final double rate;
  final LiveAd? ad;
  final bool seekAllowed;
  final bool shouldJump;
  final int prefetch;

  bool get inAd => ad != null;

  /// Ad time is not watched content (06 §6.4).
  bool get countsAsProgress => !inAd;

  bool get catchingUp => rate > 1.0 + 1e-3;

  /// Close enough to the edge that "go to live" has nothing to do.
  bool get atLiveEdge =>
      latency != null && latency! <= target + const Duration(seconds: 1);

  /// "Go to live" makes sense: behind the edge and not in an ad.
  bool get canGoLive => seekAllowed && !atLiveEdge;

  /// Rows for the OSD stats panel (UI-013): latency / target, prefetch
  /// segments, catch-up rate, ad time left. Keys like [EngineState.stats].
  Map<String, String> get info => {
    if (latency != null)
      'Live latency': '${_secs(latency!)} / ${_secs(target)}',
    if (prefetch > 0) 'Prefetch segments': '$prefetch',
    if (catchingUp) 'Catch-up': '${rate.toStringAsFixed(2)}×',
    if (ad != null)
      'Ad break': ad!.remaining == null
          ? '—'
          : '${(ad!.remaining!.inMilliseconds / 1000).ceil()} s',
  };

  static String _secs(Duration d) =>
      '${(d.inMilliseconds / 1000).toStringAsFixed(1)} s';

  @override
  bool operator ==(Object other) =>
      other is LiveStatus &&
      other.mode == mode &&
      other.latency == latency &&
      other.target == target &&
      other.rate == rate &&
      other.ad == ad &&
      other.seekAllowed == seekAllowed &&
      other.shouldJump == shouldJump &&
      other.prefetch == prefetch;

  @override
  int get hashCode => Object.hash(
    mode,
    latency,
    target,
    rate,
    ad,
    seekAllowed,
    shouldJump,
    prefetch,
  );
}

/// Drives a live stream on [engine]: polls [telemetry], applies the
/// catch-up rate and implements "go to live". Create the engine with
/// [liveMpvOptions] of the same mode.
class LivePlaybackController {
  LivePlaybackController({
    required this.engine,
    required this.telemetry,
    required this.url,
    this.catchUp = true,
    this.interval = const Duration(milliseconds: 500),
  });

  final PlayerEngine engine;
  final LiveTelemetry telemetry;

  /// The proxy's playlist URL.
  final String url;

  /// Speed up when behind (the "catch-up" switch of the player).
  bool catchUp;
  final Duration interval;

  final ValueNotifier<LiveStatus> _status = ValueNotifier(const LiveStatus());
  Timer? _timer;
  bool _ticking = false;
  bool _disposed = false;

  ValueListenable<LiveStatus> get status => _status;

  /// Opens the stream and starts polling.
  Future<void> start() async {
    await engine.open([url], autoLoadSubtitles: false);
    _timer ??= Timer.periodic(interval, (_) => tick());
  }

  /// One poll; public for tests.
  Future<void> tick() async {
    if (_ticking || _disposed) return;
    _ticking = true;
    try {
      final s = engine.state.value;
      final ahead = (s.buffer - s.position).inMicroseconds / 1e6;
      final r = await telemetry.report(
        cacheAhead: ahead < 0 ? 0 : ahead,
        rate: s.rate,
      );
      if (r == null || _disposed) return;
      var rate = s.rate;
      if (catchUp && (r.rate - s.rate).abs() > 1e-3) {
        rate = r.rate;
        await engine.setRate(rate);
      } else if (!catchUp && s.rate > 1.0 + 1e-3 && r.inCatchUpRange(s.rate)) {
        rate = 1.0;
        await engine.setRate(rate);
      }
      _status.value = LiveStatus(
        mode: r.mode,
        latency: r.latency,
        target: r.target,
        rate: rate,
        ad: r.ad,
        seekAllowed: r.seekAllowed,
        shouldJump: r.shouldJump,
        prefetch: r.prefetch,
      );
      // The stats overlay (UI-013) shows these rows next to the engine's.
      final e = engine;
      if (e is StatsEngine) (e as StatsEngine).setExtraStats(_status.value.info);
    } finally {
      _ticking = false;
    }
  }

  /// "К эфиру": reload at the live edge. Not during an ad (false).
  Future<bool> goToLive() async {
    if (!_status.value.seekAllowed) return false;
    await telemetry.resetPlayhead();
    if (_status.value.catchingUp) await engine.setRate(1.0);
    await engine.open([url], autoLoadSubtitles: false);
    _status.value = LiveStatus(
      mode: _status.value.mode,
      target: _status.value.target,
      prefetch: _status.value.prefetch,
    );
    return true;
  }

  Future<void> dispose() async {
    _disposed = true;
    _timer?.cancel();
    await telemetry.close();
    _status.dispose();
  }
}

extension on LiveReport {
  /// [rate] is one of the catch-up rates (1.05× or 1.1×).
  bool inCatchUpRange(double rate) =>
      (rate - 1.05).abs() < 1e-3 || (rate - 1.1).abs() < 1e-3;
}
