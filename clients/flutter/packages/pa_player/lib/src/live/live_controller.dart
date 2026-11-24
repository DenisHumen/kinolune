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
//
// Live DVR (TWI-025): the proxy keeps the whole live playlist window, not
// only what the player has cached. [LivePlaybackController.seekBy] and
// [LivePlaybackController.seekTo] ask the proxy to move the playhead (it
// refuses during an ad, and a forward jump stops at an ad not played yet)
// and reload the stream there; the report tells how far behind the edge the
// playhead is ("−N мин") and how long the window is.

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

DateTime? _time(Object? v) =>
    v is num ? DateTime.fromMillisecondsSinceEpoch(v.toInt()) : null;

/// The live DVR window of the proxy (TWI-025).
@immutable
class LiveDvrWindow {
  const LiveDvrWindow({
    required this.start,
    required this.length,
    this.behind,
    this.active = false,
    this.pending = false,
  });

  static LiveDvrWindow? fromJson(Object? json) {
    if (json is! Map) return null;
    final start = _time(json['window_start_ms']);
    if (start == null) return null;
    return LiveDvrWindow(
      start: start,
      length: _seconds(json['window_seconds']) ?? Duration.zero,
      behind: _seconds(json['behind_seconds']),
      active: json['active'] == true,
      pending: json['pending'] == true,
    );
  }

  /// Program date-time of the oldest moment the player can go back to.
  final DateTime start;

  /// From [start] to the live edge.
  final Duration length;

  /// How far the playhead is behind the live edge.
  final Duration? behind;

  /// The user rewound: no catch-up, forward jumps stop at ads not played.
  final bool active;

  /// A seek waits for the player to reload.
  final bool pending;

  @override
  bool operator ==(Object other) =>
      other is LiveDvrWindow &&
      other.start == start &&
      other.length == length &&
      other.behind == behind &&
      other.active == active &&
      other.pending == pending;

  @override
  int get hashCode => Object.hash(start, length, behind, active, pending);
}

/// What a DVR seek did ([LiveSeeker]).
@immutable
class LiveSeekResult {
  const LiveSeekResult({
    this.live = false,
    this.at,
    this.stoppedAtAd = false,
    this.error,
  });

  /// Refused: `ad` (the playhead is in an ad break), `no_window`,
  /// `unreachable`.
  const LiveSeekResult.refused(String this.error)
    : live = false,
      at = null,
      stoppedAtAd = false;

  factory LiveSeekResult.fromJson(Map<String, Object?> json) => json['error']
          is String
      ? LiveSeekResult.refused(json['error']! as String)
      : LiveSeekResult(
          live: json['live'] == true,
          at: _time(json['at_ms']),
          stoppedAtAd: json['stopped_at_ad'] == true,
        );

  /// Back at the live edge.
  final bool live;

  /// Where the player starts, when known.
  final DateTime? at;

  /// A forward jump stopped at an ad break not played yet.
  final bool stoppedAtAd;
  final String? error;

  bool get ok => error == null;
}

/// DVR seeking of the live proxy (TWI-025); [HttpLiveTelemetry] does it.
abstract class LiveSeeker {
  /// Moves the playhead [by] from where it is, or to the program
  /// date-time [at]. The player reloads the playlist URL afterwards.
  Future<LiveSeekResult> seek({Duration? by, DateTime? at});

  /// Starts the playlist (a VOD of a stream still on air) at its first
  /// segment; [t0] is when the stream started (VOD playlists have no
  /// program date-times).
  Future<LiveSeekResult> seekToFirst({DateTime? t0});
}

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
    this.playhead,
    this.edge,
    this.dvr,
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
      playhead: _time(json['playhead_ms']),
      edge: _time(json['edge_ms']),
      dvr: LiveDvrWindow.fromJson(json['dvr']),
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

  /// Program date-time under the playhead.
  final DateTime? playhead;

  /// Live edge: end of the newest complete segment.
  final DateTime? edge;

  /// The DVR window (TWI-025).
  final LiveDvrWindow? dvr;
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

/// [LiveTelemetry] over the proxy's loopback HTTP endpoints; DVR seeks go
/// to `seek` next to `stats.json` (TWI-025).
class HttpLiveTelemetry implements LiveTelemetry, LiveSeeker {
  HttpLiveTelemetry({required this.stats, required this.reset, Uri? seekUri})
    : seekUri = seekUri ?? stats.resolve('seek');

  final Uri stats;
  final Uri reset;
  final Uri seekUri;
  final HttpClient _client = HttpClient()
    ..connectionTimeout = const Duration(seconds: 2);

  Future<(int, String)?> _fetch(Uri uri) async {
    try {
      final req = await _client.getUrl(uri);
      final res = await req.close().timeout(const Duration(seconds: 2));
      final body = await res.transform(utf8.decoder).join();
      return (res.statusCode, body);
    } on Object {
      return null;
    }
  }

  Future<String?> _get(Uri uri) async {
    final r = await _fetch(uri);
    return r != null && r.$1 == HttpStatus.ok ? r.$2 : null;
  }

  Future<LiveSeekResult> _seek(Map<String, String> query) async {
    final r = await _fetch(seekUri.replace(queryParameters: query));
    if (r == null) return const LiveSeekResult.refused('unreachable');
    try {
      final json = jsonDecode(r.$2);
      if (json is Map<String, Object?>) return LiveSeekResult.fromJson(json);
    } on FormatException {
      // Below.
    }
    return const LiveSeekResult.refused('unreachable');
  }

  @override
  Future<LiveSeekResult> seek({Duration? by, DateTime? at}) => _seek({
    if (by != null) 'by': (by.inMilliseconds / 1000).toStringAsFixed(3),
    if (by == null && at != null) 'at': '${at.millisecondsSinceEpoch}',
  });

  @override
  Future<LiveSeekResult> seekToFirst({DateTime? t0}) => _seek({
    'first': '1',
    if (t0 != null) 't0': '${t0.millisecondsSinceEpoch}',
  });

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
    this.playhead,
    this.dvr,
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

  /// Program date-time under the playhead.
  final DateTime? playhead;

  /// The DVR window (TWI-025); null before the first playlist.
  final LiveDvrWindow? dvr;

  /// How far behind the live edge ("−N мин").
  Duration? get behind => dvr?.behind;

  /// Rewound on purpose.
  bool get rewound => dvr?.active ?? false;

  /// There is something to go back to (a window of at least 10 s) and no
  /// ad plays.
  bool get canRewind =>
      seekAllowed &&
      dvr != null &&
      dvr!.length >= const Duration(seconds: 10);

  bool get inAd => ad != null;

  /// Ad time is not watched content (06 §6.4).
  bool get countsAsProgress => !inAd;

  bool get catchingUp => rate > 1.0 + 1e-3;

  /// Close enough to the edge that "go to live" has nothing to do.
  bool get atLiveEdge =>
      !rewound &&
      latency != null &&
      latency! <= target + const Duration(seconds: 1);

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
      other.prefetch == prefetch &&
      other.playhead == playhead &&
      other.dvr == dvr;

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
    playhead,
    dvr,
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
    this.seekDebounce = const Duration(milliseconds: 400),
  });

  final PlayerEngine engine;
  final LiveTelemetry telemetry;

  /// The proxy's playlist URL.
  final String url;

  /// Speed up when behind (the "catch-up" switch of the player).
  bool catchUp;
  final Duration interval;

  /// Quick ±10 s presses (or a held key) add up for this long before the
  /// stream reloads once (TWI-025).
  final Duration seekDebounce;

  final ValueNotifier<LiveStatus> _status = ValueNotifier(const LiveStatus());
  final ValueNotifier<Duration> _pendingSeek = ValueNotifier(Duration.zero);
  final ValueNotifier<LiveSeekResult?> _lastSeek = ValueNotifier(null);
  Timer? _timer;
  Timer? _seekTimer;
  bool _ticking = false;
  bool _disposed = false;

  ValueListenable<LiveStatus> get status => _status;

  /// The ±N s added up and not sent yet (the OSD shows it at once).
  ValueListenable<Duration> get pendingSeek => _pendingSeek;

  /// The last DVR seek (the UI says why one was refused or stopped at an
  /// ad).
  ValueListenable<LiveSeekResult?> get lastSeek => _lastSeek;

  /// DVR seeking is possible: the telemetry is a [LiveSeeker].
  bool get canSeek => telemetry is LiveSeeker;

  /// Opens the stream and starts polling.
  Future<void> start() async {
    await engine.open([url], autoLoadSubtitles: false);
    startPolling();
  }

  /// Starts polling only: a player screen opened [url] itself (TWI-014).
  void startPolling() {
    if (_disposed) return;
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
        playhead: r.playhead,
        dvr: r.dvr,
      );
      // The stats overlay (UI-013) shows these rows next to the engine's.
      final e = engine;
      if (e is StatsEngine) {
        (e as StatsEngine).setExtraStats(_status.value.info);
      }
    } finally {
      _ticking = false;
    }
  }

  /// "К эфиру": reload at the live edge. Not during an ad (false). After a
  /// rewind the proxy stops at an ad break the player has not played yet.
  Future<bool> goToLive() async {
    if (!_status.value.seekAllowed) return false;
    _seekTimer?.cancel();
    _pendingSeek.value = Duration.zero;
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

  /// DVR (TWI-025): [delta] from the playhead within the whole live window.
  /// Quick repeats add up for [seekDebounce], then the stream reloads once.
  /// False when it cannot seek now (an ad plays, no DVR).
  bool seekBy(Duration delta) {
    if (_disposed || !canSeek || !_status.value.seekAllowed) return false;
    _pendingSeek.value += delta;
    _seekTimer?.cancel();
    if (seekDebounce == Duration.zero) {
      unawaited(flushSeek());
    } else {
      _seekTimer = Timer(seekDebounce, () => unawaited(flushSeek()));
    }
    return true;
  }

  /// Sends the added-up [pendingSeek] now; public for tests.
  Future<LiveSeekResult?> flushSeek() async {
    _seekTimer?.cancel();
    final delta = _pendingSeek.value;
    if (delta == Duration.zero) return null;
    return _seek(by: delta);
  }

  /// DVR: to the program date-time [at] (the seek bar).
  Future<LiveSeekResult?> seekTo(DateTime at) async {
    if (_disposed || !canSeek || !_status.value.seekAllowed) return null;
    _seekTimer?.cancel();
    return _seek(at: at);
  }

  Future<LiveSeekResult?> _seek({Duration? by, DateTime? at}) async {
    final seeker = telemetry;
    if (seeker is! LiveSeeker) return null;
    final r = await (seeker as LiveSeeker).seek(by: by, at: at);
    _pendingSeek.value = Duration.zero;
    if (_disposed) return r;
    _lastSeek.value = r;
    if (!r.ok) return r;
    if (engine.state.value.rate > 1.0 + 1e-3 && _status.value.catchingUp) {
      await engine.setRate(1.0);
    }
    // The proxy starts the reloaded playlist at the new position.
    await engine.open([url], autoLoadSubtitles: false);
    final s = _status.value;
    _status.value = LiveStatus(
      mode: s.mode,
      target: s.target,
      prefetch: s.prefetch,
      dvr: s.dvr == null || r.live
          ? s.dvr
          : LiveDvrWindow(
              start: s.dvr!.start,
              length: s.dvr!.length,
              behind: r.at == null
                  ? s.dvr!.behind
                  : s.dvr!.start.add(s.dvr!.length).difference(r.at!),
              active: true,
              pending: true,
            ),
    );
    return r;
  }

  Future<void> dispose() async {
    _disposed = true;
    _timer?.cancel();
    _seekTimer?.cancel();
    await telemetry.close();
    _status.dispose();
    _pendingSeek.dispose();
    _lastSeek.dispose();
  }
}

extension on LiveReport {
  /// [rate] is one of the catch-up rates (1.05× or 1.1×).
  bool inCatchUpRange(double rate) =>
      (rate - 1.05).abs() < 1e-3 || (rate - 1.1).abs() < 1e-3;
}
