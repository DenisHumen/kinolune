// SPDX-License-Identifier: MPL-2.0
//
// TWI-025: live DVR in the native Twitch player and «Смотреть с начала
// эфира». ←/→ and the seek bar go to the proxy over the whole live window,
// End goes live, nothing seeks during an ad; "watch from the start" plays
// the stream's VOD from 0 and switches to live at the same moment once the
// VOD reaches the live window (or straight to live near the edge). Fake
// proxy telemetry, fake engines; no network. With KINOLUNE_SCREENSHOTS_DIR
// set the frames are written to
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/twi-025 \
//   flutter test test/twitch_live_dvr_test.dart

import 'package:flutter/services.dart' show LogicalKeyboardKey;
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/twitch/chat/chat_model.dart';
import 'package:kinolune/screens/twitch/twitch_player_screen.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/twitch_from_start.dart';
import 'package:kinolune/twitch/twitch_models.dart';
import 'package:kinolune/twitch/twitch_native.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

/// The live edge in the fake reports.
const _edge = 1800000000000;

/// When the fake stream started: 2 h before the edge.
final _started = DateTime.fromMillisecondsSinceEpoch(_edge - 7200000);

const _liveUrl = 'http://127.0.0.1:9/token/live.m3u8';
const _vodUrl = 'http://127.0.0.1:9/vod/live.m3u8';

/// A live report with a 10 min DVR window.
Map<String, Object?> _live({
  double behind = 2.0,
  bool active = false,
  Map<String, Object?>? ad,
}) => {
  'mode': 'low_latency',
  'latency': behind + 0.5,
  'target': 3.0,
  'rate': 1.0,
  'should_jump': false,
  'ad': ad,
  'seek_allowed': ad == null,
  'low_latency': {'prefetch': 2, 'parts': 0},
  'totals': {'segments': 10, 'ad_segments': 0},
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

/// A report of the VOD proxy with the playhead at [playheadMs].
Map<String, Object?> _vod(int playheadMs) => {
  'mode': 'stable',
  'target': 8.0,
  'rate': 1.0,
  'seek_allowed': true,
  'playhead_ms': playheadMs,
  'edge_ms': _edge - 120000,
  'dvr': {
    'window_start_ms': _started.millisecondsSinceEpoch,
    'window_seconds': 7080.0,
    'active': true,
    'pending': false,
  },
};

class _Telemetry implements LiveTelemetry, LiveSeeker {
  _Telemetry(this.next);

  Map<String, Object?>? next;
  Map<String, Object?> answer = const {'live': false};
  final seeks = <String>[];
  int resets = 0;

  @override
  Future<LiveReport?> report({
    required double cacheAhead,
    required double rate,
  }) async {
    final r = next;
    return r == null ? null : LiveReport.fromJson(r);
  }

  @override
  Future<void> resetPlayhead() async => resets++;

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
    return const LiveSeekResult();
  }

  @override
  Future<void> close() async {}
}

class _Session implements TwitchLiveSession {
  _Session(this.playlistUrl, this.tel);

  final _Telemetry tel;
  bool closed = false;

  @override
  final String playlistUrl;

  @override
  LiveTelemetry telemetry() => tel;

  @override
  void setMode(LiveLatencyMode mode) {}

  @override
  void setVariant(String mediaPlaylistUrl) {}

  @override
  void close() => closed = true;
}

class _Source implements TwitchNativeSource {
  final live = _Telemetry(_live());
  final vod = _Telemetry(null);
  final resolved = <TwitchPlayTarget>[];
  final vodSessions = <_Session>[];

  @override
  Future<TwitchNativeStream> resolve(TwitchPlayTarget target) async {
    resolved.add(target);
    return TwitchNativeStream([
      TwitchQuality(
        id: 'chunked',
        label: '1080p60 (source)',
        url: target.isLive
            ? 'https://usher.test/live/chunked.m3u8'
            : 'https://usher.test/vod/${target.videoId}/chunked.m3u8',
        isSource: true,
        height: 1080,
      ),
    ]);
  }

  @override
  TwitchLiveSession startLive(String url, LiveLatencyMode mode) =>
      _Session(_liveUrl, live);

  @override
  TwitchLiveSession openVod(String url) {
    final s = _Session(_vodUrl, vod);
    vodSessions.add(s);
    return s;
  }
}

final _video = TwitchVideo(
  id: '9001',
  userId: '77',
  login: 'pixelheron',
  displayName: 'PixelHeron',
  title: 'Campaign day 12',
  publishedAt: _started.add(const Duration(seconds: 4)),
  duration: const Duration(hours: 1, minutes: 58),
);

class _Harness {
  final source = _Source();
  final engines = <FakePlayerEngine>[];
  final modes = <LiveLatencyMode?>[];
  (TwitchVideo, DateTime)? found = (_video, _started);
}

Future<_Harness> _play(WidgetTester t) async {
  setSurface(t, const Size(1440, 900));
  final services = await twitchServices(twitchNative: true);
  final h = _Harness();
  services.twitch.nativePlayer = (context, target) => openTwitchNative(
    context,
    target,
    source: h.source,
    engineFactory: (mode) {
      final e = FakePlayerEngine();
      h.engines.add(e);
      h.modes.add(mode);
      return e;
    },
    chat: (context, login) async => ManualChatFeed(channel: login),
    findStreamVod: (login) async => h.found,
  );
  await t.pumpWidget(testApp(services, section: Section.twitch));
  await settle(t);
  final context = t.element(find.byType(Scaffold).first);
  // ignore: unawaited_futures
  services.twitch.nativePlayer!(
    context,
    const TwitchPlayTarget.live('pixelheron'),
  );
  await settle(t, 14);
  return h;
}

TwitchPlayerScreenState _state(WidgetTester t) =>
    t.state<TwitchPlayerScreenState>(find.byType(TwitchPlayerScreen));

PlayerScreenState _player(WidgetTester t) =>
    t.state<PlayerScreenState>(find.byType(PlayerScreen));

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 4);
}

int _opens(FakePlayerEngine e) =>
    e.calls.where((c) => c.startsWith('open')).length;

void main() {
  setUpAll(loadAppFonts);

  group('the stream VOD', () {
    final start = DateTime.utc(2026, 11, 24, 18);
    TwitchVideo video(String id, Duration after, {TwitchVideoType? type}) =>
        TwitchVideo(
          id: id,
          userId: '77',
          login: 'pixelheron',
          displayName: 'PixelHeron',
          title: id,
          publishedAt: start.add(after),
          duration: const Duration(hours: 1),
          type: type ?? TwitchVideoType.archive,
        );

    test('the archive created closest to the stream start', () {
      final videos = [
        video('yesterday', const Duration(hours: -26)),
        video('highlight', Duration.zero, type: TwitchVideoType.highlight),
        video('late', const Duration(minutes: 3)),
        video('this', const Duration(seconds: 5)),
      ];
      expect(pickStreamVod(start, videos)?.id, 'this');
      expect(pickStreamVod(start, videos.take(3).toList())?.id, 'late');
      // Not saved (or not listed yet).
      expect(pickStreamVod(start, videos.take(2).toList()), isNull);
      expect(pickStreamVod(null, videos), isNull);
    });
  });

  group('catching up from the start', () {
    final edge = DateTime.fromMillisecondsSinceEpoch(_edge);
    final window = edge.subtract(const Duration(minutes: 10));
    FromStartStep at(Duration beforeEdge) => fromStartStep(
      vodPlayhead: edge.subtract(beforeEdge),
      windowStart: window,
      edge: edge,
    );

    test('VOD → live window → live', () {
      // Before the live window: the VOD goes on.
      expect(at(const Duration(minutes: 40)), FromStartStep.stay);
      expect(at(const Duration(minutes: 9, seconds: 55)), FromStartStep.stay);
      // Inside it: live at the same moment.
      expect(at(const Duration(minutes: 9)), FromStartStep.toWindow);
      expect(at(const Duration(seconds: 30)), FromStartStep.toWindow);
      // At the edge: live.
      expect(at(const Duration(seconds: 4)), FromStartStep.toLive);
      // Nothing known yet: stay.
      expect(
        fromStartStep(vodPlayhead: null, windowStart: window, edge: edge),
        FromStartStep.stay,
      );
      expect(
        fromStartStep(vodPlayhead: edge, windowStart: null, edge: null),
        FromStartStep.stay,
      );
    });
  });

  testWidgets('DVR: back over the window, −N мин, End goes live', (t) async {
    final h = await _play(t);
    final engine = h.engines.single;
    expect(engine.uris, [_liveUrl]);
    _player(t).osd.show();
    await settle(t, 2);
    // A 10 min window: the seek bar, and at the edge "LIVE".
    expect(find.byKey(const ValueKey('twitch-dvr-slider')), findsOneWidget);
    expect(find.byKey(const ValueKey('twitch-from-start')), findsOneWidget);
    expect(find.byKey(const ValueKey('twitch-go-live')), findsNothing);

    // ← twice (OSD hidden): −20 s, one reload.
    _player(t).osd.hide();
    await settle(t, 2);
    final opens = _opens(engine);
    await t.sendKeyEvent(LogicalKeyboardKey.arrowLeft);
    await t.sendKeyEvent(LogicalKeyboardKey.arrowLeft);
    await settle(t, 6);
    expect(h.source.live.seeks, ['by:-20.000']);
    expect(_opens(engine), opens + 1);
    expect(engine.uris, [_liveUrl]);

    // The proxy reports the rewound playhead: "−1 min" and "Go live".
    h.source.live.next = _live(behind: 80, active: true);
    await settle(t, 8);
    _player(t).osd.show();
    await settle(t, 2);
    expect(find.byKey(const ValueKey('twitch-behind')), findsOneWidget);
    expect(find.text('−1 min'), findsOneWidget);
    expect(find.byKey(const ValueKey('twitch-go-live')), findsOneWidget);
    await capture(t, '01-live-dvr-rewound');

    // End: back to live.
    await t.sendKeyEvent(LogicalKeyboardKey.end);
    await settle(t, 2);
    expect(h.source.live.resets, 1);
    await _close(t);
  });

  testWidgets('no seeking during an ad', (t) async {
    final h = await _play(t);
    h.source.live.next = _live(ad: {'id': 'stitched-ad-1', 'remaining': 20.0});
    await settle(t, 8);
    _player(t).seekBy(const Duration(seconds: -10));
    await settle(t, 6);
    expect(h.source.live.seeks, isEmpty);
    // Not from the start either.
    await _state(t).watchFromStart();
    await settle(t, 4);
    expect(h.source.resolved.where((r) => !r.isLive), isEmpty);
    expect(find.text('No rewinding during an ad'), findsOneWidget);
    await _close(t);
  });

  testWidgets('from the start: the VOD, then live at the same moment', (
    t,
  ) async {
    final h = await _play(t);
    await _state(t).watchFromStart();
    await settle(t, 8);
    // The stream's VOD from its first segment, times counted from the start.
    expect(h.source.resolved.last.videoId, '9001');
    expect(h.source.vod.seeks, ['first:${_started.millisecondsSinceEpoch}']);
    expect(h.modes.last, isNull, reason: 'a VOD engine');
    expect(h.engines.last.uris, [_vodUrl]);
    expect(_state(t).fromStart, isNotNull);
    expect(_state(t).playingLive, isFalse);

    // Still before the live window: the VOD goes on.
    final windowStart = _edge - 600000;
    h.source.vod.next = _vod(windowStart - 60000);
    await _state(t).checkCatchUp();
    await settle(t, 2);
    expect(_state(t).fromStart, isNotNull);
    _player(t).osd.show();
    await settle(t, 2);
    expect(
      find.byKey(const ValueKey('twitch-from-start-position')),
      findsOneWidget,
    );
    expect(
      find.textContaining('From the start · 1:49:00 · −11 min'),
      findsOneWidget,
    );
    await capture(t, '02-from-the-start');

    // Sped up and caught up: live, at the moment the VOD reached.
    final at = windowStart + 30000;
    h.source.vod.next = _vod(at);
    await _state(t).checkCatchUp();
    await settle(t, 8);
    expect(h.source.live.seeks, ['at:$at']);
    expect(_state(t).fromStart, isNull);
    expect(h.source.vodSessions.single.closed, isTrue);
    expect(h.modes.last, LiveLatencyMode.lowLatency);
    expect(h.engines.last.uris, [_liveUrl]);
    await _close(t);
  });

  testWidgets('from the start near the edge: straight to live', (t) async {
    final h = await _play(t);
    await _state(t).watchFromStart();
    await settle(t, 8);
    h.source.vod.next = _vod(_edge - 1000);
    await _state(t).checkCatchUp();
    await settle(t, 8);
    expect(h.source.live.resets, 1);
    expect(h.source.live.seeks, isEmpty);
    expect(_state(t).fromStart, isNull);
    await _close(t);
  });

  testWidgets('no recording of the stream: says so, stays live', (t) async {
    final h = await _play(t);
    h.found = null;
    await _state(t).watchFromStart();
    await settle(t, 4);
    expect(find.textContaining('has no recording'), findsOneWidget);
    expect(_state(t).fromStart, isNull);
    expect(h.source.vodSessions, isEmpty);
    await _close(t);
  });
}
