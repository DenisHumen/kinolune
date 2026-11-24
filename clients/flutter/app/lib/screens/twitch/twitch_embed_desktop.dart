// SPDX-License-Identifier: MPL-2.0
//
// The official Twitch embed on Windows and Linux (TWI-029, ADR-NNNN).
// `webview_flutter` has no desktop implementation besides macOS, so:
//
// * Windows: Microsoft Edge WebView2 through `webview_windows`
//   (BSD-3-Clause), drawn into a Flutter texture. The page is served by
//   WebView2 itself from a temporary folder mapped to the project's HTTPS
//   host (`SetVirtualHostNameToFolderMapping`), so its origin is the
//   embed's `parent` and nothing is fetched from that host. Without the
//   WebView2 Runtime (old Windows 10 without Edge updates) the section
//   offers "Open on Twitch" and says what to install.
// * Linux: the system's WebKitGTK, opened at run time by `linux/runner`
//   (twitch_webview.cc) and laid over the Flutter view at the rectangle
//   this widget reports. It is a native widget above Flutter, so it hides
//   while another route (a dialog) is on top.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/rendering.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:webview_windows/webview_windows.dart';

import '../../integration/player.dart' show setWindowFullscreen;
import 'twitch_embed_screen.dart';

/// The page's error message: the screen shows "did not load" and retry.
final _errorMessage = jsonEncode({'event': 'error'}); // l10n-ignore

/// The WebView2 Runtime is installed (Windows).
Future<bool> windowsWebView2Available() async {
  try {
    return await WebviewController.getWebViewVersion() != null;
  } on Object catch (e) {
    debugPrint('WebView2 check failed: $e');
    return false;
  }
}

/// The embed in WebView2.
class WindowsTwitchSurface implements TwitchEmbedSurface {
  WindowsTwitchSurface({
    required this.html,
    required this.baseUrl,
    required this.onMessage,
    required this.onExternalLink,
    this.onFullscreen,
    this.onBack,
  }) {
    unawaited(_init());
  }

  final String html;
  final Uri baseUrl;
  final void Function(String message) onMessage;
  final void Function(Uri url) onExternalLink;
  final void Function(bool fullscreen)? onFullscreen;
  final VoidCallback? onBack;

  final _controller = WebviewController();
  final _subscriptions = <StreamSubscription<Object?>>[];
  Directory? _folder;
  var _disposed = false;

  /// WebView2 serves mapped files by extension: the page is an `.html`.
  late final Uri _pageUrl = baseUrl.replace(
    path: '${baseUrl.path}.html', // l10n-ignore
  );

  Future<void> _init() async {
    try {
      await _controller.initialize();
      if (_disposed) return;
      final folder = await Directory.systemTemp.createTemp('kinolune-twitch-');
      _folder = folder;
      final file = File(
        [folder.path, ..._pageUrl.pathSegments].join(Platform.pathSeparator),
      );
      await file.parent.create(recursive: true);
      await file.writeAsString(html);
      await _controller.addVirtualHostNameMapping(
        baseUrl.host,
        folder.path,
        WebviewHostResourceAccessKind.allow,
      );
      // Links that open a window (the Twitch logo) stay out: "Open on
      // Twitch" is in the bar.
      await _controller.setPopupWindowPolicy(WebviewPopupWindowPolicy.deny);
      await _controller.setBackgroundColor(Colors.black);
      _subscriptions
        ..add(
          _controller.webMessage.listen((m) {
            if (m is String) onMessage(m);
          }, onError: (Object _) {}),
        )
        ..add(
          _controller.containsFullScreenElementChanged.listen((on) {
            // WebView2 fills itself; the window follows.
            unawaited(setWindowFullscreen(on));
            onFullscreen?.call(on);
          }),
        )
        ..add(_controller.url.listen(_onUrl))
        ..add(_controller.onLoadError.listen((_) => onMessage(_errorMessage)));
      if (_disposed) return;
      await _controller.loadUrl(_pageUrl.toString());
    } on Object catch (e) {
      debugPrint('Twitch embed (WebView2) failed: $e');
      if (!_disposed) onMessage(_errorMessage);
    }
  }

  /// The page left its URL (a link with target=_top): the link goes to
  /// the browser, the page comes back.
  void _onUrl(String url) {
    if (url == _pageUrl.toString() || url.startsWith('about:')) return;
    final uri = Uri.tryParse(url);
    if (uri != null && uri.scheme == 'https') onExternalLink(uri);
    unawaited(_controller.loadUrl(_pageUrl.toString()));
  }

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<WebviewValue>(
    valueListenable: _controller,
    builder: (context, value, _) => value.isInitialized
        ? Webview(_controller)
        : const ColoredBox(color: Colors.black, child: SizedBox.expand()),
  );

  @override
  Future<void> runJavaScript(String script) async {
    if (_disposed || !_controller.value.isInitialized) return;
    await _controller.executeScript(script);
  }

  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    for (final s in _subscriptions) {
      unawaited(s.cancel());
    }
    unawaited(_controller.dispose());
    final folder = _folder;
    if (folder != null) {
      unawaited(
        folder.delete(recursive: true).then((_) {}, onError: (Object _) {}),
      );
    }
  }
}

/// The channel of `linux/runner/twitch_webview.cc`.
const linuxTwitchWebViewChannel = MethodChannel(
  'app.kinolune/twitch_webview', // l10n-ignore
);

/// WebKitGTK can be opened (Linux).
Future<bool> linuxWebKitAvailable() async {
  try {
    return await linuxTwitchWebViewChannel.invokeMethod<bool>('available') ??
        false;
  } on Object catch (e) {
    debugPrint('WebKitGTK check failed: $e');
    return false;
  }
}

/// The embed in the runner's WebKitGTK view. One at a time: the runner
/// keeps a single web view.
class LinuxTwitchSurface implements TwitchEmbedSurface {
  LinuxTwitchSurface({
    required String html,
    required Uri baseUrl,
    required this.onMessage,
    required this.onExternalLink,
    this.onFullscreen,
    this.onBack,
    MethodChannel channel = linuxTwitchWebViewChannel,
  }) : _channel = channel {
    _channel.setMethodCallHandler(_onCall);
    unawaited(_create(html, baseUrl));
  }

  final void Function(String message) onMessage;
  final void Function(Uri url) onExternalLink;
  final void Function(bool fullscreen)? onFullscreen;
  final VoidCallback? onBack;
  final MethodChannel _channel;
  var _disposed = false;
  Rect? _rect;
  var _visible = false;

  Future<void> _create(String html, Uri baseUrl) async {
    try {
      final ok = await _channel.invokeMethod<bool>('create', {
        'html': html,
        'baseUrl': baseUrl.toString(),
      });
      if (ok != true) throw StateError('no WebKitGTK');
      if (_disposed) {
        await _channel.invokeMethod<void>('dispose');
        return;
      }
      // The rectangle may have come before the view.
      final rect = _rect;
      if (rect != null) await _sendRect(rect, _visible);
    } on Object catch (e) {
      debugPrint('Twitch embed (WebKitGTK) failed: $e');
      if (!_disposed) onMessage(_errorMessage);
    }
  }

  Future<Object?> _onCall(MethodCall call) async {
    if (_disposed) return null;
    switch (call.method) {
      case 'message':
        onMessage(call.arguments as String);
      case 'external':
        final url = Uri.tryParse(call.arguments as String);
        if (url != null) onExternalLink(url);
      case 'fullscreen':
        onFullscreen?.call(call.arguments == true);
      case 'back':
        onBack?.call();
      case 'failed':
        onMessage(_errorMessage);
    }
    return null;
  }

  Future<void> _sendRect(Rect rect, bool visible) =>
      _channel.invokeMethod<void>('rect', {
        'x': rect.left,
        'y': rect.top,
        'width': rect.width,
        'height': rect.height,
        'visible': visible,
      });

  void _place(Rect rect, bool visible) {
    if (_disposed || (rect == _rect && visible == _visible)) return;
    _rect = rect;
    _visible = visible;
    unawaited(_sendRect(rect, visible).catchError((Object _) {}));
  }

  @override
  Widget build(BuildContext context) => _NativeViewport(surface: this);

  @override
  Future<void> runJavaScript(String script) async {
    if (_disposed) return;
    await _channel.invokeMethod<void>('runJavaScript', {'script': script});
  }

  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _channel.setMethodCallHandler(null);
    unawaited(
      _channel.invokeMethod<void>('dispose').catchError((Object _) {}),
    );
  }
}

/// Dialogs open over the embed screen. They may belong to another
/// navigator (the root one), which leaves the screen's route current, so
/// the screen counts them here and the native view hides while any is
/// open.
final twitchEmbedCovered = ValueNotifier<int>(0);

/// A black box whose place on screen the native view takes; the view
/// shows only while this route is the top one and no dialog is open.
class _NativeViewport extends StatefulWidget {
  const _NativeViewport({required this.surface});

  final LinuxTwitchSurface surface;

  @override
  State<_NativeViewport> createState() => _NativeViewportState();
}

class _NativeViewportState extends State<_NativeViewport> {
  var _routeShown = true;
  Rect _rect = Rect.zero;

  bool get _shown => _routeShown && twitchEmbedCovered.value == 0;

  @override
  void initState() {
    super.initState();
    twitchEmbedCovered.addListener(_covered);
  }

  void _covered() => widget.surface._place(_rect, _shown);

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _routeShown =
        (ModalRoute.of(context)?.isCurrent ?? true) && TickerMode.of(context);
    widget.surface._place(_rect, _shown);
  }

  @override
  void dispose() {
    twitchEmbedCovered.removeListener(_covered);
    widget.surface._place(Rect.zero, false);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => _RectReporter(
    onRect: (rect) {
      _rect = rect;
      widget.surface._place(rect, _shown);
    },
    child: const ColoredBox(color: Colors.black, child: SizedBox.expand()),
  );
}

/// Reports its rectangle in the window after every paint.
class _RectReporter extends SingleChildRenderObjectWidget {
  const _RectReporter({required this.onRect, super.child});

  final void Function(Rect rect) onRect;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderRectReporter(onRect);

  @override
  void updateRenderObject(BuildContext context, _RenderRectReporter render) =>
      render.onRect = onRect;
}

class _RenderRectReporter extends RenderProxyBox {
  _RenderRectReporter(this.onRect);

  void Function(Rect rect) onRect;
  var _scheduled = false;

  @override
  void paint(PaintingContext context, Offset offset) {
    super.paint(context, offset);
    if (_scheduled) return;
    _scheduled = true;
    SchedulerBinding.instance.addPostFrameCallback((_) {
      _scheduled = false;
      if (!attached || !hasSize) return;
      onRect(localToGlobal(Offset.zero) & size);
    });
  }
}
