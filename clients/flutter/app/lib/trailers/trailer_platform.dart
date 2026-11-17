// SPDX-License-Identifier: MPL-2.0
//
// What trailers need from the platform (UI-036, ADR-0100): a surface that
// plays one (our player for own and server files; the official embedded
// player of YouTube or Vimeo in a WebView for TMDB videos), whether Battery
// Saver is on, whether the network is mobile data, and opening the video's
// page in the site's app where no WebView exists (Linux, Windows). Tests
// replace [TrailerPlatform.instance].
//
// YouTube plays only through its IFrame API from `youtube-nocookie.com`:
// the page gets the video id from TMDB and nothing else; no stream is ever
// downloaded or extracted (YouTube terms). The policies of the YouTube API
// forbid drawing anything over the embedded player, so the banner gives an
// online video its own frame that nothing covers (trailer_banner.dart).

import 'dart:async';
import 'dart:convert';
import 'dart:io' show Platform;

import 'package:connectivity_plus/connectivity_plus.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart'
    show PlayerEngine, PlayerEngineFactory, QueueRepeat;
import 'package:webview_flutter/webview_flutter.dart';
import 'package:webview_flutter_android/webview_flutter_android.dart';
import 'package:webview_flutter_wkwebview/webview_flutter_wkwebview.dart';

import '../integration/library.dart' show Trailer, TrailerSource;
import '../offline/network_watch.dart' show offlineNetworkOf;
import '../offline/offline_downloads.dart' show OfflineNetwork;
import '../services/external_links.dart';
import '../twitch/twitch_embed.dart' show twitchEmbedParent;

/// A playing trailer: the banner's muted preview or the full screen.
abstract class TrailerSurface {
  Widget build(BuildContext context);

  /// The video shows frames: the backdrop may fade out.
  ValueListenable<bool> get showing;

  /// The video cannot play (removed, blocked, no network).
  ValueListenable<bool> get failed;

  Future<void> play();
  Future<void> pause();
  Future<void> dispose();
}

/// Platform services of trailers; [instance] is replaced in tests.
class TrailerPlatform {
  static TrailerPlatform instance = TrailerPlatform();

  /// A WebView for the official embedded players exists here
  /// (`webview_flutter`: Android, iOS, macOS).
  bool get webEmbed =>
      !kIsWeb && (Platform.isAndroid || Platform.isIOS || Platform.isMacOS);

  /// Android Battery Saver is on; false elsewhere or when unknown.
  Future<bool> powerSaving() async {
    if (kIsWeb || !Platform.isAndroid) return false;
    try {
      return await const MethodChannel('app.kinolune/device')
              .invokeMethod<bool>('powerSaveMode') ??
          false;
    } on PlatformException {
      return false;
    } on MissingPluginException {
      return false;
    }
  }

  /// The device is on mobile data only.
  Future<bool> metered() async {
    try {
      final r = await Connectivity().checkConnectivity();
      return offlineNetworkOf(r) == OfflineNetwork.metered;
    } catch (_) {
      return false;
    }
  }

  /// A surface for [trailer], or null when this platform cannot play it
  /// in the app (an online video without a WebView). [muted] and [loop]
  /// for the banner; the full screen has sound and controls.
  TrailerSurface? surface(
    Trailer trailer, {
    required bool muted,
    required bool loop,
    required PlayerEngineFactory engines,
    String language = 'en',
  }) {
    final uri = trailer.playbackUri;
    if (uri != null) {
      return EngineTrailerSurface(uri, engines, muted: muted, loop: loop);
    }
    final key = trailer.videoKey;
    if (!trailer.source.online || key == null || !webEmbed) return null;
    return WebTrailerSurface(
      trailerEmbedHtml(
        trailer.source,
        key,
        muted: muted,
        loop: loop,
        language: language,
      ),
    );
  }

  /// Opens the video's page in the site's app or the browser.
  Future<bool> openExternal(Uri url) => openExternalLink(url);
}

/// An own or server trailer in our player (media_kit, ADR-0012).
class EngineTrailerSurface implements TrailerSurface {
  EngineTrailerSurface(
    this.uri,
    PlayerEngineFactory engines, {
    required this.muted,
    required this.loop,
  }) : _engine = engines(video: true) {
    _engine.state.addListener(_onState);
    _ready = _open();
  }

  final String uri;
  final bool muted;
  final bool loop;
  final PlayerEngine _engine;
  late final Future<void> _ready;
  bool _disposed = false;

  @override
  final ValueNotifier<bool> showing = ValueNotifier(false);
  @override
  final ValueNotifier<bool> failed = ValueNotifier(false);

  Future<void> _open() async {
    try {
      if (muted) await _engine.setVolume(0);
      if (loop) await _engine.setRepeat(QueueRepeat.one);
      await _engine.open([uri], play: false, autoLoadSubtitles: false);
    } catch (e) {
      debugPrint('Trailer not opened: $e');
      failed.value = true;
    }
  }

  void _onState() {
    final s = _engine.state.value;
    if (s.error != null) failed.value = true;
    // A quiet fade before the end of each loop.
    final left = s.duration - s.position;
    final ending =
        s.duration > Duration.zero && left < const Duration(milliseconds: 900);
    showing.value = s.playing && s.position > Duration.zero && !ending;
  }

  @override
  Widget build(BuildContext context) =>
      _engine.buildVideo(context, fit: BoxFit.cover);

  @override
  Future<void> play() async {
    await _ready;
    if (!_disposed) await _engine.play();
  }

  @override
  Future<void> pause() async {
    await _ready;
    if (!_disposed) await _engine.pause();
  }

  @override
  Future<void> dispose() async {
    if (_disposed) return;
    _disposed = true;
    _engine.state.removeListener(_onState);
    await _engine.dispose();
  }
}

/// Name of the JavaScript channel of the embed page.
const trailerEmbedChannel = 'KinoluneTrailer'; // l10n-ignore

/// The origin the embed page is loaded with (the project's HTTPS site, as
/// for the Twitch embed): YouTube's player wants a referrer; nothing is
/// fetched from that site.
Uri trailerEmbedBaseUrl() =>
    Uri.parse('https://$twitchEmbedParent/kinolune/trailer'); // l10n-ignore

/// The embed page of an online trailer: the YouTube IFrame API with the
/// `youtube-nocookie.com` host, or the Vimeo player. [key] is the id the
/// core validated; it is JSON-encoded into the page anyway.
String trailerEmbedHtml(
  TrailerSource site,
  String key, {
  required bool muted,
  required bool loop,
  String language = 'en',
}) {
  final options = jsonEncode({
    'site': site.name,
    'key': key,
    'muted': muted,
    'loop': loop,
    'controls': !muted,
    'language': language,
    'origin': trailerEmbedBaseUrl().origin,
  }).replaceAll('<', r'\u003c');
  return _page.replaceFirst('/*OPTIONS*/null', options);
}

// Events go to Dart as plain words: ready, playing, paused, ended, error.
const _page = r'''<!DOCTYPE html>
<html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="referrer" content="strict-origin-when-cross-origin">
<style>html,body{margin:0;width:100%;height:100%;background:#000;overflow:hidden}
#player,iframe{position:absolute;top:0;left:0;width:100%;height:100%;border:0}</style>
</head><body><div id="player"></div>
<script>
(function () {
  var o = /*OPTIONS*/null;
  var player = null;
  function post(m) { try { KinoluneTrailer.postMessage(m); } catch (e) {} }
  if (o.site === 'vimeo') {
    var f = document.createElement('iframe');
    var q = 'autoplay=1&dnt=1&title=0&byline=0&portrait=0' +
      (o.muted ? '&muted=1&background=1' : '') + (o.loop ? '&loop=1' : '');
    f.src = 'https://player.vimeo.com/video/' + encodeURIComponent(o.key) + '?' + q;
    f.allow = 'autoplay; fullscreen; picture-in-picture';
    document.getElementById('player').replaceWith(f);
    f.onload = function () { post('ready'); post('playing'); };
    window.kinolune = {
      play: function () { f.contentWindow.postMessage('{"method":"play"}', '*'); },
      pause: function () { f.contentWindow.postMessage('{"method":"pause"}', '*'); }
    };
    return;
  }
  window.onYouTubeIframeAPIReady = function () {
    player = new YT.Player('player', {
      host: 'https://www.youtube-nocookie.com',
      videoId: o.key,
      playerVars: {
        autoplay: 1, mute: o.muted ? 1 : 0, controls: o.controls ? 1 : 0,
        loop: o.loop ? 1 : 0, playlist: o.key, playsinline: 1, rel: 0,
        iv_load_policy: 3, fs: 0, disablekb: o.controls ? 0 : 1,
        hl: o.language, origin: o.origin
      },
      events: {
        onReady: function (e) {
          if (o.muted) e.target.mute();
          e.target.playVideo();
          post('ready');
        },
        onStateChange: function (e) {
          if (e.data === 1) post('playing');
          else if (e.data === 2) post('paused');
          else if (e.data === 0) post('ended');
        },
        onError: function () { post('error'); }
      }
    });
  };
  var s = document.createElement('script');
  s.src = 'https://www.youtube.com/iframe_api';
  s.onerror = function () { post('error'); };
  document.head.appendChild(s);
  window.kinolune = {
    play: function () { if (player && player.playVideo) player.playVideo(); },
    pause: function () { if (player && player.pauseVideo) player.pauseVideo(); }
  };
})();
</script></body></html>''';

/// A TMDB video in the official embedded player of its site.
class WebTrailerSurface implements TrailerSurface {
  WebTrailerSurface(String html) {
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
    final base = trailerEmbedBaseUrl().toString();
    unawaited(_controller.setJavaScriptMode(JavaScriptMode.unrestricted));
    unawaited(_controller.setBackgroundColor(Colors.black));
    unawaited(
      _controller.addJavaScriptChannel(
        trailerEmbedChannel,
        onMessageReceived: (m) => handle(m.message),
      ),
    );
    unawaited(
      _controller.setNavigationDelegate(
        NavigationDelegate(
          onNavigationRequest: (request) {
            // The players load in iframes; a link out of them (the
            // YouTube logo) opens in the site's app or the browser.
            if (!request.isMainFrame || request.url == base) {
              return NavigationDecision.navigate;
            }
            final url = Uri.tryParse(request.url);
            if (url != null && url.scheme == 'https') {
              unawaited(openExternalLink(url));
            }
            return NavigationDecision.prevent;
          },
        ),
      ),
    );
    unawaited(_controller.loadHtmlString(html, baseUrl: base));
  }

  late final WebViewController _controller;

  @override
  final ValueNotifier<bool> showing = ValueNotifier(false);
  @override
  final ValueNotifier<bool> failed = ValueNotifier(false);

  /// A message of the page.
  @visibleForTesting
  void handle(String message) {
    switch (message) {
      case 'playing':
        showing.value = true;
      case 'error':
        failed.value = true;
        showing.value = false;
    }
  }

  @override
  Widget build(BuildContext context) => WebViewWidget(controller: _controller);

  Future<void> _run(String js) async {
    try {
      await _controller.runJavaScript(js);
    } catch (_) {
      // The page is not ready yet: it starts by itself.
    }
  }

  @override
  Future<void> play() => _run('window.kinolune && window.kinolune.play()');

  @override
  Future<void> pause() => _run('window.kinolune && window.kinolune.pause()');

  @override
  Future<void> dispose() => pause();
}
