// SPDX-License-Identifier: MPL-2.0
//
// Multiview (TWI-022): 2–4 live streams of the favourites and follows at
// once on a desktop or a TV (two on a tablet; a phone gets a hint). The
// layouts, the heard tile and the quality caps are in
// `twitch/twitch_multiview.dart`; this is the screen and the two kinds of
// tiles:
//
// - native (TWI-014, the signed remote config allows it): each tile its
//   own engine behind the loopback live proxy in low-latency mode with
//   catch-up, so every tile stays at the live edge; the others are muted
//   by volume 0;
// - the official embed (TWI-005) where the platform's WebView can show
//   several at once (Android, iOS, macOS): one WebView per tile, nothing
//   drawn over the player (Twitch's rule), the tile bar above it; a window
//   too small for 400×300 per player gets an explanation. Elsewhere the
//   screen explains that the official player shows one stream at a time.
//
// Remote and keyboard: arrows move between tiles, OK makes a tile the
// heard (in "one big" also the big) one, OK on the heard tile opens its
// menu (remove), Back or Esc closes. Mouse: click hears, × removes. The
// chat of the heard stream can sit beside the tiles on a desktop.

import 'dart:async';
import 'dart:convert';
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart' show ValueListenable, kIsWeb;
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart';

import '../../features/twitch/chat/chat_model.dart';
import '../../features/twitch/chat/chat_panel.dart';
import '../../l10n/l10n.dart';
import '../../platform/form_factor.dart';
import '../../services/app_services.dart';
import '../../services/external_links.dart';
import '../../theme/kinolune_theme.dart';
import '../../twitch/twitch_embed.dart' show twitchEmbedBaseUrl;
import '../../twitch/twitch_models.dart';
import '../../twitch/twitch_multiview.dart';
import '../../twitch/twitch_native.dart';
import '../../twitch/twitch_service.dart';
import 'twitch_embed_screen.dart'
    show TwitchEmbedSurfaceFactory, WebViewTwitchSurface, TwitchEmbedSurface;
import 'twitch_player_screen.dart'
    show TwitchChatFactory, TwitchEngineFactory, defaultTwitchChat;

// ----- What plays the tiles ------------------------------------------------

final _nativeSources = Expando<TwitchNativeSource>('twitch-multiview');

/// Gives the multiview the native source of TWI-014 (next to
/// `registerTwitchNativePlayer`); `decideTwitchRoute` still decides.
void registerTwitchMultiview(TwitchService twitch, TwitchNativeSource source) =>
    _nativeSources[twitch] = source;

/// The native source registered for [twitch], if any.
TwitchNativeSource? twitchMultiviewSource(TwitchService twitch) =>
    _nativeSources[twitch];

/// The platform's WebView shows several embeds side by side
/// (`webview_flutter` platform views: Android, iOS, macOS).
bool get twitchMultiEmbedSupported =>
    !kIsWeb && (Platform.isAndroid || Platform.isIOS || Platform.isMacOS);

/// Whether and how this device shows a multiview.
enum MultiviewSupport {
  /// Native tiles.
  native,

  /// Official embeds, one WebView per tile.
  embed,

  /// The official player here shows one stream at a time.
  embedUnsupported,

  /// A phone: too small.
  phone,
}

/// A phone smaller than a tablet (shortest side under 600).
bool multiviewIsPhone(FormFactor formFactor, Size screen) =>
    formFactor == FormFactor.phone && screen.shortestSide < 600;

/// Streams at once: two on a tablet, four elsewhere.
int multiviewMaxFor(FormFactor formFactor) =>
    formFactor == FormFactor.phone ? 2 : kMultiviewMax;

MultiviewSupport twitchMultiviewSupport({
  required TwitchPlayRoute route,
  required bool hasNativeSource,
  required FormFactor formFactor,
  required Size screen,
  required bool multiEmbed,
}) {
  if (multiviewIsPhone(formFactor, screen)) return MultiviewSupport.phone;
  if (route == TwitchPlayRoute.native && hasNativeSource) {
    return MultiviewSupport.native;
  }
  if (route == TwitchPlayRoute.embed && multiEmbed) {
    return MultiviewSupport.embed;
  }
  return MultiviewSupport.embedUnsupported;
}

/// Live streams to pick from: favourites and follows on air.
Future<List<TwitchStream>> twitchMultiviewLive(TwitchService twitch) async =>
    (await twitch.home()).live;

/// Opens the multiview: [initial] channels, else a picker over [live]
/// (default: [twitchMultiviewLive]). Explains instead where it cannot run.
/// Tests pass [streams] and [chat]; [multiEmbed] overrides the platform.
Future<void> openTwitchMultiview(
  BuildContext context, {
  List<String> initial = const [],
  List<TwitchStream>? live,
  MultiviewStreamFactory? streams,
  TwitchChatFactory? chat,
  bool? multiEmbed,
}) async {
  final services = Services.read(context);
  final twitch = services.twitch;
  final formFactor = FormFactorScope.of(context);
  final l = context.l10n;
  final source = twitchMultiviewSource(twitch);
  final support = twitchMultiviewSupport(
    route: twitch.playRoute,
    hasNativeSource: source != null,
    formFactor: formFactor,
    screen: MediaQuery.sizeOf(context),
    multiEmbed: multiEmbed ?? twitchMultiEmbedSupported,
  );
  final MultiviewStreamFactory factory;
  if (streams != null &&
      support != MultiviewSupport.phone &&
      support != MultiviewSupport.embedUnsupported) {
    factory = streams;
  } else {
    switch (support) {
      case MultiviewSupport.phone:
        return _explain(context, l.twitchMultiviewPhone);
      case MultiviewSupport.embedUnsupported:
        return _explain(context, l.twitchMultiviewNoEmbed);
      case MultiviewSupport.native:
        factory = nativeMultiviewStreams(
          source!,
          (mode) => _defaultEngine(services, mode),
        );
      case MultiviewSupport.embed:
        factory = embedMultiviewStreams();
    }
  }
  final max = multiviewMaxFor(formFactor);
  var logins = initial.take(max).toList();
  if (logins.isEmpty) {
    final streams = live ?? await twitchMultiviewLive(twitch);
    if (!context.mounted) return;
    logins = await showMultiviewPicker(context, live: streams, max: max);
    if (logins.isEmpty || !context.mounted) return;
  }
  services.music.pause();
  await Navigator.of(context, rootNavigator: true).push(
    MaterialPageRoute<void>(
      fullscreenDialog: true,
      builder: (_) => TwitchMultiviewScreen(
        logins: logins,
        max: max,
        streams: factory,
        embed: support == MultiviewSupport.embed,
        chat: chat,
        live: live,
      ),
    ),
  );
}

PlayerEngine _defaultEngine(AppServices services, LiveLatencyMode? live) {
  final base = services.playerEngineFactory;
  if (base != null) return base(video: true);
  return MediaKitEngine(
    mpvOptions: {
      ...MediaKitEngine.defaultMpvOptions,
      ...liveMpvOptions(live ?? LiveLatencyMode.lowLatency),
    },
  );
}

Future<void> _explain(BuildContext context, String text) => showDialog<void>(
  context: context,
  builder: (context) => AlertDialog(
    key: const ValueKey('twitch-multiview-unavailable'),
    title: Text(context.l10n.twitchMultiviewTitle),
    content: Text(text),
    actions: [
      TextButton(
        autofocus: true,
        onPressed: () => Navigator.of(context).pop(),
        child: Text(context.l10n.twitchClose),
      ),
    ],
  ),
);

// ----- Native tiles ----------------------------------------------------------

/// Native tiles over [source]: low latency with catch-up, [engines] makes
/// one engine per tile.
MultiviewStreamFactory nativeMultiviewStreams(
  TwitchNativeSource source,
  TwitchEngineFactory engines,
) =>
    (login, {required muted, required quality}) => NativeMultiviewStream(
      login,
      source: source,
      engines: engines,
      muted: muted,
      quality: quality,
    );

class NativeMultiviewStream implements MultiviewStream {
  NativeMultiviewStream(
    this.login, {
    required this.source,
    required this.engines,
    required bool muted,
    required String quality,
  }) : _silent = muted,
       _preference = quality {
    unawaited(_start());
  }

  final String login;
  final TwitchNativeSource source;
  final TwitchEngineFactory engines;
  final _status = ValueNotifier(MultiviewStreamStatus.loading);
  bool _silent;
  String _preference;
  TwitchNativeStream? _stream;
  TwitchQuality? _quality;
  TwitchLiveSession? _session;
  PlayerEngine? _engine;
  LivePlaybackController? _controller;
  var _disposed = false;
  var _jumping = false;

  /// The variant playing now.
  TwitchQuality? get quality => _quality;

  @override
  ValueListenable<MultiviewStreamStatus> get status => _status;

  Future<void> _start() async {
    try {
      final stream = await source.resolve(TwitchPlayTarget.live(login));
      if (_disposed) return;
      final q = pickQuality(stream, _preference);
      if (q == null) throw const TwitchNativeError('unavailable');
      _stream = stream;
      _quality = q;
      final session = source.startLive(q.url, LiveLatencyMode.lowLatency);
      final engine = engines(LiveLatencyMode.lowLatency);
      _session = session;
      _engine = engine;
      await engine.setVolume(_silent ? 0 : 100);
      final controller = LivePlaybackController(
        engine: engine,
        telemetry: session.telemetry(),
        url: session.playlistUrl,
      );
      _controller = controller;
      // No "Go live" button on a tile: far behind, it jumps by itself so
      // the tiles stay at the live edge together.
      controller.status.addListener(() {
        if (controller.status.value.shouldJump && !_jumping) {
          _jumping = true;
          unawaited(controller.goToLive().whenComplete(() => _jumping = false));
        }
      });
      await controller.start();
      if (!_disposed) _status.value = MultiviewStreamStatus.playing;
    } on TwitchNativeError catch (e) {
      debugPrint('Twitch multiview $login: $e');
      if (!_disposed) {
        _status.value = e.offline
            ? MultiviewStreamStatus.offline
            : MultiviewStreamStatus.failed;
      }
    } catch (e) {
      debugPrint('Twitch multiview $login: $e');
      if (!_disposed) _status.value = MultiviewStreamStatus.failed;
    }
  }

  @override
  void setMuted(bool muted) {
    if (muted == _silent) return;
    _silent = muted;
    unawaited(_engine?.setVolume(muted ? 0 : 100));
  }

  @override
  void setQuality(String preference) {
    _preference = preference;
    final stream = _stream;
    final session = _session;
    final engine = _engine;
    if (stream == null || session == null || engine == null) return;
    final q = pickQuality(stream, preference);
    if (q == null || q == _quality) return;
    _quality = q;
    session.setVariant(q.url);
    final controller = _controller;
    unawaited(() async {
      // At the live edge again; during an ad the proxy keeps the playhead.
      if (controller != null && await controller.goToLive()) return;
      await engine.open([session.playlistUrl], autoLoadSubtitles: false);
    }());
  }

  @override
  Widget build(BuildContext context) {
    final engine = _engine;
    return engine == null
        ? const SizedBox.expand()
        : engine.buildVideo(context);
  }

  @override
  void dispose() {
    _disposed = true;
    final controller = _controller;
    final engine = _engine;
    if (controller != null) unawaited(controller.dispose());
    if (engine != null) unawaited(engine.dispose());
    _session?.close();
    _status.dispose();
  }
}

// ----- Embed tiles -----------------------------------------------------------

/// Official embed tiles, one WebView each ([surface]: tests pass a fake).
MultiviewStreamFactory embedMultiviewStreams({
  TwitchEmbedSurfaceFactory? surface,
}) =>
    (login, {required muted, required quality}) => EmbedMultiviewStream(
      login,
      muted: muted,
      quality: quality,
      surface: surface,
    );

class EmbedMultiviewStream implements MultiviewStream {
  EmbedMultiviewStream(
    this.login, {
    required bool muted,
    required String quality,
    TwitchEmbedSurfaceFactory? surface,
  }) : _muted = muted,
       _maxHeight = multiviewMaxHeight(quality) {
    final factory = surface ?? WebViewTwitchSurface.new;
    _surface = factory(
      html: twitchMultiviewEmbedHtml(
        login,
        muted: muted,
        maxHeight: _maxHeight,
      ),
      baseUrl: twitchEmbedBaseUrl(),
      onMessage: _onMessage,
      onExternalLink: (url) => unawaited(openExternalLink(url)),
    );
  }

  final String login;
  late final TwitchEmbedSurface _surface;
  final _status = ValueNotifier(MultiviewStreamStatus.loading);
  bool _muted;
  int _maxHeight;
  var _disposed = false;

  @override
  ValueListenable<MultiviewStreamStatus> get status => _status;

  void _onMessage(String message) {
    if (_disposed) return;
    String? event;
    try {
      event = (jsonDecode(message) as Map)['event'] as String?;
    } catch (_) {
      return;
    }
    switch (event) {
      case 'ready' || 'play' || 'online':
        _status.value = MultiviewStreamStatus.playing;
      case 'offline':
        _status.value = MultiviewStreamStatus.offline;
      case 'error':
        _status.value = MultiviewStreamStatus.failed;
    }
  }

  @override
  void setMuted(bool muted) {
    if (muted == _muted) return;
    _muted = muted;
    unawaited(_surface.runJavaScript('window.kinolune.setMuted($muted);'));
  }

  @override
  void setQuality(String preference) {
    final h = multiviewMaxHeight(preference);
    if (h == _maxHeight) return;
    _maxHeight = h;
    unawaited(_surface.runJavaScript('window.kinolune.setMaxHeight($h);'));
  }

  @override
  Widget build(BuildContext context) => _surface.build(context);

  @override
  void dispose() {
    _disposed = true;
    _status.dispose();
  }
}

// ----- Picker ----------------------------------------------------------------

/// Picks up to [max] of [live] (minus [exclude]); empty when cancelled.
Future<List<String>> showMultiviewPicker(
  BuildContext context, {
  required List<TwitchStream> live,
  required int max,
  Set<String> exclude = const {},
}) async =>
    await showDialog<List<String>>(
      context: context,
      builder: (_) => _Picker(
        live: [
          for (final s in live)
            if (!exclude.contains(s.login)) s,
        ],
        max: max,
      ),
    ) ??
    const [];

class _Picker extends StatefulWidget {
  const _Picker({required this.live, required this.max});

  final List<TwitchStream> live;
  final int max;

  @override
  State<_Picker> createState() => _PickerState();
}

class _PickerState extends State<_Picker> {
  final _picked = <String>[];

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final full = _picked.length >= widget.max;
    return AlertDialog(
      key: const ValueKey('twitch-multiview-picker'),
      title: Text(l.twitchMultiviewPickTitle),
      content: SizedBox(
        width: 420,
        child: widget.live.isEmpty
            ? Text(l.twitchMultiviewPickEmpty)
            : Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    l.twitchMultiviewPickLimit(widget.max),
                    style: theme.textTheme.bodySmall,
                  ),
                  const SizedBox(height: 8),
                  Flexible(
                    child: ListView(
                      shrinkWrap: true,
                      children: [
                        for (final (i, s) in widget.live.indexed)
                          CheckboxListTile(
                            key: ValueKey('twitch-multiview-pick-${s.login}'),
                            autofocus: i == 0,
                            value: _picked.contains(s.login),
                            onChanged: full && !_picked.contains(s.login)
                                ? null
                                : (on) => setState(() {
                                    if (on ?? false) {
                                      _picked.add(s.login);
                                    } else {
                                      _picked.remove(s.login);
                                    }
                                  }),
                            title: Text(
                              s.displayName,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                            ),
                            subtitle: Text(
                              s.gameName == null || s.gameName!.isEmpty
                                  ? s.title
                                  : '${s.gameName} · ${s.title}',
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                            ),
                          ),
                      ],
                    ),
                  ),
                ],
              ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(const <String>[]),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('twitch-multiview-pick-done'),
          onPressed: _picked.isEmpty
              ? null
              : () => Navigator.of(context).pop(List.of(_picked)),
          child: Text(l.twitchMultiviewWatch),
        ),
      ],
    );
  }
}

// ----- Screen ----------------------------------------------------------------

/// Height of the bar above each tile (name, sound, ×): nothing is drawn
/// over an embedded Twitch player.
const _tileBar = 30.0;

/// Width of the chat beside the tiles.
const _chatWidth = 340.0;

class TwitchMultiviewScreen extends StatefulWidget {
  const TwitchMultiviewScreen({
    super.key,
    required this.logins,
    required this.streams,
    this.max = kMultiviewMax,
    this.embed = false,
    this.chat,
    this.live,
  });

  final List<String> logins;
  final MultiviewStreamFactory streams;
  final int max;

  /// Official embeds: each needs 400×300 uncovered.
  final bool embed;

  /// The heard stream's chat; null: [defaultTwitchChat].
  final TwitchChatFactory? chat;

  /// Streams for "Add"; null: [twitchMultiviewLive].
  final List<TwitchStream>? live;

  @override
  State<TwitchMultiviewScreen> createState() => TwitchMultiviewScreenState();
}

/// Public for tests.
class TwitchMultiviewScreenState extends State<TwitchMultiviewScreen> {
  late final MultiviewController controller;
  final Map<String, MultiviewStream> _streams = {};
  final List<FocusNode> _slotFocus = [
    for (var i = 0; i < kMultiviewMax; i++) FocusNode(debugLabel: 'slot $i'),
  ];
  final _toolbarFocus = FocusNode(debugLabel: 'multiview toolbar');
  var _chatOn = false;
  ChatFeed? _chat;
  String? _chatLogin;

  /// The feed a chat panel shows: the panel closes it.
  ChatFeed? _shownChat;

  /// The stream of each tile (tests read it).
  Map<String, MultiviewStream> get streams => Map.unmodifiable(_streams);
  bool get chatOn => _chatOn;
  String? get chatLogin => _chatLogin;

  /// The slot that has the keyboard focus, or null.
  int? get focusedSlot {
    final i = _slotFocus.indexWhere((f) => f.hasFocus);
    return i < 0 ? null : i;
  }

  @override
  void initState() {
    super.initState();
    controller = MultiviewController(widget.logins, max: widget.max)
      ..addListener(_changed);
    _sync();
  }

  void _changed() {
    _sync();
    setState(() {});
  }

  /// Starts new tiles, stops removed ones, sets sound and quality caps.
  void _sync() {
    final logins = controller.logins;
    for (final gone in _streams.keys.where((k) => !logins.contains(k))) {
      _streams[gone]?.dispose();
    }
    _streams.removeWhere((k, _) => !logins.contains(k));
    final heard = controller.audioLogin;
    for (final login in logins) {
      final muted = login != heard;
      final quality = controller.qualityOf(login);
      final s = _streams[login];
      if (s == null) {
        _streams[login] = widget.streams(login, muted: muted, quality: quality);
      } else {
        s
          ..setMuted(muted)
          ..setQuality(quality);
      }
    }
    if (_chatOn && heard != _chatLogin) _openChat();
  }

  @override
  void dispose() {
    controller.dispose();
    for (final s in _streams.values) {
      s.dispose();
    }
    for (final f in _slotFocus) {
      f.dispose();
    }
    _toolbarFocus.dispose();
    _dropChat();
    super.dispose();
  }

  /// Forgets the current feed; closes it unless a panel shows it (the
  /// panel closes it when it goes).
  void _dropChat() {
    final feed = _chat;
    if (feed != null && !identical(feed, _shownChat)) feed.close();
    _chat = null;
  }

  void toggleChat() {
    setState(() => _chatOn = !_chatOn);
    if (_chatOn) {
      _openChat();
    } else {
      _dropChat();
      _chatLogin = null;
    }
  }

  void _openChat() {
    final login = controller.audioLogin;
    _chatLogin = login;
    _dropChat();
    if (login == null) return;
    unawaited(() async {
      final feed = await (widget.chat ?? defaultTwitchChat)(context, login);
      if (!mounted || !_chatOn || _chatLogin != login) {
        feed?.close();
        return;
      }
      setState(() => _chat = feed);
    }());
  }

  /// OK / click on the tile [index]: hear it; on the heard tile, its menu.
  Future<void> hear(int index) async {
    if (index != controller.audio) {
      controller.setAudio(index);
      // In "one big" the tile moved to the big slot: focus follows it.
      final slot = controller.slotOf(index);
      if (slot >= 0) _slotFocus[slot].requestFocus();
      return;
    }
    await _tileMenu(controller.logins[index]);
  }

  Future<void> _tileMenu(String login) async {
    final l = context.l10n;
    final name = Services.read(context).twitch.channelOf(login).displayName;
    final remove = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        key: const ValueKey('twitch-multiview-tile-menu'),
        title: Text(name),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: Text(l.commonCancel),
          ),
          FilledButton(
            key: const ValueKey('twitch-multiview-remove'),
            autofocus: true,
            onPressed: () => Navigator.of(context).pop(true),
            child: Text(l.twitchMultiviewRemove),
          ),
        ],
      ),
    );
    if (remove ?? false) removeStream(login);
  }

  /// Removes [login]; the last one closes the screen.
  void removeStream(String login) {
    controller.remove(login);
    if (controller.count == 0) {
      unawaited(Navigator.of(context).maybePop());
      return;
    }
    final slot = controller.slotOf(controller.audio);
    if (slot >= 0) _slotFocus[slot].requestFocus();
  }

  /// "Add stream": a picker over the live streams not on the screen.
  Future<void> addStreams() async {
    if (!controller.canAdd) return;
    final twitch = Services.read(context).twitch;
    final live = widget.live ?? await twitchMultiviewLive(twitch);
    if (!mounted) return;
    final picked = await showMultiviewPicker(
      context,
      live: live,
      max: controller.max - controller.count,
      exclude: controller.logins.toSet(),
    );
    picked.forEach(controller.add);
  }

  void _close() => unawaited(Navigator.of(context).maybePop());

  KeyEventResult _onSlotKey(int slot, KeyEvent e, Size area) {
    if (e is! KeyDownEvent && e is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    final k = e.logicalKey;
    final direction = switch (k) {
      LogicalKeyboardKey.arrowLeft => AxisDirection.left,
      LogicalKeyboardKey.arrowRight => AxisDirection.right,
      LogicalKeyboardKey.arrowUp => AxisDirection.up,
      LogicalKeyboardKey.arrowDown => AxisDirection.down,
      _ => null,
    };
    if (direction != null) {
      final slots = multiviewSlots(controller.layout, controller.count, area);
      final next = multiviewNeighbor(slots, slot, direction);
      if (next != null) {
        _slotFocus[next].requestFocus();
      } else if (direction == AxisDirection.up) {
        _toolbarFocus.requestFocus();
      }
      return KeyEventResult.handled;
    }
    if (e is KeyRepeatEvent) return KeyEventResult.ignored;
    if (_okKeys.contains(k)) {
      final tile = controller.slotTiles[slot];
      if (tile == null) {
        unawaited(addStreams());
      } else {
        unawaited(hear(tile));
      }
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  KeyEventResult _onScreenKey(FocusNode _, KeyEvent e) {
    if (e is! KeyDownEvent) return KeyEventResult.ignored;
    final k = e.logicalKey;
    if (k == LogicalKeyboardKey.escape || k == LogicalKeyboardKey.goBack) {
      _close();
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.keyC && _chatAllowed(context)) {
      toggleChat();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  bool _chatAllowed(BuildContext context) =>
      FormFactorScope.of(context) == FormFactor.desktop &&
      MediaQuery.sizeOf(context).width >= 840 + _chatWidth;

  String _layoutName(BuildContext context, MultiviewLayout layout) {
    final l = context.l10n;
    return switch (layout) {
      MultiviewLayout.pair => l.twitchMultiviewLayoutPair,
      MultiviewLayout.grid => l.twitchMultiviewLayoutGrid,
      MultiviewLayout.focus => l.twitchMultiviewLayoutFocus,
    };
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final chatAllowed = _chatAllowed(context);
    final chat = _chat;
    final showChat = _chatOn && chatAllowed;
    return Focus(
      // Hears the keys of the tiles and the bar; never the focus itself,
      // or Down from the bar would land on the whole screen.
      canRequestFocus: false,
      onKeyEvent: _onScreenKey,
      child: Scaffold(
        key: const ValueKey('twitch-multiview'),
        backgroundColor: Colors.black,
        body: SafeArea(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _Toolbar(
                focusNode: _toolbarFocus,
                title: l.twitchMultiviewTitle,
                hint: l.twitchMultiviewHint,
                onAdd: controller.canAdd ? addStreams : null,
                layoutLabel: multiviewLayoutsFor(controller.count).length > 1
                    ? _layoutName(context, controller.layout)
                    : null,
                onLayout: controller.nextLayout,
                chatLabel: chatAllowed
                    ? (_chatOn
                          ? l.twitchPlayerHideChat
                          : l.twitchPlayerShowChat)
                    : null,
                onChat: toggleChat,
                onClose: _close,
              ),
              Expanded(
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Expanded(
                      child: LayoutBuilder(
                        builder: (context, box) => _grid(context, box.biggest),
                      ),
                    ),
                    if (showChat)
                      SizedBox(
                        key: const ValueKey('twitch-multiview-chat'),
                        width: _chatWidth,
                        child: chat == null
                            ? const Center(child: CircularProgressIndicator())
                            : Builder(
                                builder: (context) {
                                  _shownChat = chat;
                                  return TwitchChatPanel(
                                    key: ValueKey<String?>(_chatLogin),
                                    feed: chat,
                                  );
                                },
                              ),
                      ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _grid(BuildContext context, Size area) {
    final layout = controller.layout;
    final slots = multiviewSlots(layout, controller.count, area);
    if (widget.embed &&
        !multiviewFitsEmbed([
          for (final r in slots)
            Rect.fromLTWH(r.left, r.top, r.width, r.height - _tileBar),
        ])) {
      return _Message(
        key: const ValueKey('twitch-multiview-too-small'),
        icon: Icons.aspect_ratio,
        text: context.l10n.twitchMultiviewTooSmall,
      );
    }
    final tiles = controller.slotTiles;
    final logins = controller.logins;
    final audio = controller.audio;
    return Stack(
      children: [
        for (final (slot, rect) in slots.indexed)
          Positioned.fromRect(
            // A tile keeps its player when it moves to another slot.
            key: tiles[slot] == null
                ? ValueKey<int>(slot)
                : ValueKey<String>(logins[tiles[slot]!]),
            rect: rect,
            child: Focus(
              focusNode: _slotFocus[slot],
              autofocus: tiles[slot] == audio,
              onKeyEvent: (_, e) => _onSlotKey(slot, e, area),
              child: tiles[slot] == null
                  ? _FreeSlot(
                      key: ValueKey('twitch-multiview-free-$slot'),
                      focusNode: _slotFocus[slot],
                      onTap: controller.canAdd ? addStreams : null,
                    )
                  : _Tile(
                      key: ValueKey(
                        'twitch-multiview-tile-${logins[tiles[slot]!]}',
                      ),
                      name: Services.read(context).twitch
                          .channelOf(logins[tiles[slot]!])
                          .displayName,
                      stream: _streams[logins[tiles[slot]!]]!,
                      heard: tiles[slot] == audio,
                      focusNode: _slotFocus[slot],
                      onTap: () => unawaited(hear(tiles[slot]!)),
                      onRemove: () => removeStream(logins[tiles[slot]!]),
                    ),
            ),
          ),
      ],
    );
  }
}

final _okKeys = <LogicalKeyboardKey>{
  LogicalKeyboardKey.select,
  LogicalKeyboardKey.enter,
  LogicalKeyboardKey.numpadEnter,
  LogicalKeyboardKey.gameButtonA,
};

class _Toolbar extends StatelessWidget {
  const _Toolbar({
    required this.focusNode,
    required this.title,
    required this.hint,
    required this.onAdd,
    required this.layoutLabel,
    required this.onLayout,
    required this.chatLabel,
    required this.onChat,
    required this.onClose,
  });

  final FocusNode focusNode;
  final String title;
  final String hint;
  final VoidCallback? onAdd;
  final String? layoutLabel;
  final VoidCallback onLayout;
  final String? chatLabel;
  final VoidCallback onChat;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final layout = layoutLabel;
    final chat = chatLabel;
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 6, 6, 6),
      child: Row(
        children: [
          Flexible(
            child: Text(
              title,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: theme.textTheme.titleMedium?.copyWith(color: Colors.white),
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Text(
              hint,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: theme.textTheme.bodySmall?.copyWith(
                color: KColors.textTertiary,
              ),
            ),
          ),
          TextButton.icon(
            key: const ValueKey('twitch-multiview-add'),
            focusNode: focusNode,
            onPressed: onAdd,
            icon: const Icon(Icons.add),
            label: Text(l.twitchMultiviewAdd),
          ),
          if (layout != null)
            TextButton.icon(
              key: const ValueKey('twitch-multiview-layout'),
              onPressed: onLayout,
              icon: const Icon(Icons.grid_view),
              label: Text(layout),
            ),
          if (chat != null)
            TextButton.icon(
              key: const ValueKey('twitch-multiview-chat-toggle'),
              onPressed: onChat,
              icon: const Icon(Icons.chat_bubble_outline),
              label: Text(chat),
            ),
          IconButton(
            key: const ValueKey('twitch-multiview-close'),
            tooltip: l.twitchClose,
            onPressed: onClose,
            icon: const Icon(Icons.close, color: Colors.white),
          ),
        ],
      ),
    );
  }
}

class _Tile extends StatelessWidget {
  const _Tile({
    super.key,
    required this.name,
    required this.stream,
    required this.heard,
    required this.focusNode,
    required this.onTap,
    required this.onRemove,
  });

  final String name;
  final MultiviewStream stream;
  final bool heard;
  final FocusNode focusNode;
  final VoidCallback onTap;
  final VoidCallback onRemove;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    return ListenableBuilder(
      listenable: focusNode,
      builder: (context, _) {
        final focused = focusNode.hasFocus;
        return GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: onTap,
          child: DecoratedBox(
            position: DecorationPosition.foreground,
            decoration: BoxDecoration(
              border: Border.all(
                width: focused ? 3 : 1,
                color: focused
                    ? KColors.accent
                    : heard
                    ? KColors.live
                    : Colors.white24,
              ),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                SizedBox(
                  height: _tileBar,
                  child: Row(
                    children: [
                      const SizedBox(width: 8),
                      Icon(
                        heard ? Icons.volume_up : Icons.volume_off,
                        size: 16,
                        semanticLabel: heard
                            ? l.twitchMultiviewHeard
                            : l.twitchMultiviewMuted,
                        color: heard ? KColors.live : KColors.textTertiary,
                      ),
                      const SizedBox(width: 6),
                      Expanded(
                        child: Text(
                          name,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: theme.textTheme.labelMedium?.copyWith(
                            color: Colors.white,
                          ),
                        ),
                      ),
                      ExcludeFocus(
                        child: IconButton(
                          tooltip: l.twitchMultiviewRemove,
                          iconSize: 16,
                          visualDensity: VisualDensity.compact,
                          onPressed: onRemove,
                          icon: const Icon(Icons.close, color: Colors.white70),
                        ),
                      ),
                    ],
                  ),
                ),
                Expanded(
                  child: ValueListenableBuilder<MultiviewStreamStatus>(
                    valueListenable: stream.status,
                    builder: (context, status, _) => switch (status) {
                      MultiviewStreamStatus.playing => stream.build(context),
                      MultiviewStreamStatus.loading => Stack(
                        fit: StackFit.expand,
                        children: [
                          // The embed loads inside its WebView: keep it.
                          stream.build(context),
                          const Center(child: CircularProgressIndicator()),
                        ],
                      ),
                      MultiviewStreamStatus.offline => _Message(
                        icon: Icons.sensors_off_outlined,
                        text: l.twitchMultiviewOffline,
                      ),
                      MultiviewStreamStatus.failed => _Message(
                        icon: Icons.error_outline,
                        text: l.twitchMultiviewFailed,
                      ),
                    },
                  ),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}

class _FreeSlot extends StatelessWidget {
  const _FreeSlot({super.key, required this.focusNode, required this.onTap});

  final FocusNode focusNode;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return ListenableBuilder(
      listenable: focusNode,
      builder: (context, _) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: KColors.bgElevated,
            border: Border.all(
              width: focusNode.hasFocus ? 3 : 1,
              color: focusNode.hasFocus ? KColors.accent : Colors.white12,
            ),
          ),
          child: onTap == null
              ? const SizedBox.expand()
              : Center(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      const Icon(Icons.add, color: Colors.white70, size: 32),
                      const SizedBox(height: 6),
                      Text(
                        l.twitchMultiviewAdd,
                        style: const TextStyle(color: Colors.white70),
                      ),
                    ],
                  ),
                ),
        ),
      ),
    );
  }
}

class _Message extends StatelessWidget {
  const _Message({super.key, required this.icon, required this.text});

  final IconData icon;
  final String text;

  @override
  Widget build(BuildContext context) => Center(
    child: Padding(
      padding: const EdgeInsets.all(16),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon, color: Colors.white54, size: 32),
          const SizedBox(height: 8),
          Text(
            text,
            textAlign: TextAlign.center,
            style: const TextStyle(color: Colors.white70),
          ),
        ],
      ),
    ),
  );
}
