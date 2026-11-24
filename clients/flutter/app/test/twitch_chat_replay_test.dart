// SPDX-License-Identifier: MPL-2.0
//
// TWI-016: VOD chat replay. The sync of [ReplayChatFeed] with the player
// (binary search, forward play, seeks back and ahead, pause, the ±N s
// delay, prefetch of the next stretch, failures), the panel (delay
// buttons, «Чат записи недоступен») and the native player on a desktop
// (chat beside the video in step with the engine, resizable and
// remembered width, theater overlay). Store builds open no replay at all.
// Made-up lines, fake usher, fake engine; no network.
//
// With KINOLUNE_SCREENSHOTS_DIR set (full flavor) the frames are written
// there:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/twi-016 \
//     flutter test --dart-define=KINOLUNE_FLAVOR=full \
//     test/twitch_chat_replay_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/twitch/chat/chat_controller.dart';
import 'package:kinolune/features/twitch/chat/chat_model.dart';
import 'package:kinolune/features/twitch/chat/chat_settings.dart';
import 'package:kinolune/features/twitch/chat/replay_feed.dart';
import 'package:kinolune/features/twitch/chat/replay_panel.dart';
import 'package:kinolune/flavor.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/twitch/twitch_player_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/theme/kinolune_theme.dart';
import 'package:kinolune/twitch/twitch_native.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

const _desktop = Size(1440, 900);

/// A line every [everyMs] from 0 to [untilMs]: `m<offset>`.
List<ReplayLine> _lines({int everyMs = 1000, int untilMs = 600000}) => [
  for (var t = 0; t <= untilMs; t += everyMs)
    ReplayLine(
      t,
      ChatLine(
        id: 'm$t',
        login: 'viewer_${t ~/ 1000 % 7}',
        displayName: 'Viewer_${t ~/ 1000 % 7}',
        color: '#1E90FF',
        badges: t % 3000 == 0
            ? const [ChatBadge(setId: 'subscriber', version: '12')]
            : const [],
        fragments: [
          ChatFragment.text('message at ${t ~/ 1000} s '),
          if (t % 5000 == 0) const ChatFragment.emote('Kappa', 'twitch:25'),
        ],
      ),
    ),
];

/// Feeds the updates into a controller like the panel does.
(ReplayChatFeed, ChatController, ManualReplaySource) _replay({
  List<ReplayLine>? lines,
  Duration? step,
  int window = 5,
  DateTime Function()? clock,
}) {
  final source = ManualReplaySource(lines ?? _lines(), step: step);
  final feed = ReplayChatFeed(source, window: window, clock: clock);
  final c = ChatController(feed, capacity: 500);
  return (feed, c, source);
}

List<String> _ids(ChatController c) => [for (final l in c.lines) l.id];

/// Every line shown is at or before [ms], and nothing before it is
/// missing from the end of the list.
void _matches(ChatController c, int ms) {
  final ids = _ids(c);
  expect(ids, isNotEmpty);
  final offsets = [for (final id in ids) int.parse(id.substring(1))];
  expect(offsets.every((o) => o <= ms), isTrue, reason: 'nothing ahead of $ms');
  expect(offsets.last, ms - ms % 1000, reason: 'the newest line up to $ms');
}

Future<void> _tick(ReplayChatFeed f, int ms) async {
  f.update(Duration(milliseconds: ms));
  // Chunk futures and the stream controller.
  for (var i = 0; i < 6; i++) {
    await Future<void>.delayed(Duration.zero);
  }
}

class _Session implements TwitchLiveSession {
  @override
  String get playlistUrl => 'http://127.0.0.1:9/token/vod.m3u8';

  @override
  LiveTelemetry telemetry() => throw UnsupportedError('VOD');

  @override
  void setMode(LiveLatencyMode m) {}

  @override
  void setVariant(String url) {}

  @override
  void close() {}
}

class _Source implements TwitchNativeSource {
  @override
  Future<TwitchNativeStream> resolve(TwitchPlayTarget target) async =>
      TwitchNativeStream([
        TwitchQuality(
          id: 'chunked',
          label: '1080p60 (source)',
          url: 'https://usher.test/vod/${target.videoId}/chunked.m3u8',
          isSource: true,
          height: 1080,
          frameRate: 60,
        ),
      ]);

  @override
  TwitchLiveSession startLive(String url, LiveLatencyMode mode) =>
      throw UnsupportedError('live');

  @override
  TwitchLiveSession openVod(String url) => _Session();
}

class _Player {
  _Player(this.services, {this.failure});

  final AppServices services;
  final ChatReplayUnavailable? failure;
  final engines = <FakePlayerEngine>[];
  ReplayChatFeed? feed;
  var opened = 0;

  FakePlayerEngine get engine => engines.last;

  void register() {
    services.twitch.nativePlayer = (context, target) => openTwitchNative(
      context,
      target,
      source: _Source(),
      engineFactory: (_) {
        final e = FakePlayerEngine(loadedDuration: const Duration(hours: 3));
        engines.add(e);
        return e;
      },
      replay: (context, target) async {
        opened++;
        final source = ManualReplaySource(
          _lines(everyMs: 700, untilMs: 3600000),
          channel: target.login ?? '',
          failure: failure,
        );
        return feed = ReplayChatFeed(source, window: 40);
      },
    );
  }
}

Future<_Player> _player(WidgetTester t, {ChatReplayUnavailable? failure}) async {
  setSurface(t, _desktop);
  final services = await twitchServices(twitchNative: true);
  final p = _Player(services, failure: failure)..register();
  await t.pumpWidget(testApp(services, section: Section.twitch));
  await settle(t);
  final context = t.element(find.byType(Scaffold).first);
  // ignore: unawaited_futures
  services.twitch.nativePlayer!(
    context,
    const TwitchPlayTarget.vod(
      '9101',
      login: 'pixelheron',
      title: 'Campaign day 11: the long siege',
    ),
  );
  await settle(t, 14);
  return p;
}

/// Text of every chat line on screen.
List<String> _shown(WidgetTester t) => t
    .widgetList<RichText>(find.byType(RichText))
    .map((r) => r.text.toPlainText())
    .toList();

bool _showing(WidgetTester t, String text) =>
    _shown(t).any((s) => s.contains(text));

/// The panel alone, with the app's services and theme.
Widget _panelHost(AppServices services, ReplayChatFeed feed, {Locale? locale}) =>
    Services(
      services: services,
      child: FormFactorScope(
        formFactor: FormFactor.desktop,
        child: MaterialApp(
          locale: locale ?? const Locale('en'),
          supportedLocales: AppLocalizations.supportedLocales,
          localizationsDelegates: kLocalizationsDelegates,
          theme: buildKinoluneTheme(FormFactor.desktop),
          home: Scaffold(body: TwitchReplayChatPanel(feed: feed)),
        ),
      ),
    );

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 4);
}

void main() {
  group('sync with the playhead', () {
    test('forward play adds the lines passed since the last tick', () async {
      final (f, c, _) = _replay();
      await _tick(f, 0);
      expect(f.status.value, ChatReplayStatus.ready);
      expect(_ids(c), ['m0']);
      await _tick(f, 2500);
      expect(_ids(c), ['m0', 'm1000', 'm2000']);
      await _tick(f, 4200);
      expect(_ids(c), ['m0', 'm1000', 'm2000', 'm3000', 'm4000']);
      c.dispose();
    });

    // TWI-016 "done when": after a seek the chat is redrawn and matches the
    // video position.
    test('after a seek the chat matches the video position', () async {
      final (f, c, _) = _replay(window: 5);
      await _tick(f, 4000);
      // Every step is a seek: back, or further ahead than playback goes.
      for (final ms in [300000, 120500, 4000, 599999, 0, 61001, 125000]) {
        await _tick(f, ms);
        _matches(c, ms);
        expect(c.length, lessThanOrEqualTo(5), reason: 'the window at $ms');
        expect(c.following, isTrue);
      }
      // Seek back: only lines before the new position, the last five.
      await _tick(f, 90400);
      expect(_ids(c), ['m86000', 'm87000', 'm88000', 'm89000', 'm90000']);
      c.dispose();
    });

    test('a pause keeps the chat still; speed follows the position', () async {
      final (f, c, _) = _replay();
      await _tick(f, 10000);
      final before = _ids(c);
      for (var i = 0; i < 5; i++) {
        await _tick(f, 10000);
      }
      expect(_ids(c), before, reason: 'paused: the position stands');
      // 2×: two seconds of chat per second of ticks, nothing skipped.
      await _tick(f, 12000);
      await _tick(f, 14000);
      expect(_ids(c).sublist(_ids(c).length - 4), [
        'm11000',
        'm12000',
        'm13000',
        'm14000',
      ]);
      c.dispose();
    });

    test('the delay shifts the chat ±N s and redraws it', () async {
      final (f, c, _) = _replay();
      await _tick(f, 30000);
      expect(_ids(c).last, 'm30000');
      f.setDelay(const Duration(seconds: 5));
      await _tick(f, 30000);
      expect(_ids(c).last, 'm25000', reason: 'chat 5 s later');
      f.setDelay(const Duration(seconds: -3));
      await _tick(f, 30000);
      expect(_ids(c).last, 'm33000', reason: 'chat 3 s sooner');
      f.setDelay(const Duration(minutes: 5));
      expect(f.delay.value, kChatReplayMaxDelay);
      c.dispose();
    });

    test('pages around the playhead, the next one ahead of time', () async {
      final (f, c, s) = _replay(step: const Duration(seconds: 60));
      await _tick(f, 100000);
      expect(s.requests.first, (100000, 20000), reason: 'seek: look back 20 s');
      _matches(c, 100000);
      // 30 s before the end of what is here the next stretch is asked for.
      await _tick(f, 125000);
      expect(s.requests.length, 1);
      await _tick(f, 131000);
      expect(s.requests.last, (160000, 0), reason: 'prefetch from the end');
      await _tick(f, 165000);
      _matches(c, 165000);
      expect(s.requests.length, 2, reason: 'already there: no request');
      c.dispose();
    });

    test('Twitch gives no chat: unavailable, retried later', () async {
      var now = DateTime(2026, 11, 25, 12);
      final (f, c, s) = _replay(clock: () => now);
      s.failure = const ChatReplayUnavailable('integrity_required');
      await _tick(f, 5000);
      expect(f.status.value, ChatReplayStatus.unavailable);
      expect(f.failure, 'integrity_required');
      await _tick(f, 6000);
      expect(s.requests.length, 1, reason: 'no hammering');
      now = now.add(const Duration(seconds: 11));
      s.failure = null;
      await _tick(f, 7000);
      expect(f.status.value, ChatReplayStatus.ready);
      expect(_ids(c).last, 'm7000');
      c.dispose();
    });

    test('filters reload the lines; sending is refused', () async {
      final (f, c, s) = _replay();
      await _tick(f, 8000);
      c.applySettings(const ChatSettings(blocked: ['viewer_3']));
      await _tick(f, 8000);
      expect(s.filters?.blocked, ['viewer_3']);
      expect(s.requests.length, 2, reason: 'asked again with the new rules');
      expect(f.canSend, isFalse);
      expect(
        await f.send('hi'),
        isA<ChatSendFailed>().having(
          (e) => e.error,
          'error',
          ChatSendError.readOnly,
        ),
      );
      c.dispose();
      expect(s.closed, isTrue, reason: 'the controller closes the feed');
    });
  });

  group('panel', () {
    testWidgets('delay buttons and the in-step mark', (t) async {
      setSurface(t, const Size(420, 800));
      final services = await twitchServices();
      final f = ReplayChatFeed(ManualReplaySource(_lines()));
      await t.pumpWidget(_panelHost(services, f));
      f.update(const Duration(seconds: 20));
      await settle(t);
      final l = t.element(find.byType(TwitchReplayChatPanel)).l10n;
      expect(find.text(l.twitchReplayChatTitle), findsOneWidget);
      expect(find.text(l.twitchReplayChatInSync), findsOneWidget);
      expect(_showing(t, 'message at 20 s'), isTrue);
      await t.tap(find.byKey(const ValueKey('twitch-replay-later')));
      await t.tap(find.byKey(const ValueKey('twitch-replay-later')));
      await settle(t, 3);
      expect(f.delay.value, const Duration(seconds: 2));
      expect(find.text(l.twitchReplayChatLater(2)), findsOneWidget);
      expect(_showing(t, 'message at 18 s'), isTrue);
      expect(_showing(t, 'message at 20 s'), isFalse);
      await t.tap(find.byKey(const ValueKey('twitch-replay-sync')));
      await settle(t, 3);
      expect(f.delay.value, Duration.zero);
      await t.tap(find.byKey(const ValueKey('twitch-replay-sooner')));
      await settle(t, 3);
      expect(find.text(l.twitchReplayChatSooner(1)), findsOneWidget);
      // No input: the replay is read-only.
      expect(find.byType(TextField), findsNothing);
      await _close(t);
    });

    testWidgets('«Чат записи недоступен» when Twitch refuses', (t) async {
      setSurface(t, const Size(420, 800));
      final services = await twitchServices();
      final source = ManualReplaySource(
        _lines(),
        failure: const ChatReplayUnavailable('gql_hash_invalid'),
      );
      final f = ReplayChatFeed(source);
      await t.pumpWidget(_panelHost(services, f, locale: const Locale('ru')));
      f.update(const Duration(seconds: 3));
      await settle(t);
      expect(find.text('Чат записи недоступен'), findsOneWidget);
      expect(find.byKey(const ValueKey('twitch-replay-unavailable')), findsOne);
      await _close(t);
    });
  });

  group('native player on a desktop', () {
    testWidgets('store builds have no chat replay', (t) async {
      final p = await _player(t);
      final state = t.state<TwitchPlayerScreenState>(
        find.byType(TwitchPlayerScreen),
      );
      if (AppFlavor.twitchNative) {
        expect(p.opened, 1);
        expect(state.replay, isNotNull);
      } else {
        expect(p.opened, 0, reason: 'the factory is never called');
        expect(state.replay, isNull);
        expect(find.byType(TwitchReplayChatPanel), findsNothing);
      }
      await _close(t);
    });

    testWidgets(
      'chat beside the video follows the engine, seeks and pauses',
      (t) async {
        final p = await _player(t);
        final l = t.element(find.byType(TwitchPlayerScreen)).l10n;
        expect(find.byType(TwitchReplayChatPanel), findsOneWidget);
        final video = t.getRect(
          find.byKey(const ValueKey('twitch-player-video')),
        );
        final chat = t.getRect(find.byType(TwitchReplayChatPanel));
        expect(chat.left, greaterThanOrEqualTo(video.right), reason: 'side');
        expect(chat.width, inInclusiveRange(300, 420));

        p.engine.emit(
          (s) => s.copyWith(position: const Duration(minutes: 12), playing: true),
        );
        await settle(t, 4);
        expect(find.text(l.twitchReplayChatTitle), findsOneWidget);
        // 12:00 = 720000 ms; lines every 700 ms: the newest is 719600.
        expect(_showing(t, 'message at 719 s'), isTrue);
        expect(_showing(t, 'message at 721 s'), isFalse);
        await capture(t, '01-vod-chat-replay-desktop');

        // Seek back to 5:00.
        await p.engine.seek(const Duration(minutes: 5));
        await settle(t, 4);
        expect(_showing(t, 'message at 299 s'), isTrue);
        expect(_showing(t, 'message at 719 s'), isFalse);
        final c = t
            .widget<TwitchReplayChatPanel>(find.byType(TwitchReplayChatPanel))
            .feed;
        expect(c.held, greaterThan(0));

        // Paused: the position stands, the chat too.
        await p.engine.pause();
        await settle(t, 6);
        expect(_showing(t, 'message at 300 s'), isFalse);
        await _close(t);
      },
      skip: !AppFlavor.twitchNative,
    );

    testWidgets(
      'drag the handle: the chat width changes and is remembered',
      (t) async {
        final p = await _player(t);
        final before = t.getRect(find.byType(TwitchReplayChatPanel)).width;
        await t.drag(
          find.byKey(const ValueKey('twitch-chat-resize')),
          const Offset(-120, 0),
        );
        await settle(t, 3);
        final after = t.getRect(find.byType(TwitchReplayChatPanel)).width;
        // 120 px less the touch slop the drag needs to start.
        expect(after, greaterThan(before + 80));
        expect(after, lessThanOrEqualTo(before + 120));
        final saved = await p.services.preferences.read(
          Preferences.twitchChatWidth,
        );
        expect(double.parse(saved!), closeTo(after, 1));
        // The theater mode puts the chat over the video.
        final state = t.state<TwitchPlayerScreenState>(
          find.byType(TwitchPlayerScreen),
        );
        state.toggleTheater();
        await settle(t, 3);
        expect(find.byKey(const ValueKey('twitch-chat-overlay')), findsOne);
        state.toggleChat();
        await settle(t, 3);
        expect(find.byKey(const ValueKey('twitch-chat-overlay')), findsNothing);
        await _close(t);
      },
      skip: !AppFlavor.twitchNative,
    );

    testWidgets(
      'unavailable chat: the video plays, the panel says so',
      (t) async {
        final p = await _player(
          t,
          failure: const ChatReplayUnavailable('not_found'),
        );
        p.engine.emit((s) => s.copyWith(position: const Duration(minutes: 1)));
        await settle(t, 4);
        final l = t.element(find.byType(TwitchPlayerScreen)).l10n;
        expect(find.text(l.twitchReplayChatUnavailable), findsOneWidget);
        expect(find.byType(PlayerScreen), findsOneWidget);
        await capture(t, '02-vod-chat-replay-unavailable');
        await _close(t);
      },
      skip: !AppFlavor.twitchNative,
    );
  });

}
