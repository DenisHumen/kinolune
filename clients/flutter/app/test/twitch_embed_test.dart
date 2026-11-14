// SPDX-License-Identifier: MPL-2.0
//
// TWI-005: the official Twitch embed. The page (parent, live and VOD
// options), its events, VOD position sync into Kinolune progress and the
// handoff session, the embed screen as `TwitchService.embedPlayer` with a
// fake WebView channel, and "Open" in the Open on Twitch dialog
// (url_launcher on phones and desktops, QR code only on TVs). No network.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/integration/library.dart' show Progress;
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/twitch/twitch_embed_screen.dart';
import 'package:kinolune/screens/twitch/twitch_play.dart';
import 'package:kinolune/screens/twitch/twitch_widgets.dart';
import 'package:kinolune/services/external_links.dart';
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
  });

  final String html;
  final Uri baseUrl;
  final void Function(String message) onMessage;
  final void Function(Uri url) onExternalLink;
  final List<String> scripts = [];

  static final List<FakeEmbedSurface> created = [];

  static FakeEmbedSurface factory({
    required String html,
    required Uri baseUrl,
    required void Function(String message) onMessage,
    required void Function(Uri url) onExternalLink,
  }) {
    final s = FakeEmbedSurface(
      html: html,
      baseUrl: baseUrl,
      onMessage: onMessage,
      onExternalLink: onExternalLink,
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

    test('no value can close the script element', () {
      final html = twitchEmbedHtml(
        const TwitchPlayTarget.live('a</script><b>'),
      );
      expect(html, isNot(contains('a</script>')));
      expect(html, contains(r'a</script>'));
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
      services.twitch.embedPlayer = (context, target) => openTwitchEmbed(
        context,
        target,
        surface: FakeEmbedSurface.factory,
      );
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
      expect(
        sync.heartbeats.map((h) => h.itemId),
        contains('twitch:vod:9101'),
      );

      // "Open on Twitch" continues at the position reached here.
      await _tap(t, find.byKey(const ValueKey('twitch-embed-open')));
      expect(
        find.text('https://www.twitch.tv/videos/9101?t=0h20m34s'),
        findsOneWidget,
      );
      await t.tap(find.text('Close'));
      await settle(t, 4);

      page.post('pause', position: 1300, duration: 5400);
      await settle(t, 2);
      await t.pageBack();
      await settle(t, 4);
      expect(find.byKey(const ValueKey('twitch-embed')), findsNothing);
      expect(
        (await services.library.progress('twitch:vod:9101'))?.positionMs,
        1300000,
      );
      expect(sync.heartbeats.last.state.name, 'stopped');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('live plays in the embed; a failed load can retry', (
      t,
    ) async {
      setSurface(t, _desktop);
      final services = await twitchServices();
      services.twitch.embedPlayer = (context, target) => openTwitchEmbed(
        context,
        target,
        surface: FakeEmbedSurface.factory,
      );
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

    test('registered only where a WebView exists', () async {
      final services = await twitchServices();
      registerTwitchEmbed(services.twitch);
      // The tests run on a desktop host without webview_flutter.
      expect(services.twitch.embedPlayer != null, twitchEmbedSupported);
    });
  });

  group('Open on Twitch', () {
    late ExternalLinkOpener saved;
    setUp(() => saved = openExternalLink);
    tearDown(() => openExternalLink = saved);

    Future<void> openDialog(WidgetTester t, FormFactor formFactor) async {
      final services = await twitchServices();
      await t.pumpWidget(
        testApp(
          services,
          section: Section.twitch,
          formFactor: formFactor,
        ),
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
