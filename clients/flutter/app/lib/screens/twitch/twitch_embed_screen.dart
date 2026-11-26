// SPDX-License-Identifier: MPL-2.0
//
// The official Twitch embed in a web view (TWI-005, TWI-029): the screen
// registered as `TwitchService.embedPlayer`. Android, iOS and macOS show
// it in `webview_flutter` (BSD-3-Clause); Windows in Microsoft Edge
// WebView2 (`webview_windows`, BSD-3-Clause) and Linux in the system's
// WebKitGTK (LGPL-2.1, opened at run time by `linux/runner`), see
// `twitch_embed_desktop.dart` and ADR-0144. Where the runtime is missing
// the hook stays empty and the Streams section offers "Open on Twitch"
// with what to install. The page, its events and the progress sync live
// in `twitch/twitch_embed.dart`; tests replace the web view with a fake
// surface ([TwitchEmbedSurfaceFactory]).
//
// TWI-041: a live stream gets the app's chat panel beside the embed (the
// native player's panel, its resize handle and its remembered width), with
// «Показать чат / Скрыть чат» in the bar and C. Nothing may cover the
// embed (Twitch rules), so the panel is a column of its own, hidden in
// fullscreen; the web view's box — and the rectangle the Linux runner
// places its native view at (BUG-028) — is what is left of the row. The
// embed is Twitch's player alone (`Twitch.Player`, not `Twitch.Embed`):
// Twitch's own chat never shows.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart' show kIsWeb, defaultTargetPlatform;
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart'
    show
        SleepTimerController,
        SleepTimerHost,
        SleepTimerMenu,
        SleepTimerStrings,
        SleepTimerWarning,
        formatSleepRemaining;
import 'package:webview_flutter/webview_flutter.dart';
import 'package:webview_flutter_android/webview_flutter_android.dart';
import 'package:webview_flutter_wkwebview/webview_flutter_wkwebview.dart';

import '../../features/twitch/chat/chat_host.dart';
import '../../features/twitch/chat/chat_model.dart';
import '../../flavor.dart';
import '../../integration/player.dart' show setWindowFullscreen;
import '../../l10n/l10n.dart';
import '../../platform/form_factor.dart';
import '../../services/app_services.dart';
import '../../services/external_links.dart';
import '../../twitch/twitch_embed.dart';
import '../../twitch/twitch_service.dart';
import '../../widgets/error_boundary.dart';
import 'twitch_embed_desktop.dart';
import 'twitch_follow.dart';
import 'twitch_player_screen.dart'
    show
        KColorsChat,
        TwitchChatFactory,
        TwitchChatResizeHandle,
        defaultTwitchChat,
        kTwitchChatHandleWidth,
        kTwitchChatMaxWidth,
        kTwitchChatMinWidth,
        readTwitchChatWidth,
        saveTwitchChatWidth,
        twitchSideChatWidth,
        twitchWideLayout;
import 'twitch_record_live.dart';
import 'twitch_widgets.dart';

/// Twitch: the embed is at least 400×300.
const kTwitchEmbedMinWidth = 400.0;

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
  TwitchChatFactory? chat,
}) => Navigator.of(context, rootNavigator: true).push(
  // A player screen like pa_player's: over the whole window (no sidebar),
  // so fullscreen leaves nothing but the video (TWI-029).
  // BUG-026: a failure while building (a web view call the platform lacks)
  // is an error screen with «Назад», not a gray window.
  MaterialPageRoute<void>(
    builder: (_) => ErrorBoundary(
      name: 'twitch-embed',
      child: TwitchEmbedScreen(
        target: target,
        surface: surface,
        engine: engine,
        chat: chat,
      ),
    ),
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
    this.chat,
  });

  final TwitchPlayTarget target;

  /// The live chat beside the embed (TWI-041); null: [defaultTwitchChat].
  final TwitchChatFactory? chat;

  /// The web view; null = the one of [engine].
  final TwitchEmbedSurfaceFactory? surface;

  /// null = this platform's.
  final TwitchEmbedEngine? engine;

  @override
  State<TwitchEmbedScreen> createState() => TwitchEmbedScreenState();
}

class TwitchEmbedScreenState extends State<TwitchEmbedScreen>
    implements SleepTimerHost {
  late TwitchEmbedSession _session;
  late TwitchEmbedSurface _surface;
  late AppServices _services;
  var _attempt = 0;
  var _phone = false;

  /// The player fills the window: no bar above it (TWI-029).
  var _fullscreen = false;

  /// The window itself is fullscreen because of this screen.
  var _windowFullscreen = false;

  /// The screen's keys; Escape in the chat input gives focus back here.
  final _keys = FocusNode(debugLabel: 'twitch-embed-keys');

  /// The live chat beside the embed (TWI-041); null: none (yet).
  ChatFeed? _chat;
  var _chatStarted = false;
  var _chatOn = true;

  /// Width of the chat; null: the default share of the window.
  double? _chatWidth;

  /// The chat is on (it shows beside the embed on a wide window).
  bool get chatOn => _chat != null && _chatOn && !_fullscreen;

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
    _services.sleepTimer.attach(this);
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

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_chatStarted) return;
    _chatStarted = true;
    // Live only (a VOD has no live chat); a TV keeps the embed alone.
    final login = widget.target.login;
    if (!widget.target.isLive || login == null || login.isEmpty) return;
    if (FormFactorScope.of(context) == FormFactor.tv) return;
    unawaited(_loadChatWidth());
    unawaited(_openChat(login));
  }

  Future<void> _loadChatWidth() async {
    final w = await readTwitchChatWidth(_services);
    if (w != null && mounted) setState(() => _chatWidth = w);
  }

  /// The chat was dragged to [width] (remembered when the drag ends): the
  /// same setting as the native player's.
  void setChatWidth(double width, {bool save = false}) {
    final w = width.clamp(kTwitchChatMinWidth, kTwitchChatMaxWidth);
    if (w != _chatWidth) setState(() => _chatWidth = w);
    if (save) unawaited(saveTwitchChatWidth(_services, w));
  }

  Future<void> _openChat(String login) async {
    final feed = await (widget.chat ?? defaultTwitchChat)(context, login);
    if (!mounted) {
      feed?.close();
      return;
    }
    setState(() => _chat = feed);
  }

  /// After a sign-in from the chat: open it again, now signed in. The old
  /// panel's controller closes the old feed.
  void _reopenChat() {
    setState(() => _chat = null);
    unawaited(_openChat(widget.target.login!));
  }

  /// «Показать чат / Скрыть чат» and C.
  void toggleChat() => setState(() => _chatOn = !_chatOn);

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

  // ---- PLAY-022: the sleep timer through the embed's JS API ---------------

  SleepTimerController get sleepTimer => _services.sleepTimer;

  /// The timer ran out: the embed pauses (Twitch keeps the VOD position;
  /// the session saves ours on the pause event).
  @override
  void sleepPause() {
    if (!mounted) return;
    unawaited(_surface.runJavaScript('window.kinolune.pause();'));
  }

  /// The fade of the last 30 s: the embed's volume (`setVolume`).
  @override
  void sleepFade(double factor) {
    if (!mounted) return;
    final f = factor.clamp(0.0, 1.0).toStringAsFixed(3);
    unawaited(
      _surface.runJavaScript('window.kinolune.fade($f);'), // l10n-ignore
    );
  }

  /// The menu: a dialog (a native web view lies over Flutter and hides
  /// while it is open). No «в конце серии» on Twitch.
  Future<void> openSleepTimer() async {
    twitchEmbedCovered.value++;
    try {
      await showDialog<void>(
        context: context,
        builder: (context) => Dialog(
          key: const ValueKey('twitch-embed-sleep-dialog'),
          backgroundColor: const Color(0xFF202029),
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 420),
            child: SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(12, 16, 12, 12),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Padding(
                    padding: const EdgeInsets.fromLTRB(14, 0, 14, 8),
                    child: Text(
                      SleepTimerStrings.of(context).sleepTimer,
                      style: const TextStyle(
                        color: Colors.white,
                        fontSize: 20,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ),
                  SleepTimerMenu(
                    controller: sleepTimer,
                    endOfItem: false,
                    stillWatching: false,
                    onPicked: () => Navigator.of(context).pop(),
                  ),
                ],
              ),
            ),
          ),
        ),
      );
    } finally {
      twitchEmbedCovered.value--;
    }
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

  /// Text is being typed (the chat input): the keys are its letters.
  static bool get _typing {
    final context = FocusManager.instance.primaryFocus?.context;
    return context != null &&
        (context.widget is EditableText ||
            context.findAncestorWidgetOfExactType<EditableText>() != null);
  }

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    sleepTimer.userInput();
    final key = event.logicalKey;
    final vod = !widget.target.isLive;
    if (_typing) {
      // Escape leaves the input; the next one leaves the screen.
      if (key != LogicalKeyboardKey.escape) return KeyEventResult.ignored;
      _keys.requestFocus();
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.keyC && _chat != null) {
      toggleChat();
    } else if (key == LogicalKeyboardKey.escape) {
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
    // PLAY-022: the timer belongs to this viewing.
    _services.sleepTimer
      ..detach(this)
      ..cancel();
    final twitch = _services.twitch;
    unawaited(_session.close().then((_) => twitch.progressChanged()));
    _surface.dispose();
    if (_phone) unawaited(SystemChrome.setPreferredOrientations(const []));
    if (_windowFullscreen) unawaited(setWindowFullscreen(false));
    // The panel owns the chat controller; the feed is ours.
    _chat?.close();
    _keys.dispose();
    super.dispose();
  }

  /// Room for the chat beside the embed in [size]: a wide layout (as the
  /// native player's side chat) that leaves the embed its 400.
  bool _chatFits(BuildContext context, Size size) =>
      twitchWideLayout(size, FormFactorScope.of(context)) &&
      size.width - kTwitchEmbedMinWidth - kTwitchChatHandleWidth >=
          kTwitchChatMinWidth;

  /// The embed and, for a live stream in a wide window, the chat beside
  /// it (TWI-041). The row never changes shape: the embed stays its first
  /// child, so a web view is never rebuilt (that would reload the
  /// stream); a hidden chat is offstage, keeping its lines and listener.
  Widget _layout(BuildContext context, BoxConstraints c) {
    final player = ValueListenableBuilder<TwitchEmbedState>(
      key: ValueKey(_attempt),
      valueListenable: _session.state,
      builder: (context, state, _) => state == TwitchEmbedState.failed
          ? _Failed(onRetry: _retry, onOpen: _openOnTwitch)
          : KeyedSubtree(
              key: ValueKey('twitch-embed-surface-$_attempt'),
              child: _surface.build(context),
            ),
    );
    final feed = _chat;
    // Twitch: the embed stays at least 400 wide beside the chat.
    final room = c.maxWidth - kTwitchEmbedMinWidth - kTwitchChatHandleWidth;
    final width = math.max(
      kTwitchChatMinWidth,
      math.min(twitchSideChatWidth(_chatWidth, c.maxWidth), room),
    );
    final side = chatOn && _chatFits(context, c.biggest);
    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Expanded(child: player),
        if (feed != null) ...[
          if (side)
            TwitchChatResizeHandle(
              // From the state, not this build: several drag events may
              // come before the next frame.
              onDrag: (dx) => setChatWidth((_chatWidth ?? width) - dx),
              onEnd: () => setChatWidth(_chatWidth ?? width, save: true),
            ),
          Offstage(
            offstage: !side,
            child: SizedBox(
              width: width,
              child: ColoredBox(
                color: KColorsChat.bg,
                child: KeyedSubtree(
                  key: ObjectKey(feed),
                  child: TwitchAppChatPanel(
                    key: const ValueKey('twitch-embed-chat'),
                    feed: feed,
                    onReopen: _reopenChat,
                  ),
                ),
              ),
            ),
          ),
        ],
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final login = widget.target.login;
    final title = login == null || login.isEmpty
        ? l.twitchEmbedTitle
        : _services.twitch.channelOf(login).displayName;
    final chatToggle =
        _chat != null && _chatFits(context, MediaQuery.sizeOf(context));
    return Focus(
      focusNode: _keys,
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
                  // TWI-035: record this broadcast (full builds).
                  if (AppFlavor.twitchArchive &&
                      widget.target.isLive &&
                      login != null &&
                      login.isNotEmpty &&
                      _services.visibleArchive != null)
                    TwitchRecordLiveButton(
                      login: login,
                      userId: _services.twitch.channelOf(login).userId,
                      compact: _phone,
                    ),
                  // TWI-033: follow the channel on Twitch.
                  if (login != null &&
                      login.isNotEmpty &&
                      _services.twitch.account != null)
                    TwitchFollowButton(
                      channel: _services.twitch.channelOf(login),
                      compact: true,
                    ),
                  ListenableBuilder(
                    listenable: sleepTimer,
                    builder: (context, _) {
                      final name = SleepTimerStrings.of(context).sleepTimer;
                      final left = sleepTimer.remaining;
                      return IconButton(
                        key: const ValueKey('twitch-embed-sleep'),
                        tooltip: sleepTimer.active && left != null
                            ? '$name · ${formatSleepRemaining(left)}'
                            : name,
                        icon: Icon(
                          sleepTimer.active
                              ? Icons.bedtime
                              : Icons.bedtime_outlined,
                        ),
                        onPressed: openSleepTimer,
                      );
                    },
                  ),
                  if (chatToggle)
                    IconButton(
                      key: const ValueKey('twitch-embed-chat-toggle'),
                      tooltip: _chatOn
                          ? l.twitchPlayerHideChat
                          : l.twitchPlayerShowChat,
                      icon: Icon(
                        _chatOn
                            ? Icons.speaker_notes_off_outlined
                            : Icons.chat_outlined,
                      ),
                      onPressed: toggleChat,
                    ),
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
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              // PLAY-022: «Пауза через 30 с — Продлить» above the embed
              // (nothing may cover it).
              ListenableBuilder(
                listenable: sleepTimer,
                builder: (context, _) => sleepTimer.warning
                    ? Padding(
                        padding: const EdgeInsets.symmetric(vertical: 6),
                        child: Center(
                          child: SleepTimerWarning(controller: sleepTimer),
                        ),
                      )
                    : const SizedBox.shrink(),
              ),
              Expanded(child: LayoutBuilder(builder: _layout)),
            ],
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

/// Runs one setup call of the web view; a call this platform lacks throws
/// (synchronously or in its future) and is logged instead (BUG-026).
void _setUp(String what, Future<void> Function() call) {
  try {
    unawaited(
      call().catchError((Object e) {
        debugPrint('Twitch embed web view: $what failed: $e');
      }),
    );
  } catch (e) {
    debugPrint('Twitch embed web view: $what failed: $e');
  }
}

/// `setBackgroundColor` of `webview_flutter`: WKWebView on macOS has no
/// background colour or opaque flag and throws `UnimplementedError` from
/// the call itself, which made the embed screen fail in initState — a gray
/// window in release builds (BUG-026).
bool webViewBackgroundSupported(TargetPlatform platform) =>
    platform != TargetPlatform.macOS;

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
      _setUp(
        'autoplay',
        () => platform.setMediaPlaybackRequiresUserGesture(false),
      );
    }
    _setUp(
      'JavaScript',
      () => _controller.setJavaScriptMode(JavaScriptMode.unrestricted),
    );
    if (webViewBackgroundSupported(defaultTargetPlatform)) {
      _setUp('background', () => _controller.setBackgroundColor(Colors.black));
    }
    _setUp(
      'channel',
      () => _controller.addJavaScriptChannel(
        twitchEmbedChannel,
        onMessageReceived: (m) => onMessage(m.message),
      ),
    );
    _setUp(
      'navigation',
      () => _controller.setNavigationDelegate(
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
    _setUp(
      'page',
      () => _controller.loadHtmlString(html, baseUrl: baseUrl.toString()),
    );
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
