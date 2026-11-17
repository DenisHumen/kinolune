// SPDX-License-Identifier: MPL-2.0
//
// The trailer of a movie or show page (UI-036, ADR-0100): which one (the
// core picks it), when the banner plays it, and the full screen.
//
// The banner starts muted about 1.5 s after the page opened, in a loop,
// when everything allows it:
//
// - autoplay is not «никогда», nor «только Wi-Fi» on mobile data (for
//   trailers that stream: online or from the server);
// - the system does not ask to reduce motion, Battery Saver is off;
// - the banner is on screen (not scrolled away), the page is not covered
//   by another route, the app is in the foreground, and on a TV the focus
//   is still in the top part of the page.
//
// Otherwise it pauses, and resumes when allowed again. Without a playable
// trailer the backdrop slowly moves instead («живой» фон), under the same
// conditions. Full screen plays with sound: own and server files in the
// app's player, online videos in the official embedded player (or in the
// site's app where there is no WebView). No request reaches YouTube before
// a page is opened; with internet trailers off none at all.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart' show MediaKitEngine;

import '../integration/library.dart';
import '../integration/player.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../sources/jellyfin_link.dart' show JellyfinTrailers;
import 'trailer_platform.dart';
import 'trailer_settings.dart';

/// How long the page shows its still backdrop before the trailer starts.
const trailerAutoplayDelay = Duration(milliseconds: 1500);

/// What the top of the page shows.
enum TrailerBannerMode {
  /// The still backdrop.
  still,

  /// The backdrop slowly moving: no playable trailer.
  live,

  /// The trailer behind the scrims (own and server files).
  fullBleed,

  /// An online trailer in its own frame that nothing covers (YouTube and
  /// Vimeo policies).
  framed,
}

class TrailerController extends ChangeNotifier with WidgetsBindingObserver {
  TrailerController({required this.item, TrailerPlatform? platform})
    : _platform = platform ?? TrailerPlatform.instance;

  final LibraryItem item;
  final TrailerPlatform _platform;

  Trailer? _trailer;
  bool _loaded = false;
  bool _disposed = false;
  TrailerSettings _settings = const TrailerSettings();
  bool _powerSaving = false;
  bool _metered = false;
  PlayerEngineFactory? _engines;
  String _language = 'en';

  // Conditions of the banner.
  bool _delayDone = false;
  bool _visible = true;
  bool _covered = false;
  bool _foreground = true;
  bool _focusAway = false;
  bool _reduceMotion = false;
  bool _frameFits = true;
  Timer? _delay;

  TrailerSurface? _surface;
  bool _noSurface = false;
  bool _playing = false;

  /// The page's trailer; null while loading or when there is none.
  Trailer? get trailer => _trailer;
  bool get loaded => _loaded;
  TrailerSettings get settings => _settings;

  /// The banner's video, once it was started.
  TrailerSurface? get surface => _surface;

  /// The banner's video plays now (tests).
  bool get playing => _playing;

  /// Reads the settings and asks the core for the trailer; starts the
  /// autoplay delay. Call once from the page.
  Future<void> load(BuildContext context) async {
    final services = Services.read(context);
    _engines = services.playerEngineFactory;
    _language = Localizations.localeOf(context).toLanguageTag();
    WidgetsBinding.instance.addObserver(this);
    _settings = await TrailerSettings.read(services.preferences);
    String? server;
    if (services.jellyfin case final JellyfinTrailers j) {
      try {
        server = await j.serverTrailer(item.id);
      } catch (e) {
        debugPrint('Server trailer of ${item.id} not read: $e');
      }
    }
    Trailer? found;
    if (services.library case final LibraryTrailers lib) {
      try {
        found = await lib.trailer(
          item.id,
          uiLanguage: _language,
          internet: _settings.internet,
          serverUrl: server,
        );
      } catch (e) {
        debugPrint('Trailer of ${item.id} not found: $e');
      }
    }
    // «Только Wi-Fi»: a trailer that streams waits for Wi-Fi.
    if (found != null &&
        found.source != TrailerSource.file &&
        _settings.autoplay == TrailerAutoplay.wifiOnly) {
      _metered = await _platform.metered();
    }
    _powerSaving = await _platform.powerSaving();
    if (_disposed) return;
    _trailer = found;
    _loaded = true;
    _delay = Timer(trailerAutoplayDelay, () {
      _delayDone = true;
      _update();
    });
    _update();
  }

  /// Autoplay is allowed by the settings, Battery Saver and the network.
  bool get autoplayAllowed {
    if (_settings.autoplay == TrailerAutoplay.never || _powerSaving) {
      return false;
    }
    final streams = _trailer?.source != TrailerSource.file;
    return !(_settings.autoplay == TrailerAutoplay.wifiOnly &&
        _metered &&
        streams);
  }

  bool get _conditions =>
      _loaded &&
      _delayDone &&
      autoplayAllowed &&
      !_reduceMotion &&
      _visible &&
      !_covered &&
      _foreground &&
      !_focusAway;

  /// What the top of the page shows now.
  TrailerBannerMode get mode {
    final t = _trailer;
    if (!_loaded || !_delayDone || !autoplayAllowed || _reduceMotion) {
      return TrailerBannerMode.still;
    }
    if (t == null || _noSurface || (_surface?.failed.value ?? false)) {
      return TrailerBannerMode.live;
    }
    if (t.source.online) {
      return _frameFits ? TrailerBannerMode.framed : TrailerBannerMode.live;
    }
    return TrailerBannerMode.fullBleed;
  }

  /// The backdrop moves (and not when the page is away or paused).
  bool get backdropMoves => mode == TrailerBannerMode.live && _conditions;

  // ---- conditions reported by the page

  /// More than half of the banner is on screen.
  void setVisible(bool v) => _set(() => _visible = v, _visible != v);

  /// Another route covers the page (full screen, player, a dialog page).
  void setCovered(bool v) => _set(() => _covered = v, _covered != v);

  /// TV: the focus left the top part of the page.
  void setFocusAway(bool v) => _set(() => _focusAway = v, _focusAway != v);

  /// The system asks to reduce motion.
  void setReduceMotion(bool v) =>
      _set(() => _reduceMotion = v, _reduceMotion != v);

  /// An online trailer's frame fits above the title and buttons.
  void setFrameFits(bool v) => _set(() => _frameFits = v, _frameFits != v);

  void _set(VoidCallback change, bool changed) {
    if (!changed) return;
    change();
    _update();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    final fg = switch (state) {
      AppLifecycleState.resumed || AppLifecycleState.inactive => true,
      _ => false,
    };
    if (fg && !_foreground) {
      // Battery Saver may have changed while away.
      unawaited(
        _platform.powerSaving().then((v) {
          if (_disposed || v == _powerSaving) return;
          _powerSaving = v;
          _update();
        }),
      );
    }
    _set(() => _foreground = fg, _foreground != fg);
  }

  void _update() {
    if (_disposed) return;
    final t = _trailer;
    final wanted =
        _conditions &&
        t != null &&
        !_noSurface &&
        (!t.source.online || _frameFits);
    if (wanted && _surface == null) {
      final s = _platform.surface(
        t,
        muted: true,
        loop: true,
        engines: _engines ?? MediaKitEngine.create,
        language: _language,
      );
      if (s == null) {
        _noSurface = true;
      } else {
        _surface = s;
        s.failed.addListener(_onFailed);
        s.showing.addListener(notifyListeners);
      }
    }
    final s = _surface;
    if (s != null && wanted != _playing) {
      _playing = wanted;
      unawaited(wanted ? s.play() : s.pause());
    }
    notifyListeners();
  }

  void _onFailed() {
    if (_surface?.failed.value ?? false) {
      _playing = false;
      notifyListeners();
    }
  }

  /// Plays the trailer full screen with sound; back returns to the page
  /// where it was. Online videos without a WebView open in the site's app.
  Future<void> openFullScreen(BuildContext context) async {
    final t = _trailer;
    if (t == null) return;
    final services = Services.read(context);
    final uri = t.playbackUri;
    if (uri != null) {
      services.music.pause();
      await Navigator.of(context, rootNavigator: true).push(
        MaterialPageRoute<void>(
          fullscreenDialog: true,
          builder: (_) => PlayerScreen(
            request: PlayRequest(
              uri: uri,
              title: t.name.isEmpty
                  ? item.title
                  : '${item.title}  ${t.name}', // l10n-ignore
              artworkUri: item.backdropUrl ?? item.posterUrl,
              durationMs: t.extra?.durationMs,
            ),
            engineFactory: services.playerEngineFactory,
            onProgress: (_, _) {},
            subtitleSettings: services.subtitleSettings,
            subtitleLanguages: services.subtitleLanguages,
            seekStep: Duration(seconds: services.skipSeconds.value),
          ),
        ),
      );
      return;
    }
    if (!_platform.webEmbed || t.videoKey == null) {
      final url = t.url == null ? null : Uri.tryParse(t.url!);
      if (url != null) await _platform.openExternal(url);
      return;
    }
    services.music.pause();
    await Navigator.of(context, rootNavigator: true).push(
      MaterialPageRoute<void>(
        fullscreenDialog: true,
        builder: (_) => TrailerFullScreen(
          item: item,
          trailer: t,
          platform: _platform,
          language: _language,
        ),
      ),
    );
  }

  @override
  void dispose() {
    _disposed = true;
    _delay?.cancel();
    WidgetsBinding.instance.removeObserver(this);
    final s = _surface;
    if (s != null) {
      s.failed.removeListener(_onFailed);
      s.showing.removeListener(notifyListeners);
      unawaited(s.dispose());
    }
    super.dispose();
  }
}

/// An online trailer full screen with sound, in the official embedded
/// player; nothing is drawn over the player. Back (the remote's, Esc, the
/// bar's arrow) returns to the page.
class TrailerFullScreen extends StatefulWidget {
  const TrailerFullScreen({
    super.key,
    required this.item,
    required this.trailer,
    required this.platform,
    this.language = 'en',
  });

  final LibraryItem item;
  final Trailer trailer;
  final TrailerPlatform platform;
  final String language;

  @override
  State<TrailerFullScreen> createState() => _TrailerFullScreenState();
}

class _TrailerFullScreenState extends State<TrailerFullScreen> {
  TrailerSurface? _surface;

  @override
  void initState() {
    super.initState();
    _surface = widget.platform.surface(
      widget.trailer,
      muted: false,
      loop: false,
      engines: MediaKitEngine.create,
      language: widget.language,
    );
    unawaited(_surface?.play());
  }

  @override
  void dispose() {
    unawaited(_surface?.dispose());
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final s = _surface;
    return Scaffold(
      key: const ValueKey('trailer-full-screen'),
      backgroundColor: Colors.black,
      // The bar sits above the player, never over it.
      appBar: AppBar(
        backgroundColor: Colors.black,
        foregroundColor: Colors.white,
        toolbarHeight: 44,
        title: Text(
          l.trailerTitle(widget.item.title),
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
        ),
      ),
      body: SafeArea(
        top: false,
        child: s == null
            ? Center(child: Text(l.trailerUnavailable))
            : ValueListenableBuilder<bool>(
                valueListenable: s.failed,
                builder: (context, failed, _) => failed
                    ? Center(child: Text(l.trailerUnavailable))
                    : s.build(context),
              ),
      ),
    );
  }
}
