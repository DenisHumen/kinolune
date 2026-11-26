// SPDX-License-Identifier: MPL-2.0
//
// TWI-005, TWI-029: the official Twitch embed. The page (parent, live and
// VOD options), its events, VOD position sync into Kinolune progress and the
// handoff session, the embed screen as `TwitchService.embedPlayer` with a
// fake WebView channel, the player choice per platform (webview_flutter,
// WebView2 on Windows, WebKitGTK on Linux, "Open on Twitch" without a
// runtime; native first when the config allows it), the desktop keys and
// fullscreen, the Linux runner channel, and "Open" in the Open on Twitch
// dialog (url_launcher on phones and desktops, QR code only on TVs), and
// the app's chat beside a live embed (TWI-041). No network.

import 'dart:async';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/features/twitch/chat/chat_model.dart';
import 'package:kinolune/integration/library.dart' show Progress;
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/twitch/twitch_embed_desktop.dart';
import 'package:kinolune/screens/twitch/twitch_embed_screen.dart';
import 'package:kinolune/screens/twitch/twitch_play.dart';
import 'package:kinolune/screens/twitch/twitch_widgets.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/external_links.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/twitch_embed.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

const _desktop = Size(1280, 800);

/// The WebView of the tests: records the page and lets the test post the
/// messages the embed page would.
class FakeEmbedSurface implements TwitchEmbedSurface {
  FakeEmbedSurface({
    required this.html,
    required this.baseUrl,
    required this.onMessage,
    required this.onExternalLink,
    this.onFullscreen,
    this.onBack,
  });

  final String html;
  final Uri baseUrl;
  final void Function(String message) onMessage;
  final void Function(Uri url) onExternalLink;
  final void Function(bool fullscreen)? onFullscreen;
  final VoidCallback? onBack;
  final List<String> scripts = [];
  var disposed = false;

  static final List<FakeEmbedSurface> created = [];

  static FakeEmbedSurface factory({
    required String html,
    required Uri baseUrl,
    required void Function(String message) onMessage,
    required void Function(Uri url) onExternalLink,
    void Function(bool fullscreen)? onFullscreen,
    VoidCallback? onBack,
  }) {
    final s = FakeEmbedSurface(
      html: html,
      baseUrl: baseUrl,
      onMessage: onMessage,
      onExternalLink: onExternalLink,
      onFullscreen: onFullscreen,
      onBack: onBack,
    );
    created.add(s);
    return s;
  }

  void post(String event, {num? position, num? duration}) => onMessage(
    '{"event":"$event"'
    '${position == null ? '' : ',"position":$position'}'
    '${duration == null ? '' : ',"duration":$duration'}}',
  );

  @override
  Widget build(BuildContext context) =>
      const SizedBox.expand(key: ValueKey('fake-embed-webview'));

  @override
  Future<void> runJavaScript(String script) async => scripts.add(script);

  @override
  void dispose() => disposed = true;
}

Future<void> _tap(WidgetTester t, Finder f) async {
  await t.ensureVisible(f.first);
  await settle(t, 4);
  await t.tap(f.first);
  await settle(t, 4);
}

void main() {
  group('embed page', () {
    test('live: channel and the parent domain', () {
      const target = TwitchPlayTarget.live('pixelheron');
      final html = twitchEmbedHtml(target, parent: 'example.org');
      expect(html, contains('"channel":"pixelheron"'));
      expect(html, contains('"parent":["example.org"]'));
      expect(html, contains('https://player.twitch.tv/js/embed/v1.js'));
      expect(html, contains('var vod = false;'));
      expect(html, isNot(contains('"video"')));
      expect(
        twitchEmbedBaseUrl('example.org').toString(),
        startsWith('https://example.org/'),
      );
    });

    test('VOD: v-prefixed id and the start time', () {
      const target = TwitchPlayTarget.vod(
        '9101',
        login: 'pixelheron',
        startAt: Duration(hours: 1, minutes: 2, seconds: 3),
      );
      final options = twitchEmbedOptions(target, parent: 'example.org');
      expect(options['video'], 'v9101');
      expect(options['time'], '1h2m3s');
      expect(options.containsKey('channel'), isFalse);
      expect(twitchEmbedHtml(target), contains('var vod = true;'));
      expect(
        twitchEmbedOptions(const TwitchPlayTarget.vod('1')).containsKey('time'),
        isFalse,
      );
    });

    test('SEEK reports the position the event carries', () {
      // On a real VOD getCurrentTime() still returned the old position at
      // SEEK (TWI-014 checks); the event's payload has the new one.
      final html = twitchEmbedHtml(const TwitchPlayTarget.vod('9101'));
      expect(html, contains('seekTo = at;'));
      expect(html, contains('m.position = seekTo;'));
    });

    test('no value can close the script element', () {
      final html = twitchEmbedHtml(
        const TwitchPlayTarget.live('a</script><b>'),
      );
      expect(html, isNot(contains('a</script>')));
      expect(html, contains(r'a\u003c/script>'));
    });

    test('events of the page', () {
      final e = TwitchEmbedEvent.parse(
        '{"event":"time","position":61.5,"duration":3600}',
      )!;
      expect(e.kind, TwitchEmbedEventKind.time);
      expect(e.position, const Duration(milliseconds: 61500));
      expect(e.duration, const Duration(hours: 1));
      expect(TwitchEmbedEvent.parse('{"event":"pause"}')!.position, isNull);
      expect(TwitchEmbedEvent.parse('{"event":"nope"}'), isNull);
      expect(TwitchEmbedEvent.parse('not json'), isNull);
      expect(TwitchEmbedEvent.parse('[1]'), isNull);
    });
  });

  group('position sync', () {
    test('VOD reports become progress under twitch:vod:<id>', () async {
      final saved = <Progress>[];
      final s = TwitchEmbedSession(
        target: const TwitchPlayTarget.vod(
          '9101',
          startAt: Duration(minutes: 10),
        ),
        saveProgress: (p) async => saved.add(p),
        clock: () => DateTime(2026, 11, 14),
      );
      expect(s.itemId, 'twitch:vod:9101');
      // READY before `time` applies: a zero position is not saved.
      await s.handle('{"event":"ready","position":0,"duration":3600}');
      expect(saved, isEmpty);
      await s.handle('{"event":"play","position":600,"duration":3600}');
      expect(s.state.value, TwitchEmbedState.playing);
      await s.handle('{"event":"time","position":605,"duration":3600}');
      await s.handle('{"event":"time","position":605,"duration":3600}');
      await s.handle('{"event":"pause","position":610,"duration":3600}');
      expect(s.state.value, TwitchEmbedState.paused);
      await s.handle('{"event":"seek","position":1800,"duration":3600}');
      expect([for (final p in saved) p.positionMs], [605000, 610000, 1800000]);
      expect(saved.every((p) => p.itemId == 'twitch:vod:9101'), isTrue);
      expect(saved.last.durationMs, 3600000);
      await s.handle('{"event":"ended","position":3600,"duration":3600}');
      expect(saved.last.watched, isTrue);
      expect(s.state.value, TwitchEmbedState.ended);
      await s.close();
      expect(saved.length, 4);
    });

    test('live streams store no progress', () async {
      final saved = <Progress>[];
      final s = TwitchEmbedSession(
        target: const TwitchPlayTarget.live('pixelheron'),
        saveProgress: (p) async => saved.add(p),
      );
      await s.handle('{"event":"ready"}');
      await s.handle('{"event":"play"}');
      await s.handle('{"event":"offline"}');
      expect(s.state.value, TwitchEmbedState.offline);
      await s.close();
      expect(saved, isEmpty);
      expect(s.itemId, isNull);
    });

    test('the script failing to load is reported', () async {
      final s = TwitchEmbedSession(
        target: const TwitchPlayTarget.live('pixelheron'),
        saveProgress: (_) async {},
      );
      await s.handle('{"event":"error","message":"script"}');
      expect(s.state.value, TwitchEmbedState.failed);
    });
  });

  group('embed screen', () {
    setUp(FakeEmbedSurface.created.clear);

    testWidgets('VOD plays in the embed and syncs its position', (t) async {
      setSurface(t, _desktop);
      final sync = FakeDeviceSync(id: 'dev-a', name: 'Desk');
      final services = await twitchServices(sync: sync);
      services.twitch.embedPlayer = (context, target) =>
          openTwitchEmbed(context, target, surface: FakeEmbedSurface.factory);
      expect(services.twitch.playRoute, TwitchPlayRoute.embed);
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron')));
      await _tap(t, find.byKey(const ValueKey('twitch-vod-9101')));
      expect(find.byKey(const ValueKey('twitch-embed')), findsOneWidget);
      expect(find.byKey(const ValueKey('fake-embed-webview')), findsOneWidget);
      final page = FakeEmbedSurface.created.single;
      expect(page.html, contains('"video":"v9101"'));
      expect(page.html, contains('"parent":["$twitchEmbedParent"]'));
      expect(page.baseUrl.host, twitchEmbedParent);

      page.post('ready', position: 0, duration: 5400);
      page.post('play', position: 0, duration: 5400);
      page.post('time', position: 1234, duration: 5400);
      await settle(t, 2);
      final p = await services.library.progress('twitch:vod:9101');
      expect(p?.positionMs, 1234000);
      expect(p?.durationMs, 5400000);
      expect(sync.started, contains('twitch:vod:9101'));
      expect(sync.heartbeats.map((h) => h.itemId), contains('twitch:vod:9101'));

      // "Open on Twitch" continues at the position reached here.
      await _tap(t, find.byKey(const ValueKey('twitch-embed-open')));
      expect(
        find.text('https://www.twitch.tv/videos/9101?t=0h20m34s'),
        findsOneWidget,
      );
      await t.tap(find.widgetWithText(TextButton, 'Close'));
      await settle(t, 4);
      expect(find.byKey(const ValueKey('twitch-open-dialog')), findsNothing);

      page.post('pause', position: 1300, duration: 5400);
      await settle(t, 2);
      await t.tap(
        find.descendant(
          of: find.byKey(const ValueKey('twitch-embed')),
          matching: find.byType(BackButton),
        ),
      );
      await settle(t);
      expect(find.byKey(const ValueKey('twitch-embed')), findsNothing);
      expect(
        (await services.library.progress('twitch:vod:9101'))?.positionMs,
        1300000,
      );
      expect(sync.heartbeats.last.state.name, 'stopped');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('live plays in the embed; a failed load can retry', (t) async {
      setSurface(t, _desktop);
      final services = await twitchServices();
      services.twitch.embedPlayer = (context, target) =>
          openTwitchEmbed(context, target, surface: FakeEmbedSurface.factory);
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron')));
      await _tap(t, find.byKey(const ValueKey('twitch-watch-live')));
      final page = FakeEmbedSurface.created.single;
      expect(page.html, contains('"channel":"pixelheron"'));
      page.post('error');
      await settle(t, 2);
      expect(find.byKey(const ValueKey('twitch-embed-failed')), findsOneWidget);
      await t.tap(find.text('Try again'));
      await settle(t, 4);
      expect(FakeEmbedSurface.created, hasLength(2));
      expect(find.byKey(const ValueKey('fake-embed-webview')), findsOneWidget);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('desktop: fullscreen, Escape, space and arrows', (t) async {
      setSurface(t, _desktop);
      final services = await twitchServices();
      services.twitch.embedPlayer = (context, target) => openTwitchEmbed(
        context,
        target,
        surface: FakeEmbedSurface.factory,
        engine: TwitchEmbedEngine.webKitGtk,
      );
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron')));
      await _tap(t, find.byKey(const ValueKey('twitch-vod-9101')));
      final page = FakeEmbedSurface.created.single;
      page.post('play', position: 600, duration: 5400);
      await settle(t, 2);

      await t.sendKeyEvent(LogicalKeyboardKey.space);
      expect(page.scripts.last, 'window.kinolune.pause();');
      await t.sendKeyEvent(LogicalKeyboardKey.arrowRight);
      expect(page.scripts.last, 'window.kinolune.seek(610);');
      await t.sendKeyEvent(LogicalKeyboardKey.arrowLeft);
      expect(page.scripts.last, 'window.kinolune.seek(590);');

      // F or the bar's button: the player fills the window, no bar.
      expect(
        find.byKey(const ValueKey('twitch-embed-fullscreen')),
        findsOneWidget,
      );
      await t.sendKeyEvent(LogicalKeyboardKey.keyF);
      await settle(t, 2);
      expect(find.byKey(const ValueKey('twitch-embed-open')), findsNothing);
      // Escape leaves fullscreen first.
      await t.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(t, 2);
      expect(find.byKey(const ValueKey('twitch-embed-open')), findsOneWidget);
      expect(find.byKey(const ValueKey('twitch-embed')), findsOneWidget);

      // The player's own fullscreen button.
      page.onFullscreen!(true);
      await settle(t, 2);
      expect(find.byKey(const ValueKey('twitch-embed-open')), findsNothing);
      page.onFullscreen!(false);
      await settle(t, 2);
      expect(find.byKey(const ValueKey('twitch-embed-open')), findsOneWidget);

      // Escape inside the web view (the Linux runner) leaves the screen.
      page.onBack!();
      await settle(t);
      expect(find.byKey(const ValueKey('twitch-embed')), findsNothing);
      expect(page.disposed, isTrue);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('phones have no fullscreen button of their own', (t) async {
      setSurface(t, _desktop);
      final services = await twitchServices();
      services.twitch.embedPlayer = (context, target) => openTwitchEmbed(
        context,
        target,
        surface: FakeEmbedSurface.factory,
        engine: TwitchEmbedEngine.webviewFlutter,
      );
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron')));
      await _tap(t, find.byKey(const ValueKey('twitch-watch-live')));
      expect(find.byKey(const ValueKey('twitch-embed')), findsOneWidget);
      expect(
        find.byKey(const ValueKey('twitch-embed-fullscreen')),
        findsNothing,
      );
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  });

  group('player choice per platform (TWI-029)', () {
    test('each platform has its web view', () {
      expect(
        twitchEmbedEngineFor(TargetPlatform.android),
        TwitchEmbedEngine.webviewFlutter,
      );
      expect(
        twitchEmbedEngineFor(TargetPlatform.iOS),
        TwitchEmbedEngine.webviewFlutter,
      );
      expect(
        twitchEmbedEngineFor(TargetPlatform.macOS),
        TwitchEmbedEngine.webviewFlutter,
      );
      expect(
        twitchEmbedEngineFor(TargetPlatform.windows),
        TwitchEmbedEngine.webView2,
      );
      expect(
        twitchEmbedEngineFor(TargetPlatform.linux),
        TwitchEmbedEngine.webKitGtk,
      );
      expect(twitchEmbedEngineFor(TargetPlatform.fuchsia), isNull);
      expect(twitchEmbedMissingFor(TwitchEmbedEngine.webviewFlutter), isNull);
    });

    for (final engine in TwitchEmbedEngine.values) {
      test('${engine.name}: the embed plays when the native player is off, '
          'native first when the config allows it', () async {
        // Store build: no native player at all.
        final store = await twitchServices();
        await registerTwitchEmbed(
          store.twitch,
          engine: engine,
          available: (_) async => true,
        );
        expect(store.twitch.embedPlayer, isNotNull);
        expect(store.twitch.embedMissing, isNull);
        expect(store.twitch.playRoute, TwitchPlayRoute.embed);

        // Full build whose config allows native playback: native wins.
        final full = await twitchServices(twitchNative: true);
        full.twitch.nativePlayer = (context, target) async {};
        await registerTwitchEmbed(
          full.twitch,
          engine: engine,
          available: (_) async => true,
        );
        expect(full.twitch.playRoute, TwitchPlayRoute.native);

        // Full build with the flag off (TWI-030 not signed yet): embed.
        expect(
          decideTwitchRoute(
            fullBuild: true,
            nativeAllowedByConfig: false,
            hasNativePlayer: true,
            hasEmbedPlayer: true,
          ),
          TwitchPlayRoute.embed,
        );
      });
    }

    test('without WebView2 or WebKitGTK: "Open on Twitch"', () async {
      for (final (engine, missing) in [
        (TwitchEmbedEngine.webView2, TwitchEmbedMissing.webView2),
        (TwitchEmbedEngine.webKitGtk, TwitchEmbedMissing.webKitGtk),
      ]) {
        final services = await twitchServices();
        await registerTwitchEmbed(
          services.twitch,
          engine: engine,
          available: (_) async => false,
        );
        expect(services.twitch.embedPlayer, isNull);
        expect(services.twitch.embedMissing, missing);
        expect(services.twitch.playRoute, TwitchPlayRoute.openOnTwitch);
      }
      // A check that throws counts as missing.
      final services = await twitchServices();
      await registerTwitchEmbed(
        services.twitch,
        engine: TwitchEmbedEngine.webView2,
        available: (_) => throw StateError('no runtime'),
      );
      expect(services.twitch.embedMissing, TwitchEmbedMissing.webView2);
    });

    testWidgets('"Open on Twitch" says what to install', (t) async {
      setSurface(t, _desktop);
      final services = await twitchServices();
      await registerTwitchEmbed(
        services.twitch,
        engine: TwitchEmbedEngine.webView2,
        available: (_) async => false,
      );
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron')));
      await _tap(t, find.byKey(const ValueKey('twitch-watch-live')));
      expect(find.byKey(const ValueKey('twitch-open-dialog')), findsOneWidget);
      expect(find.textContaining('WebView2 Runtime'), findsOneWidget);
      await t.tap(find.widgetWithText(TextButton, 'Close'));
      await settle(t, 4);

      services.twitch.embedMissing = TwitchEmbedMissing.webKitGtk;
      final context = t.element(find.byType(Scaffold).first);
      unawaited(playTwitch(context, const TwitchPlayTarget.live('pixelheron')));
      await settle(t, 4);
      expect(find.textContaining('libwebkit2gtk-4.1'), findsOneWidget);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  });

  group('Linux runner channel (TWI-029)', () {
    testWidgets('the web view follows its box, hides under dialogs and goes '
        'with the screen', (t) async {
      setSurface(t, _desktop);
      final messenger = t.binding.defaultBinaryMessenger;
      final calls = <MethodCall>[];
      messenger.setMockMethodCallHandler(linuxTwitchWebViewChannel, (
        call,
      ) async {
        calls.add(call);
        return call.method == 'available' || call.method == 'create'
            ? true
            : null;
      });
      addTearDown(
        () =>
            messenger.setMockMethodCallHandler(linuxTwitchWebViewChannel, null),
      );
      Map<Object?, Object?> lastRect() =>
          calls.lastWhere((c) => c.method == 'rect').arguments
              as Map<Object?, Object?>;

      final services = await twitchServices();
      await registerTwitchEmbed(
        services.twitch,
        engine: TwitchEmbedEngine.webKitGtk,
      );
      expect(calls.single.method, 'available');
      expect(services.twitch.embedPlayer, isNotNull);

      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron')));
      await _tap(t, find.byKey(const ValueKey('twitch-watch-live')));
      expect(find.byKey(const ValueKey('twitch-embed')), findsOneWidget);
      final create =
          calls.firstWhere((c) => c.method == 'create').arguments
              as Map<Object?, Object?>;
      expect(create['html'], contains('"channel":"pixelheron"'));
      expect(create['baseUrl'], startsWith('https://$twitchEmbedParent/'));
      // The route's slide is over: under the bar, the whole width of the
      // window (a player screen on the root navigator).
      await t.pump(const Duration(seconds: 1));
      await t.pump();
      expect(lastRect()['visible'], isTrue);
      expect(lastRect()['y'] as double, greaterThanOrEqualTo(44));
      expect(lastRect()['x'], 0);
      expect(lastRect()['width'], _desktop.width);

      // A message of the page reaches the session.
      await messenger.handlePlatformMessage(
        linuxTwitchWebViewChannel.name,
        linuxTwitchWebViewChannel.codec.encodeMethodCall(
          const MethodCall('message', '{"event":"error"}'),
        ),
        (_) {},
      );
      await settle(t, 2);
      expect(find.byKey(const ValueKey('twitch-embed-failed')), findsOneWidget);
      // The failed screen replaced the box: the view is hidden.
      expect(lastRect()['visible'], isFalse);
      await t.tap(find.text('Try again'));
      await settle(t, 4);
      expect(calls.where((c) => c.method == 'create'), hasLength(2));
      expect(lastRect()['visible'], isTrue);

      // A dialog over the screen: the native view hides, then comes back.
      await _tap(t, find.byKey(const ValueKey('twitch-embed-open')));
      expect(find.byKey(const ValueKey('twitch-open-dialog')), findsOneWidget);
      expect(lastRect()['visible'], isFalse);
      await t.tap(find.widgetWithText(TextButton, 'Close'));
      await settle(t, 4);
      expect(lastRect()['visible'], isTrue);

      // Leaving the screen releases the view.
      await t.tap(
        find.descendant(
          of: find.byKey(const ValueKey('twitch-embed')),
          matching: find.byType(BackButton),
        ),
      );
      await settle(t);
      expect(calls.last.method, 'dispose');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    // BUG-028: on Wayland the runner keeps the page out of its own
    // fullscreen and toggles the window instead; the view then has to take
    // the whole window from the rectangle alone.
    testWidgets('the player\'s fullscreen message gives the view the whole '
        'window and back (BUG-028)', (t) async {
      setSurface(t, _desktop);
      final messenger = t.binding.defaultBinaryMessenger;
      final calls = <MethodCall>[];
      messenger.setMockMethodCallHandler(linuxTwitchWebViewChannel, (
        call,
      ) async {
        calls.add(call);
        return call.method == 'available' || call.method == 'create'
            ? true
            : null;
      });
      addTearDown(
        () =>
            messenger.setMockMethodCallHandler(linuxTwitchWebViewChannel, null),
      );
      Map<Object?, Object?> lastRect() =>
          calls.lastWhere((c) => c.method == 'rect').arguments
              as Map<Object?, Object?>;
      Future<void> fromRunner(bool on) => messenger.handlePlatformMessage(
        linuxTwitchWebViewChannel.name,
        linuxTwitchWebViewChannel.codec.encodeMethodCall(
          MethodCall('fullscreen', on),
        ),
        (_) {},
      );

      final services = await twitchServices();
      await registerTwitchEmbed(
        services.twitch,
        engine: TwitchEmbedEngine.webKitGtk,
      );
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron')));
      await _tap(t, find.byKey(const ValueKey('twitch-watch-live')));
      await t.pump(const Duration(seconds: 1));
      await t.pump();
      expect(lastRect()['visible'], isTrue);
      expect(lastRect()['y'] as double, greaterThanOrEqualTo(44));

      await fromRunner(true);
      await settle(t, 4);
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('twitch-embed')),
          matching: find.byType(AppBar),
        ),
        findsNothing,
      );
      expect(lastRect()['visible'], isTrue);
      expect(lastRect()['x'], 0);
      expect(lastRect()['y'], 0);
      expect(lastRect()['width'], _desktop.width);
      expect(lastRect()['height'], _desktop.height);

      await fromRunner(false);
      await settle(t, 4);
      expect(lastRect()['y'] as double, greaterThanOrEqualTo(44));
      expect(lastRect()['height'] as double, lessThan(_desktop.height));

      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    // BUG-028: Flutter draws into its own Wayland subsurface, above the
    // window's surface, so an overlay child of the window is hidden under
    // it (a black player on GNOME and KDE). The runner has to put the web
    // view into a subsurface of its own on Wayland. A guard on the C++
    // source, which no Dart test runs: removing that path fails here.
    test('the runner gives the web view its own subsurface on Wayland '
        '(BUG-028)', () {
      final src = File('linux/runner/twitch_webview.cc').readAsStringSync();
      expect(src, contains('GDK_IS_WAYLAND_DISPLAY'));
      expect(src, contains('gtk_window_new(GTK_WINDOW_POPUP)'));
      expect(src, contains('gtk_window_set_transient_for'));
      expect(src, contains('twitch_webview_use_subsurface('));
      // X11 keeps the overlay.
      expect(src, contains('gtk_overlay_add_overlay'));
    });
  });

  // TWI-041: the app's chat beside the embed on desktops (owner report
  // 2026-11-26: "no way to turn the chat on beside the stream").
  group('chat beside the embed (TWI-041)', () {
    setUp(FakeEmbedSurface.created.clear);

    Future<(AppServices, List<ManualChatFeed>)> open(
      WidgetTester t, {
      TwitchEmbedEngine engine = TwitchEmbedEngine.webKitGtk,
      TwitchEmbedSurfaceFactory? surface = FakeEmbedSurface.factory,
      bool signedIn = false,
      bool vod = false,
      AppServices? reuse,
    }) async {
      final services = reuse ?? await twitchServices();
      final feeds = <ManualChatFeed>[];
      services.twitch.embedPlayer = (context, target) => openTwitchEmbed(
        context,
        target,
        surface: surface,
        engine: engine,
        chat: (context, login) async {
          final f = ManualChatFeed(
            channel: login,
            canSend: signedIn,
            ownLogin: signedIn ? 'viewer' : null,
          );
          feeds.add(f);
          return f;
        },
      );
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron')));
      await _tap(
        t,
        find.byKey(ValueKey(vod ? 'twitch-vod-9101' : 'twitch-watch-live')),
      );
      await t.pump(const Duration(seconds: 1));
      await settle(t, 4);
      return (services, feeds);
    }

    final chat = find.byKey(const ValueKey('twitch-embed-chat'));
    final toggle = find.byKey(const ValueKey('twitch-embed-chat-toggle'));
    final webView = find.byKey(const ValueKey('fake-embed-webview'));

    Future<void> close(WidgetTester t) async {
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    }

    testWidgets('a live stream shows the chat beside the embed; the bar\'s '
        'button and C hide and show it', (t) async {
      setSurface(t, _desktop);
      final (_, feeds) = await open(t);
      expect(feeds.single.channel, 'pixelheron');
      expect(chat, findsOneWidget);
      // Beside, not over: the embed ends where the chat begins.
      final embed = t.getRect(webView);
      final panel = t.getRect(chat);
      expect(embed.right, lessThanOrEqualTo(panel.left));
      expect(panel.right, _desktop.width);
      expect(embed.width, greaterThanOrEqualTo(kTwitchEmbedMinWidth));
      // Twitch's own chat is not in the page: only the player.
      final page = FakeEmbedSurface.created.single;
      expect(page.html, contains('new Twitch.Player('));
      expect(page.html, isNot(contains('Twitch.Embed')));

      final l = t.element(chat).l10n;
      expect(t.widget<IconButton>(toggle).tooltip, l.twitchPlayerHideChat);
      await _tap(t, toggle);
      expect(chat, findsNothing);
      expect(t.getRect(webView).width, _desktop.width);
      expect(t.widget<IconButton>(toggle).tooltip, l.twitchPlayerShowChat);
      await _tap(t, toggle);
      expect(chat, findsOneWidget);

      await t.sendKeyEvent(LogicalKeyboardKey.keyC);
      await settle(t, 2);
      expect(chat, findsNothing);
      await t.sendKeyEvent(LogicalKeyboardKey.keyC);
      await settle(t, 2);
      expect(chat, findsOneWidget);
      // Hidden and shown again: the same feed, not a second one.
      expect(feeds, hasLength(1));
      await close(t);
    });

    testWidgets('the width is dragged and remembered for both players', (
      t,
    ) async {
      setSurface(t, _desktop);
      final (services, _) = await open(t);
      final before = t.getRect(chat).width;
      await t.drag(
        find.byKey(const ValueKey('twitch-chat-resize')),
        const Offset(-120, 0),
      );
      await settle(t, 3);
      final after = t.getRect(chat).width;
      // 120 px less the touch slop the drag needs to start.
      expect(after, greaterThan(before + 80));
      expect(after, lessThanOrEqualTo(before + 120));
      expect(t.getRect(webView).right, lessThanOrEqualTo(t.getRect(chat).left));
      final saved = await services.preferences.read(
        Preferences.twitchChatWidth,
      );
      expect(double.parse(saved!), closeTo(after, 1));
      await close(t);

      // The next stream opens at that width.
      FakeEmbedSurface.created.clear();
      await open(t, reuse: services);
      expect(t.getRect(chat).width, closeTo(after, 1));
      await close(t);
    });

    testWidgets('fullscreen leaves the embed alone; leaving it brings the '
        'chat back', (t) async {
      setSurface(t, _desktop);
      await open(t);
      await t.sendKeyEvent(LogicalKeyboardKey.keyF);
      await settle(t, 2);
      expect(chat, findsNothing);
      expect(t.getRect(webView), Offset.zero & _desktop);
      await t.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(t, 2);
      expect(chat, findsOneWidget);
      // The player's own fullscreen button too.
      FakeEmbedSurface.created.single.onFullscreen!(true);
      await settle(t, 2);
      expect(chat, findsNothing);
      await close(t);
    });

    testWidgets('signed out: read-only with "sign in to chat"', (t) async {
      setSurface(t, _desktop);
      await open(t);
      final l = t.element(chat).l10n;
      expect(
        find.descendant(of: chat, matching: find.byType(TextField)),
        findsNothing,
      );
      expect(
        find.descendant(
          of: chat,
          matching: find.text(l.twitchChatSignInToChat),
        ),
        findsOneWidget,
      );
      await close(t);
    });

    testWidgets('typing in the chat does not drive the player', (t) async {
      setSurface(t, _desktop);
      await open(t, signedIn: true);
      final page = FakeEmbedSurface.created.single;
      page.post('play');
      await settle(t, 2);
      final input = find.descendant(of: chat, matching: find.byType(TextField));
      expect(input, findsOneWidget);
      await t.tap(input);
      await settle(t, 2);
      for (final key in [
        LogicalKeyboardKey.keyC,
        LogicalKeyboardKey.space,
        LogicalKeyboardKey.keyK,
        LogicalKeyboardKey.keyF,
      ]) {
        await t.sendKeyEvent(key);
      }
      await settle(t, 2);
      expect(page.scripts, isEmpty);
      expect(chat, findsOneWidget);
      expect(find.byKey(const ValueKey('twitch-embed-open')), findsOneWidget);
      // Escape leaves the input, not the screen; then the keys work.
      await t.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(t, 2);
      expect(find.byKey(const ValueKey('twitch-embed')), findsOneWidget);
      await t.sendKeyEvent(LogicalKeyboardKey.space);
      expect(page.scripts.last, 'window.kinolune.pause();');
      await close(t);
    });

    testWidgets('no chat for a VOD or in a narrow window', (t) async {
      setSurface(t, _desktop);
      final (_, vodFeeds) = await open(t, vod: true);
      expect(vodFeeds, isEmpty);
      expect(chat, findsNothing);
      expect(toggle, findsNothing);
      await close(t);

      // A window too narrow for the embed's 400 and the chat: no panel,
      // no button.
      setSurface(t, const Size(820, 700));
      final (_, feeds) = await open(t);
      expect(feeds, hasLength(1));
      expect(chat, findsNothing);
      expect(toggle, findsNothing);
      await close(t);
    });

    testWidgets('Linux: the native view\'s rectangle leaves the chat out and '
        'shrinks when the chat widens (BUG-028)', (t) async {
      setSurface(t, _desktop);
      final messenger = t.binding.defaultBinaryMessenger;
      final calls = <MethodCall>[];
      messenger.setMockMethodCallHandler(linuxTwitchWebViewChannel, (
        call,
      ) async {
        calls.add(call);
        return call.method == 'available' || call.method == 'create'
            ? true
            : null;
      });
      addTearDown(
        () =>
            messenger.setMockMethodCallHandler(linuxTwitchWebViewChannel, null),
      );
      Map<Object?, Object?> lastRect() =>
          calls.lastWhere((c) => c.method == 'rect').arguments
              as Map<Object?, Object?>;
      double right() =>
          (lastRect()['x'] as double) + (lastRect()['width'] as double);

      await open(t, surface: null);
      await t.pump(const Duration(seconds: 1));
      await t.pump();
      expect(lastRect()['visible'], isTrue);
      final panel = t.getRect(chat);
      expect(right(), lessThanOrEqualTo(panel.left));
      expect(lastRect()['width'], greaterThanOrEqualTo(kTwitchEmbedMinWidth));
      final before = lastRect()['width'] as double;

      await t.drag(
        find.byKey(const ValueKey('twitch-chat-resize')),
        const Offset(-120, 0),
      );
      await settle(t, 3);
      await t.pump();
      expect(lastRect()['width'] as double, lessThan(before - 80));
      expect(right(), lessThanOrEqualTo(t.getRect(chat).left));

      // Hidden chat: the view takes the whole width again.
      await t.sendKeyEvent(LogicalKeyboardKey.keyC);
      await settle(t, 3);
      await t.pump();
      expect(lastRect()['width'], _desktop.width);
      await close(t);
    });
  });

  group('Open on Twitch', () {
    late ExternalLinkOpener saved;
    setUp(() => saved = openExternalLink);
    tearDown(() => openExternalLink = saved);

    Future<void> openDialog(WidgetTester t, FormFactor formFactor) async {
      final services = await twitchServices();
      await t.pumpWidget(
        testApp(services, section: Section.twitch, formFactor: formFactor),
      );
      await settle(t);
      final context = t.element(find.byType(Scaffold).first);
      showOpenOnTwitch(context, 'https://www.twitch.tv/pixelheron');
      await settle(t, 4);
    }

    testWidgets('phones and desktops open the link directly', (t) async {
      setSurface(t, _desktop);
      final opened = <Uri>[];
      openExternalLink = (url) async {
        opened.add(url);
        return true;
      };
      await openDialog(t, FormFactor.desktop);
      expect(find.byKey(const ValueKey('twitch-open-dialog')), findsOneWidget);
      await t.tap(find.byKey(const ValueKey('twitch-open-link')));
      await settle(t, 4);
      expect(opened, [Uri.parse('https://www.twitch.tv/pixelheron')]);
      expect(find.byKey(const ValueKey('twitch-open-dialog')), findsNothing);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('TVs keep the QR code and the link only', (t) async {
      setSurface(t, const Size(1920, 1080));
      await openDialog(t, FormFactor.tv);
      expect(find.byKey(const ValueKey('twitch-open-dialog')), findsOneWidget);
      expect(find.byKey(const ValueKey('twitch-open-link')), findsNothing);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  });
}
