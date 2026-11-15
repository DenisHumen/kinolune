// SPDX-License-Identifier: MPL-2.0
import 'dart:async';
import 'dart:io' show Platform;
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:media_kit_video/media_kit_video.dart' as mkv;

import '../engine/media_kit_engine.dart';
import '../engine/player_engine.dart';
import '../l10n/pa_player_strings.dart';
import '../play_request.dart';
import '../subtitles/subtitle_overlay.dart';
import '../subtitles/subtitle_policy.dart';
import '../subtitles/subtitle_rendering.dart';
import '../subtitles/subtitle_style.dart';
import '../system/media_session.dart';
import '../system/pip.dart';
import '../ui/focus_button.dart';
import '../ui/scrubber.dart';
import '../ui/tokens.dart';
import 'osd_controller.dart';
import 'osd_model.dart';
import 'osd_overlays.dart';
import 'osd_panels.dart';
import 'progress_reporter.dart';
import 'screen_brightness.dart';
import 'up_next.dart';

/// Fullscreen video player with a tvOS-style OSD.
///
/// Texts come from [PaPlayerStrings.of] (English and Russian).
///
/// Controls: mouse/touch; keyboard (Space play/pause, ←/→ ±[seekStep],
/// ↑/↓ show OSD and move focus, PageUp/PageDown or media prev/next chapter,
/// [ / ] slower/faster, Z zoom, S statistics, C chapters, F fullscreen,
/// I info, M mute, Esc back); Android TV remote (Select play/pause or
/// activate, Skip Intro while the OSD is hidden; ←/→ seek; Back closes
/// panel / exits).
///
/// OSD (UI-013, design 01 §2.2–2.3): scrubber with trickplay previews and
/// chapter markers, "remaining" and "ends at", track, speed and zoom
/// panels, Skip Intro / Credits ([PlayRequest.skipSegments]), the Up Next
/// card (also during the credits), a live stats overlay; on touch screens
/// double-tap the sides to seek, swipe up/down on the left half for
/// brightness and on the right half for volume, pinch to zoom, and hold
/// for 2× speed. It hides after [PaTokens.osdAutoHide] of inactivity while
/// playing.
///
/// Playback session (PLAY-005): progress is reported every
/// [progressInterval] while playing, on pause, seek, exit and when the app
/// goes to the background. With [loadNext] the end of the video shows an
/// Up Next card that counts [upNextCountdown] down and plays the next item
/// in the same player ([continuousPlayback]); Enter plays it now, Esc
/// cancels.
///
/// Subtitles (PLAY-007): the sidecars of [PlayRequest.subtitles] are loaded
/// on open and a track is picked by [pickSubtitle] (the series' remembered
/// choice, else the user's [subtitleLanguages] and forced tracks); the
/// overlay draws them in the [subtitleSettings] style, by default the
/// system caption style. A choice or delay made in the subtitles panel is
/// reported to [onSubtitleMemory] for the request's series and carries over
/// to the next episode.
///
/// System integration (PLAY-AND-008): the player claims the
/// [mediaSession] (notification, lock screen, headset buttons, audio
/// focus) while it is open, and on Android enters picture-in-picture
/// ([pip]) from the OSD button or by itself when the user leaves the app
/// during playback; in the PiP window only the picture and subtitles are
/// drawn, and closing the window pauses the video.
class PlayerScreen extends StatefulWidget {
  const PlayerScreen({
    super.key,
    required this.request,
    required this.onProgress,
    this.onFinished,
    this.engineFactory,
    this.progressInterval = const Duration(seconds: 5),
    this.seekStep = const Duration(seconds: 10),
    this.loadNext,
    this.continuousPlayback = true,
    this.upNextCountdown = const Duration(seconds: 10),
    this.skipMode = SkipMode.button,
    this.autoSkipDelay = const Duration(seconds: 3),
    this.subtitleSettings,
    this.subtitleLanguages = const [],
    this.captionStyleLoader,
    this.onSubtitleMemory,
    this.screenBrightness,
    this.clock,
    this.speeds = kPlaybackSpeeds,
    this.osdActions,
    this.liveBar,
    this.canSeek,
    this.mediaSession,
    this.pip,
  });

  final PlayRequest request;

  /// Called every [progressInterval] while playing, and on pause, seek, exit.
  final void Function(int positionMs, int durationMs) onProgress;

  /// Called once when playback reaches the end.
  final VoidCallback? onFinished;

  /// Engine override (tests). Defaults to [MediaKitEngine].
  final PlayerEngineFactory? engineFactory;
  final Duration progressInterval;
  final Duration seekStep;

  /// Loads what plays after [request] (Up Next); null result: nothing.
  final Future<UpNextItem?> Function()? loadNext;

  /// Play the next item by itself after the countdown; off: the Up Next
  /// card waits for "Play now".
  final bool continuousPlayback;
  final Duration upNextCountdown;

  /// What happens in an intro, recap or credits segment
  /// ([PlayRequest.skipSegments]): a button, an automatic skip after
  /// [autoSkipDelay] of playback or at once, or nothing. Skipping credits
  /// that run to the end plays the Up Next item.
  final SkipMode skipMode;
  final Duration autoSkipDelay;

  /// Subtitle style settings; null: the defaults (system style).
  final ValueListenable<SubtitleSettings>? subtitleSettings;

  /// Languages the user reads, best first (ISO 639 codes, e.g. the UI
  /// language). The system caption language comes first if set.
  final List<String> subtitleLanguages;

  /// Reads the system caption style; default [SystemCaptionStyle.read].
  final Future<SystemCaptionStyle?> Function()? captionStyleLoader;

  /// Called when the user picks a subtitle track or changes the delay of
  /// an item with a [PlayRequest.seriesKey].
  final void Function(String seriesKey, SubtitleMemory memory)?
  onSubtitleMemory;

  /// Brightness of the swipe gesture; default [ScreenBrightness.platform].
  final ScreenBrightness? screenBrightness;

  /// Wall clock for "ends at" (tests); default [DateTime.now].
  final DateTime Function()? clock;

  /// Rates of the speed panel and `[`/`]` (Twitch VODs: up to 3×).
  final List<double> speeds;

  /// Extra buttons at the end of the OSD top bar (Twitch: quality, chat).
  final List<Widget> Function(BuildContext context)? osdActions;

  /// A live stream (TWI-014): this bar replaces the scrubber, and there are
  /// no ±[seekStep] buttons. Seeking with keys stays within the cache.
  final Widget Function(BuildContext context)? liveBar;

  /// False while seeking is not allowed (a live ad break); null: always.
  final bool Function()? canSeek;

  /// The system media session; default [SystemMediaSession.instance].
  final SystemMediaSession? mediaSession;

  /// Picture-in-picture; default [SystemPip.instance].
  final SystemPip? pip;

  @override
  State<PlayerScreen> createState() => PlayerScreenState();
}

/// Public for tests.
class PlayerScreenState extends State<PlayerScreen>
    with WidgetsBindingObserver {
  late final PlayerEngine engine;
  ProgressReporter? _progress;
  late final UpNextController upNext;

  /// What is playing now: [PlayerScreen.request] or an Up Next item.
  late PlayRequest request;
  late ProgressCallback _onProgress;
  Future<UpNextItem?> Function()? _loadNext;
  UpNextItem? _next;
  int _nextGeneration = 0;
  bool _endHandled = false;
  final OsdController osd = OsdController(autoHide: PaTokens.osdAutoHide);
  final FocusNode _rootFocus = FocusNode(debugLabel: 'player-root');
  final FocusNode _scrubberFocus = FocusNode(debugLabel: 'player-scrubber');
  final FocusNode _playFocus = FocusNode(debugLabel: 'player-play');
  bool _finished = false;
  bool _subtitlesLoaded = false;

  /// Auto-selection of the subtitle track runs once per file, after its
  /// sidecars are in the track list (or after [_subtitleWait]).
  bool _autoSelectPending = false;
  Timer? _autoSelectTimer;
  int _subtitleGeneration = 0;
  static const _subtitleWait = Duration(milliseconds: 1500);

  /// System caption style (null: none on this platform).
  SystemCaptionStyle? systemCaptions;
  late Future<void> _captionsLoaded;

  /// Choice and delay made in this session, and the series they belong to:
  /// the next episode of the same series starts with them.
  SubtitleMemory? _sessionMemory;
  String? _sessionSeries;
  bool _fullscreen = false;
  bool _closing = false;
  Offset? _doubleTapAt;

  // ---- UI-013 OSD state ----------------------------------------------------

  /// How the picture fills the screen (zoom panel, Z key).
  final ValueNotifier<VideoZoom> zoom = ValueNotifier(VideoZoom.fit);

  /// Pinch zoom of the picture, [kMinPinchScale]..[kMaxPinchScale].
  final ValueNotifier<double> pinchScale = ValueNotifier(1.0);

  /// The live statistics overlay is shown.
  final ValueNotifier<bool> statsVisible = ValueNotifier(false);

  /// Brightness of the swipe, 0..1 (window brightness on Android, a dimming
  /// layer elsewhere).
  final ValueNotifier<double> brightness = ValueNotifier(1.0);
  late final ScreenBrightness _brightness;
  bool _brightnessChanged = false;

  /// The transient feedback pill (volume, brightness, seek, zoom).
  final ValueNotifier<OsdFeedback?> feedback = ValueNotifier(null);
  Timer? _feedbackTimer;

  /// The video is held: playing at 2× until released.
  final ValueNotifier<bool> holding = ValueNotifier(false);
  double? _rateBeforeHold;

  /// Wall clock for "ends at", refreshed while paused too.
  final ValueNotifier<DateTime> now = ValueNotifier(DateTime.now());
  Timer? _clockTimer;

  /// Double-tap seeks in a row add up ("+30 s").
  Duration _tapSeekTotal = Duration.zero;
  Timer? _tapSeekTimer;

  /// One-finger vertical swipe or pinch in progress.
  _Gesture _gesture = _Gesture.none;
  Offset _gestureStart = Offset.zero;
  double _gestureFrom = 0;

  /// The credits segment the Up Next card was offered for.
  SkipSegment? _creditsOffered;

  /// The segment an automatic skip counts down in and the position the
  /// count started at; segments already skipped automatically are not
  /// skipped again (the user went back into them).
  SkipSegment? _autoSegment;
  Duration _autoEntry = Duration.zero;
  final Set<SkipSegment> _autoSkipped = {};
  final FocusNode _skipFocus = FocusNode(debugLabel: 'player-skip');

  bool get _isDesktop =>
      !kIsWeb && (Platform.isLinux || Platform.isWindows || Platform.isMacOS);

  // ---- PLAY-AND-008: media session and picture-in-picture -----------------

  late final MediaSessionClaim _sessionClaim;
  late final SystemPip pip;

  /// The device can show this player in a PiP window (the OSD button).
  bool pipSupported = false;

  /// Volume before another app's sound ducked the video.
  double? _duckedFrom;

  @override
  void initState() {
    super.initState();
    final factory = widget.engineFactory ?? MediaKitEngine.create;
    engine = factory(video: true);
    request = widget.request;
    _onProgress = widget.onProgress;
    _sessionClaim = (widget.mediaSession ?? SystemMediaSession.instance).claim(
      _onSessionCommand,
      debugLabel: 'video',
    );
    pip = widget.pip ?? SystemPip.instance;
    pip.dismissals.addListener(_onPipDismissed);
    pip.supported().then((v) {
      if (mounted && v != pipSupported) setState(() => pipSupported = v);
    });
    _loadNext = widget.loadNext;
    upNext = UpNextController(
      onPlay: playNext,
      countdown: widget.upNextCountdown,
      continuous: widget.continuousPlayback,
    );
    _startProgress();
    engine.state.addListener(_onEngine);
    osd.addListener(_onOsd);
    widget.subtitleSettings?.addListener(_onSubtitleSettings);
    _applySubtitleRendering();
    WidgetsBinding.instance.addObserver(this);
    _captionsLoaded = _loadCaptionStyle();
    _brightness = widget.screenBrightness ?? ScreenBrightness.platform();
    _readBrightness();
    now.value = _now();
    _clockTimer = Timer.periodic(
      const Duration(seconds: 15),
      (_) => now.value = _now(),
    );
    _openCurrent();
    _fetchNext();
    if (!kIsWeb && (Platform.isAndroid || Platform.isIOS)) {
      SystemChrome.setEnabledSystemUIMode(SystemUiMode.immersiveSticky);
    }
  }

  bool get _live => widget.liveBar != null;

  /// Publishes the playback to the system: the media session's state and
  /// whether leaving the app enters PiP (only while a picture plays).
  void _syncSystem(EngineState s) {
    if (_closing) return;
    final r = request;
    _sessionClaim.update(
      MediaSessionInfo(
        kind: _live ? MediaSessionKind.live : MediaSessionKind.video,
        id: r.itemId ?? r.uri,
        title: r.title,
        subtitle: r.artist,
        album: r.album,
        artworkUri: r.artworkUri,
        duration: _live ? Duration.zero : s.duration,
        position: s.position,
        playing: s.playing,
        buffering: s.buffering,
        ended: s.completed,
        rate: s.rate,
        hasNext: _next != null,
        seekable: !_live && widget.canSeek?.call() != false,
      ),
    );
    final w = s.videoWidth ?? 0;
    final h = s.videoHeight ?? 0;
    unawaited(
      pip.configure(
        autoEnter: s.playing && w > 0 && h > 0,
        width: w > 0 ? w : null,
        height: h > 0 ? h : null,
      ),
    );
  }

  /// A command from the notification, lock screen, headset or audio focus.
  void _onSessionCommand(MediaSessionCommand command, Duration? position) {
    if (!mounted || _closing) return;
    final s = engine.state.value;
    switch (command) {
      case MediaSessionCommand.play:
        if (s.completed) engine.seek(Duration.zero);
        engine.play();
      case MediaSessionCommand.pause:
      case MediaSessionCommand.stop:
        engine.pause();
      case MediaSessionCommand.seek:
        if (position != null && widget.canSeek?.call() != false) {
          engine.seek(position);
          _progress?.onSeek(position);
        }
      case MediaSessionCommand.next:
        final next = _next;
        if (next != null) playNext(next);
      case MediaSessionCommand.previous:
        if (widget.canSeek?.call() != false && !_live) {
          engine.seek(Duration.zero);
          _progress?.onSeek(Duration.zero);
        }
      case MediaSessionCommand.duck:
        if (_duckedFrom != null) return;
        _duckedFrom = s.volume;
        engine.setVolume(s.volume * 0.2);
      case MediaSessionCommand.unduck:
        final v = _duckedFrom;
        _duckedFrom = null;
        if (v != null) engine.setVolume(v);
    }
  }

  /// The user closed the PiP window: the video stops, as in other apps.
  void _onPipDismissed() {
    if (!mounted || _closing) return;
    engine.pause();
    _progress?.report();
  }

  /// Enters PiP now (the OSD button).
  Future<void> enterPip() async {
    final s = engine.state.value;
    final w = s.videoWidth ?? 0;
    final h = s.videoHeight ?? 0;
    osd.hide();
    await pip.enter(width: w > 0 ? w : null, height: h > 0 ? h : null);
  }

  DateTime _now() => (widget.clock ?? DateTime.now)();

  Future<void> _readBrightness() async {
    final v = await _brightness.read();
    if (!mounted || _brightnessChanged) return;
    brightness.value = v;
    setState(() {}); // dimsPicture may have changed
  }

  /// Chapters of what is playing: the app's (Twitch, probe), else the
  /// file's own.
  List<PlaybackChapter> chaptersOf(EngineState s) =>
      request.chapters ?? s.chapters;

  /// The skippable segment at the playhead, if any (never with
  /// [SkipMode.off]).
  SkipSegment? skipSegmentOf(EngineState s) => widget.skipMode == SkipMode.off
      ? null
      : skipSegmentAt(request.skipSegments, s.position);

  /// Whole seconds until the automatic skip of [seg] ([SkipMode.auto]),
  /// or null if none is counting down.
  int? autoSkipCountdown(SkipSegment seg, EngineState s) {
    if (widget.skipMode != SkipMode.auto || _autoSegment != seg) return null;
    final left = widget.autoSkipDelay - (s.position - _autoEntry);
    if (left <= Duration.zero) return null;
    return (left.inMilliseconds + 999) ~/ 1000;
  }

  /// Opens [request] in the engine with its subtitle delay.
  Future<void> _openCurrent() {
    _subtitleGeneration++;
    _autoSelectTimer?.cancel();
    _autoSelectPending = false;
    // The first duration the engine reports after this loads the file's
    // subtitles (engines reset the duration when a file is opened).
    _subtitlesLoaded = false;
    final opened = engine.open(
      [request.uri],
      start: Duration(milliseconds: request.startPositionMs),
      autoLoadSubtitles: request.subtitles == null,
    );
    // sub-delay is an option of the player, not of the file: set it for
    // every file, to the series' value or back to zero.
    final delay = _memoryFor(request)?.delay ?? Duration.zero;
    if (delay != engine.state.value.subtitleDelay) {
      engine.setSubtitleDelay(delay);
    }
    return opened;
  }

  Future<void> _loadCaptionStyle() async {
    final loader = widget.captionStyleLoader ?? SystemCaptionStyle.read;
    SystemCaptionStyle? style;
    try {
      style = await loader();
    } catch (_) {
      style = null;
    }
    if (!mounted) return;
    if (style != systemCaptions) setState(() => systemCaptions = style);
  }

  void _onSubtitleSettings() {
    _applySubtitleRendering();
    if (mounted) setState(() {});
  }

  /// Tells an engine that draws subtitles itself (PLAY-006) whether ASS
  /// keeps its own style and how bright subtitles are over HDR.
  void _applySubtitleRendering() {
    final e = engine;
    if (e is! SubtitleRenderingEngine) return;
    final options = (widget.subtitleSettings?.value ?? const SubtitleSettings())
        .renderOptions;
    (e as SubtitleRenderingEngine).setSubtitleRendering(options);
  }

  /// The subtitle look now.
  SubtitleAppearance get subtitleAppearance =>
      (widget.subtitleSettings?.value ?? const SubtitleSettings()).resolve(
        systemCaptions,
      );

  /// What is remembered for [r]'s series: this session's choice if it is
  /// the same series, else what the app stored.
  SubtitleMemory? _memoryFor(PlayRequest r) {
    final key = r.seriesKey;
    if (key != null && key == _sessionSeries) return _sessionMemory;
    return r.subtitleMemory;
  }

  SubtitlePreferences get subtitlePreferences => SubtitlePreferences(
    languages: [?systemCaptions?.locale, ...widget.subtitleLanguages],
    captionsOn: systemCaptions?.enabled ?? false,
  );

  /// Loads the sidecars of the file just opened and schedules the
  /// auto-selection of a track.
  Future<void> _loadSubtitles() async {
    final generation = _subtitleGeneration;
    final r = request;
    for (final sub in r.subtitles ?? const <ExternalSubtitle>[]) {
      await engine.addSubtitle(sub);
      if (generation != _subtitleGeneration) return;
    }
    if (r.subtitleUri != null) {
      // An explicit file of the request: shown as is.
      await engine.addExternalSubtitle(r.subtitleUri!);
      return;
    }
    if (!mounted || generation != _subtitleGeneration) return;
    _autoSelectPending = true;
    _autoSelectTimer?.cancel();
    _autoSelectTimer = Timer(
      (r.subtitles ?? const []).isEmpty
          ? const Duration(milliseconds: 300)
          : _subtitleWait,
      _autoSelect,
    );
    _checkSidecarsListed();
  }

  /// Auto-selects as soon as every sidecar is in the track list.
  void _checkSidecarsListed() {
    final subs = request.subtitles;
    if (!_autoSelectPending || subs == null || subs.isEmpty) return;
    final titles = {for (final t in engine.state.value.subtitleTracks) t.title};
    if (subs.every((s) => titles.contains(s.title))) _autoSelect();
  }

  Future<void> _autoSelect() async {
    if (!_autoSelectPending) return;
    _autoSelectPending = false;
    _autoSelectTimer?.cancel();
    final generation = _subtitleGeneration;
    await _captionsLoaded;
    if (!mounted || _closing || generation != _subtitleGeneration) return;
    final s = engine.state.value;
    final pick = pickSubtitle(
      s.subtitleTracks,
      preferences: subtitlePreferences,
      remembered: _memoryFor(request)?.choice,
      audioLanguage: s.audioTrack?.language,
    );
    if (pick != null) await engine.setSubtitleTrack(pick);
  }

  /// The user picked a subtitle track (or off) in the panel.
  Future<void> selectSubtitle(MediaTrack track) async {
    _autoSelectPending = false;
    _autoSelectTimer?.cancel();
    await engine.setSubtitleTrack(track);
    _remember(
      (_memoryFor(request) ?? const SubtitleMemory()).copyWith(
        choice: SubtitleChoice.of(track),
      ),
    );
  }

  /// The user loaded a subtitle file from the panel.
  Future<void> addSubtitleFile(String path) async {
    _autoSelectPending = false;
    _autoSelectTimer?.cancel();
    await engine.addExternalSubtitle(path);
  }

  /// Shifts subtitles; positive: later. Limited to
  /// ±[SubtitleMemory.maxDelayMs].
  Future<void> setSubtitleDelay(Duration delay) async {
    final ms = delay.inMilliseconds.clamp(
      -SubtitleMemory.maxDelayMs,
      SubtitleMemory.maxDelayMs,
    );
    final d = Duration(milliseconds: ms);
    await engine.setSubtitleDelay(d);
    _remember(
      (_memoryFor(request) ?? const SubtitleMemory()).copyWith(delay: d),
    );
  }

  void _remember(SubtitleMemory memory) {
    final key = request.seriesKey;
    _sessionMemory = memory;
    _sessionSeries = key;
    if (key != null) widget.onSubtitleMemory?.call(key, memory);
  }

  void _startProgress() {
    _progress = ProgressReporter(
      engine: engine,
      onProgress: _onProgress,
      interval: widget.progressInterval,
    );
  }

  /// Asks for the item after the current one; a late answer for an item
  /// that is no longer playing is dropped.
  Future<void> _fetchNext() async {
    _next = null;
    final load = _loadNext;
    if (load == null) return;
    final generation = ++_nextGeneration;
    UpNextItem? next;
    try {
      next = await load();
    } catch (_) {
      next = null;
    }
    if (!mounted || generation != _nextGeneration) return;
    _next = next;
    if (next != null && _endHandled && !upNext.visible && !_closing) {
      upNext.show(next);
    }
  }

  void _onEngine() {
    final s = engine.state.value;
    osd.playing = s.playing;
    _syncSystem(s);
    if (!_subtitlesLoaded && s.duration > Duration.zero) {
      _subtitlesLoaded = true;
      _loadSubtitles();
    }
    _checkSidecarsListed();
    if (s.completed && !_finished && s.duration > Duration.zero) {
      _finished = true;
      _progress?.report(position: s.duration);
      osd.show();
      widget.onFinished?.call();
    }
    final ended = s.completed && s.duration > Duration.zero;
    _checkCredits(s, ended);
    if (!ended) _checkAutoSkip(s);
    if (ended && !_endHandled) {
      _endHandled = true;
      final next = _next;
      if (next != null && !_closing) upNext.show(next);
    } else if (!ended && _endHandled) {
      // Played again or seeking back: the card is no longer relevant.
      _endHandled = false;
      upNext.hide();
    }
  }

  /// During the credits (a [SkipKind.credits] segment) the Up Next card
  /// comes up early, as on tvOS; leaving the credits backwards hides it.
  void _checkCredits(EngineState s, bool ended) {
    if (ended) return;
    final seg = skipSegmentOf(s);
    final credits = seg?.kind == SkipKind.credits ? seg : null;
    if (credits == null) {
      if (_creditsOffered != null) {
        _creditsOffered = null;
        if (upNext.visible && !_endHandled) upNext.hide();
      }
      return;
    }
    if (credits == _creditsOffered) return;
    final next = _next;
    if (next == null || _closing) return;
    _creditsOffered = credits;
    upNext.show(next);
  }

  /// [SkipMode.auto] and [SkipMode.instant]: skips the segment at the
  /// playhead once it is due. Only while playing, once per segment.
  void _checkAutoSkip(EngineState s) {
    final mode = widget.skipMode;
    if (mode != SkipMode.auto && mode != SkipMode.instant) return;
    final seg = skipSegmentOf(s);
    if (seg == null) {
      _autoSegment = null;
      return;
    }
    if (_autoSkipped.contains(seg) || _closing) return;
    if (_autoSegment != seg) {
      _autoSegment = seg;
      _autoEntry = s.position;
    }
    if (!s.playing) return;
    if (mode == SkipMode.auto &&
        s.position - _autoEntry < widget.autoSkipDelay) {
      return;
    }
    _autoSkipped.add(seg);
    _autoSegment = null;
    // Credits up to the end: on to the next item if there is one.
    final next = _next;
    final toEnd =
        s.duration > Duration.zero &&
        s.duration - seg.end <= const Duration(seconds: 5);
    if (seg.kind == SkipKind.credits && toEnd && next != null) {
      playNext(next);
      return;
    }
    if (widget.canSeek?.call() == false) return;
    engine.seek(seg.end);
    _progress?.onSeek(seg.end);
  }

  /// Switches the player to [next] (Up Next): the old item gets its final
  /// report, the new one its own progress callback.
  void playNext(UpNextItem next) {
    if (_closing || !mounted) return;
    _progress?.close();
    _progress = null;
    final old = request.trickplay;
    if (!identical(old, next.request.trickplay)) old?.dispose();
    setState(() {
      request = next.request;
      _onProgress = next.onProgress;
      _loadNext = next.loadNext;
      _finished = false;
      _endHandled = false;
      _creditsOffered = null;
      _autoSegment = null;
      _autoSkipped.clear();
    });
    pinchScale.value = 1.0;
    final generation = ++_nextGeneration;
    _openCurrent().then((_) {
      if (mounted && !_closing && generation == _nextGeneration) {
        _startProgress();
        _fetchNext();
      }
    });
    osd.show();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      // The caption settings may have changed while the app was away.
      _captionsLoaded = _loadCaptionStyle();
      return;
    }
    // Backgrounded, hidden or about to be killed: save now.
    _progress?.report();
    if (state != AppLifecycleState.inactive) upNext.stopCountdown();
  }

  void _onOsd() {
    if (!mounted) return;
    final primary = FocusManager.instance.primaryFocus;
    if (!osd.visible) {
      // Never keep focus on invisible buttons.
      if (primary != _rootFocus) _rootFocus.requestFocus();
    } else if (osd.panel == OsdPanel.none &&
        (primary == _rootFocus || primary == null)) {
      _scrubberFocus.requestFocus();
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    engine.state.removeListener(_onEngine);
    _sessionClaim.release();
    pip.dismissals.removeListener(_onPipDismissed);
    unawaited(pip.configure(autoEnter: false));
    osd.removeListener(_onOsd);
    widget.subtitleSettings?.removeListener(_onSubtitleSettings);
    _autoSelectTimer?.cancel();
    _feedbackTimer?.cancel();
    _tapSeekTimer?.cancel();
    _clockTimer?.cancel();
    final e = engine;
    if (statsVisible.value && e is StatsEngine) {
      (e as StatsEngine).setStatsActive(false);
    }
    if (_brightnessChanged) _brightness.reset();
    zoom.dispose();
    pinchScale.dispose();
    statsVisible.dispose();
    brightness.dispose();
    feedback.dispose();
    holding.dispose();
    now.dispose();
    _skipFocus.dispose();
    _progress?.close();
    request.trickplay?.dispose();
    upNext.dispose();
    osd.dispose();
    _rootFocus.dispose();
    _scrubberFocus.dispose();
    _playFocus.dispose();
    engine.dispose();
    if (!kIsWeb && (Platform.isAndroid || Platform.isIOS)) {
      SystemChrome.setEnabledSystemUIMode(SystemUiMode.edgeToEdge);
    }
    if (_fullscreen) mkv.defaultExitNativeFullscreen();
    super.dispose();
  }

  // ---- actions -------------------------------------------------------------

  void togglePlay() {
    final s = engine.state.value;
    if (s.playing) {
      engine.pause();
    } else {
      if (s.completed) engine.seek(Duration.zero);
      engine.play();
    }
    osd.show();
  }

  void seekBy(Duration delta) {
    final s = engine.state.value;
    var target = s.position + delta;
    if (target < Duration.zero) target = Duration.zero;
    if (s.duration > Duration.zero && target > s.duration) target = s.duration;
    seekTo(target);
  }

  void seekTo(Duration target) {
    if (widget.canSeek?.call() == false) {
      osd.show();
      return;
    }
    engine.seek(target);
    _progress?.onSeek(target);
    osd.show();
  }

  /// Seeks past the skip segment at the playhead (Skip Intro).
  void skipSegment() {
    final seg = skipSegmentOf(engine.state.value);
    if (seg == null) return;
    seekTo(seg.end);
  }

  /// Previous chapter (or the start of this one after a few seconds).
  void previousChapter() {
    final s = engine.state.value;
    final target = previousChapterStart(chaptersOf(s), s.position);
    if (target != null) _seekChapter(target);
  }

  void nextChapter() {
    final s = engine.state.value;
    final target = nextChapterStart(chaptersOf(s), s.position);
    if (target != null) _seekChapter(target);
  }

  void _seekChapter(Duration target) {
    final chapters = chaptersOf(engine.state.value);
    seekTo(target);
    final i = chapterIndexAt(chapters, target);
    if (i >= 0) {
      showFeedback(
        OsdFeedback(
          icon: Icons.bookmark_rounded,
          label: chapterTitle(chapters, i, PaPlayerStrings.of(context)),
        ),
      );
    }
  }

  void setZoom(VideoZoom z) {
    zoom.value = z;
    pinchScale.value = 1.0;
    showFeedback(
      OsdFeedback(
        icon: Icons.aspect_ratio_rounded,
        label: OsdSidePanelLabels.zoom(z, PaPlayerStrings.of(context)),
      ),
    );
  }

  void cycleZoom() => setZoom(zoom.value.next);

  /// Speed one step slower / faster along [PlayerScreen.speeds].
  void stepSpeed(int direction) {
    final rate = engine.state.value.rate;
    final speeds = widget.speeds;
    var i = speeds.indexWhere((r) => (r - rate).abs() < 0.001);
    if (i < 0) {
      // Between two steps: the nearest one in that direction.
      i = direction > 0
          ? speeds.lastIndexWhere((r) => r < rate)
          : speeds.indexWhere((r) => r > rate);
      if (i < 0) i = direction > 0 ? -1 : speeds.length;
    }
    final j = (i + direction).clamp(0, speeds.length - 1);
    engine.setRate(speeds[j]);
    showFeedback(
      OsdFeedback(icon: Icons.speed_rounded, label: formatRate(speeds[j])),
    );
  }

  void setStatsVisible(bool v) {
    if (statsVisible.value == v) return;
    statsVisible.value = v;
    final e = engine;
    if (e is StatsEngine) (e as StatsEngine).setStatsActive(v);
  }

  /// Shows [f] in the pill for a moment.
  void showFeedback(OsdFeedback f) {
    _feedbackTimer?.cancel();
    feedback.value = f;
    _feedbackTimer = Timer(
      const Duration(milliseconds: 900),
      () => feedback.value = null,
    );
  }

  void setVolume(double v) {
    final value = v.clamp(0.0, 100.0);
    engine.setVolume(value);
    final l = PaPlayerStrings.of(context);
    showFeedback(
      OsdFeedback(
        icon: value == 0
            ? Icons.volume_off_rounded
            : value < 50
            ? Icons.volume_down_rounded
            : Icons.volume_up_rounded,
        label: '${l.volume} ${value.round()} %',
        level: value / 100,
      ),
    );
  }

  void setBrightness(double v) {
    final value = v.clamp(0.0, 1.0);
    _brightnessChanged = true;
    brightness.value = value;
    _brightness.write(value);
    final l = PaPlayerStrings.of(context);
    showFeedback(
      OsdFeedback(
        icon: Icons.brightness_6_rounded,
        label: '${l.brightness} ${(value * 100).round()} %',
        level: value,
      ),
    );
  }

  /// Long press on the video: 2× until released.
  void startHold() {
    final s = engine.state.value;
    if (!s.playing || holding.value) return;
    _rateBeforeHold = s.rate;
    holding.value = true;
    engine.setRate(2.0);
    HapticFeedback.mediumImpact();
  }

  void endHold() {
    if (!holding.value) return;
    holding.value = false;
    engine.setRate(_rateBeforeHold ?? 1.0);
    _rateBeforeHold = null;
  }

  /// A double tap on a side: seek by [PlayerScreen.seekStep], adding up
  /// quick repeats in the feedback ("+30 s").
  void tapSeek(int direction) {
    final step = direction < 0 ? -widget.seekStep : widget.seekStep;
    if (_tapSeekTotal != Duration.zero &&
        _tapSeekTotal.isNegative != step.isNegative) {
      _tapSeekTotal = Duration.zero;
    }
    _tapSeekTotal += step;
    _tapSeekTimer?.cancel();
    _tapSeekTimer = Timer(
      const Duration(milliseconds: 900),
      () => _tapSeekTotal = Duration.zero,
    );
    seekBy(step);
    showFeedback(
      OsdFeedback(
        icon: direction < 0
            ? Icons.fast_rewind_rounded
            : Icons.fast_forward_rounded,
        label: PaPlayerStrings.of(context).seconds(_tapSeekTotal.inSeconds),
        alignment: direction < 0
            ? const Alignment(-0.7, 0)
            : const Alignment(0.7, 0),
      ),
    );
  }

  Future<void> toggleFullscreen() async {
    if (!_isDesktop) return;
    _fullscreen = !_fullscreen;
    _fullscreen
        ? await mkv.defaultEnterNativeFullscreen()
        : await mkv.defaultExitNativeFullscreen();
  }

  void closePanel() {
    osd.closePanel();
    _scrubberFocus.requestFocus();
  }

  /// Back/Esc: close panel, else leave fullscreen (desktop), else exit.
  void handleBack() {
    if (osd.panel != OsdPanel.none) {
      closePanel();
      return;
    }
    if (_fullscreen) {
      toggleFullscreen();
      return;
    }
    exit();
  }

  void exit() {
    if (_closing) return;
    _closing = true;
    upNext.hide();
    _progress?.report();
    final nav = Navigator.maybeOf(context);
    if (nav != null && nav.canPop()) nav.pop();
  }

  // ---- input ---------------------------------------------------------------

  bool get _seekFocus {
    final p = FocusManager.instance.primaryFocus;
    return !osd.visible || p == _rootFocus || p == _scrubberFocus;
  }

  KeyEventResult _onKey(FocusNode node, KeyEvent e) {
    if (e is KeyUpEvent) return KeyEventResult.ignored;
    final k = e.logicalKey;
    final wasVisible = osd.visible;
    final repeat = e is KeyRepeatEvent;

    if (upNext.visible) {
      if (k == LogicalKeyboardKey.escape ||
          k == LogicalKeyboardKey.goBack ||
          k == LogicalKeyboardKey.browserBack) {
        if (!repeat) upNext.hide();
        return KeyEventResult.handled;
      }
      if (k == LogicalKeyboardKey.select ||
          k == LogicalKeyboardKey.enter ||
          k == LogicalKeyboardKey.numpadEnter ||
          k == LogicalKeyboardKey.gameButtonA ||
          k == LogicalKeyboardKey.mediaPlay ||
          k == LogicalKeyboardKey.mediaPlayPause) {
        if (!repeat) upNext.playNow();
        return KeyEventResult.handled;
      }
    }
    if (k == LogicalKeyboardKey.escape ||
        k == LogicalKeyboardKey.goBack ||
        k == LogicalKeyboardKey.browserBack) {
      if (!repeat) handleBack();
      return KeyEventResult.handled;
    }
    if (osd.panel != OsdPanel.none) {
      osd.show();
      return KeyEventResult.ignored; // let the panel traverse/activate
    }
    if (k == LogicalKeyboardKey.space ||
        k == LogicalKeyboardKey.mediaPlayPause ||
        k == LogicalKeyboardKey.mediaPlay ||
        k == LogicalKeyboardKey.mediaPause) {
      if (repeat) return KeyEventResult.handled;
      if (k == LogicalKeyboardKey.mediaPlay) {
        engine.play();
      } else if (k == LogicalKeyboardKey.mediaPause) {
        engine.pause();
      } else {
        togglePlay();
      }
      osd.show();
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.select ||
        k == LogicalKeyboardKey.enter ||
        k == LogicalKeyboardKey.numpadEnter ||
        k == LogicalKeyboardKey.gameButtonA) {
      // With the OSD hidden, Select takes the Skip Intro offer.
      if (!wasVisible && skipSegmentOf(engine.state.value) != null) {
        if (!repeat) skipSegment();
        return KeyEventResult.handled;
      }
      if (_seekFocus) {
        if (!repeat) togglePlay();
        return KeyEventResult.handled;
      }
      osd.show();
      return KeyEventResult.ignored; // activates the focused button
    }
    if (k == LogicalKeyboardKey.pageUp ||
        k == LogicalKeyboardKey.mediaTrackPrevious) {
      if (!repeat) previousChapter();
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.pageDown ||
        k == LogicalKeyboardKey.mediaTrackNext) {
      if (!repeat) nextChapter();
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.mediaFastForward ||
        k == LogicalKeyboardKey.mediaSkipForward) {
      seekBy(widget.seekStep);
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.mediaRewind ||
        k == LogicalKeyboardKey.mediaSkipBackward) {
      seekBy(-widget.seekStep);
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.arrowLeft ||
        k == LogicalKeyboardKey.arrowRight) {
      if (_seekFocus) {
        seekBy(
          k == LogicalKeyboardKey.arrowLeft
              ? -widget.seekStep
              : widget.seekStep,
        );
        return KeyEventResult.handled;
      }
      osd.show();
      return KeyEventResult.ignored; // move between OSD buttons
    }
    if (k == LogicalKeyboardKey.arrowUp || k == LogicalKeyboardKey.arrowDown) {
      osd.show();
      if (!wasVisible) {
        _scrubberFocus.requestFocus();
        return KeyEventResult.handled;
      }
      final p = FocusManager.instance.primaryFocus;
      if (k == LogicalKeyboardKey.arrowDown &&
          (p == _scrubberFocus || p == _rootFocus)) {
        _playFocus.requestFocus();
        return KeyEventResult.handled;
      }
      if (k == LogicalKeyboardKey.arrowUp && p != _scrubberFocus) {
        _scrubberFocus.requestFocus();
        return KeyEventResult.handled;
      }
      return KeyEventResult.handled;
    }
    if (repeat) return KeyEventResult.ignored;
    if (k == LogicalKeyboardKey.keyF) {
      toggleFullscreen();
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.keyI || k == LogicalKeyboardKey.info) {
      osd.openPanel(osd.panel == OsdPanel.info ? OsdPanel.none : OsdPanel.info);
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.keyM) {
      final v = engine.state.value.volume;
      setVolume(v > 0 ? 0 : 100);
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.keyZ) {
      cycleZoom();
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.keyS) {
      setStatsVisible(!statsVisible.value);
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.keyC) {
      if (chaptersOf(engine.state.value).isNotEmpty) {
        osd.openPanel(OsdPanel.chapters);
      }
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.bracketLeft ||
        k == LogicalKeyboardKey.bracketRight) {
      stepSpeed(k == LogicalKeyboardKey.bracketLeft ? -1 : 1);
      return KeyEventResult.handled;
    }
    osd.show();
    return KeyEventResult.ignored;
  }

  // ---- build ---------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) handleBack();
      },
      child: Theme(
        data: ThemeData.dark(useMaterial3: true),
        child: Focus(
          focusNode: _rootFocus,
          autofocus: true,
          onKeyEvent: _onKey,
          child: Scaffold(
            backgroundColor: const Color(0xFF000000),
            body: MouseRegion(
              onHover: (_) => osd.show(),
              cursor: SystemMouseCursors.basic,
              child: ValueListenableBuilder<bool>(
                valueListenable: pip.active,
                builder: (context, inPip, _) => Stack(
                  fit: StackFit.expand,
                  children: [
                    GestureDetector(
                      key: const ValueKey('osd-video'),
                      behavior: HitTestBehavior.opaque,
                      onTap: osd.toggle,
                      onDoubleTapDown: (d) => _doubleTapAt = d.localPosition,
                      onDoubleTap: _onDoubleTap,
                      onLongPressStart: (_) => startHold(),
                      onLongPressEnd: (_) => endHold(),
                      onLongPressCancel: endHold,
                      onScaleStart: _onScaleStart,
                      onScaleUpdate: _onScaleUpdate,
                      onScaleEnd: _onScaleEnd,
                      child: _ZoomedVideo(state: this),
                    ),
                    _DimLayer(state: this),
                    ValueListenableBuilder<EngineState>(
                      valueListenable: engine.state,
                      builder: (context, s, _) => SubtitleOverlay(
                        // Styled ASS and bitmaps are in the video (PLAY-006).
                        lines:
                            s.subtitleTrack?.isOff == true ||
                                s.subtitleRenderer == SubtitleRenderer.engine
                            ? const []
                            : s.subtitleLines,
                        appearance: subtitleAppearance,
                      ),
                    ),
                    _BufferingIndicator(engine: engine),
                    // In a PiP window: the picture and subtitles only.
                    if (!inPip) ...[
                      ValueListenableBuilder<bool>(
                        valueListenable: statsVisible,
                        builder: (context, on, _) => on
                            ? StatsOverlay(engine: engine, zoom: zoom)
                            : const SizedBox.shrink(),
                      ),
                      OsdFeedbackPill(feedback: feedback),
                      HoldSpeedBadge(holding: holding),
                      ListenableBuilder(
                        listenable: osd,
                        builder: (context, _) => _Osd(
                          state: this,
                          visible: osd.visible,
                          panel: osd.panel,
                        ),
                      ),
                      _SkipButtonLayer(state: this),
                      ListenableBuilder(
                        listenable: upNext,
                        builder: (context, _) => upNext.visible
                            ? _UpNextCard(controller: upNext)
                            : const SizedBox.shrink(),
                      ),
                    ],
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  void _onDoubleTap() {
    final at = _doubleTapAt;
    final w = MediaQuery.sizeOf(context).width;
    if (at == null || w <= 0) return;
    if (at.dx < w / 3) {
      tapSeek(-1);
    } else if (at.dx > w * 2 / 3) {
      tapSeek(1);
    } else {
      toggleFullscreen();
    }
  }

  // ---- touch gestures: swipe (brightness / volume), pinch (zoom) ----------

  void _onScaleStart(ScaleStartDetails d) {
    if (d.pointerCount >= 2 || d.kind == PointerDeviceKind.trackpad) {
      _gesture = _Gesture.pinch;
      _gestureFrom = pinchScale.value;
      return;
    }
    // A mouse drag on the desktop is not a swipe.
    if (d.kind != PointerDeviceKind.touch &&
        d.kind != PointerDeviceKind.stylus) {
      _gesture = _Gesture.none;
      return;
    }
    _gesture = _Gesture.pending;
    _gestureStart = d.localFocalPoint;
  }

  void _onScaleUpdate(ScaleUpdateDetails d) {
    final size = MediaQuery.sizeOf(context);
    switch (_gesture) {
      case _Gesture.none:
        return;
      case _Gesture.pinch:
        pinchScale.value = clampPinchScale(_gestureFrom * d.scale);
        showFeedback(
          OsdFeedback(
            icon: Icons.zoom_in_rounded,
            label: '${pinchScale.value.toStringAsFixed(1)}×',
          ),
        );
        return;
      case _Gesture.pending:
        if (d.pointerCount >= 2) {
          _gesture = _Gesture.pinch;
          _gestureFrom = pinchScale.value / (d.scale <= 0 ? 1 : d.scale);
          return;
        }
        final delta = d.localFocalPoint - _gestureStart;
        if (delta.distance < 16) return;
        // Mostly horizontal: not a level swipe.
        if (delta.dy.abs() < delta.dx.abs() * 1.5) {
          _gesture = _Gesture.none;
          return;
        }
        if (_gestureStart.dx < size.width / 2) {
          _gesture = _Gesture.brightness;
          _gestureFrom = brightness.value;
        } else {
          _gesture = _Gesture.volume;
          _gestureFrom = engine.state.value.volume / 100;
        }
        _gestureStart = d.localFocalPoint;
        return;
      case _Gesture.brightness:
      case _Gesture.volume:
        // A swipe over 70 % of the height goes from 0 to 100 %.
        final travel = math.max(1.0, size.height * 0.7);
        final v =
            _gestureFrom - (d.localFocalPoint.dy - _gestureStart.dy) / travel;
        _gesture == _Gesture.brightness ? setBrightness(v) : setVolume(v * 100);
        return;
    }
  }

  void _onScaleEnd(ScaleEndDetails d) {
    if (_gesture == _Gesture.pinch && pinchScale.value < 1.05) {
      pinchScale.value = 1.0;
    }
    _gesture = _Gesture.none;
  }
}

enum _Gesture { none, pending, brightness, volume, pinch }

/// The video surface in the zoom mode and pinch scale. The tree keeps one
/// shape in every mode, so the engine's surface is never rebuilt.
class _ZoomedVideo extends StatelessWidget {
  const _ZoomedVideo({required this.state});
  final PlayerScreenState state;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: Listenable.merge([state.zoom, state.pinchScale]),
      builder: (context, _) {
        final z = state.zoom.value;
        return ClipRect(
          child: Transform.scale(
            key: const ValueKey('osd-video-scale'),
            scale: state.pinchScale.value,
            child: LayoutBuilder(
              builder: (context, c) {
                var w = c.maxWidth;
                var h = c.maxHeight;
                final aspect = z.aspect;
                if (aspect != null && w.isFinite && h.isFinite && h > 0) {
                  if (w / h > aspect) {
                    w = h * aspect;
                  } else {
                    h = w / aspect;
                  }
                }
                return Center(
                  child: SizedBox(
                    key: const ValueKey('osd-video-box'),
                    width: w,
                    height: h,
                    child: state.engine.buildVideo(context, fit: z.boxFit),
                  ),
                );
              },
            ),
          ),
        );
      },
    );
  }
}

/// The dimming layer of the brightness swipe where the platform has no
/// window brightness.
class _DimLayer extends StatelessWidget {
  const _DimLayer({required this.state});
  final PlayerScreenState state;

  @override
  Widget build(BuildContext context) {
    if (!state._brightness.dimsPicture) return const SizedBox.shrink();
    return IgnorePointer(
      child: ValueListenableBuilder<double>(
        valueListenable: state.brightness,
        builder: (context, b, _) => b >= 0.999
            ? const SizedBox.shrink()
            : ColoredBox(
                key: const ValueKey('osd-dim'),
                color: Color.fromRGBO(0, 0, 0, (1 - b) * 0.85),
              ),
      ),
    );
  }
}

/// The Skip Intro / Credits button while the playhead is in a segment.
class _SkipButtonLayer extends StatelessWidget {
  const _SkipButtonLayer({required this.state});
  final PlayerScreenState state;

  @override
  Widget build(BuildContext context) {
    if (state.request.skipSegments.isEmpty ||
        state.widget.skipMode == SkipMode.off) {
      return const SizedBox.shrink();
    }
    return ListenableBuilder(
      listenable: Listenable.merge([
        state.engine.state,
        state.osd,
        state.upNext,
      ]),
      builder: (context, _) {
        final seg = state.skipSegmentOf(state.engine.state.value);
        // The Up Next card replaces "Skip Credits".
        if (seg == null || state.upNext.visible) {
          return const SizedBox.shrink();
        }
        return Stack(
          fit: StackFit.expand,
          children: [
            SkipSegmentButton(
              segment: seg,
              countdown: state.autoSkipCountdown(seg, state.engine.state.value),
              onSkip: state.skipSegment,
              compact: _isCompactOsd(MediaQuery.sizeOf(context).width),
              osdVisible: state.osd.visible,
              focusNode: state._skipFocus,
            ),
          ],
        );
      },
    );
  }
}

/// The Up Next card, bottom right above the OSD bar.
class _UpNextCard extends StatelessWidget {
  const _UpNextCard({required this.controller});
  final UpNextController controller;

  @override
  Widget build(BuildContext context) {
    final l = PaPlayerStrings.of(context);
    final item = controller.item;
    if (item == null) return const SizedBox.shrink();
    final width = MediaQuery.sizeOf(context).width;
    final compact = _isCompactOsd(width);
    final seconds = (controller.remaining.inMilliseconds / 1000).ceil();
    return Positioned(
      right: compact ? 16 : 48,
      bottom: compact ? 168 : 132,
      child: SafeArea(
        child: Container(
          key: const ValueKey('up-next'),
          width: (width - 32).clamp(0.0, 360.0),
          padding: const EdgeInsets.all(16),
          decoration: BoxDecoration(
            color: PaTokens.glassStrong,
            borderRadius: BorderRadius.circular(PaTokens.radius),
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                l.upNext,
                style: const TextStyle(
                  color: PaTokens.textSecondary,
                  fontSize: 13,
                ),
              ),
              const SizedBox(height: 4),
              Text(
                item.request.title,
                key: const ValueKey('up-next-title'),
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(
                  color: PaTokens.text,
                  fontSize: 18,
                  fontWeight: FontWeight.w600,
                ),
              ),
              if (controller.phase == UpNextPhase.countdown) ...[
                const SizedBox(height: 4),
                Text(
                  l.upNextIn(seconds),
                  key: const ValueKey('up-next-countdown'),
                  style: const TextStyle(
                    color: PaTokens.textSecondary,
                    fontSize: 13,
                  ),
                ),
              ],
              const SizedBox(height: 12),
              Wrap(
                spacing: 8,
                runSpacing: 8,
                children: [
                  FilledButton.icon(
                    key: const ValueKey('up-next-play'),
                    onPressed: controller.playNow,
                    icon: const Icon(Icons.play_arrow_rounded),
                    label: Text(l.playNow),
                  ),
                  TextButton(
                    key: const ValueKey('up-next-cancel'),
                    onPressed: controller.hide,
                    child: Text(l.cancel),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _BufferingIndicator extends StatelessWidget {
  const _BufferingIndicator({required this.engine});
  final PlayerEngine engine;

  @override
  Widget build(BuildContext context) {
    return IgnorePointer(
      child: ValueListenableBuilder<EngineState>(
        valueListenable: engine.state,
        builder: (context, s, _) => s.buffering
            ? const Center(
                child: SizedBox(
                  width: 56,
                  height: 56,
                  child: CircularProgressIndicator(
                    strokeWidth: 3,
                    color: PaTokens.text,
                  ),
                ),
              )
            : const SizedBox.shrink(),
      ),
    );
  }
}

class _Osd extends StatelessWidget {
  const _Osd({required this.state, required this.visible, required this.panel});

  final PlayerScreenState state;
  final bool visible;
  final OsdPanel panel;

  @override
  Widget build(BuildContext context) {
    final engine = state.engine;
    final req = state.request;
    final l = PaPlayerStrings.of(context);
    final compact = _isCompactOsd(MediaQuery.sizeOf(context).width);
    final side = compact ? 16.0 : 48.0;
    // Reduce Motion: no slide, an instant fade (design 01 §4.7).
    final still = MediaQuery.of(context).disableAnimations;
    final fade = still ? Duration.zero : PaTokens.osdFade;
    return Stack(
      fit: StackFit.expand,
      children: [
        IgnorePointer(
          ignoring: !visible,
          child: AnimatedOpacity(
            key: const ValueKey('osd'),
            opacity: visible ? 1 : 0,
            duration: fade,
            child: AnimatedSlide(
              offset: visible || still ? Offset.zero : const Offset(0, 0.02),
              duration: fade,
              child: Stack(
                fit: StackFit.expand,
                children: [
                  // scrims
                  const Positioned(
                    left: 0,
                    right: 0,
                    top: 0,
                    height: 160,
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        gradient: LinearGradient(
                          begin: Alignment.topCenter,
                          end: Alignment.bottomCenter,
                          colors: [Color(0xAA000000), Color(0x00000000)],
                        ),
                      ),
                    ),
                  ),
                  const Positioned(
                    left: 0,
                    right: 0,
                    bottom: 0,
                    height: 260,
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        gradient: LinearGradient(
                          begin: Alignment.bottomCenter,
                          end: Alignment.topCenter,
                          colors: [Color(0xCC000000), Color(0x00000000)],
                        ),
                      ),
                    ),
                  ),
                  Positioned(
                    left: compact ? 8 : 24,
                    right: compact ? 8 : 24,
                    top: 16,
                    child: SafeArea(
                      bottom: false,
                      child: Row(
                        children: [
                          FocusIconButton(
                            icon: Icons.arrow_back_rounded,
                            tooltip: l.back,
                            onPressed: state.exit,
                          ),
                          const SizedBox(width: 12),
                          Expanded(
                            child: Column(
                              mainAxisSize: MainAxisSize.min,
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text(
                                  req.title,
                                  key: const ValueKey('osd-title'),
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: const TextStyle(
                                    color: PaTokens.text,
                                    fontSize: 24,
                                    fontWeight: FontWeight.w600,
                                  ),
                                ),
                                _ChapterLine(state: state),
                              ],
                            ),
                          ),
                          ...?state.widget.osdActions?.call(context),
                        ],
                      ),
                    ),
                  ),
                  Positioned(
                    left: 0,
                    right: 0,
                    bottom: 0,
                    child: SafeArea(
                      top: false,
                      child: Padding(
                        padding: EdgeInsets.fromLTRB(side, 0, side, 28),
                        child: _BottomBar(
                          state: state,
                          engine: engine,
                          compact: compact,
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
        if (panel != OsdPanel.none)
          Positioned(
            top: 0,
            bottom: 0,
            right: 0,
            child: SafeArea(
              child: OsdSidePanel(
                panel: panel,
                engine: engine,
                request: req,
                osd: state.osd,
                onClose: state.closePanel,
                onSelectSubtitle: state.selectSubtitle,
                onSubtitleFile: state.addSubtitleFile,
                onSubtitleDelay: state.setSubtitleDelay,
                zoom: state.zoom,
                onZoom: state.setZoom,
                pinchScale: state.pinchScale,
                onResetPinch: () => state.pinchScale.value = 1.0,
                chapters: state.chaptersOf(engine.state.value),
                onSeek: state.seekTo,
                statsVisible: state.statsVisible,
                onStats: state.setStatsVisible,
                speeds: state.widget.speeds,
              ),
            ),
          ),
      ],
    );
  }
}

/// Narrow windows (phones in portrait): the OSD bottom bar puts transport
/// and panel buttons on two centred rows instead of one.
bool _isCompactOsd(double width) => width < 600;

class _BottomBar extends StatelessWidget {
  const _BottomBar({
    required this.state,
    required this.engine,
    required this.compact,
  });
  final PlayerScreenState state;
  final PlayerEngine engine;
  final bool compact;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: Listenable.merge([engine.state, state.now]),
      builder: (context, _) {
        final s = engine.state.value;
        final remaining = remainingOf(s);
        final endsLabel = endsAtLabel(state.now.value, s);
        final l = PaPlayerStrings.of(context);
        final step = state.widget.seekStep.inSeconds;
        final chapters = state.chaptersOf(s);
        const gap = SizedBox(width: 8);
        final liveBar = state.widget.liveBar;
        final live = liveBar != null;
        final transport = <Widget>[
          if (!live) ...[
            FocusIconButton(
              key: const ValueKey('osd-back10'),
              icon: _skipIcon(step, back: true),
              tooltip: l.skipBackBy(step),
              onPressed: () => state.seekBy(-state.widget.seekStep),
            ),
            gap,
          ],
          FocusIconButton(
            key: const ValueKey('osd-play'),
            focusNode: state._playFocus,
            icon: s.playing ? Icons.pause_rounded : Icons.play_arrow_rounded,
            tooltip: s.playing ? l.pause : l.play,
            size: 60,
            onPressed: state.togglePlay,
          ),
          if (!live) ...[
            gap,
            FocusIconButton(
              key: const ValueKey('osd-fwd10'),
              icon: _skipIcon(step, back: false),
              tooltip: l.skipForwardBy(step),
              onPressed: () => state.seekBy(state.widget.seekStep),
            ),
          ],
        ];
        final panels = <Widget>[
          FocusIconButton(
            key: const ValueKey('osd-audio'),
            icon: Icons.graphic_eq_rounded,
            tooltip: l.audio,
            onPressed: () => state.osd.openPanel(OsdPanel.audio),
          ),
          gap,
          FocusIconButton(
            key: const ValueKey('osd-subtitles'),
            icon: Icons.subtitles_rounded,
            tooltip: l.subtitles,
            active: s.subtitleTrack != null && !s.subtitleTrack!.isOff,
            onPressed: () => state.osd.openPanel(OsdPanel.subtitles),
          ),
          gap,
          FocusIconButton(
            key: const ValueKey('osd-speed'),
            icon: Icons.speed_rounded,
            tooltip: l.speedWithRate(formatRate(s.rate)),
            active: s.rate != 1.0,
            onPressed: () => state.osd.openPanel(OsdPanel.speed),
          ),
          gap,
          FocusIconButton(
            key: const ValueKey('osd-zoom'),
            icon: Icons.aspect_ratio_rounded,
            tooltip: l.zoom,
            active: state.zoom.value != VideoZoom.fit,
            onPressed: () => state.osd.openPanel(OsdPanel.zoom),
          ),
          if (chapters.isNotEmpty) ...[
            gap,
            FocusIconButton(
              key: const ValueKey('osd-chapters'),
              icon: Icons.format_list_bulleted_rounded,
              tooltip: l.chapters,
              onPressed: () => state.osd.openPanel(OsdPanel.chapters),
            ),
          ],
          gap,
          FocusIconButton(
            key: const ValueKey('osd-info'),
            icon: Icons.info_outline_rounded,
            tooltip: l.info,
            onPressed: () => state.osd.openPanel(OsdPanel.info),
          ),
          if (state.pipSupported) ...[
            gap,
            FocusIconButton(
              key: const ValueKey('osd-pip'),
              icon: Icons.picture_in_picture_alt_rounded,
              tooltip: l.pictureInPicture,
              onPressed: state.enterPip,
            ),
          ],
          if (state._isDesktop) ...[
            gap,
            FocusIconButton(
              icon: Icons.fullscreen_rounded,
              tooltip: l.fullscreen,
              onPressed: state.toggleFullscreen,
            ),
          ],
        ];
        return FocusTraversalGroup(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (live)
                liveBar(context)
              else
                Scrubber(
                  key: const ValueKey('osd-scrubber'),
                  focusNode: state._scrubberFocus,
                  position: s.position,
                  duration: s.duration,
                  buffer: s.buffer,
                  onSeek: state.seekTo,
                  trickplay: state.request.trickplay,
                  chapters: chapters,
                  chapterName: (i) => chapterTitle(chapters, i, l),
                  endLabel: s.duration > Duration.zero
                      ? l.remainingEndsAt(formatDuration(remaining), endsLabel)
                      : '',
                ),
              const SizedBox(height: 8),
              if (compact) ...[
                Row(
                  key: const ValueKey('osd-transport'),
                  mainAxisAlignment: MainAxisAlignment.center,
                  children: transport,
                ),
                const SizedBox(height: 4),
                // Wraps to a second line when the chapters button does not
                // fit a narrow phone.
                Wrap(
                  key: const ValueKey('osd-panels'),
                  alignment: WrapAlignment.center,
                  runSpacing: 4,
                  children: panels,
                ),
              ] else
                Row(children: [...transport, const Spacer(), ...panels]),
            ],
          ),
        );
      },
    );
  }
}

/// Time left at normal speed.
Duration remainingOf(EngineState s) {
  final left = s.duration - s.position;
  return left.isNegative ? Duration.zero : left;
}

/// "21:40": when the video ends at the current speed, from [now].
String endsAtLabel(DateTime now, EngineState s) {
  final rate = s.rate <= 0 ? 1.0 : s.rate;
  final endsAt = now.add(
    Duration(milliseconds: (remainingOf(s).inMilliseconds / rate).round()),
  );
  return '${endsAt.hour.toString().padLeft(2, '0')}:'
      '${endsAt.minute.toString().padLeft(2, '0')}';
}

/// Icon of the skip buttons for a step of [seconds].
IconData _skipIcon(int seconds, {required bool back}) => switch (seconds) {
  5 => back ? Icons.replay_5_rounded : Icons.forward_5_rounded,
  10 => back ? Icons.replay_10_rounded : Icons.forward_10_rounded,
  30 => back ? Icons.replay_30_rounded : Icons.forward_30_rounded,
  _ => back ? Icons.fast_rewind_rounded : Icons.fast_forward_rounded,
};

/// The chapter playing now, under the title.
class _ChapterLine extends StatelessWidget {
  const _ChapterLine({required this.state});
  final PlayerScreenState state;

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder<EngineState>(
      valueListenable: state.engine.state,
      builder: (context, s, _) {
        final chapters = state.chaptersOf(s);
        final i = chapterIndexAt(chapters, s.position);
        if (i < 0 || chapters.length < 2) return const SizedBox.shrink();
        return Text(
          chapterTitle(chapters, i, PaPlayerStrings.of(context)),
          key: const ValueKey('osd-chapter'),
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: const TextStyle(color: PaTokens.textSecondary, fontSize: 15),
        );
      },
    );
  }
}
