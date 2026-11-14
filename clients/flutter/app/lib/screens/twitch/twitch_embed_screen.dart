// SPDX-License-Identifier: MPL-2.0
//
// The official Twitch embed in a WebView (TWI-005): the screen registered
// as `TwitchService.embedPlayer`. `webview_flutter` (BSD-3-Clause) has
// Android, iOS and macOS implementations; Linux and Windows have none, so
// there the hook stays empty and the Streams section offers "Open on
// Twitch" (browser through `url_launcher`). The page, its events and the
// progress sync live in `twitch/twitch_embed.dart`; tests replace the
// WebView with a fake surface ([TwitchEmbedSurfaceFactory]).

import 'dart:async';
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:webview_flutter/webview_flutter.dart';
import 'package:webview_flutter_android/webview_flutter_android.dart';
import 'package:webview_flutter_wkwebview/webview_flutter_wkwebview.dart';

import '../../l10n/l10n.dart';
import '../../platform/form_factor.dart';
import '../../services/app_services.dart';
import '../../services/external_links.dart';
import '../../twitch/twitch_embed.dart';
import '../../twitch/twitch_service.dart';
import 'twitch_widgets.dart';

/// A WebView that shows the embed page; tests use a fake one.
abstract class TwitchEmbedSurface {
  Widget build(BuildContext context);

  /// Runs JavaScript in the page (`window.kinolune.*`).
  Future<void> runJavaScript(String script);
}

typedef TwitchEmbedSurfaceFactory =
    TwitchEmbedSurface Function({
      required String html,
      required Uri baseUrl,
      required void Function(String message) onMessage,
      required void Function(Uri url) onExternalLink,
    });

/// The platform has a WebView for the embed (`webview_flutter`).
bool get twitchEmbedSupported =>
    !kIsWeb && (Platform.isAndroid || Platform.isIOS || Platform.isMacOS);

/// The `TwitchService.embedPlayer` hook: the embed screen for [target].
Future<void> openTwitchEmbed(
  BuildContext context,
  TwitchPlayTarget target, {
  TwitchEmbedSurfaceFactory? surface,
}) => Navigator.of(context).push(
  MaterialPageRoute<void>(
    builder: (_) => TwitchEmbedScreen(target: target, surface: surface),
  ),
);

/// Registers the embed as the Twitch section's fallback player where a
/// WebView exists.
void registerTwitchEmbed(TwitchService twitch) {
  if (twitchEmbedSupported) twitch.embedPlayer = openTwitchEmbed;
}

class TwitchEmbedScreen extends StatefulWidget {
  const TwitchEmbedScreen({super.key, required this.target, this.surface});

  final TwitchPlayTarget target;

  /// The WebView; null = `webview_flutter`.
  final TwitchEmbedSurfaceFactory? surface;

  @override
  State<TwitchEmbedScreen> createState() => _TwitchEmbedScreenState();
}

class _TwitchEmbedScreenState extends State<TwitchEmbedScreen> {
  late TwitchEmbedSession _session;
  late TwitchEmbedSurface _surface;
  late AppServices _services;
  var _attempt = 0;
  var _phone = false;

  @override
  void initState() {
    super.initState();
    _services = Services.read(context);
    _start();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      // Twitch: the embed is at least 400×300; a phone shows it landscape.
      _phone = FormFactorScope.of(context) == FormFactor.phone;
      if (_phone) {
        unawaited(
          SystemChrome.setPreferredOrientations(const [
            DeviceOrientation.landscapeLeft,
            DeviceOrientation.landscapeRight,
          ]),
        );
      }
    });
  }

  void _start() {
    _session = TwitchEmbedSession(
      target: widget.target,
      saveProgress: _services.library.saveProgress,
      handoff: _services.handoff,
      clock: _services.clock,
    );
    final factory = widget.surface ?? WebViewTwitchSurface.new;
    _surface = factory(
      html: twitchEmbedHtml(widget.target),
      baseUrl: twitchEmbedBaseUrl(),
      onMessage: _session.handle,
      onExternalLink: (url) => unawaited(openExternalLink(url)),
    );
  }

  Future<void> _retry() async {
    await _session.close();
    if (!mounted) return;
    setState(() {
      _attempt++;
      _start();
    });
  }

  /// "Open on Twitch" at the position reached here.
  String get _twitchUrl {
    final url = widget.target.url;
    final at = _session.position;
    if (widget.target.isLive || at <= Duration.zero) return url;
    return '$url?t=${twitchEmbedTime(at)}'; // l10n-ignore
  }

  @override
  void dispose() {
    final twitch = _services.twitch;
    unawaited(_session.close().then((_) => twitch.progressChanged()));
    if (_phone) unawaited(SystemChrome.setPreferredOrientations(const []));
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final login = widget.target.login;
    final title = login == null || login.isEmpty
        ? l.twitchEmbedTitle
        : _services.twitch.channelOf(login).displayName;
    return Scaffold(
      key: const ValueKey('twitch-embed'),
      backgroundColor: Colors.black,
      // Nothing may cover the embed (Twitch rules): the bar sits above it.
      appBar: AppBar(
        backgroundColor: Colors.black,
        foregroundColor: Colors.white,
        toolbarHeight: 44,
        title: Text(title, maxLines: 1, overflow: TextOverflow.ellipsis),
        actions: [
          IconButton(
            key: const ValueKey('twitch-embed-open'),
            tooltip: l.twitchOpenOnTwitch,
            icon: const Icon(Icons.open_in_new),
            onPressed: () => showOpenOnTwitch(context, _twitchUrl),
          ),
        ],
      ),
      body: SafeArea(
        top: false,
        child: ValueListenableBuilder<TwitchEmbedState>(
          key: ValueKey(_attempt),
          valueListenable: _session.state,
          builder: (context, state, _) => state == TwitchEmbedState.failed
              ? _Failed(
                  onRetry: _retry,
                  onOpen: () => showOpenOnTwitch(context, _twitchUrl),
                )
              : KeyedSubtree(
                  key: ValueKey('twitch-embed-surface-$_attempt'),
                  child: _surface.build(context),
                ),
        ),
      ),
    );
  }
}

class _Failed extends StatelessWidget {
  const _Failed({required this.onRetry, required this.onOpen});

  final VoidCallback onRetry;
  final VoidCallback onOpen;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    return Center(
      key: const ValueKey('twitch-embed-failed'),
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              l.twitchEmbedFailed,
              textAlign: TextAlign.center,
              style: theme.textTheme.bodyLarge?.copyWith(color: Colors.white),
            ),
            const SizedBox(height: 16),
            Wrap(
              spacing: 12,
              runSpacing: 8,
              alignment: WrapAlignment.center,
              children: [
                FilledButton(
                  autofocus: true,
                  onPressed: onRetry,
                  child: Text(l.twitchEmbedRetry),
                ),
                OutlinedButton(
                  onPressed: onOpen,
                  child: Text(l.twitchOpenOnTwitch),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// The embed page in `webview_flutter`: inline autoplay, the JavaScript
/// channel, and links that leave the page opened outside the app.
class WebViewTwitchSurface implements TwitchEmbedSurface {
  WebViewTwitchSurface({
    required String html,
    required Uri baseUrl,
    required void Function(String message) onMessage,
    required void Function(Uri url) onExternalLink,
  }) {
    final PlatformWebViewControllerCreationParams params =
        WebViewPlatform.instance is WebKitWebViewPlatform
        ? WebKitWebViewControllerCreationParams(
            allowsInlineMediaPlayback: true,
            mediaTypesRequiringUserAction: const {},
          )
        : const PlatformWebViewControllerCreationParams();
    _controller = WebViewController.fromPlatformCreationParams(params);
    final platform = _controller.platform;
    if (platform is AndroidWebViewController) {
      unawaited(platform.setMediaPlaybackRequiresUserGesture(false));
    }
    unawaited(_controller.setJavaScriptMode(JavaScriptMode.unrestricted));
    unawaited(_controller.setBackgroundColor(Colors.black));
    unawaited(
      _controller.addJavaScriptChannel(
        twitchEmbedChannel,
        onMessageReceived: (m) => onMessage(m.message),
      ),
    );
    unawaited(
      _controller.setNavigationDelegate(
        NavigationDelegate(
          onNavigationRequest: (request) {
            // The player itself loads in an iframe; the page never leaves
            // its base URL. A link that would (the Twitch logo, a channel
            // name) opens in the Twitch app or the browser.
            if (!request.isMainFrame || request.url == baseUrl.toString()) {
              return NavigationDecision.navigate;
            }
            final url = Uri.tryParse(request.url);
            if (url != null && url.scheme == 'https') onExternalLink(url);
            return NavigationDecision.prevent;
          },
        ),
      ),
    );
    unawaited(_controller.loadHtmlString(html, baseUrl: baseUrl.toString()));
  }

  late final WebViewController _controller;

  @override
  Widget build(BuildContext context) => WebViewWidget(controller: _controller);

  @override
  Future<void> runJavaScript(String script) =>
      _controller.runJavaScript(script);
}
