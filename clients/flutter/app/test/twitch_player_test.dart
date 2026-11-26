// SPDX-License-Identifier: MPL-2.0
//
// TWI-014: the native Twitch player. Route (store builds and a switched-off
// flag keep the embed), qualities and audio only, VOD chapters, speeds and
// progress under `twitch:vod:<id>`, live latency / ad badge / "Go live" /
// latency mode over a fake live proxy, and the chat per form factor. Fake
// usher, fake proxy, fake engine, made-up chat; no network.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/features/twitch/chat/chat_model.dart';
import 'package:kinolune/integration/library.dart' show Progress;
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/twitch/twitch_player_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sync/device_sync.dart';
import 'package:kinolune/twitch/twitch_models.dart';
import 'package:kinolune/twitch/twitch_native.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

const _desktop = Size(1440, 900);
const _phonePortrait = Size(390, 844);
const _phoneLandscape = Size(844, 390);
const _tv = Size(1920, 1080);

List<TwitchQuality> _qualities(String base) => [
  TwitchQuality(
    id: '480p30',
    label: '480p30',
    url: '$base/480p30.m3u8',
    height: 480,
    frameRate: 30,
    bandwidth: 1400000,
  ),
  TwitchQuality(
    id: 'audio_only',
    label: 'audio_only',
    url: '$base/audio_only.m3u8',
    audioOnly: true,
    bandwidth: 160000,
  ),
  TwitchQuality(
    id: 'chunked',
    label: '1080p60 (source)',
    url: '$base/chunked.m3u8',
    isSource: true,
    height: 1080,
    frameRate: 60,
    bandwidth: 6000000,
  ),
  TwitchQuality(
    id: '720p60',
    label: '720p60',
    url: '$base/720p60.m3u8',
    height: 720,
    frameRate: 60,
    bandwidth: 3000000,
  ),
];

/// The proxy's telemetry: answers every poll with [report].
class _Telemetry implements LiveTelemetry {
  Map<String, Object?>? next = _report();
  int resets = 0;
  int polls = 0;

  @override
  Future<LiveReport?> report({
    required double cacheAhead,
    required double rate,
  }) async {
    polls++;
    final r = next;
    return r == null ? null : LiveReport.fromJson(r);
  }

  @override
  Future<void> resetPlayhead() async => resets++;

  @override
  Future<void> close() async {}
}

Map<String, Object?> _report({
  double latency = 2.5,
  double target = 3.0,
  Map<String, Object?>? ad,
}) => {
  'mode': 'low_latency',
  'latency': latency,
  'target': target,
  'rate': 1.0,
  'should_jump': latency > target + 20,
  'ad': ad,
  'seek_allowed': ad == null,
  'low_latency': {'prefetch': 2, 'parts': 0},
  'totals': {'segments': 10, 'ad_segments': ad == null ? 0 : 2},
};

class _Session implements TwitchLiveSession {
  _Session(
    this.variant,
    this.mode,
    this.telemetry_, {
    this.playlistUrl = 'http://127.0.0.1:9/token/live.m3u8',
  });

  String variant;
  LiveLatencyMode mode;
  final _Telemetry telemetry_;
  bool closed = false;

  @override
  final String playlistUrl;

  @override
  LiveTelemetry telemetry() => telemetry_;

  @override
  void setMode(LiveLatencyMode m) => mode = m;

  @override
  void setVariant(String url) => variant = url;

  @override
  void close() => closed = true;
}

class _Source implements TwitchNativeSource {
  TwitchNativeError? error;
  final resolved = <TwitchPlayTarget>[];
  final sessions = <_Session>[];
  final vodSessions = <_Session>[];
  final telemetry = _Telemetry();

  @override
  Future<TwitchNativeStream> resolve(TwitchPlayTarget target) async {
    resolved.add(target);
    final e = error;
    if (e != null) throw e;
    final base = target.isLive
        ? 'https://usher.test/live/${target.login}'
        : 'https://usher.test/vod/${target.videoId}';
    return TwitchNativeStream(_qualities(base));
  }

  @override
  TwitchLiveSession startLive(String url, LiveLatencyMode mode) {
    final s = _Session(url, mode, telemetry);
    sessions.add(s);
    return s;
  }

  @override
  TwitchLiveSession openVod(String url) {
    final s = _Session(
      url,
      LiveLatencyMode.stable,
      telemetry,
      playlistUrl: _vodProxy,
    );
    vodSessions.add(s);
    return s;
  }
}

/// The loopback proxy in front of a VOD variant (usher is HTTPS only, our
/// FFmpeg has no TLS).
const _vodProxy = 'http://127.0.0.1:9/token/vod.m3u8';

/// Engines the player created, with the live mode of each.
class _Engines {
  final engines = <FakePlayerEngine>[];
  final modes = <LiveLatencyMode?>[];

  FakePlayerEngine get last => engines.last;

  PlayerEngine create(LiveLatencyMode? mode) {
    final e = FakePlayerEngine(loadedDuration: const Duration(minutes: 245));
    engines.add(e);
    modes.add(mode);
    return e;
  }
}

class _Harness {
  _Harness(this.services);

  final AppServices services;
  final source = _Source();
  final engines = _Engines();
  final chats = <ManualChatFeed>[];

  void register() {
    services.twitch.nativePlayer = (context, target) => openTwitchNative(
      context,
      target,
      source: source,
      engineFactory: engines.create,
      chat: (context, login) async {
        final f = ManualChatFeed(channel: login);
        chats.add(f);
        return f;
      },
    );
  }
}

Future<_Harness> _harness({bool native = true, DeviceSync? sync}) async {
  final services = await twitchServices(twitchNative: native, sync: sync);
  return _Harness(services)..register();
}

Future<void> _tap(WidgetTester t, Finder f) async {
  // The OSD hides after 4 s of playing: bring it back like a user would.
  final player = find.byType(PlayerScreen);
  if (player.evaluate().isNotEmpty) {
    t.state<PlayerScreenState>(player.first).osd.show();
    await settle(t, 3);
  }
  await t.ensureVisible(f.first);
  await settle(t, 4);
  await t.tap(f.first);
  await settle(t, 4);
}

/// Opens [target] in the player from inside the app.
Future<void> _play(
  WidgetTester t,
  _Harness h,
  TwitchPlayTarget target, {
  FormFactor formFactor = FormFactor.desktop,
}) async {
  await t.pumpWidget(
    testApp(h.services, section: Section.twitch, formFactor: formFactor),
  );
  await settle(t);
  final context = t.element(find.byType(Scaffold).first);
  // ignore: unawaited_futures
  h.services.twitch.nativePlayer!(context, target);
  await settle(t, 14);
}

TwitchPlayerScreenState _state(WidgetTester t) =>
    t.state<TwitchPlayerScreenState>(find.byType(TwitchPlayerScreen));

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 4);
}

final _markers = find.byWidgetPredicate(
  (w) =>
      w.key is ValueKey<String> &&
      (w.key! as ValueKey<String>).value.startsWith('chapter-marker-'),
);

void main() {
  group('qualities', () {
    final stream = TwitchNativeStream(_qualities('u'));

    test('source first, audio only last', () {
      expect(stream.qualities.map((q) => q.id), [
        'chunked',
        '720p60',
        '480p30',
        'audio_only',
      ]);
      expect(stream.source?.id, 'chunked');
      expect(stream.audioOnly?.id, 'audio_only');
    });

    test('the remembered choice', () {
      expect(pickQuality(stream, null)?.id, 'chunked');
      expect(pickQuality(stream, 'source')?.id, 'chunked');
      expect(pickQuality(stream, 'audio_only')?.id, 'audio_only');
      expect(pickQuality(stream, '720')?.id, '720p60');
      expect(pickQuality(stream, '600')?.id, '480p30');
      // Below every variant: the smallest picture.
      expect(pickQuality(stream, '240')?.id, '480p30');
      expect(pickQuality(stream, 'nonsense')?.id, 'chunked');
      expect(pickQuality(TwitchNativeStream(const []), null), isNull);
      // No audio-only variant: the source.
      final video = TwitchNativeStream(
        _qualities('u').where((q) => !q.audioOnly).toList(),
      );
      expect(pickQuality(video, 'audio_only')?.id, 'chunked');
      expect(qualityPreference(stream.qualities[0]), 'source');
      expect(qualityPreference(stream.qualities[1]), '720');
      expect(qualityPreference(stream.qualities[3]), 'audio_only');
    });
  });

  test('the lab flag: never in release builds, loopback only', () {
    const lab = 'http://127.0.0.1:18714';
    // A release build ignores it: native playback needs the signed config.
    expect(twitchLabOrigin(lab, release: true), isNull);
    expect(twitchLabOrigin(lab, release: false), Uri.parse(lab));
    expect(twitchLabOrigin('http://localhost:9', release: false), isNotNull);
    expect(twitchLabOrigin('http://[::1]:9', release: false), isNotNull);
    // Never a real usher or any other host.
    for (final v in [
      'https://usher.ttvnw.net',
      'http://192.168.1.20:18714',
      'http://127.0.0.1.example.com',
      'file:///tmp/x',
      '',
    ]) {
      expect(twitchLabOrigin(v, release: false), isNull, reason: v);
    }
    // The tests run in debug: the default follows kReleaseMode.
    expect(twitchLabOrigin(lab), Uri.parse(lab));
  });

  test('chapters on the player timeline', () {
    final c = playbackChapters(const [
      TwitchChapter(
        gameId: '1',
        gameName: 'Hollow Lanterns',
        start: Duration.zero,
        end: Duration(minutes: 40),
      ),
      TwitchChapter(
        gameId: '2',
        gameName: 'Talk & Tea',
        start: Duration(minutes: 40),
        approximate: true,
      ),
      TwitchChapter(gameId: '3', start: Duration(minutes: 90)),
    ]);
    expect(
      c[0],
      const PlaybackChapter(
        start: Duration.zero,
        end: Duration(minutes: 40),
        title: 'Hollow Lanterns',
      ),
    );
    expect(c[1].title, '≈ Talk & Tea');
    expect(c[2].title, isNull);
  });

  test('chat layout per form factor', () {
    TwitchChatLayout at(
      Size s,
      FormFactor f, {
      bool on = true,
      bool th = false,
    }) => twitchChatLayout(size: s, formFactor: f, chatOn: on, theater: th);
    expect(at(_desktop, FormFactor.desktop), TwitchChatLayout.side);
    expect(
      at(_desktop, FormFactor.desktop, th: true),
      TwitchChatLayout.overlay,
    );
    expect(at(_desktop, FormFactor.desktop, on: false), TwitchChatLayout.none);
    expect(
      at(const Size(700, 500), FormFactor.desktop),
      TwitchChatLayout.overlay,
    );
    expect(at(_phonePortrait, FormFactor.phone), TwitchChatLayout.below);
    expect(at(_phoneLandscape, FormFactor.phone), TwitchChatLayout.overlay);
    expect(
      at(const Size(1194, 834), FormFactor.desktop),
      TwitchChatLayout.side,
    );
    // A tablet is an Android "phone" with room: beside in landscape,
    // below in portrait.
    expect(at(const Size(1280, 800), FormFactor.phone), TwitchChatLayout.side);
    expect(at(const Size(800, 1280), FormFactor.phone), TwitchChatLayout.below);
    expect(at(_tv, FormFactor.tv), TwitchChatLayout.side);
    // PLAY-AND-008: a PiP window shows only the video.
    expect(
      twitchChatLayout(
        size: const Size(320, 180),
        formFactor: FormFactor.phone,
        chatOn: true,
        theater: false,
        pip: true,
      ),
      TwitchChatLayout.none,
    );
  });

  group('VOD progress', () {
    test('saved under twitch:vod:<id>, not before a real position', () async {
      final saved = <Progress>[];
      final sync = FakeDeviceSync(id: 'dev-a', name: 'Desk');
      final services = await demoServices(sync: sync);
      final p = TwitchVodProgress(
        videoId: '9101',
        saveProgress: (x) async => saved.add(x),
        handoff: services.handoff,
        clock: () => DateTime(2026, 11, 14),
      );
      expect(p.itemId, 'twitch:vod:9101');
      await p.report(0, 3600000);
      await p.report(5000, 0);
      expect(saved, isEmpty);
      await p.report(60000, 3600000);
      await p.report(60000, 3600000);
      await p.finished(3600000);
      await p.report(70000, 3600000);
      expect([for (final x in saved) x.positionMs], [60000, 3600000]);
      expect(saved.last.watched, isTrue);
      expect(saved.every((x) => x.itemId == 'twitch:vod:9101'), isTrue);
      p.close();
    });
  });

  group('route', () {
    testWidgets('store builds and a switched-off flag keep the embed', (
      t,
    ) async {
      setSurface(t, _desktop);
      final h = await _harness(native: false);
      expect(h.services.twitch.nativePlayer, isNotNull);
      var embedded = 0;
      h.services.twitch.embedPlayer = (context, target) async => embedded++;
      expect(h.services.twitch.playRoute, TwitchPlayRoute.embed);
      await t.pumpWidget(testApp(h.services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron')));
      await _tap(t, find.byKey(const ValueKey('twitch-watch-live')));
      expect(embedded, 1);
      expect(h.source.resolved, isEmpty);
      expect(find.byType(TwitchPlayerScreen), findsNothing);
      // The decision itself.
      TwitchPlayRoute route({
        bool full = true,
        bool allowed = true,
        bool native = true,
        bool embed = true,
      }) => decideTwitchRoute(
        fullBuild: full,
        nativeAllowedByConfig: allowed,
        hasNativePlayer: native,
        hasEmbedPlayer: embed,
      );
      expect(route(), TwitchPlayRoute.native);
      expect(route(full: false), TwitchPlayRoute.embed);
      expect(route(allowed: false), TwitchPlayRoute.embed);
      expect(route(native: false), TwitchPlayRoute.embed);
      expect(route(allowed: false, embed: false), TwitchPlayRoute.openOnTwitch);
      await _close(t);
    });

    testWidgets('a full build with the flag on plays natively', (t) async {
      setSurface(t, _desktop);
      final h = await _harness();
      expect(h.services.twitch.playRoute, TwitchPlayRoute.native);
      await t.pumpWidget(testApp(h.services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron')));
      await _tap(t, find.byKey(const ValueKey('twitch-watch-live')));
      expect(find.byType(TwitchPlayerScreen), findsOneWidget);
      expect(h.source.resolved.single.login, 'pixelheron');
      await _close(t);
    });
  });

  group('VOD', () {
    testWidgets('chapters, speeds up to 3×, quality, progress', (t) async {
      setSurface(t, _desktop);
      final sync = FakeDeviceSync(id: 'dev-a', name: 'Desk');
      final h = await _harness(sync: sync);
      await t.pumpWidget(testApp(h.services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron')));
      await _tap(t, find.byKey(const ValueKey('twitch-vod-9101')));
      expect(find.byType(TwitchPlayerScreen), findsOneWidget);
      // Source quality, resumed where the progress was; no live chat.
      final e = h.engines.last;
      expect(h.engines.modes.single, isNull);
      // The player opens the loopback proxy, never usher's HTTPS directly.
      expect(e.uris, [_vodProxy]);
      final vod = h.source.vodSessions.single;
      expect(vod.variant, 'https://usher.test/vod/9101/chunked.m3u8');
      expect(h.source.sessions, isEmpty);
      expect(e.calls.first, startsWith('open:1:0:${95 * 60000}:'));
      expect(
        t.widget<Text>(find.byKey(const ValueKey('osd-title'))).data,
        'Campaign day 11: the long siege',
      );
      expect(h.chats, isEmpty);
      expect(find.byKey(const ValueKey('twitch-chat-toggle')), findsNothing);
      // TWI-008 chapters on the scrubber (none at 0:00).
      expect(_markers, findsNWidgets(2));

      // Speeds 0.5–3×.
      await _tap(t, find.byKey(const ValueKey('osd-speed')));
      expect(find.byKey(const ValueKey('speed-3.0')), findsOneWidget);
      expect(find.byKey(const ValueKey('speed-0.5')), findsOneWidget);
      await _tap(t, find.byKey(const ValueKey('speed-3.0')));
      expect(e.state.value.rate, 3.0);

      // Progress under twitch:vod:<id> while playing.
      e.emit((s) => s.copyWith(position: const Duration(minutes: 100)));
      await t.pump(const Duration(seconds: 6));
      await settle(t, 2);
      final p = await h.services.library.progress('twitch:vod:9101');
      expect(p?.positionMs, const Duration(minutes: 100).inMilliseconds);
      expect(sync.started, contains('twitch:vod:9101'));

      // Another quality reopens at the same position and is remembered.
      await _tap(t, find.byKey(const ValueKey('twitch-quality')));
      expect(
        find.byKey(const ValueKey('twitch-quality-dialog')),
        findsOneWidget,
      );
      expect(find.text('1080p60 (source)'), findsWidgets);
      expect(find.text('Audio only'), findsOneWidget);
      await _tap(t, find.byKey(const ValueKey('twitch-quality-720p60')));
      expect(h.engines.engines, hasLength(2));
      expect(e.disposed, isTrue);
      expect(h.engines.last.uris, [_vodProxy]);
      expect(vod.variant, 'https://usher.test/vod/9101/720p60.m3u8');
      expect(
        h.engines.last.calls.firstWhere((c) => c.startsWith('open')),
        startsWith('open:1:0:${100 * 60000}:'),
      );
      // The speed carries over to the new engine.
      expect(h.engines.last.state.value.rate, 3.0);
      expect(
        await h.services.preferences.read(Preferences.twitchQuality),
        '720',
      );

      // Audio only: the note over the black picture.
      await _tap(t, find.byKey(const ValueKey('twitch-quality')));
      await _tap(t, find.byKey(const ValueKey('twitch-quality-audio_only')));
      expect(find.byKey(const ValueKey('twitch-audio-only')), findsOneWidget);
      expect(h.engines.last.uris, [_vodProxy]);
      expect(vod.variant, 'https://usher.test/vod/9101/audio_only.m3u8');
      expect(
        h.source.vodSessions,
        hasLength(1),
        reason: 'one proxy per screen',
      );
      await _close(t);
      expect(vod.closed, isTrue);
      expect(sync.heartbeats.last.state.name, 'stopped');
    });
  });

  testWidgets('VOD from Continue: stored chapters and the title', (t) async {
    setSurface(t, _desktop);
    final h = await _harness();
    await _play(
      t,
      h,
      const TwitchPlayTarget.vod(
        '9101',
        title: 'Campaign day 11: the long siege',
        startAt: Duration(minutes: 5),
      ),
    );
    expect(_markers, findsNWidgets(2));
    expect(
      t.widget<Text>(find.byKey(const ValueKey('osd-title'))).data,
      'Campaign day 11: the long siege',
    );
    await _close(t);
  });

  group('live', () {
    testWidgets('latency, ad badge, go live, latency mode, chat beside', (
      t,
    ) async {
      setSurface(t, _desktop);
      final h = await _harness();
      await _play(t, h, const TwitchPlayTarget.live('pixelheron'));
      final session = h.source.sessions.single;
      expect(
        session.variant,
        'https://usher.test/live/pixelheron/chunked.m3u8',
      );
      expect(session.mode, LiveLatencyMode.lowLatency);
      expect(h.engines.modes.single, LiveLatencyMode.lowLatency);
      // The player opens the proxy, not usher.
      expect(h.engines.last.uris, [session.playlistUrl]);
      // No ±10 s buttons and no scrubber: the live bar.
      expect(find.byKey(const ValueKey('osd-back10')), findsNothing);
      expect(find.byKey(const ValueKey('osd-scrubber')), findsNothing);
      expect(find.byKey(const ValueKey('twitch-live-bar')), findsOneWidget);
      expect(find.text('LIVE'), findsOneWidget);
      expect(find.text('Delay 2.5 s'), findsOneWidget);
      // At the edge: nothing to go to.
      expect(find.byKey(const ValueKey('twitch-go-live')), findsNothing);
      // The latency in the stats panel (UI-013 rows).
      final osd = t.state<PlayerScreenState>(find.byType(PlayerScreen));
      osd.setStatsVisible(true);
      await settle(t, 8);
      expect(h.engines.last.state.value.stats['Live latency'], '2.5 s / 3.0 s');
      osd.setStatsVisible(false);
      await settle(t, 2);

      // Chat beside the video on a wide screen.
      expect(h.chats.single.channel, 'pixelheron');
      final chat = t.getRect(find.byKey(const ValueKey('twitch-player-chat')));
      final video = t.getRect(
        find.byKey(const ValueKey('twitch-player-video')),
      );
      expect(chat.left, greaterThanOrEqualTo(video.right - 1));

      // An ad break: the badge, no seeking, no speeding up.
      h.source.telemetry.next = _report(
        ad: {'id': 'stitched-ad-1', 'remaining': 15.0},
      );
      await settle(t, 8);
      expect(find.text('Ad · 0:15'), findsOneWidget);
      final s = _state(t);
      expect(s.liveController!.status.value.seekAllowed, isFalse);
      final before = h.engines.last.calls.length;
      t
          .state<PlayerScreenState>(find.byType(PlayerScreen))
          .seekBy(const Duration(seconds: -10));
      await settle(t, 2);
      expect(
        h.engines.last.calls.skip(before).where((c) => c.startsWith('seek')),
        isEmpty,
      );
      expect(h.engines.last.state.value.rate, 1.0);

      // Behind the edge: "Go live" resets the proxy and reopens.
      h.source.telemetry.next = _report(latency: 14);
      await settle(t, 8);
      expect(find.text('Ad · 0:15'), findsNothing);
      expect(find.text('Delay 14.0 s'), findsOneWidget);
      await _tap(t, find.byKey(const ValueKey('twitch-go-live')));
      expect(h.source.telemetry.resets, 1);
      expect(
        h.engines.last.calls.where((c) => c.startsWith('open')),
        hasLength(2),
      );

      // "Stable": the proxy switches and a new engine gets its options.
      h.source.telemetry.next = _report();
      await _tap(t, find.byKey(const ValueKey('twitch-latency-mode')));
      expect(find.text('Low latency'), findsOneWidget);
      await _tap(t, find.byKey(const ValueKey('twitch-mode-stable')));
      expect(session.mode, LiveLatencyMode.stable);
      expect(h.engines.modes, [
        LiveLatencyMode.lowLatency,
        LiveLatencyMode.stable,
      ]);
      expect(
        await h.services.preferences.read(Preferences.twitchLatencyMode),
        'stable',
      );

      // Quality: the proxy follows the variant.
      await _tap(t, find.byKey(const ValueKey('twitch-quality')));
      await _tap(t, find.byKey(const ValueKey('twitch-quality-480p30')));
      expect(session.variant, 'https://usher.test/live/pixelheron/480p30.m3u8');
      expect(h.engines.last.uris, [session.playlistUrl]);

      // Theater: the chat over the video; hidden on request.
      await _tap(t, find.byKey(const ValueKey('twitch-theater')));
      expect(find.byKey(const ValueKey('twitch-chat-overlay')), findsOneWidget);
      await _tap(t, find.byKey(const ValueKey('twitch-chat-toggle')));
      expect(find.byKey(const ValueKey('twitch-player-chat')), findsNothing);

      await _close(t);
      expect(session.closed, isTrue);
    });

    testWidgets('phone: chat below in portrait, over the video in landscape', (
      t,
    ) async {
      setSurface(t, _phonePortrait);
      final h = await _harness();
      await _play(
        t,
        h,
        const TwitchPlayTarget.live('pixelheron'),
        formFactor: FormFactor.phone,
      );
      final chat = t.getRect(find.byKey(const ValueKey('twitch-player-chat')));
      final video = t.getRect(
        find.byKey(const ValueKey('twitch-player-video')),
      );
      expect(chat.top, greaterThanOrEqualTo(video.bottom - 1));
      // At least 16:9, tall enough for the controls.
      expect(video.height, greaterThanOrEqualTo(video.width * 9 / 16));
      expect(video.height, lessThanOrEqualTo(340));
      await _close(t);

      setSurface(t, _phoneLandscape);
      final h2 = await _harness();
      await _play(
        t,
        h2,
        const TwitchPlayTarget.live('pixelheron'),
        formFactor: FormFactor.phone,
      );
      expect(find.byKey(const ValueKey('twitch-chat-overlay')), findsOneWidget);
      expect(find.byKey(const ValueKey('twitch-theater')), findsNothing);
      await _close(t);
    });

    testWidgets('TV: chat off at first, a column when turned on', (t) async {
      setSurface(t, _tv);
      final h = await _harness();
      await _play(
        t,
        h,
        const TwitchPlayTarget.live('pixelheron'),
        formFactor: FormFactor.tv,
      );
      expect(find.byKey(const ValueKey('twitch-player-chat')), findsNothing);
      await _tap(t, find.byKey(const ValueKey('twitch-chat-toggle')));
      final chat = t.getRect(find.byKey(const ValueKey('twitch-player-chat')));
      expect(chat.width, closeTo(_tv.width * 0.28, 1));
      await _close(t);
    });

    testWidgets('offline channel; a failure offers the Twitch player', (
      t,
    ) async {
      setSurface(t, _desktop);
      final h = await _harness();
      h.source.error = const TwitchNativeError('unavailable');
      await _play(t, h, const TwitchPlayTarget.live('pixelheron'));
      expect(
        find.byKey(const ValueKey('twitch-player-failed')),
        findsOneWidget,
      );
      expect(find.textContaining('is offline right now'), findsOneWidget);
      expect(
        find.byKey(const ValueKey('twitch-player-use-embed')),
        findsNothing,
      );
      expect(h.source.sessions, isEmpty);

      // Retry after it comes online.
      h.source.error = null;
      await _tap(t, find.text('Try again'));
      expect(find.byKey(const ValueKey('twitch-live-bar')), findsOneWidget);
      await _close(t);

      final h2 = await _harness();
      final embedded = <TwitchPlayTarget>[];
      h2.services.twitch.embedPlayer = (context, target) async =>
          embedded.add(target);
      h2.source.error = const TwitchNativeError(
        'integrity_required',
        fallsBackToEmbed: true,
      );
      await _play(
        t,
        h2,
        const TwitchPlayTarget.vod('9101', login: 'pixelheron'),
      );
      expect(
        find.text(
          'The stream did not start. Check the connection or '
          'try the Twitch player.',
        ),
        findsOneWidget,
      );
      await _tap(t, find.byKey(const ValueKey('twitch-player-use-embed')));
      expect(embedded.single.videoId, '9101');
      await settle(t, 8);
      expect(find.byType(TwitchPlayerScreen), findsNothing);
      await _close(t);
    });
  });

  // PLAY-022: the sleep timer in the native Twitch player — live and VOD,
  // phone, computer and TV, ru/en/uk; never «в конце серии».
  group('sleep timer', () {
    const texts = {
      'ru': (m30: '30 мин', end: 'В конце этой серии или фильма'),
      'en': (m30: '30 min', end: 'At the end of this episode or movie'),
      'uk': (m30: '30 хв', end: 'Наприкінці цієї серії або фільму'),
    };
    const devices = {
      FormFactor.phone: _phoneLandscape,
      FormFactor.desktop: _desktop,
      FormFactor.tv: _tv,
    };
    for (final MapEntry(key: form, value: size) in devices.entries) {
      for (final MapEntry(key: lang, value: x) in texts.entries) {
        for (final live in [true, false]) {
          final kind = live ? 'live' : 'VOD';
          testWidgets(
            '${form.name} $lang $kind: menu without «в конце серии»',
            (t) async {
              setSurface(t, size);
              final h = await _harness();
              await h.services.setLocale(Locale(lang));
              await _play(
                t,
                h,
                live
                    ? const TwitchPlayTarget.live('pixelheron')
                    : const TwitchPlayTarget.vod('9101', login: 'pixelheron'),
                formFactor: form,
              );
              await _tap(t, find.byKey(const ValueKey('osd-sleep')));
              expect(find.byKey(const ValueKey('osd-panel')), findsOneWidget);
              expect(find.text(x.m30), findsOneWidget);
              expect(find.text(x.end), findsNothing);
              expect(
                find.byKey(const ValueKey('sleep-ask-more')),
                findsNothing,
              );
              await t.tap(find.text(x.m30));
              await settle(t, 4);
              final sleep = h.services.sleepTimer;
              expect(sleep.option, SleepTimerOption.minutes30);
              expect(t.takeException(), isNull);
              // The fade and the pause reach the engine.
              final player = t.state<PlayerScreenState>(
                find.byType(PlayerScreen),
              );
              final engine = h.engines.last;
              player.sleepFade(0.5);
              expect(engine.state.value.volume, 50);
              player.sleepPause();
              player.sleepFade(1);
              expect(engine.state.value.playing, isFalse);
              expect(engine.state.value.volume, 100);
              await _close(t);
              // Leaving the player cancels the timer.
              expect(sleep.active, isFalse);
            },
          );
        }
      }
    }
  });

  testWidgets('Russian strings', (t) async {
    setSurface(t, _desktop);
    final h = await _harness();
    await h.services.setLocale(const Locale('ru'));
    await _play(t, h, const TwitchPlayTarget.live('pixelheron'));
    expect(find.text('В ЭФИРЕ'), findsOneWidget);
    expect(find.text('Задержка 2.5 с'), findsOneWidget);
    h.source.telemetry.next = _report(
      ad: {'id': 'stitched-ad-1', 'remaining': 15.0},
    );
    await settle(t, 8);
    expect(find.text('Реклама · 0:15'), findsOneWidget);
    h.source.telemetry.next = _report(latency: 14);
    await settle(t, 8);
    expect(find.text('К эфиру'), findsOneWidget);
    final l = t.element(find.byType(TwitchPlayerScreen)).l10n;
    expect(l.twitchPlayerLowLatency, 'Низкая задержка');
    expect(l.twitchPlayerStable, 'Стабильно');
    expect(l.twitchPlayerAudioOnly, 'Только звук');
    await _close(t);
  });
}
