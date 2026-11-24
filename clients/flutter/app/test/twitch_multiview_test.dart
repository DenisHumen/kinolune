// SPDX-License-Identifier: MPL-2.0
//
// TWI-022: multiview of 2–4 live streams. Layouts and their slots, the
// heard tile (one at a time), the quality cap per number of tiles, arrows
// / OK / Back on a TV, the chat of the heard stream on a desktop, where it
// is not offered (phone, an embed that shows one stream at a time, a
// window too small for several embeds), and the native tile over a fake
// usher and a fake engine. No network.

import 'dart:async';

import 'package:flutter/foundation.dart' show ValueListenable, ValueNotifier;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/twitch/chat/chat_model.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/twitch/twitch_multiview_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/twitch_models.dart';
import 'package:kinolune/twitch/twitch_multiview.dart';
import 'package:kinolune/twitch/twitch_native.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

const _desktop = Size(1440, 900);
const _tv = Size(1920, 1080);
const _phone = Size(390, 844);

const _four = ['alpha', 'bravo', 'charlie', 'delta'];

class _FakeStream implements MultiviewStream {
  _FakeStream(this.login, {required this.muted, required this.quality});

  final String login;
  bool muted;
  String quality;
  bool disposed = false;
  final _status = ValueNotifier(MultiviewStreamStatus.playing);

  @override
  ValueListenable<MultiviewStreamStatus> get status => _status;

  @override
  void setMuted(bool m) => muted = m;

  @override
  void setQuality(String q) => quality = q;

  @override
  Widget build(BuildContext context) =>
      ColoredBox(key: ValueKey('video-$login'), color: Colors.blueGrey);

  @override
  void dispose() => disposed = true;
}

class _Streams {
  final all = <String, _FakeStream>{};

  _FakeStream operator [](String login) => all[login]!;

  MultiviewStream create(
    String login, {
    required bool muted,
    required String quality,
  }) => all[login] = _FakeStream(login, muted: muted, quality: quality);

  List<String> get heard => [
    for (final s in all.values)
      if (!s.muted && !s.disposed) s.login,
  ];
}

TwitchStream _live(String login) => TwitchStream(
  userId: 'id-$login',
  login: login,
  displayName: login.toUpperCase(),
  title: 'Stream of $login',
  viewers: 100,
);

/// A route: [native] (full build, the config allows it, a native source)
/// or the official embed.
Future<AppServices> _services({bool native = true}) async {
  final services = await twitchServices(twitchNative: native);
  final twitch = services.twitch;
  if (native) {
    twitch.nativePlayer = (context, target) async {};
    registerTwitchMultiview(twitch, _Source());
  }
  twitch.embedPlayer = (context, target) async {};
  return services;
}

Future<void> _open(
  WidgetTester t,
  AppServices services,
  _Streams streams, {
  List<String> initial = _four,
  List<TwitchStream>? live,
  FormFactor formFactor = FormFactor.desktop,
  bool multiEmbed = true,
  List<ManualChatFeed>? chats,
}) async {
  await t.pumpWidget(
    testApp(services, section: Section.twitch, formFactor: formFactor),
  );
  await settle(t);
  final context = t.element(find.byType(Scaffold).first);
  unawaited(
    openTwitchMultiview(
      context,
      initial: initial,
      live: live ?? [for (final l in _four) _live(l)],
      streams: streams.create,
      multiEmbed: multiEmbed,
      chat: (context, login) async {
        final f = ManualChatFeed(channel: login);
        chats?.add(f);
        return f;
      },
    ),
  );
  await settle(t, 6);
}

TwitchMultiviewScreenState _state(WidgetTester t) =>
    t.state<TwitchMultiviewScreenState>(find.byType(TwitchMultiviewScreen));

Finder _tile(String login) =>
    find.byKey(ValueKey('twitch-multiview-tile-$login'));

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

void main() {
  group('layouts', () {
    test('which layouts fit how many streams', () {
      expect(multiviewLayoutsFor(1), [MultiviewLayout.pair]);
      expect(multiviewLayoutsFor(2), [MultiviewLayout.pair]);
      expect(multiviewLayoutsFor(3), [
        MultiviewLayout.grid,
        MultiviewLayout.focus,
      ]);
      expect(multiviewDefaultLayout(4), MultiviewLayout.grid);
    });

    test('slots: 1+1, 2×2 and one big with three small', () {
      const area = Size(1600, 900);
      final pair = multiviewSlots(MultiviewLayout.pair, 2, area, gap: 0);
      expect(pair, [
        const Rect.fromLTWH(0, 0, 800, 900),
        const Rect.fromLTWH(800, 0, 800, 900),
      ]);
      // A portrait tablet stacks them.
      final stacked = multiviewSlots(
        MultiviewLayout.pair,
        2,
        const Size(800, 1200),
        gap: 0,
      );
      expect(stacked[1], const Rect.fromLTWH(0, 600, 800, 600));
      expect(multiviewSlots(MultiviewLayout.pair, 1, area), [
        Offset.zero & area,
      ]);
      final grid = multiviewSlots(MultiviewLayout.grid, 4, area, gap: 0);
      expect(grid[3], const Rect.fromLTWH(800, 450, 800, 450));
      final focus = multiviewSlots(MultiviewLayout.focus, 4, area, gap: 0);
      expect(focus[0], const Rect.fromLTWH(0, 0, 1200, 900));
      expect(focus[2], const Rect.fromLTWH(1200, 300, 400, 300));
    });

    test('one big: the heard stream takes the big slot', () {
      expect(multiviewSlotTiles(MultiviewLayout.focus, 4, 2), [2, 0, 1, 3]);
      expect(multiviewSlotTiles(MultiviewLayout.grid, 4, 2), [0, 1, 2, 3]);
      // Three in a grid: the fourth place is free ("add a stream").
      expect(multiviewSlotTiles(MultiviewLayout.grid, 3, 0), [0, 1, 2, null]);
      expect(multiviewSlotTiles(MultiviewLayout.pair, 2, 1), [0, 1]);
    });

    test('quality cap per number of tiles', () {
      expect(multiviewQuality(MultiviewLayout.pair, 1, 0), '1080');
      expect(multiviewQuality(MultiviewLayout.pair, 2, 1), '720');
      for (var slot = 0; slot < 4; slot++) {
        expect(multiviewQuality(MultiviewLayout.grid, 4, slot), '720');
      }
      expect(multiviewQuality(MultiviewLayout.focus, 4, 0), '1080');
      expect(multiviewQuality(MultiviewLayout.focus, 4, 3), '480');
      expect(multiviewMaxHeight('720'), 720);
      expect(multiviewMaxHeight('source'), 0);
    });

    test('arrows between tiles', () {
      final grid = multiviewSlots(
        MultiviewLayout.grid,
        4,
        const Size(1600, 900),
      );
      expect(multiviewNeighbor(grid, 0, AxisDirection.right), 1);
      expect(multiviewNeighbor(grid, 0, AxisDirection.down), 2);
      expect(multiviewNeighbor(grid, 3, AxisDirection.left), 2);
      expect(multiviewNeighbor(grid, 3, AxisDirection.up), 1);
      expect(multiviewNeighbor(grid, 0, AxisDirection.up), isNull);
      expect(multiviewNeighbor(grid, 1, AxisDirection.right), isNull);
      final focus = multiviewSlots(
        MultiviewLayout.focus,
        4,
        const Size(1600, 900),
      );
      // From the big tile right: the small one in line with its middle.
      expect(multiviewNeighbor(focus, 0, AxisDirection.right), 2);
      expect(multiviewNeighbor(focus, 1, AxisDirection.down), 2);
      expect(multiviewNeighbor(focus, 3, AxisDirection.left), 0);
      expect(multiviewNeighbor(focus, 3, AxisDirection.down), isNull);
      // Up from the full-height big tile: nothing (the bar).
      expect(multiviewNeighbor(focus, 0, AxisDirection.up), isNull);
    });

    test('the embed needs 400×300 per player', () {
      expect(
        multiviewFitsEmbed(
          multiviewSlots(MultiviewLayout.grid, 4, const Size(1920, 1000)),
        ),
        isTrue,
      );
      expect(
        multiviewFitsEmbed(
          multiviewSlots(MultiviewLayout.focus, 4, const Size(1440, 900)),
        ),
        isFalse,
      );
    });
  });

  group('controller', () {
    test('add, remove, the heard stream and the layout', () {
      final c = MultiviewController(['a', 'b', 'a']);
      expect(c.logins, ['a', 'b']);
      expect(c.audioLogin, 'a');
      expect(c.layout, MultiviewLayout.pair);
      expect(c.add('a'), isFalse);
      expect(c.add('c'), isTrue);
      expect(c.layout, MultiviewLayout.grid);
      c.setAudio(2);
      expect(c.audioLogin, 'c');
      c.setLayout(MultiviewLayout.focus);
      expect(c.qualityOf('c'), '1080');
      expect(c.qualityOf('a'), '480');
      c.remove('a');
      // The heard stream stays the heard one; two left: side by side.
      expect(c.audioLogin, 'c');
      expect(c.layout, MultiviewLayout.pair);
      expect(c.qualityOf('c'), '720');
      c.remove('c');
      expect(c.audioLogin, 'b');
      expect(c.add('d'), isTrue);
      expect(c.add('e'), isTrue);
      expect(c.add('f'), isTrue);
      expect(c.add('g'), isFalse, reason: 'four at most');
      final tablet = MultiviewController(['a', 'b', 'c'], max: 2);
      expect(tablet.logins, ['a', 'b']);
      expect(tablet.canAdd, isFalse);
    });

    test('where multiview is offered', () {
      MultiviewSupport support(
        TwitchPlayRoute route, {
        FormFactor formFactor = FormFactor.desktop,
        Size screen = _desktop,
        bool multiEmbed = true,
      }) => twitchMultiviewSupport(
        route: route,
        hasNativeSource: true,
        formFactor: formFactor,
        screen: screen,
        multiEmbed: multiEmbed,
      );
      expect(support(TwitchPlayRoute.native), MultiviewSupport.native);
      expect(support(TwitchPlayRoute.embed), MultiviewSupport.embed);
      expect(
        support(TwitchPlayRoute.embed, multiEmbed: false),
        MultiviewSupport.embedUnsupported,
      );
      expect(
        support(TwitchPlayRoute.openOnTwitch),
        MultiviewSupport.embedUnsupported,
      );
      expect(
        support(
          TwitchPlayRoute.native,
          formFactor: FormFactor.phone,
          screen: _phone,
        ),
        MultiviewSupport.phone,
      );
      // A tablet: two streams.
      expect(
        support(
          TwitchPlayRoute.native,
          formFactor: FormFactor.phone,
          screen: const Size(1280, 800),
        ),
        MultiviewSupport.native,
      );
      expect(multiviewMaxFor(FormFactor.phone), 2);
      expect(multiviewMaxFor(FormFactor.tv), 4);
    });

    test('embed page: muted, capped, nothing closes the script', () {
      final html = twitchMultiviewEmbedHtml(
        'some</script>',
        muted: true,
        maxHeight: 720,
        parent: 'example.org',
      );
      expect(html, contains('"muted":true'));
      expect(html, contains('"channel":"some\\u003c/script>"'));
      expect(html, contains('var maxHeight = 720;'));
      expect(html, contains('"parent":["example.org"]'));
      expect(html, contains('setMuted'));
    });
  });

  group('screen', () {
    testWidgets('two streams side by side: the first heard, 720p each', (
      t,
    ) async {
      setSurface(t, _desktop);
      final streams = _Streams();
      await _open(t, await _services(), streams, initial: ['alpha', 'bravo']);
      expect(_tile('alpha'), findsOneWidget);
      expect(_tile('bravo'), findsOneWidget);
      expect(streams.heard, ['alpha']);
      expect(streams['alpha'].quality, '720');
      expect(streams['bravo'].quality, '720');
      // Side by side: one layout only, no layout button.
      expect(
        find.byKey(const ValueKey('twitch-multiview-layout')),
        findsNothing,
      );
      // A click gives the sound to the other stream.
      await t.tap(find.byKey(const ValueKey('video-bravo')));
      await settle(t, 4);
      expect(streams.heard, ['bravo']);
      await _close(t);
    });

    testWidgets('four at 720p; one big: 1080p for it, 480p for the rest', (
      t,
    ) async {
      setSurface(t, _desktop);
      final streams = _Streams();
      await _open(t, await _services(), streams);
      for (final l in _four) {
        expect(streams[l].quality, '720', reason: l);
      }
      await t.tap(find.byKey(const ValueKey('twitch-multiview-layout')));
      await settle(t, 4);
      expect(_state(t).controller.layout, MultiviewLayout.focus);
      expect(streams['alpha'].quality, '1080');
      expect(streams['bravo'].quality, '480');
      // The big tile fills three quarters of the width.
      expect(t.getSize(_tile('alpha')).width, greaterThan(1000));
      // Hearing another stream makes it the big one; same players.
      await t.tap(find.byKey(const ValueKey('video-charlie')));
      await settle(t, 4);
      expect(streams.heard, ['charlie']);
      expect(streams['charlie'].quality, '1080');
      expect(streams['alpha'].quality, '480');
      expect(t.getSize(_tile('charlie')).width, greaterThan(1000));
      expect(streams.all.length, 4, reason: 'no player was restarted');
      await _close(t);
    });

    testWidgets('remove and add a stream from the live list', (t) async {
      setSurface(t, _desktop);
      final streams = _Streams();
      await _open(t, await _services(), streams, initial: ['alpha', 'bravo']);
      expect(
        find.byKey(const ValueKey('twitch-multiview-free-2')),
        findsNothing,
      );
      await t.tap(find.byKey(const ValueKey('twitch-multiview-add')));
      await settle(t, 4);
      expect(
        find.byKey(const ValueKey('twitch-multiview-pick-alpha')),
        findsNothing,
        reason: 'already on screen',
      );
      await t.tap(find.byKey(const ValueKey('twitch-multiview-pick-charlie')));
      await settle(t, 2);
      await t.tap(find.byKey(const ValueKey('twitch-multiview-pick-done')));
      await settle(t, 4);
      expect(_tile('charlie'), findsOneWidget);
      expect(_state(t).controller.layout, MultiviewLayout.grid);
      // Three in a 2×2: the fourth place adds a stream.
      expect(
        find.byKey(const ValueKey('twitch-multiview-free-3')),
        findsOneWidget,
      );
      expect(streams['charlie'].muted, isTrue);
      // × on the heard tile: its player stops, the sound goes on.
      final remove = find.descendant(
        of: _tile('alpha'),
        matching: find.byIcon(Icons.close),
      );
      await t.tap(remove);
      await settle(t, 4);
      expect(_tile('alpha'), findsNothing);
      expect(streams['alpha'].disposed, isTrue);
      expect(streams.heard, ['bravo']);
      expect(_state(t).controller.layout, MultiviewLayout.pair);
      await _close(t);
    });

    testWidgets('picker first when nothing is chosen', (t) async {
      setSurface(t, _desktop);
      final streams = _Streams();
      await _open(t, await _services(), streams, initial: const []);
      expect(
        find.byKey(const ValueKey('twitch-multiview-picker')),
        findsOneWidget,
      );
      for (final l in ['bravo', 'delta']) {
        await t.tap(find.byKey(ValueKey('twitch-multiview-pick-$l')));
        await settle(t, 2);
      }
      await t.tap(find.byKey(const ValueKey('twitch-multiview-pick-done')));
      await settle(t, 6);
      expect(_state(t).controller.logins, ['bravo', 'delta']);
      await _close(t);
    });

    testWidgets('TV: arrows move, OK hears, OK again offers remove, Back '
        'closes', (t) async {
      setSurface(t, _tv);
      final streams = _Streams();
      await _open(t, await _services(), streams, formFactor: FormFactor.tv);
      // The heard tile has the focus.
      expect(_state(t).focusedSlot, 0);
      await pressKey(t, LogicalKeyboardKey.arrowRight);
      expect(_state(t).focusedSlot, 1);
      await pressKey(t, LogicalKeyboardKey.arrowDown);
      expect(_state(t).focusedSlot, 3);
      await pressKey(t, LogicalKeyboardKey.arrowLeft);
      expect(_state(t).focusedSlot, 2);
      await pressKey(t, LogicalKeyboardKey.select);
      expect(streams.heard, ['charlie']);
      // No chat on a TV.
      expect(
        find.byKey(const ValueKey('twitch-multiview-chat-toggle')),
        findsNothing,
      );
      // OK on the heard tile: its menu.
      await pressKey(t, LogicalKeyboardKey.select);
      expect(
        find.byKey(const ValueKey('twitch-multiview-tile-menu')),
        findsOneWidget,
      );
      await t.tap(find.byKey(const ValueKey('twitch-multiview-remove')));
      await settle(t, 4);
      expect(_state(t).controller.logins, ['alpha', 'bravo', 'delta']);
      expect(_state(t).focusedSlot, isNotNull);
      // In "one big" the focus follows the tile to the big slot.
      _state(t).controller.setLayout(MultiviewLayout.focus);
      await settle(t, 2);
      await pressKey(t, LogicalKeyboardKey.arrowRight);
      final slot = _state(t).focusedSlot!;
      await pressKey(t, LogicalKeyboardKey.select);
      expect(_state(t).focusedSlot, 0, reason: 'from slot $slot');
      // Up from the top: the bar; Back closes the multiview.
      await pressKey(t, LogicalKeyboardKey.arrowUp);
      expect(_state(t).focusedSlot, isNull);
      await pressKey(t, LogicalKeyboardKey.arrowDown);
      // The remote's Back (KEYCODE_BACK).
      await t.sendKeyEvent(
        LogicalKeyboardKey.goBack,
        platform: 'web',
        physicalKey: PhysicalKeyboardKey.browserBack,
      );
      await settle(t, 4);
      expect(find.byType(TwitchMultiviewScreen), findsNothing);
      expect(streams.all.values.every((s) => s.disposed), isTrue);
      await _close(t);
    });

    testWidgets('desktop: the heard stream\'s chat beside the tiles', (
      t,
    ) async {
      setSurface(t, _desktop);
      final streams = _Streams();
      final chats = <ManualChatFeed>[];
      await _open(t, await _services(), streams, chats: chats);
      expect(find.byKey(const ValueKey('twitch-multiview-chat')), findsNothing);
      await t.tap(find.byKey(const ValueKey('twitch-multiview-chat-toggle')));
      await settle(t, 4);
      expect(
        find.byKey(const ValueKey('twitch-multiview-chat')),
        findsOneWidget,
      );
      expect(chats.map((c) => c.channel), ['alpha']);
      await t.tap(find.byKey(const ValueKey('video-delta')));
      await settle(t, 4);
      expect(chats.map((c) => c.channel), ['alpha', 'delta']);
      expect(_state(t).chatLogin, 'delta');
      // C hides it again.
      await pressKey(t, LogicalKeyboardKey.keyC);
      expect(find.byKey(const ValueKey('twitch-multiview-chat')), findsNothing);
      await _close(t);
    });

    testWidgets('a phone gets a hint', (t) async {
      setSurface(t, _phone);
      final streams = _Streams();
      await _open(t, await _services(), streams, formFactor: FormFactor.phone);
      expect(
        find.byKey(const ValueKey('twitch-multiview-unavailable')),
        findsOneWidget,
      );
      expect(find.byType(TwitchMultiviewScreen), findsNothing);
      await _close(t);
    });

    testWidgets('the embed: one at a time here, or a window too small', (
      t,
    ) async {
      setSurface(t, _desktop);
      var streams = _Streams();
      final services = await _services(native: false);
      await _open(t, services, streams, multiEmbed: false);
      expect(
        find.byKey(const ValueKey('twitch-multiview-unavailable')),
        findsOneWidget,
      );
      expect(streams.all, isEmpty);
      await _close(t);

      setSurface(t, const Size(1000, 700));
      streams = _Streams();
      await _open(t, services, streams);
      expect(find.byType(TwitchMultiviewScreen), findsOneWidget);
      expect(
        find.byKey(const ValueKey('twitch-multiview-too-small')),
        findsOneWidget,
      );
      // Two side by side fit.
      _state(t).removeStream('delta');
      _state(t).removeStream('charlie');
      await settle(t, 4);
      expect(
        find.byKey(const ValueKey('twitch-multiview-too-small')),
        findsNothing,
      );
      expect(_tile('alpha'), findsOneWidget);
      await _close(t);
    });
  });

  group('native tile', () {
    test('muted by volume, quality cap switches the variant once', () async {
      final source = _Source();
      final engines = <FakePlayerEngine>[];
      final s = NativeMultiviewStream(
        'alpha',
        source: source,
        engines: (mode) {
          expect(mode, LiveLatencyMode.lowLatency);
          return engines.addAndReturn(FakePlayerEngine());
        },
        muted: true,
        quality: '720',
      );
      await _until(() => s.status.value != MultiviewStreamStatus.loading);
      expect(s.status.value, MultiviewStreamStatus.playing);
      expect(s.quality?.id, '720p60');
      final e = engines.single;
      expect(e.calls, contains('volume:0.0'));
      expect(e.uris, [source.sessions.single.playlistUrl]);
      s.setMuted(false);
      await pumpEventQueue();
      expect(e.calls.last, 'volume:100.0');
      // 480 → the 480p variant: the proxy switches, the engine reopens.
      final opens = e.calls.where((c) => c.startsWith('open')).length;
      s.setQuality('480');
      await pumpEventQueue();
      expect(source.sessions.single.variant, endsWith('/480p30.m3u8'));
      expect(e.calls.where((c) => c.startsWith('open')).length, opens + 1);
      // The same variant again: nothing happens.
      s.setQuality('600');
      await pumpEventQueue();
      expect(e.calls.where((c) => c.startsWith('open')).length, opens + 1);
      s.dispose();
      await pumpEventQueue();
      expect(source.sessions.single.closed, isTrue);
      expect(e.disposed, isTrue);
    });

    test('an offline channel says so', () async {
      final source = _Source()..error = const TwitchNativeError('unavailable');
      final s = NativeMultiviewStream(
        'alpha',
        source: source,
        engines: (_) => FakePlayerEngine(),
        muted: false,
        quality: '720',
      );
      await _until(() => s.status.value != MultiviewStreamStatus.loading);
      expect(s.status.value, MultiviewStreamStatus.offline);
      s.dispose();
    });
  });
}

extension<T> on List<T> {
  T addAndReturn(T v) {
    add(v);
    return v;
  }
}

Future<void> _until(bool Function() done) async {
  for (var i = 0; i < 100 && !done(); i++) {
    await pumpEventQueue();
  }
}

// ----- Fake usher and proxy ------------------------------------------------

class _Telemetry implements LiveTelemetry {
  @override
  Future<LiveReport?> report({
    required double cacheAhead,
    required double rate,
  }) async => null;

  @override
  Future<void> resetPlayhead() async {}

  @override
  Future<void> close() async {}
}

class _Session implements TwitchLiveSession {
  _Session(this.variant);

  String variant;
  bool closed = false;

  @override
  String get playlistUrl => 'http://127.0.0.1:9/token/live.m3u8';

  @override
  LiveTelemetry telemetry() => _Telemetry();

  @override
  void setMode(LiveLatencyMode m) {}

  @override
  void setVariant(String url) => variant = url;

  @override
  void close() => closed = true;
}

class _Source implements TwitchNativeSource {
  TwitchNativeError? error;
  final sessions = <_Session>[];

  @override
  Future<TwitchNativeStream> resolve(TwitchPlayTarget target) async {
    final e = error;
    if (e != null) throw e;
    final base = 'https://usher.test/live/${target.login}';
    return TwitchNativeStream([
      TwitchQuality(
        id: 'chunked',
        label: '1080p60 (source)',
        url: '$base/chunked.m3u8',
        isSource: true,
        height: 1080,
      ),
      TwitchQuality(
        id: '720p60',
        label: '720p60',
        url: '$base/720p60.m3u8',
        height: 720,
      ),
      TwitchQuality(
        id: '480p30',
        label: '480p30',
        url: '$base/480p30.m3u8',
        height: 480,
      ),
    ]);
  }

  @override
  TwitchLiveSession startLive(String url, LiveLatencyMode mode) {
    final s = _Session(url);
    sessions.add(s);
    return s;
  }

  @override
  TwitchLiveSession openVod(String url) => _Session(url);
}
