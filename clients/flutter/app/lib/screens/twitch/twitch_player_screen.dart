// SPDX-License-Identifier: MPL-2.0
//
// The native Twitch player (TWI-014, full builds only), registered as
// `TwitchService.nativePlayer`. `decideTwitchRoute` (ADR-0049) sends a
// target here only in a full build whose signed remote config allows
// native playback; store builds and a switched-off flag keep the official
// embed (TWI-005).
//
// - Live: the usher variant goes through the loopback live proxy of the
//   core (TWI-006); `LivePlaybackController` polls its telemetry for the
//   latency (also in the OSD stats), catch-up, the "Реклама · 0:15" badge
//   (seeking is blocked during an ad; the ad plays as Twitch serves it) and
//   "К эфиру". "Низкая задержка / Стабильно" reopens the stream with the
//   matching libmpv options.
// - VOD: chapters (TWI-008) on the scrubber, speeds 0.5–3×, progress under
//   `twitch:vod:<id>` like the embed, handoff heartbeats.
// - Quality: the usher variants, "audio only" included; the choice is
//   remembered.
// - Chat (TWI-012, live only): beside the video on wide screens, below it
//   on a phone in portrait, over the video on a phone in landscape and in
//   "theater" mode; on TV a column that squeezes the video, off by default.
//
// The OSD is pa_player's `PlayerScreen` (UI-013) with Twitch buttons in its
// top bar and the live bar in place of the scrubber.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:material_ui/material_ui.dart';
import 'package:pa_library/pa_library.dart' show RustLibrary;
import 'package:pa_player/pa_player.dart';

import '../../config.dart';
import '../../features/twitch/chat/chat_model.dart';
import '../../features/twitch/chat/chat_panel.dart';
import '../../features/twitch/chat/core_chat_feed.dart';
import '../../l10n/l10n.dart';
import '../../platform/form_factor.dart';
import '../../services/app_services.dart';
import '../../services/preferences.dart';
import '../../theme/kinolune_theme.dart';
import '../../twitch/twitch_models.dart';
import '../../twitch/twitch_native.dart';
import '../../twitch/twitch_service.dart';
import 'twitch_widgets.dart';

/// Creates the player engine: [live] is the latency mode of a live stream,
/// null for a VOD.
typedef TwitchEngineFactory = PlayerEngine Function(LiveLatencyMode? live);

/// Opens the live chat of a channel (null: no chat).
typedef TwitchChatFactory = Future<ChatFeed?> Function(
  BuildContext context,
  String login,
);

/// VOD speeds (research 06 §6.4: 0.5–3×).
const kTwitchVodSpeeds = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 2.5, 3.0];

/// Live plays at 1× (catch-up changes the rate by itself).
const kTwitchLiveSpeeds = [1.0];

/// Registers the native player as `TwitchService.nativePlayer`.
void registerTwitchNativePlayer(
  TwitchService twitch,
  TwitchNativeSource source,
) {
  twitch.nativePlayer = (context, target) =>
      openTwitchNative(context, target, source: source);
}

/// The `TwitchService.nativePlayer` hook: the player for [target].
Future<void> openTwitchNative(
  BuildContext context,
  TwitchPlayTarget target, {
  required TwitchNativeSource source,
  TwitchEngineFactory? engineFactory,
  TwitchChatFactory? chat,
}) {
  Services.read(context).music.pause();
  return Navigator.of(context, rootNavigator: true).push(
    MaterialPageRoute<void>(
      fullscreenDialog: true,
      builder: (_) => TwitchPlayerScreen(
        target: target,
        source: source,
        engineFactory: engineFactory,
        chat: chat,
      ),
    ),
  );
}

/// The engine of the app: media_kit with the live options of [live].
PlayerEngine defaultTwitchEngine(AppServices services, LiveLatencyMode? live) {
  final base = services.playerEngineFactory;
  if (base != null) return base(video: true);
  if (live == null) return MediaKitEngine.create(video: true);
  return MediaKitEngine(
    mpvOptions: {...MediaKitEngine.defaultMpvOptions, ...liveMpvOptions(live)},
  );
}

/// The anonymous read-only chat of the core; the demo and the player lab
/// get the made-up chat.
Future<ChatFeed?> defaultTwitchChat(BuildContext context, String login) async {
  final services = Services.read(context);
  try {
    await RustLibrary.ensureInitialized();
    if (!context.mounted) return null;
    final paused =
        services.twitchChatPauseAnimations.value ||
        MediaQuery.disableAnimationsOf(context);
    final options = await chatOptions(context, animate: !paused);
    return AppConfig.demo || twitchLabOrigin(AppConfig.twitchLab) != null
        ? CoreChatFeed.demo(20, options)
        : CoreChatFeed.anonymous(login, options);
  } catch (e) {
    debugPrint('Twitch chat: $e');
    return null;
  }
}

/// Where the chat goes.
enum TwitchChatLayout {
  /// Hidden (or a VOD: no live chat).
  none,

  /// A column beside the video (desktop, tablet, TV).
  side,

  /// Below the video (phone in portrait).
  below,

  /// Over the right side of the video (phone in landscape, theater).
  overlay,
}

/// The chat layout for a player of [size] on [formFactor].
TwitchChatLayout twitchChatLayout({
  required Size size,
  required FormFactor formFactor,
  required bool chatOn,
  required bool theater,
}) {
  if (!chatOn) return TwitchChatLayout.none;
  if (formFactor == FormFactor.tv) return TwitchChatLayout.side;
  if (size.height > size.width) return TwitchChatLayout.below;
  if (theater || !twitchWideLayout(size, formFactor)) {
    return TwitchChatLayout.overlay;
  }
  return TwitchChatLayout.side;
}

/// Room for the chat beside the video: a desktop window or a tablet
/// (shortest side ≥ 600) at least 840 wide; a phone in landscape is not.
bool twitchWideLayout(Size size, FormFactor formFactor) =>
    size.width >= 840 &&
    (formFactor != FormFactor.phone || size.shortestSide >= 600);

String _latencyPreference(LiveLatencyMode m) =>
    m == LiveLatencyMode.stable ? 'stable' : 'low';

class TwitchPlayerScreen extends StatefulWidget {
  const TwitchPlayerScreen({
    super.key,
    required this.target,
    required this.source,
    this.engineFactory,
    this.chat,
  });

  final TwitchPlayTarget target;
  final TwitchNativeSource source;

  /// Engines; null: [defaultTwitchEngine].
  final TwitchEngineFactory? engineFactory;

  /// Live chat; null: [defaultTwitchChat].
  final TwitchChatFactory? chat;

  @override
  State<TwitchPlayerScreen> createState() => TwitchPlayerScreenState();
}

/// Public for tests.
class TwitchPlayerScreenState extends State<TwitchPlayerScreen> {
  late final AppServices _services;
  TwitchNativeStream? _stream;
  TwitchQuality? _quality;
  LiveLatencyMode _mode = LiveLatencyMode.lowLatency;
  TwitchLiveSession? _live;
  LivePlaybackController? _controller;
  final List<LivePlaybackController> _retired = [];
  PlayerEngine? _engine;
  TwitchVodProgress? _progress;
  Object? _error;

  /// The current player (a new one per quality or mode) and its OSD: the
  /// chat over the video steps aside while the controls are shown.
  GlobalKey<PlayerScreenState> _playerKey = GlobalKey();
  OsdController? _osd;
  final ValueNotifier<bool> _osdVisible = ValueNotifier(true);
  Duration? _resumeAt;

  /// VOD speed carried over to the next engine (quality change).
  double _rate = 1.0;

  /// VOD chapters (TWI-008): the caller's, else the stored ones.
  List<TwitchChapter> _chapters = const [];

  ChatFeed? _chat;
  var _chatStarted = false;
  bool? _chatOn;
  var _theater = false;

  bool get isLive => widget.target.isLive;
  TwitchQuality? get quality => _quality;
  LiveLatencyMode get mode => _mode;
  LivePlaybackController? get liveController => _controller;
  PlayerEngine? get engine => _engine;

  @override
  void initState() {
    super.initState();
    _services = Services.read(context);
    final id = widget.target.videoId;
    if (id != null) {
      _progress = TwitchVodProgress(
        videoId: id,
        saveProgress: _services.library.saveProgress,
        handoff: _services.handoff,
        clock: _services.clock,
      );
    }
    _resumeAt = widget.target.startAt;
    _chapters = widget.target.chapters;
    unawaited(_start());
  }

  Future<void> _start() async {
    final prefs = _services.preferences;
    try {
      final stream = await widget.source.resolve(widget.target);
      final pref = await prefs.read(Preferences.twitchQuality);
      final mode = await prefs.read(Preferences.twitchLatencyMode);
      final id = widget.target.videoId;
      if (id != null && _chapters.isEmpty) {
        try {
          _chapters = await _services.twitch.backend.chapters(id);
        } catch (e) {
          debugPrint('Twitch chapters of $id: $e');
        }
      }
      if (!mounted) return;
      final q = pickQuality(stream, pref);
      if (q == null) throw const TwitchNativeError('unavailable');
      _mode = mode == 'stable'
          ? LiveLatencyMode.stable
          : LiveLatencyMode.lowLatency;
      if (isLive) _live = widget.source.startLive(q.url, _mode);
      setState(() {
        _stream = stream;
        _quality = q;
        _error = null;
      });
    } catch (e) {
      debugPrint('Twitch native playback: $e');
      if (mounted) setState(() => _error = e);
    }
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_chatStarted || !isLive) return;
    _chatStarted = true;
    _chatOn ??= FormFactorScope.of(context) != FormFactor.tv;
    unawaited(_openChat());
  }

  Future<void> _openChat() async {
    final login = widget.target.login;
    if (login == null) return;
    final feed = await (widget.chat ?? defaultTwitchChat)(context, login);
    if (!mounted) {
      feed?.close();
      return;
    }
    setState(() => _chat = feed);
  }

  void _attachOsd() {
    final osd = _playerKey.currentState?.osd;
    if (osd == null || identical(osd, _osd)) return;
    _detachOsd();
    _osd = osd..addListener(_onOsd);
    _onOsd();
  }

  void _detachOsd() {
    try {
      _osd?.removeListener(_onOsd);
    } catch (_) {
      // The old player disposed its controller already.
    }
    _osd = null;
  }

  void _onOsd() => _osdVisible.value = _osd?.visible ?? false;

  @override
  void dispose() {
    _detachOsd();
    _osdVisible.dispose();
    _progress?.close();
    _retireController();
    for (final c in _retired) {
      unawaited(c.dispose());
    }
    _live?.close();
    // The panel owns the chat controller; the feed is ours.
    _chat?.close();
    final twitch = _services.twitch;
    WidgetsBinding.instance.addPostFrameCallback(
      (_) => twitch.progressChanged(),
    );
    super.dispose();
  }

  void _retireController() {
    final c = _controller;
    _controller = null;
    if (c != null) _retired.add(c);
  }

  /// A new engine and player for the current quality and mode.
  void _reopen() {
    final e = _engine;
    if (!isLive && e != null) {
      final pos = e.state.value.position;
      if (pos > Duration.zero) _resumeAt = pos;
      _rate = e.state.value.rate;
    }
    _retireController();
    setState(() {
      _playerKey = GlobalKey();
    });
    // The old player is gone after this frame: its controller can go too.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final old = [..._retired];
      _retired.clear();
      for (final c in old) {
        unawaited(c.dispose());
      }
    });
  }

  /// Picks another usher variant (remembered for next time).
  Future<void> setQuality(TwitchQuality q) async {
    if (q == _quality) return;
    _quality = q;
    _live?.setVariant(q.url);
    _reopen();
    await _services.preferences.write(
      Preferences.twitchQuality,
      qualityPreference(q),
    );
  }

  /// "Низкая задержка" / "Стабильно" (remembered).
  Future<void> setMode(LiveLatencyMode m) async {
    if (m == _mode || !isLive) return;
    _mode = m;
    _live?.setMode(m);
    _reopen();
    await _services.preferences.write(
      Preferences.twitchLatencyMode,
      _latencyPreference(m),
    );
  }

  /// "К эфиру".
  Future<void> goToLive() async => _controller?.goToLive();

  void toggleChat() => setState(() => _chatOn = !(_chatOn ?? true));

  void toggleTheater() => setState(() => _theater = !_theater);

  PlayerEngine _createEngine() {
    final factory =
        widget.engineFactory ??
        (LiveLatencyMode? m) => defaultTwitchEngine(_services, m);
    final engine = factory(isLive ? _mode : null);
    if (!isLive && _rate != 1.0) unawaited(engine.setRate(_rate));
    _engine = engine;
    final live = _live;
    if (live != null) {
      _controller = LivePlaybackController(
        engine: engine,
        telemetry: live.telemetry(),
        url: live.playlistUrl,
      )..startPolling();
    }
    return engine;
  }

  String _title(BuildContext context) {
    final l = context.l10n;
    final login = widget.target.login;
    final channel = login == null || login.isEmpty
        ? null
        : _services.twitch.channelOf(login).displayName;
    if (isLive) return channel ?? l.twitchEmbedTitle;
    final title = widget.target.title;
    if (title != null && title.isNotEmpty) return title;
    return channel == null
        ? l.twitchPlayerVod
        : '$channel · ${l.twitchPlayerVod}';
  }

  @override
  Widget build(BuildContext context) {
    final error = _error;
    final q = _quality;
    Widget body;
    if (error != null) {
      body = _Failed(
        error: error,
        target: widget.target,
        onRetry: () {
          setState(() => _error = null);
          unawaited(_start());
        },
      );
    } else if (q == null) {
      body = const Center(
        key: ValueKey('twitch-player-loading'),
        child: CircularProgressIndicator(),
      );
    } else {
      body = _layout(context, _player(context, q));
    }
    return Scaffold(
      key: const ValueKey('twitch-player'),
      backgroundColor: Colors.black,
      body: body,
    );
  }

  Widget _player(BuildContext context, TwitchQuality q) {
    final live = _live;
    final request = PlayRequest(
      uri: live?.playlistUrl ?? q.url,
      title: _title(context),
      startPositionMs: isLive ? 0 : (_resumeAt?.inMilliseconds ?? 0),
      chapters: isLive || _chapters.isEmpty
          ? null
          : playbackChapters(_chapters),
    );
    final progress = _progress;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _attachOsd();
    });
    return Stack(
      fit: StackFit.expand,
      children: [
        PlayerScreen(
          key: _playerKey,
          request: request,
          engineFactory: ({bool video = true}) => _createEngine(),
          onProgress: progress == null ? (_, _) {} : progress.report,
          onFinished: progress == null
              ? null
              : () => progress.finished(
                  _engine?.state.value.duration.inMilliseconds ?? 0,
                ),
          seekStep: Duration(seconds: _services.skipSeconds.value),
          speeds: isLive ? kTwitchLiveSpeeds : kTwitchVodSpeeds,
          osdActions: _osdActions,
          liveBar: isLive ? _liveBar : null,
          canSeek: isLive
              ? () => _controller?.status.value.seekAllowed ?? true
              : null,
        ),
        if (q.audioOnly) const _AudioOnlyNote(),
        if (isLive) _AdBadgeLayer(state: this),
      ],
    );
  }

  Widget _liveBar(BuildContext context) {
    final c = _controller;
    if (c == null) return const SizedBox(height: 48);
    return TwitchLiveBar(status: c.status, onGoLive: goToLive);
  }

  List<Widget> _osdActions(BuildContext context) {
    final l = context.l10n;
    final q = _quality;
    final wide = twitchWideLayout(
      MediaQuery.sizeOf(context),
      FormFactorScope.of(context),
    );
    final compact = MediaQuery.sizeOf(context).width < 600;
    return [
      if (q != null && compact)
        IconButton(
          key: const ValueKey('twitch-quality'),
          color: Colors.white,
          tooltip: '${l.twitchPlayerQuality}: ${qualityLabel(l, q)}',
          onPressed: () => _chooseQuality(context),
          icon: Icon(
            q.audioOnly ? Icons.headphones_rounded : Icons.high_quality_rounded,
          ),
        )
      else if (q != null)
        TextButton.icon(
          key: const ValueKey('twitch-quality'),
          style: TextButton.styleFrom(foregroundColor: Colors.white),
          onPressed: () => _chooseQuality(context),
          icon: Icon(
            q.audioOnly ? Icons.headphones_rounded : Icons.high_quality_rounded,
          ),
          label: Text(qualityLabel(l, q)),
        ),
      if (isLive)
        IconButton(
          key: const ValueKey('twitch-latency-mode'),
          color: Colors.white,
          tooltip: l.twitchPlayerLatencyMode,
          icon: Icon(
            _mode == LiveLatencyMode.lowLatency
                ? Icons.bolt_rounded
                : Icons.shield_outlined,
          ),
          onPressed: () => _chooseMode(context),
        ),
      if (isLive && _chat != null) ...[
        if (wide && (_chatOn ?? true))
          IconButton(
            key: const ValueKey('twitch-theater'),
            color: Colors.white,
            tooltip: l.twitchPlayerTheater,
            isSelected: _theater,
            icon: const Icon(Icons.crop_7_5_rounded),
            onPressed: toggleTheater,
          ),
        IconButton(
          key: const ValueKey('twitch-chat-toggle'),
          color: Colors.white,
          tooltip: (_chatOn ?? true)
              ? l.twitchPlayerHideChat
              : l.twitchPlayerShowChat,
          icon: Icon(
            (_chatOn ?? true)
                ? Icons.speaker_notes_off_outlined
                : Icons.chat_outlined,
          ),
          onPressed: toggleChat,
        ),
      ],
    ];
  }

  Future<void> _chooseQuality(BuildContext context) async {
    final stream = _stream;
    if (stream == null) return;
    final l = context.l10n;
    final picked = await showDialog<TwitchQuality>(
      context: context,
      builder: (context) => SimpleDialog(
        key: const ValueKey('twitch-quality-dialog'),
        title: Text(l.twitchPlayerQuality),
        children: [
          for (final q in stream.qualities)
            SimpleDialogOption(
              key: ValueKey('twitch-quality-${q.id}'),
              onPressed: () => Navigator.pop(context, q),
              child: Row(
                children: [
                  Icon(
                    q == _quality
                        ? Icons.radio_button_checked
                        : Icons.radio_button_off,
                    size: 20,
                  ),
                  const SizedBox(width: 12),
                  Expanded(child: Text(qualityLabel(l, q))),
                ],
              ),
            ),
        ],
      ),
    );
    if (picked != null) await setQuality(picked);
  }

  Future<void> _chooseMode(BuildContext context) async {
    final l = context.l10n;
    final picked = await showDialog<LiveLatencyMode>(
      context: context,
      builder: (context) => SimpleDialog(
        key: const ValueKey('twitch-mode-dialog'),
        title: Text(l.twitchPlayerLatencyMode),
        children: [
          for (final m in LiveLatencyMode.values)
            SimpleDialogOption(
              key: ValueKey('twitch-mode-${m.name}'),
              onPressed: () => Navigator.pop(context, m),
              child: Row(
                children: [
                  Icon(
                    m == _mode
                        ? Icons.radio_button_checked
                        : Icons.radio_button_off,
                    size: 20,
                  ),
                  const SizedBox(width: 12),
                  Text(
                    m == LiveLatencyMode.lowLatency
                        ? l.twitchPlayerLowLatency
                        : l.twitchPlayerStable,
                  ),
                ],
              ),
            ),
        ],
      ),
    );
    if (picked != null) await setMode(picked);
  }

  /// The player and the chat move between layouts without being rebuilt:
  /// a new player would mean a new engine, a new chat panel a second
  /// listener of the feed.
  final GlobalKey _videoKey = GlobalKey(debugLabel: 'twitch-video');
  final GlobalKey _chatKey = GlobalKey(debugLabel: 'twitch-chat');

  Widget _layout(BuildContext context, Widget player) {
    final video = KeyedSubtree(
      key: _videoKey,
      child: KeyedSubtree(
        key: const ValueKey('twitch-player-video'),
        child: player,
      ),
    );
    final feed = _chat;
    if (!isLive || feed == null) {
      return Stack(fit: StackFit.expand, children: [video]);
    }
    final f = FormFactorScope.of(context);
    final services = Services.of(context);
    final chat = KeyedSubtree(
      key: _chatKey,
      child: ValueListenableBuilder<bool>(
        valueListenable: services.twitchChatPauseAnimations,
        builder: (context, setting, _) => TwitchChatPanel(
          key: const ValueKey('twitch-player-chat'),
          feed: feed,
          pauseAnimations: setting || MediaQuery.disableAnimationsOf(context),
          onPauseAnimationsChanged: services.setTwitchChatPauseAnimations,
        ),
      ),
    );
    return LayoutBuilder(
      builder: (context, c) {
        final layout = twitchChatLayout(
          size: c.biggest,
          formFactor: f,
          chatOn: _chatOn ?? true,
          theater: _theater,
        );
        switch (layout) {
          case TwitchChatLayout.none:
            // Hidden, not removed: it keeps its lines and its listener.
            return Stack(
              fit: StackFit.expand,
              children: [
                video,
                Offstage(child: chat),
              ],
            );
          case TwitchChatLayout.below:
            return Column(
              children: [
                SafeArea(
                  bottom: false,
                  // 16:9, but tall enough for the player's controls
                  // (a phone's 16:9 strip is ~200 dp).
                  child: SizedBox(
                    height: math.max(
                      c.maxWidth * 9 / 16,
                      math.min(c.maxHeight * 0.45, 340.0),
                    ),
                    child: video,
                  ),
                ),
                Expanded(
                  child: ColoredBox(color: KColorsChat.bg, child: chat),
                ),
              ],
            );
          case TwitchChatLayout.side:
            final width = f == FormFactor.tv
                ? c.maxWidth * 0.28
                : (c.maxWidth * 0.3).clamp(300.0, 420.0);
            return Row(
              children: [
                Expanded(child: video),
                SizedBox(
                  width: width,
                  child: ColoredBox(
                    color: KColorsChat.bg,
                    child: SafeArea(left: false, child: chat),
                  ),
                ),
              ],
            );
          case TwitchChatLayout.overlay:
            final width = f == FormFactor.phone
                ? c.maxWidth * 0.38
                : (c.maxWidth * 0.3).clamp(300.0, 420.0);
            return Stack(
              fit: StackFit.expand,
              children: [
                video,
                Positioned(
                  key: const ValueKey('twitch-chat-overlay'),
                  top: 0,
                  right: 0,
                  bottom: 0,
                  width: width,
                  child: ValueListenableBuilder<bool>(
                    valueListenable: _osdVisible,
                    builder: (context, osd, child) => IgnorePointer(
                      ignoring: osd,
                      child: AnimatedOpacity(
                        opacity: osd ? 0 : 1,
                        duration: const Duration(milliseconds: 200),
                        child: child,
                      ),
                    ),
                    child: ColoredBox(
                      color: KColorsChat.overlay,
                      child: SafeArea(left: false, child: chat),
                    ),
                  ),
                ),
              ],
            );
        }
      },
    );
  }
}

/// Chat backgrounds of the player.
abstract final class KColorsChat {
  static const bg = Color(0xFF0E0E10);

  /// Over the video: the picture shows through.
  static const overlay = Color(0xB30E0E10);
}

/// `1080p60 (source)`, `720p60`, `Audio only`.
String qualityLabel(AppLocalizations l, TwitchQuality q) {
  if (q.audioOnly) return l.twitchPlayerAudioOnly;
  final name = q.label.replaceAll(RegExp(r'\s*\(source\)$'), '');
  return q.isSource ? l.twitchPlayerSource(name) : name;
}

/// `0:15`.
String _clock(Duration d) {
  final s = (d.inMilliseconds / 1000).ceil();
  return '${s ~/ 60}:${(s % 60).toString().padLeft(2, '0')}';
}

/// The live bar in place of the scrubber: LIVE, the delay and "К эфиру".
class TwitchLiveBar extends StatelessWidget {
  const TwitchLiveBar({
    super.key,
    required this.status,
    required this.onGoLive,
  });

  final ValueListenable<LiveStatus> status;
  final VoidCallback onGoLive;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return ValueListenableBuilder<LiveStatus>(
      valueListenable: status,
      builder: (context, s, _) {
        final latency = s.latency;
        return SizedBox(
          height: 48,
          child: Row(
            key: const ValueKey('twitch-live-bar'),
            children: [
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                decoration: BoxDecoration(
                  color: s.atLiveEdge || latency == null
                      ? KColors.live
                      : Colors.white24,
                  borderRadius: BorderRadius.circular(4),
                ),
                child: Text(
                  l.twitchPlayerLive,
                  style: const TextStyle(
                    color: Colors.white,
                    fontSize: 13,
                    fontWeight: FontWeight.w700,
                    letterSpacing: 0.5,
                  ),
                ),
              ),
              const SizedBox(width: 12),
              if (latency != null)
                Expanded(
                  child: Text(
                    [
                      l.twitchPlayerLatency(
                        (latency.inMilliseconds / 1000).toStringAsFixed(1),
                      ),
                      if (s.catchingUp) '${s.rate.toStringAsFixed(2)}×',
                    ].join(' · '),
                    key: const ValueKey('twitch-latency'),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(color: Colors.white70, fontSize: 15),
                  ),
                ),
              if (latency == null) const Spacer(),
              if (s.canGoLive && latency != null)
                FilledButton.icon(
                  key: const ValueKey('twitch-go-live'),
                  onPressed: onGoLive,
                  icon: const Icon(Icons.skip_next_rounded),
                  label: Text(l.twitchPlayerGoLive),
                ),
            ],
          ),
        );
      },
    );
  }
}

/// "Реклама · 0:15" while the playhead is in an ad break, OSD or not.
class _AdBadgeLayer extends StatelessWidget {
  const _AdBadgeLayer({required this.state});

  final TwitchPlayerScreenState state;

  @override
  Widget build(BuildContext context) {
    final c = state._controller;
    if (c == null) return const SizedBox.shrink();
    final l = context.l10n;
    return IgnorePointer(
      child: ValueListenableBuilder<LiveStatus>(
        valueListenable: c.status,
        builder: (context, s, _) {
          final ad = s.ad;
          if (ad == null) return const SizedBox.shrink();
          final left = ad.remaining;
          return SafeArea(
            child: Align(
              alignment: Alignment.topLeft,
              child: Container(
                key: const ValueKey('twitch-ad-badge'),
                margin: const EdgeInsets.fromLTRB(16, 72, 16, 16),
                padding: const EdgeInsets.symmetric(
                  horizontal: 10,
                  vertical: 4,
                ),
                decoration: BoxDecoration(
                  color: const Color(0xE6FFC400),
                  borderRadius: BorderRadius.circular(4),
                ),
                child: Text(
                  left == null
                      ? l.twitchPlayerAdBadge
                      : l.twitchPlayerAd(_clock(left)),
                  style: const TextStyle(
                    color: Colors.black,
                    fontSize: 14,
                    fontWeight: FontWeight.w700,
                  ),
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}

class _AudioOnlyNote extends StatelessWidget {
  const _AudioOnlyNote();

  @override
  Widget build(BuildContext context) {
    return IgnorePointer(
      child: Center(
        child: Column(
          key: const ValueKey('twitch-audio-only'),
          mainAxisSize: MainAxisSize.min,
          children: [
            const Icon(
              Icons.headphones_rounded,
              color: Colors.white54,
              size: 64,
            ),
            const SizedBox(height: 12),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 24),
              child: Text(
                context.l10n.twitchPlayerAudioOnlyNote,
                textAlign: TextAlign.center,
                style: const TextStyle(color: Colors.white70, fontSize: 15),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _Failed extends StatelessWidget {
  const _Failed({
    required this.error,
    required this.target,
    required this.onRetry,
  });

  final Object error;
  final TwitchPlayTarget target;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final twitch = Services.read(context).twitch;
    final e = error;
    final offline = e is TwitchNativeError && e.offline && target.isLive;
    final embed =
        twitch.embedPlayer != null &&
        (e is! TwitchNativeError || e.fallsBackToEmbed);
    final login = target.login ?? '';
    return SafeArea(
      child: Stack(
        children: [
          Align(
            alignment: Alignment.topLeft,
            child: IconButton(
              color: Colors.white,
              tooltip: MaterialLocalizations.of(context).backButtonTooltip,
              icon: const Icon(Icons.arrow_back_rounded),
              onPressed: () => Navigator.of(context).maybePop(),
            ),
          ),
          Center(
            key: const ValueKey('twitch-player-failed'),
            child: Padding(
              padding: const EdgeInsets.all(24),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    offline
                        ? l.twitchPlayerOffline(
                            twitch.channelOf(login).displayName,
                          )
                        : l.twitchPlayerFailed,
                    textAlign: TextAlign.center,
                    style: const TextStyle(color: Colors.white, fontSize: 17),
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
                        child: Text(l.twitchPlayerRetry),
                      ),
                      if (embed && !offline)
                        OutlinedButton(
                          key: const ValueKey('twitch-player-use-embed'),
                          onPressed: () {
                            final nav = Navigator.of(context);
                            final open = twitch.embedPlayer!;
                            nav.pop();
                            unawaited(open(nav.context, target));
                          },
                          child: Text(l.twitchPlayerUseEmbed),
                        ),
                      OutlinedButton(
                        onPressed: () => showOpenOnTwitch(context, target.url),
                        child: Text(l.twitchOpenOnTwitch),
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
