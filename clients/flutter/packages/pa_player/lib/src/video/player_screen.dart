// SPDX-License-Identifier: MPL-2.0
import 'dart:async';
import 'dart:io' show Platform;
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:media_kit_video/media_kit_video.dart' as mkv;

import '../engine/audio_processing.dart';
import '../engine/media_kit_engine.dart';
import '../engine/playback_stats.dart';
import '../engine/player_engine.dart';
import '../l10n/pa_player_strings.dart';
import '../play_request.dart';
import '../subtitles/subtitle_overlay.dart';
import '../subtitles/subtitle_policy.dart';
import '../subtitles/subtitle_rendering.dart';
import '../subtitles/subtitle_style.dart';
import '../system/media_session.dart';
import '../system/pip.dart';
import '../tracks/track_policy.dart';
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
/// ↑/↓ show OSD and move focus (with [arrowChapters] and the OSD hidden:
/// previous/next chapter), PageUp/PageDown or media prev/next chapter,
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
/// Tracks and sound (PLAY-004): when a file opens, the audio track is
/// picked by [pickAudio] from the profile's [trackPolicy] (Auto, the
/// title's [PlayRequest.originalLanguage], or the user's languages) and
/// the subtitle track by [pickSubtitle] for that audio; a choice the user
/// made for the series wins over both. The audio track and the audio
/// delay chosen in the audio panel are remembered for the series with the
/// subtitle choice ([TrackMemory]). [audioProcessing] (stereo downmix,
/// LFE, Volume Boost with a limiter) is applied to the engine; the audio
/// panel's quick toggles report changes to [onAudioProcessing].
///
/// Track panels (UI-014, ADR-0114): audio, subtitles and video are tabs of
/// one panel, beside the picture, as a bottom sheet on a phone-sized
/// screen, or across the top with [tvLayout]. The subtitles tab has a
/// quick style (size, height, background) reported to
/// [onSubtitleSettings]; the video tab switches video streams of a
/// [VideoTrackEngine] (not remembered).
///
/// System integration (PLAY-AND-008): the player claims the
/// [mediaSession] (notification, lock screen, headset buttons, audio
/// focus) while it is open, and on Android enters picture-in-picture
/// ([pip]) from the OSD button or by itself when the user leaves the app
/// during playback; in the PiP window only the picture and subtitles are
/// drawn, and closing the window pauses the video. Without a PiP window
/// (PiP off or unsupported, Android TV) the video pauses when the app goes
/// to the background ([pauseInBackground], ADR-NNNN): only music plays on
/// without the UI.
class PlayerScreen extends StatefulWidget {
  const PlayerScreen({
    super.key,
    required this.request,
    required this.onProgress,
    this.onFinished,
    this.engineFactory,
    this.progressInterval = const Duration(seconds: 1),
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
    this.trackPolicy,
    this.audioProcessing,
    this.onAudioProcessing,
    this.screenBrightness,
    this.clock,
    this.speeds = kPlaybackSpeeds,
    this.osdActions,
    this.liveBar,
    this.canSeek,
    this.onLiveSeek,
    this.onGoLive,
    this.mediaSession,
    this.pip,
    this.pauseInBackground,
    this.tvLayout = false,
    this.onSubtitleSettings,
    this.arrowChapters = false,
    this.timelineHeader,
  });

  final PlayRequest request;

  /// Saves the stats of the playing item into a diagnostic report and
  /// shows where it went (PLAY-010, CORE-008); set once by the app. Null:
  /// the info panel has no «Сохранить отчёт» row.
  static StatsReportExporter? statsExporter;

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

  /// Languages the user reads and listens to, best first (ISO 639 codes,
  /// e.g. the UI language): the "Auto" languages of [trackPolicy]. For
  /// subtitles the system caption language comes first if set.
  final List<String> subtitleLanguages;

  /// What the player picks by itself (PLAY-004); null: [TrackPolicy] with
  /// Auto everywhere (the PLAY-007 behaviour).
  final TrackPolicy? trackPolicy;

  /// Sound processing of the profile (PLAY-004); null: none.
  final ValueListenable<AudioProcessing>? audioProcessing;

  /// The audio panel changed the sound processing (quick toggles); the app
  /// stores it for the profile and updates [audioProcessing]. Null: the
  /// toggles are not shown.
  final ValueChanged<AudioProcessing>? onAudioProcessing;

  /// Reads the system caption style; default [SystemCaptionStyle.read].
  final Future<SystemCaptionStyle?> Function()? captionStyleLoader;

  /// Called when the user picks a subtitle or audio track or changes a
  /// delay of an item with a [PlayRequest.seriesKey].
  final void Function(String seriesKey, TrackMemory memory)? onSubtitleMemory;

  /// Brightness of the swipe gesture; default [ScreenBrightness.platform].
  final ScreenBrightness? screenBrightness;

  /// Wall clock for "ends at" (tests); default [DateTime.now].
  final DateTime Function()? clock;

  /// Rates of the speed panel and `[`/`]` (Twitch VODs: up to 3×).
  final List<double> speeds;

  /// Extra buttons at the end of the OSD top bar (Twitch: quality, chat).
  final List<Widget> Function(BuildContext context)? osdActions;

  /// A live stream (TWI-014): this bar replaces the scrubber, and there are
  /// no ±[seekStep] buttons. Seeking with keys stays within the cache
  /// unless [onLiveSeek] takes it (DVR over the whole window, TWI-025).
  final Widget Function(BuildContext context)? liveBar;

  /// False while seeking is not allowed (a live ad break); null: always.
  final bool Function()? canSeek;

  /// Live DVR (TWI-025): with [liveBar], seeks (←/→, double taps, the
  /// remote's rewind / fast-forward) go here as a step from the playhead
  /// instead of to the engine: ±[seekStep] a press, ±30 s every 0.3 s
  /// while ←/→ is held. Null: seeking stays within the player's cache.
  final ValueChanged<Duration>? onLiveSeek;

  /// "В эфир" from the keyboard or remote (End, the remote's "next"),
  /// with [liveBar].
  final VoidCallback? onGoLive;

  /// The system media session; default [SystemMediaSession.instance].
  final SystemMediaSession? mediaSession;

  /// Picture-in-picture; default [SystemPip.instance].
  final SystemPip? pip;

  /// Pause when the app is hidden and not in a PiP window; null: on
  /// Android and iOS (a desktop window stays as the user left it).
  final bool? pauseInBackground;

  /// A TV (UI-014): the track panels open across the top with tabs a
  /// remote reaches; otherwise beside the picture, or as a bottom sheet
  /// on a phone-sized screen.
  final bool tvLayout;

  /// The quick style of the subtitles tab changed the subtitle style; the
  /// app stores it and updates [subtitleSettings]. Null: no quick style.
  final ValueChanged<SubtitleSettings>? onSubtitleSettings;

  /// ↑/↓ with the OSD hidden jump to the previous / next chapter (a TV
  /// remote on a Twitch VOD, TWI-027) and the OSD stays hidden; off: they
  /// show the OSD. PageUp/PageDown jump to chapters either way.
  final bool arrowChapters;

  /// A row above the scrubber, rebuilt with the playback state (Twitch:
  /// the chapter strip with game covers, TWI-027). Not shown with
  /// [liveBar].
  final Widget Function(BuildContext context, EngineState state)?
  timelineHeader;

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

  /// The audio track of the file is still to be picked (PLAY-004): as soon
  /// as the engine lists the tracks.
  bool _audioSelectPending = false;
  Timer? _autoSelectTimer;
  int _subtitleGeneration = 0;
  static const _subtitleWait = Duration(milliseconds: 1500);

  /// System caption style (null: none on this platform).
  SystemCaptionStyle? systemCaptions;
  late Future<void> _captionsLoaded;

  /// Choice and delay made in this session, and the series they belong to:
  /// the next episode of the same series starts with them.
  TrackMemory? _sessionMemory;
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

  /// When a held ←/→ last seeked a live stream (TWI-025).
  DateTime? _lastLiveRepeat;

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
    widget.audioProcessing?.addListener(_applyAudioProcessing);
    _applyAudioProcessing();
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
    // The engine may drop the size for a moment (a new surface when the
    // window resizes, a quality switch): keep the last known shape.
    final w = s.videoWidth ?? 0;
    final h = s.videoHeight ?? 0;
    if (w > 0 && h > 0) _videoSize = (w, h);
    final size = _videoSize;
    unawaited(
      pip.configure(
        autoEnter: s.playing && !s.completed && size != null,
        width: size?.$1,
        height: size?.$2,
      ),
    );
  }

  /// The last video size the engine reported (PiP window shape).
  (int, int)? _videoSize;

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
    if (w > 0 && h > 0) _videoSize = (w, h);
    osd.hide();
    await pip.enter(width: _videoSize?.$1, height: _videoSize?.$2);
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
    _audioSelectPending = true;
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
    final memory = _memoryFor(request);
    final delay = memory?.delay ?? Duration.zero;
    if (delay != engine.state.value.subtitleDelay) {
      engine.setSubtitleDelay(delay);
    }
    // The same for the audio delay (PLAY-004).
    final e = engine;
    final audioDelay = memory?.audioDelay ?? Duration.zero;
    if (e is AudioProcessingEngine &&
        audioDelay != engine.state.value.audioDelay) {
      (e as AudioProcessingEngine).setAudioDelay(audioDelay);
    }
    return opened;
  }

  /// Hands the profile's sound processing to the engine (PLAY-004).
  void _applyAudioProcessing() {
    final e = engine;
    if (e is! AudioProcessingEngine) return;
    (e as AudioProcessingEngine).setAudioProcessing(
      widget.audioProcessing?.value ?? const AudioProcessing(),
    );
  }

  /// The policy of the profile.
  TrackPolicy get trackPolicy => widget.trackPolicy ?? TrackPolicy();

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
  TrackMemory? _memoryFor(PlayRequest r) {
    final key = r.seriesKey;
    if (key != null && key == _sessionSeries) return _sessionMemory;
    return r.subtitleMemory;
  }

  SubtitlePreferences get subtitlePreferences =>
      trackPolicy.subtitlePreferences(
        interface: [?systemCaptions?.locale, ...widget.subtitleLanguages],
        original: request.originalLanguage,
        captionsOn: systemCaptions?.enabled ?? false,
      );

  /// Audio languages to look for, best first (PLAY-004).
  List<String> get audioLanguages => trackPolicy.audioLanguages(
    interface: widget.subtitleLanguages,
    original: request.originalLanguage,
  );

  /// Picks the audio track of the file just opened, once: the series'
  /// choice, else the policy. An automatic pick is not remembered.
  Future<void> _autoSelectAudio() async {
    if (!_audioSelectPending) return;
    final s = engine.state.value;
    if (s.audioTracks.isEmpty) return;
    _audioSelectPending = false;
    final pick = pickAudio(
      s.audioTracks,
      languages: audioLanguages,
      remembered: _memoryFor(request)?.audio,
    );
    if (pick != null && pick != s.audioTrack) {
      await engine.setAudioTrack(pick);
    }
  }

  /// The user picked an audio track in the panel: remembered for the
  /// series.
  Future<void> selectAudio(MediaTrack track) async {
    _audioSelectPending = false;
    await engine.setAudioTrack(track);
    _remember(
      (_memoryFor(request) ?? const TrackMemory()).copyWith(
        audio: AudioChoice.of(track),
      ),
    );
  }

  /// The user picked a video stream in the video tab (UI-014). Not
  /// remembered: streams differ from file to file.
  Future<void> selectVideo(MediaTrack track) async {
    final e = engine;
    if (e is! VideoTrackEngine) return;
    await (e as VideoTrackEngine).setVideoTrack(track);
  }

  /// Where panels open on this screen (UI-014).
  OsdPanelPlacement panelPlacement(Size size) {
    if (widget.tvLayout) return OsdPanelPlacement.top;
    return size.shortestSide < 600
        ? OsdPanelPlacement.bottom
        : OsdPanelPlacement.side;
  }

  /// Shifts the sound; positive: later. Limited to
  /// ±[TrackMemory.maxAudioDelayMs]; remembered for the series.
  Future<void> setAudioDelay(Duration delay) async {
    final e = engine;
    if (e is! AudioProcessingEngine) return;
    final ms = delay.inMilliseconds.clamp(
      -TrackMemory.maxAudioDelayMs,
      TrackMemory.maxAudioDelayMs,
    );
    final d = Duration(milliseconds: ms);
    await (e as AudioProcessingEngine).setAudioDelay(d);
    _remember(
      (_memoryFor(request) ?? const TrackMemory()).copyWith(audioDelay: d),
    );
  }

  /// The audio panel's quick toggles changed the sound processing.
  void changeAudioProcessing(AudioProcessing processing) {
    final report = widget.onAudioProcessing;
    if (report != null) {
      report(processing);
    } else {
      final e = engine;
      if (e is AudioProcessingEngine) {
        (e as AudioProcessingEngine).setAudioProcessing(processing);
      }
    }
  }

  /// Loads the sidecars of the file just opened and schedules the
  /// auto-selection of a track.
  Future<void> _loadSubtitles() async {
    final generation = _subtitleGeneration;
    final r = request;
    // PLAY-004: the audio first, so the sound is right from the start and
    // the subtitle pick sees the audio language.
    await _autoSelectAudio();
    if (!mounted || generation != _subtitleGeneration) return;
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
    await _autoSelectAudio();
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
      (_memoryFor(request) ?? const TrackMemory()).copyWith(
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

  /// A subtitle downloaded by «Найти субтитры» (PLAY-008): loaded,
  /// selected and remembered for the series like a pick from the list.
  Future<void> addDownloadedSubtitle(ExternalSubtitle subtitle) async {
    _autoSelectPending = false;
    _autoSelectTimer?.cancel();
    await engine.addSubtitle(subtitle, select: true);
    final track = engine.state.value.subtitleTrack;
    if (track != null && !track.isOff) {
      _remember(
        (_memoryFor(request) ?? const TrackMemory()).copyWith(
          choice: SubtitleChoice.of(track),
        ),
      );
    }
  }

  /// Shifts subtitles; positive: later. Limited to
  /// ±[TrackMemory.maxDelayMs].
  Future<void> setSubtitleDelay(Duration delay) async {
    final ms = delay.inMilliseconds.clamp(
      -TrackMemory.maxDelayMs,
      TrackMemory.maxDelayMs,
    );
    final d = Duration(milliseconds: ms);
    await engine.setSubtitleDelay(d);
    _remember((_memoryFor(request) ?? const TrackMemory()).copyWith(delay: d));
  }

  void _remember(TrackMemory memory) {
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
    } else if (_audioSelectPending &&
        _subtitlesLoaded &&
        s.audioTracks.isNotEmpty) {
      // The tracks came after the duration.
      _autoSelectAudio();
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
    if (state == AppLifecycleState.hidden ||
        state == AppLifecycleState.paused) {
      _pauseIfBackground();
    }
    if (state != AppLifecycleState.inactive) upNext.stopCountdown();
  }

  bool get _pausesInBackground =>
      widget.pauseInBackground ??
      (!kIsWeb && (Platform.isAndroid || Platform.isIOS));

  /// The app left the screen without a PiP window: nobody sees the video.
  void _pauseIfBackground() {
    if (!_pausesInBackground || _closing || pip.active.value) return;
    if (!engine.state.value.playing) return;
    debugPrint('player: paused in the background (no PiP)');
    engine.pause();
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
    widget.audioProcessing?.removeListener(_applyAudioProcessing);
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
    final liveSeek = widget.onLiveSeek;
    if (_live && liveSeek != null) {
      if (widget.canSeek?.call() != false) liveSeek(delta);
      osd.show();
      return;
    }
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

  void _seekChapter(Duration target, {bool quiet = false}) {
    final chapters = chaptersOf(engine.state.value);
    if (quiet) {
      // A remote's ↑/↓ with the OSD hidden: the pill only.
      if (widget.canSeek?.call() == false) return;
      engine.seek(target);
      _progress?.onSeek(target);
    } else {
      seekTo(target);
    }
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

  /// «Сохранить отчёт» (PLAY-010): one fresh reading of the stats, then
  /// the report files to [PlayerScreen.statsExporter].
  Future<void> exportStats() async {
    final export = PlayerScreen.statsExporter;
    if (export == null) return;
    final e = engine;
    if (e is StatsEngine && !statsVisible.value) {
      await (e as StatsEngine).setStatsActive(true);
      await (e as StatsEngine).setStatsActive(false);
    }
    if (!mounted) return;
    final s = engine.state.value;
    await export(
      context,
      playbackReportFiles(
        s.playbackStats ?? const PlaybackStats(),
        title: widget.request.title,
        extra: s.stats,
      ),
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

  /// Back/Esc: back from a subtitle page to the subtitles tab, else close
  /// the panel, else leave fullscreen (desktop), else exit.
  void handleBack() {
    if (osd.panel.isSubtitlePage) {
      osd.openPanel(OsdPanel.subtitles);
      return;
    }
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

  /// Opens the current item again after a playback error (BUG-003), from
  /// where it stopped if it had played.
  Future<void> retry() {
    final at = engine.state.value.position;
    final start = Duration(milliseconds: request.startPositionMs);
    final opened = _openCurrent();
    if (at > start) {
      return opened.then((_) => engine.seek(at));
    }
    return opened;
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
    // BUG-003: over a playback error, Back leaves the player and the
    // other keys move between and press its buttons.
    if (engine.state.value.error != null) {
      if (k == LogicalKeyboardKey.escape ||
          k == LogicalKeyboardKey.goBack ||
          k == LogicalKeyboardKey.browserBack) {
        if (!repeat) exit();
        return KeyEventResult.handled;
      }
      return KeyEventResult.ignored;
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
    if (_live &&
        widget.onGoLive != null &&
        (k == LogicalKeyboardKey.end ||
            k == LogicalKeyboardKey.mediaTrackNext)) {
      if (!repeat) widget.onGoLive!();
      osd.show();
      return KeyEventResult.handled;
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
      if (_seekFocus && _live && widget.onLiveSeek != null) {
        // Live DVR: a press is ±seekStep, a held key ±30 s every 0.3 s.
        final now = DateTime.now();
        if (repeat &&
            _lastLiveRepeat != null &&
            now.difference(_lastLiveRepeat!) <
                const Duration(milliseconds: 300)) {
          return KeyEventResult.handled;
        }
        _lastLiveRepeat = repeat ? now : null;
        final step = repeat ? const Duration(seconds: 30) : widget.seekStep;
        seekBy(k == LogicalKeyboardKey.arrowLeft ? -step : step);
        return KeyEventResult.handled;
      }
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
      if (widget.arrowChapters &&
          !wasVisible &&
          chaptersOf(engine.state.value).length > 1) {
        if (!repeat) {
          final s = engine.state.value;
          final target = k == LogicalKeyboardKey.arrowUp
              ? previousChapterStart(chaptersOf(s), s.position)
              : nextChapterStart(chaptersOf(s), s.position);
          if (target != null) _seekChapter(target, quiet: true);
        }
        return KeyEventResult.handled;
      }
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
  void didChangeDependencies() {
    super.didChangeDependencies();
    // UI-021: with TalkBack, VoiceOver, Narrator or Orca on, the OSD stays.
    osd.holdVisible = MediaQuery.maybeAccessibleNavigationOf(context) ?? false;
  }

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
                    Semantics(
                      label: PaPlayerStrings.of(context).toggleControls,
                      onTap: osd.toggle,
                      child: GestureDetector(
                        key: const ValueKey('osd-video'),
                        excludeFromSemantics: true,
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
                      _PlaybackErrorLayer(state: this),
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

/// A failure that stops playback (BUG-003, UI-044): a human title and
/// hint by the stable code, the code itself small below, and two actions:
/// try again and back.
class _PlaybackErrorLayer extends StatelessWidget {
  const _PlaybackErrorLayer({required this.state});
  final PlayerScreenState state;

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder<EngineState>(
      valueListenable: state.engine.state,
      builder: (context, s, _) {
        final error = s.error;
        if (error == null) return const SizedBox.shrink();
        final l = PaPlayerStrings.of(context);
        final theme = Theme.of(context);
        return ColoredBox(
          key: const ValueKey('player-error'),
          color: const Color(0xE6000000),
          child: SafeArea(
            child: Center(
              child: SingleChildScrollView(
                padding: const EdgeInsets.all(24),
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 520),
                  child: Semantics(
                    liveRegion: true,
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        const Icon(
                          Icons.error_outline_rounded,
                          size: 48,
                          color: PaTokens.text,
                        ),
                        const SizedBox(height: 16),
                        Text(
                          l.playbackErrorTitle(error.code),
                          textAlign: TextAlign.center,
                          style: theme.textTheme.titleLarge?.copyWith(
                            color: PaTokens.text,
                          ),
                        ),
                        const SizedBox(height: 8),
                        Text(
                          l.playbackErrorHint(error.code),
                          textAlign: TextAlign.center,
                          style: theme.textTheme.bodyMedium?.copyWith(
                            color: PaTokens.textSecondary,
                          ),
                        ),
                        const SizedBox(height: 24),
                        Wrap(
                          alignment: WrapAlignment.center,
                          spacing: 12,
                          runSpacing: 12,
                          children: [
                            FilledButton.icon(
                              key: const ValueKey('player-error-retry'),
                              autofocus: true,
                              onPressed: state.retry,
                              icon: const Icon(Icons.refresh_rounded),
                              label: Text(l.retry),
                            ),
                            OutlinedButton.icon(
                              key: const ValueKey('player-error-back'),
                              onPressed: state.exit,
                              icon: const Icon(Icons.arrow_back_rounded),
                              label: Text(l.back),
                            ),
                          ],
                        ),
                        const SizedBox(height: 16),
                        // UI-044: the code and mpv's message only under
                        // «Подробнее» (and in the report).
                        _ErrorDetails(
                          label: l.errorDetails,
                          details: [
                            '${l.errorCode}: ${error.code.code}',
                            if (error.detail.isNotEmpty) error.detail,
                          ].join('\n'),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

/// «Подробнее» under a playback error: folded until asked for.
class _ErrorDetails extends StatefulWidget {
  const _ErrorDetails({required this.label, required this.details});

  final String label;
  final String details;

  @override
  State<_ErrorDetails> createState() => _ErrorDetailsState();
}

class _ErrorDetailsState extends State<_ErrorDetails> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final style = Theme.of(
      context,
    ).textTheme.bodySmall?.copyWith(color: PaTokens.textSecondary);
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        TextButton.icon(
          key: const ValueKey('player-error-details'),
          onPressed: () => setState(() => _open = !_open),
          icon: Icon(
            _open ? Icons.expand_less_rounded : Icons.expand_more_rounded,
            color: PaTokens.textSecondary,
          ),
          label: Text(widget.label, style: style),
        ),
        if (_open)
          SelectableText(
            widget.details,
            key: const ValueKey('player-error-details-text'),
            textAlign: TextAlign.center,
            style: style,
          ),
      ],
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
    final placement = state.panelPlacement(MediaQuery.sizeOf(context));
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
                  // A TV panel across the top covers the title bar.
                  if (placement != OsdPanelPlacement.top ||
                      panel == OsdPanel.none)
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
          Positioned.fill(
            child: SafeArea(
              bottom: placement != OsdPanelPlacement.bottom,
              child: Align(
                alignment: switch (placement) {
                  OsdPanelPlacement.side => Alignment.centerRight,
                  OsdPanelPlacement.bottom => Alignment.bottomCenter,
                  OsdPanelPlacement.top => Alignment.topCenter,
                },
                child: OsdSidePanel(
                  panel: panel,
                  placement: placement,
                  onSelectVideo: engine is VideoTrackEngine
                      ? state.selectVideo
                      : null,
                  subtitleSettings: state.widget.subtitleSettings,
                  onSubtitleSettings: state.widget.subtitleSettings == null
                      ? null
                      : state.widget.onSubtitleSettings,
                  engine: engine,
                  request: req,
                  osd: state.osd,
                  onClose: state.closePanel,
                  onSelectSubtitle: state.selectSubtitle,
                  onSubtitleFile: state.addSubtitleFile,
                  onSubtitleDelay: state.setSubtitleDelay,
                  onSubtitleDownloaded: state.addDownloadedSubtitle,
                  subtitleLanguages: state.subtitlePreferences.languages,
                  onSelectAudio: state.selectAudio,
                  onAudioDelay: engine is AudioProcessingEngine
                      ? state.setAudioDelay
                      : null,
                  audioProcessing: state.widget.onAudioProcessing == null
                      ? null
                      : state.widget.audioProcessing,
                  onAudioProcessing: state.changeAudioProcessing,
                  zoom: state.zoom,
                  onZoom: state.setZoom,
                  pinchScale: state.pinchScale,
                  onResetPinch: () => state.pinchScale.value = 1.0,
                  chapters: state.chaptersOf(engine.state.value),
                  onSeek: state.seekTo,
                  statsVisible: state.statsVisible,
                  onStats: state.setStatsVisible,
                  onExportStats: PlayerScreen.statsExporter == null
                      ? null
                      : state.exportStats,
                  speeds: state.widget.speeds,
                ),
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
              if (!live && state.widget.timelineHeader != null) ...[
                state.widget.timelineHeader!(context, s),
                const SizedBox(height: 6),
              ],
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
