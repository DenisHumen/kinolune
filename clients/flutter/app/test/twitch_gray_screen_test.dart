// SPDX-License-Identifier: MPL-2.0
//
// BUG-026: in 0.1.0-alpha.20 on macOS a Twitch live stream or VOD opened
// as a plain gray window (the release ErrorWidget) that only a restart
// cleared. The cause: the embed's web view setup called
// `setBackgroundColor`, which webview_flutter_wkwebview on macOS throws
// from synchronously (`UnimplementedError`), so the embed screen failed in
// initState — and since TWI-029 that screen covers the whole window.
//
// - the real `WebViewTwitchSurface` over a fake web view platform that
//   throws like WKWebView on macOS: the screen opens (failed before);
// - live and VOD from the Streams section on macOS, Windows and Linux,
//   through the native player and the embed, with the self-update strip
//   of full builds in the tree: no exception while building;
// - the error boundary: a screen that throws while building shows a
//   readable error with «Повторить» and «Назад», never a gray window.

import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/twitch/chat/chat_model.dart';
import 'package:kinolune/features/updates/app_version.dart';
import 'package:kinolune/features/updates/releases.dart';
import 'package:kinolune/features/updates/update_controller.dart';
import 'package:kinolune/features/updates/update_platform.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/screens/twitch/twitch_embed_screen.dart';
import 'package:kinolune/screens/twitch/twitch_player_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/twitch_embed.dart';
import 'package:kinolune/twitch/twitch_native.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:kinolune/widgets/error_boundary.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';
import 'package:webview_flutter_platform_interface/webview_flutter_platform_interface.dart';

import 'support/test_app.dart';

class _Telemetry implements LiveTelemetry {
  @override
  Future<LiveReport?> report({
    required double cacheAhead,
    required double rate,
  }) async => LiveReport.fromJson({
    'mode': 'low_latency',
    'latency': 2.5,
    'target': 3.0,
    'rate': 1.0,
    'should_jump': false,
    'ad': null,
    'seek_allowed': true,
    'low_latency': {'prefetch': 2, 'parts': 0},
    'totals': {'segments': 10, 'ad_segments': 0},
  });

  @override
  Future<void> resetPlayhead() async {}

  @override
  Future<void> close() async {}
}

class _Session implements TwitchLiveSession {
  @override
  final String playlistUrl = 'http://127.0.0.1:9/token/live.m3u8';

  @override
  LiveTelemetry telemetry() => _Telemetry();

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
          url: 'https://usher.test/chunked.m3u8',
          isSource: true,
          height: 1080,
          frameRate: 60,
          bandwidth: 6000000,
        ),
        TwitchQuality(
          id: 'audio_only',
          label: 'audio_only',
          url: 'https://usher.test/audio_only.m3u8',
          audioOnly: true,
          bandwidth: 160000,
        ),
      ]);

  @override
  TwitchLiveSession startLive(String url, LiveLatencyMode mode) => _Session();

  @override
  TwitchLiveSession openVod(String url) => _Session();
}

class _Surface implements TwitchEmbedSurface {
  @override
  Widget build(BuildContext context) =>
      const ColoredBox(key: ValueKey('fake-embed'), color: Colors.black);

  @override
  Future<void> runJavaScript(String script) async {}

  @override
  void dispose() {}
}

TwitchEmbedSurface _surface({
  required String html,
  required Uri baseUrl,
  required void Function(String message) onMessage,
  required void Function(Uri url) onExternalLink,
  void Function(bool fullscreen)? onFullscreen,
  VoidCallback? onBack,
}) => _Surface();

class _NoReleases implements ReleaseSource {
  @override
  Future<List<AppRelease>> releases() async => const [];
}

/// The self-update strip of full builds (PKG-018): always in the tree.
UpdateController _updates() => UpdateController(
  current: AppVersion.tryParse('0.1.0-alpha.20')!,
  target: const UpdateTarget(UpdateKind.macApp),
  source: _NoReleases(),
  verify: ({
    required File file,
    required String assetName,
    required List<int> sums,
    required String? signature,
  }) async {},
  cacheDir: Directory('${Directory.systemTemp.path}/bug026-updates'),
  preferences: MemoryPreferences(),
  clock: () => demoNow.toUtc(),
);

/// A web view platform that behaves like webview_flutter_wkwebview 3.27 on
/// macOS when [macOS]: `setBackgroundColor` throws from the call itself.
class _WebViews extends WebViewPlatform {
  _WebViews({required this.macOS});

  final bool macOS;
  final controllers = <_WebController>[];

  @override
  PlatformWebViewController createPlatformWebViewController(
    PlatformWebViewControllerCreationParams params,
  ) {
    final c = _WebController(params, macOS: macOS);
    controllers.add(c);
    return c;
  }

  @override
  PlatformNavigationDelegate createPlatformNavigationDelegate(
    PlatformNavigationDelegateCreationParams params,
  ) => _Navigation(params);

  @override
  PlatformWebViewWidget createPlatformWebViewWidget(
    PlatformWebViewWidgetCreationParams params,
  ) => _WebWidget(params);
}

class _WebController extends PlatformWebViewController {
  _WebController(super.params, {required this.macOS}) : super.implementation();

  final bool macOS;
  final calls = <String>[];

  @override
  Future<void> setJavaScriptMode(JavaScriptMode javaScriptMode) async =>
      calls.add('js');

  @override
  Future<void> setBackgroundColor(Color color) {
    // `PlatformWebView.setOpaque` on macOS, called before any future.
    if (macOS) {
      throw UnimplementedError('opaque is not implemented on macOS');
    }
    calls.add('background');
    return Future.value();
  }

  @override
  Future<void> addJavaScriptChannel(
    JavaScriptChannelParams javaScriptChannelParams,
  ) async => calls.add('channel:${javaScriptChannelParams.name}');

  @override
  Future<void> setPlatformNavigationDelegate(
    PlatformNavigationDelegate handler,
  ) async => calls.add('navigation');

  @override
  Future<void> loadHtmlString(String html, {String? baseUrl}) async =>
      calls.add('html');

  @override
  Future<void> runJavaScript(String javaScript) async =>
      calls.add('run:$javaScript');
}

class _Navigation extends PlatformNavigationDelegate {
  _Navigation(super.params) : super.implementation();

  @override
  Future<void> setOnNavigationRequest(
    NavigationRequestCallback onNavigationRequest,
  ) async {}
}

class _WebWidget extends PlatformWebViewWidget {
  _WebWidget(super.params) : super.implementation();

  @override
  Widget build(BuildContext context) =>
      const ColoredBox(key: ValueKey('fake-web-view'), color: Colors.black);
}

Future<void> _tap(WidgetTester t, Finder f) async {
  await t.ensureVisible(f.first);
  await settle(t, 4);
  await t.tap(f.first);
  await settle(t, 14);
}

/// Opens live or a VOD of `pixelheron` from the channel page.
Future<void> _openFromChannel(
  WidgetTester t,
  AppServices services, {
  required bool live,
}) async {
  await t.pumpWidget(testApp(services, section: Section.twitch));
  await settle(t);
  await _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron')));
  await _tap(
    t,
    find.byKey(ValueKey(live ? 'twitch-watch-live' : 'twitch-vod-9101')),
  );
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 4);
}

/// Runs [body] as if on [platform].
Future<void> _on(TargetPlatform platform, Future<void> Function() body) async {
  debugDefaultTargetPlatformOverride = platform;
  try {
    await body();
  } finally {
    debugDefaultTargetPlatformOverride = null;
  }
}

void main() {
  group('the cause: the web view setup on macOS', () {
    test('no background colour where WKWebView has none', () {
      expect(webViewBackgroundSupported(TargetPlatform.macOS), isFalse);
      expect(webViewBackgroundSupported(TargetPlatform.iOS), isTrue);
      expect(webViewBackgroundSupported(TargetPlatform.android), isTrue);
    });

    for (final live in [true, false]) {
      testWidgets('macOS: the embed opens ${live ? 'live' : 'a VOD'}', (
        t,
      ) async {
        await _on(TargetPlatform.macOS, () async {
          setSurface(t, const Size(1440, 900));
          final webViews = _WebViews(macOS: true);
          WebViewPlatform.instance = webViews;
          final services = await twitchServices(updates: _updates());
          // The hook main() registers on macOS: the real surface.
          await registerTwitchEmbed(services.twitch);
          expect(services.twitch.playRoute, TwitchPlayRoute.embed);
          await _openFromChannel(t, services, live: live);
          expect(t.takeException(), isNull);
          expect(find.byType(ErrorWidget), findsNothing);
          expect(find.byKey(const ValueKey('twitch-embed')), findsOneWidget);
          expect(find.byKey(const ValueKey('fake-web-view')), findsOneWidget);
          // The way back is on screen.
          expect(find.byType(BackButton), findsOneWidget);
          final calls = webViews.controllers.single.calls;
          expect(calls, containsAll(['js', 'navigation', 'html']));
          expect(calls, contains('channel:$twitchEmbedChannel'));
          expect(calls, isNot(contains('background')));
          await _close(t);
        });
      });
    }

    testWidgets('a web view call that throws is logged, not fatal', (t) async {
      await _on(TargetPlatform.android, () async {
        setSurface(t, const Size(1440, 900));
        // Throws like macOS although the platform says Android.
        final webViews = _WebViews(macOS: true);
        WebViewPlatform.instance = webViews;
        final services = await twitchServices(updates: _updates());
        await registerTwitchEmbed(services.twitch);
        await _openFromChannel(t, services, live: true);
        expect(t.takeException(), isNull);
        expect(find.byKey(const ValueKey('fake-web-view')), findsOneWidget);
        expect(webViews.controllers.single.calls, contains('html'));
        await _close(t);
      });
    });

    testWidgets('Android keeps the black background', (t) async {
      await _on(TargetPlatform.android, () async {
        setSurface(t, const Size(1440, 900));
        final webViews = _WebViews(macOS: false);
        WebViewPlatform.instance = webViews;
        final services = await twitchServices(updates: _updates());
        await registerTwitchEmbed(services.twitch);
        await _openFromChannel(t, services, live: true);
        expect(t.takeException(), isNull);
        expect(webViews.controllers.single.calls, contains('background'));
        await _close(t);
      });
    });
  });

  group('live and VOD build on every desktop', () {
    for (final platform in [
      TargetPlatform.macOS,
      TargetPlatform.windows,
      TargetPlatform.linux,
    ]) {
      for (final native in [true, false]) {
        final path = native ? 'native player' : 'embed';
        for (final live in [true, false]) {
          final what = live ? 'live' : 'VOD';
          testWidgets('${platform.name}: $what through the $path', (t) async {
            await _on(platform, () async {
              setSurface(t, const Size(1440, 900));
              final services = await twitchServices(
                twitchNative: native,
                updates: _updates(),
              );
              services.twitch.nativePlayer = (context, target) =>
                  openTwitchNative(
                    context,
                    target,
                    source: _Source(),
                    engineFactory: (_) => FakePlayerEngine(
                      loadedDuration: const Duration(minutes: 245),
                    ),
                    chat: (context, login) async =>
                        ManualChatFeed(channel: login),
                    findStreamVod: (_) async => null,
                  );
              services.twitch.embedPlayer = (context, target) =>
                  openTwitchEmbed(
                    context,
                    target,
                    surface: _surface,
                    engine: twitchEmbedEngineFor(platform),
                  );
              await _openFromChannel(t, services, live: live);
              expect(t.takeException(), isNull);
              expect(find.byType(ErrorWidget), findsNothing);
              expect(
                find.byType(native ? TwitchPlayerScreen : TwitchEmbedScreen),
                findsOneWidget,
              );
              await _close(t);
            });
          });
        }
      }
    }
  });

  group('error boundary', () {
    late ErrorWidgetBuilder previous;
    setUp(() {
      previous = ErrorWidget.builder;
      installErrorBoundaries();
    });
    tearDown(() => uninstallErrorBoundaries(previous));

    testWidgets('a Twitch screen that throws: an error, Retry, Back', (
      t,
    ) async {
      setSurface(t, const Size(1440, 900));
      final services = await twitchServices(updates: _updates());
      await services.setLocale(const Locale('ru'));
      var failures = 1;
      services.twitch.embedPlayer = (context, target) => openTwitchEmbed(
        context,
        target,
        surface:
            ({
              required String html,
              required Uri baseUrl,
              required void Function(String message) onMessage,
              required void Function(Uri url) onExternalLink,
              void Function(bool fullscreen)? onFullscreen,
              VoidCallback? onBack,
            }) {
              if (failures-- > 0) throw UnimplementedError('no web view');
              return _Surface();
            },
      );
      await _openFromChannel(t, services, live: true);
      expect(t.takeException(), isA<UnimplementedError>());
      await settle(t, 2);
      // A readable error instead of a gray window.
      expect(find.byType(ErrorWidget), findsNothing);
      expect(find.byKey(const ValueKey('screen-error')), findsOneWidget);
      final ru = lookupAppLocalizations(const Locale('ru'));
      expect(find.text(ru.screenErrorTitle), findsOneWidget);
      expect(find.text('Повторить'), findsOneWidget);
      expect(find.text('Назад'), findsOneWidget);
      // «Повторить» builds the screen anew; now it works.
      await _tap(t, find.byKey(const ValueKey('screen-error-retry')));
      expect(t.takeException(), isNull);
      expect(find.byKey(const ValueKey('fake-embed')), findsOneWidget);
      await _tap(t, find.byType(BackButton));
      expect(find.byType(TwitchEmbedScreen), findsNothing);

      // It fails again: «Назад» returns to the channel page.
      failures = 1;
      await _tap(t, find.byKey(const ValueKey('twitch-watch-live')));
      expect(t.takeException(), isA<UnimplementedError>());
      await settle(t, 2);
      expect(find.byKey(const ValueKey('screen-error')), findsOneWidget);
      await _tap(t, find.byKey(const ValueKey('screen-error-back')));
      expect(find.byKey(const ValueKey('screen-error')), findsNothing);
      expect(find.byKey(const ValueKey('twitch-watch-live')), findsOneWidget);
      await _close(t);
    });

    testWidgets('outside a boundary the previous builder stays', (t) async {
      await t.pumpWidget(
        const Directionality(
          textDirection: TextDirection.ltr,
          child: _Thrower(),
        ),
      );
      expect(t.takeException(), isA<StateError>());
      expect(find.byType(ErrorWidget), findsOneWidget);
    });
  });
}

class _Thrower extends StatelessWidget {
  const _Thrower();

  @override
  Widget build(BuildContext context) => throw StateError('broken');
}
