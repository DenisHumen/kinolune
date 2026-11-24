// SPDX-License-Identifier: MPL-2.0
//
// The official Twitch embed in a web view (TWI-005, TWI-029): the screen
// registered as `TwitchService.embedPlayer`. Android, iOS and macOS show
// it in `webview_flutter` (BSD-3-Clause); Windows in Microsoft Edge
// WebView2 (`webview_windows`, BSD-3-Clause) and Linux in the system's
// WebKitGTK (LGPL-2.1, opened at run time by `linux/runner`), see
// `twitch_embed_desktop.dart` and ADR-NNNN. Where the runtime is missing
// the hook stays empty and the Streams section offers "Open on Twitch"
// with what to install. The page, its events and the progress sync live
// in `twitch/twitch_embed.dart`; tests replace the web view with a fake
// surface ([TwitchEmbedSurfaceFactory]).

import 'dart:async';

import 'package:flutter/foundation.dart' show kIsWeb, defaultTargetPlatform;
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:webview_flutter/webview_flutter.dart';
import 'package:webview_flutter_android/webview_flutter_android.dart';
import 'package:webview_flutter_wkwebview/webview_flutter_wkwebview.dart';

import '../../integration/player.dart' show setWindowFullscreen;
import '../../l10n/l10n.dart';
import '../../platform/form_factor.dart';
import '../../services/app_services.dart';
import '../../services/external_links.dart';
import '../../twitch/twitch_embed.dart';
import '../../twitch/twitch_service.dart';
import 'twitch_embed_desktop.dart';
import 'twitch_widgets.dart';

/// A web view that shows the embed page; tests use a fake one.
abstract class TwitchEmbedSurface {
  Widget build(BuildContext context);

  /// Runs JavaScript in the page (`window.kinolune.*`).
  Future<void> runJavaScript(String script);

  /// The screen closes: release the web view.
  void dispose();
}

typedef TwitchEmbedSurfaceFactory = TwitchEmbedSurface Function({
  required String html,
  required Uri baseUrl,
  required void Function(String message) onMessage,
  required void Function(Uri url) onExternalLink,
  void Function(bool fullscreen)? onFullscreen,
  VoidCallback? onBack,
});

/// The web view of [engine].
TwitchEmbedSurfaceFactory twitchEmbedSurfaceFor(TwitchEmbedEngine engine) =>
    switch (engine) {
      TwitchEmbedEngine.webviewFlutter => WebViewTwitchSurface.new,
      TwitchEmbedEngine.webView2 => WindowsTwitchSurface.new,
      TwitchEmbedEngine.webKitGtk => LinuxTwitchSurface.new,
    };

/// The engine of this platform, null on the web.
TwitchEmbedEngine? get twitchEmbedEngine =>
    kIsWeb ? null : twitchEmbedEngineFor(defaultTargetPlatform);

/// Is the runtime of [engine] installed: always for `webview_flutter`,
/// the WebView2 Runtime on Windows, WebKitGTK on Linux.
Future<bool> twitchEmbedRuntimeAvailable(TwitchEmbedEngine engine) =>
    switch (engine) {
      TwitchEmbedEngine.webviewFlutter => Future.value(true),
      TwitchEmbedEngine.webView2 => windowsWebView2Available(),
      TwitchEmbedEngine.webKitGtk => linuxWebKitAvailable(),
    };

/// The `TwitchService.embedPlayer` hook: the embed screen for [target].
Future<void> openTwitchEmbed(
  BuildContext context,
  TwitchPlayTarget target, {
  TwitchEmbedSurfaceFactory? surface,
  TwitchEmbedEngine? engine,
}) => Navigator.of(context, rootNavigator: true).push(
  // A player screen like pa_player's: over the whole window (no sidebar),
  // so fullscreen leaves nothing but the video (TWI-029).
  MaterialPageRoute<void>(
    builder: (_) =>
        TwitchEmbedScreen(target: target, surface: surface, engine: engine),
  ),
);

/// Registers the embed as the Twitch section's player where a web view
/// exists (TWI-005, TWI-029). Without its runtime a desktop keeps "Open on
/// Twitch" and [TwitchService.embedMissing] says what to install.
/// [engine] and [available] are for tests.
Future<void> registerTwitchEmbed(
  TwitchService twitch, {
  TwitchEmbedEngine? engine,
  Future<bool> Function(TwitchEmbedEngine engine)? available,
}) async {
  final e = engine ?? twitchEmbedEngine;
  if (e == null) return;
  var ok = false;
  try {
    ok = await (available ?? twitchEmbedRuntimeAvailable)(e);
  } catch (error) {
    debugPrint('Twitch embed runtime check failed: $error');
  }
  if (ok) {
    twitch.embedMissing = null;
    twitch.embedPlayer = (context, target) =>
        openTwitchEmbed(context, target, engine: e);
  } else {
    twitch.embedPlayer = null;
    twitch.embedMissing = twitchEmbedMissingFor(e);
  }
}

class TwitchEmbedScreen extends StatefulWidget {
  const TwitchEmbedScreen({
    super.key,
    required this.target,
    this.surface,
    this.engine,
  });

  final TwitchPlayTarget target;

  /// The web view; null = the one of [engine].
  final TwitchEmbedSurfaceFactory? surface;

  /// null = this platform's.
  final TwitchEmbedEngine? engine;

  @override
  State<TwitchEmbedScreen> createState() => _TwitchEmbedScreenState();
}

class _TwitchEmbedScreenState extends State<TwitchEmbedScreen> {
  late TwitchEmbedSession _session;
  late TwitchEmbedSurface _surface;
  late AppServices _services;
  var _attempt = 0;
  var _phone = false;

  /// The player fills the window: no bar above it (TWI-029).
  var _fullscreen = false;

  /// The window itself is fullscreen because of this screen.
  var _windowFullscreen = false;

  TwitchEmbedEngine get _engine =>
      widget.engine ?? twitchEmbedEngine ?? TwitchEmbedEngine.webviewFlutter;

  bool get _desktop =>
      _engine == TwitchEmbedEngine.webView2 ||
      _engine == TwitchEmbedEngine.webKitGtk;

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
    final factory = widget.surface ?? twitchEmbedSurfaceFor(_engine);
    _surface = factory(
      html: twitchEmbedHtml(widget.target),
      baseUrl: twitchEmbedBaseUrl(),
      onMessage: _session.handle,
      onExternalLink: (url) => unawaited(openExternalLink(url)),
      // The player's own fullscreen button: the web view already filled
      // the window (WebKitGTK) or asked for it (WebView2).
      onFullscreen: (on) {
        if (mounted) setState(() => _fullscreen = on);
      },
      onBack: _escape,
    );
  }

  Future<void> _retry() async {
    await _session.close();
    _surface.dispose();
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

  /// The dialog: a native web view (Linux) lies over Flutter and hides
  /// while it is open.
  Future<void> _openOnTwitch() async {
    twitchEmbedCovered.value++;
    try {
      await showOpenOnTwitch(context, _twitchUrl);
    } finally {
      twitchEmbedCovered.value--;
    }
  }

  /// F and the bar's button on desktops: the player fills the window.
  void _toggleFullscreen() {
    final on = !_fullscreen;
    setState(() => _fullscreen = on);
    _windowFullscreen = on;
    unawaited(setWindowFullscreen(on));
  }

  /// Escape, Back on a remote: leave fullscreen first, then the screen.
  void _escape() {
    if (!mounted) return;
    if (_fullscreen) {
      setState(() => _fullscreen = false);
      _windowFullscreen = false;
      unawaited(setWindowFullscreen(false));
      return;
    }
    unawaited(Navigator.of(context).maybePop());
  }

  void _togglePause() {
    final playing = _session.state.value == TwitchEmbedState.playing;
    final call = playing ? 'pause' : 'play'; // l10n-ignore
    unawaited(_surface.runJavaScript('window.kinolune.$call();'));
  }

  /// Arrows seek a VOD by 10 s (Twitch's own step).
  void _seekBy(int seconds) {
    var at = _session.position.inSeconds + seconds;
    final end = _session.duration.inSeconds;
    if (end > 0 && at > end) at = end;
    if (at < 0) at = 0;
    unawaited(_surface.runJavaScript('window.kinolune.seek($at);'));
  }

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    final key = event.logicalKey;
    final vod = !widget.target.isLive;
    if (key == LogicalKeyboardKey.escape) {
      _escape();
    } else if (key == LogicalKeyboardKey.space ||
        key == LogicalKeyboardKey.keyK ||
        key == LogicalKeyboardKey.mediaPlayPause) {
      _togglePause();
    } else if (key == LogicalKeyboardKey.keyF && _desktop) {
      _toggleFullscreen();
    } else if (vod &&
        (key == LogicalKeyboardKey.arrowLeft ||
            key == LogicalKeyboardKey.mediaRewind)) {
      _seekBy(-10);
    } else if (vod &&
        (key == LogicalKeyboardKey.arrowRight ||
            key == LogicalKeyboardKey.mediaFastForward)) {
      _seekBy(10);
    } else {
      return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  @override
  void dispose() {
    final twitch = _services.twitch;
    unawaited(_session.close().then((_) => twitch.progressChanged()));
    _surface.dispose();
    if (_phone) unawaited(SystemChrome.setPreferredOrientations(const []));
    if (_windowFullscreen) unawaited(setWindowFullscreen(false));
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final login = widget.target.login;
    final title = login == null || login.isEmpty
        ? l.twitchEmbedTitle
        : _services.twitch.channelOf(login).displayName;
    return Focus(
      autofocus: true,
      onKeyEvent: _onKey,
      child: Scaffold(
        key: const ValueKey('twitch-embed'),
        backgroundColor: Colors.black,
        // Nothing may cover the embed (Twitch rules): the bar sits above it
        // and goes away in fullscreen.
        appBar: _fullscreen
            ? null
            : AppBar(
                backgroundColor: Colors.black,
                foregroundColor: Colors.white,
                toolbarHeight: 44,
                title: Text(
                  title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
                actions: [
                  if (_desktop)
                    IconButton(
                      key: const ValueKey('twitch-embed-fullscreen'),
                      tooltip: l.twitchEmbedFullscreen,
                      icon: const Icon(Icons.fullscreen),
                      onPressed: _toggleFullscreen,
                    ),
                  IconButton(
                    key: const ValueKey('twitch-embed-open'),
                    tooltip: l.twitchOpenOnTwitch,
                    icon: const Icon(Icons.open_in_new),
                    onPressed: _openOnTwitch,
                  ),
                ],
              ),
        body: SafeArea(
          top: false,
          child: ValueListenableBuilder<TwitchEmbedState>(
            key: ValueKey(_attempt),
            valueListenable: _session.state,
            builder: (context, state, _) => state == TwitchEmbedState.failed
                ? _Failed(onRetry: _retry, onOpen: _openOnTwitch)
                : KeyedSubtree(
                    key: ValueKey('twitch-embed-surface-$_attempt'),
                    child: _surface.build(context),
                  ),
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
    void Function(bool fullscreen)? onFullscreen,
    VoidCallback? onBack,
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

  @override
  void dispose() {}
}
