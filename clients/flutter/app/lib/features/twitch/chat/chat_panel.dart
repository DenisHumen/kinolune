// SPDX-License-Identifier: MPL-2.0
//
// A chat panel (TWI-012): header with the channel, the read-only mark of
// the anonymous chat and the "pause animations" switch, above the list.
// The player (TWI-014) puts the same panel beside or over the video.
// TWI-024: chat settings (filters, highlights) of the active profile, the
// input with room modes and autocomplete, and line actions (mention,
// block).

import 'package:material_ui/material_ui.dart';
import 'package:pa_core/pa_core.dart';
import 'package:pa_library/pa_library.dart' show RustLibrary;
import 'package:path_provider/path_provider.dart';

import '../../../l10n/l10n.dart';
import '../../../platform/form_factor.dart';
import '../../../services/app_services.dart';
import '../../../theme/kinolune_theme.dart';
import 'chat_controller.dart';
import 'chat_host.dart';
import 'chat_input.dart';
import 'chat_model.dart';
import 'chat_settings.dart';
import 'chat_settings_page.dart';
import 'chat_view.dart';
import 'core_chat_feed.dart';

/// Lines kept per form factor (research 06 §3.3: 500–1000; TV shows
/// fewer and thins the flow).
int chatCapacityFor(FormFactor f) => f == FormFactor.tv ? 150 : 500;

/// TV thinning: lines per second (`+N` for the rest).
int? chatRateLimitFor(FormFactor f) => f == FormFactor.tv ? 8 : null;

/// Chat options of the core for this device.
Future<CoreChatOptions> chatOptions(
  BuildContext context, {
  required bool animate,
}) async {
  final dpr = MediaQuery.devicePixelRatioOf(context);
  final cache = await getApplicationCacheDirectory();
  return CoreChatOptions(
    cacheDir: cache.path,
    devicePixelRatio: dpr,
    dark: true,
    animate: animate,
    flushMs: 0,
  );
}

/// The core's check of a word rule (the regex engine that filters).
String? coreChatRuleCheck(ChatWordRule r) => checkChatWordRule(
  rule: CoreChatWordRule(
    pattern: r.pattern,
    regex: r.regex,
    hideMessage: r.hide,
  ),
);

/// Header + list (+ input) for [feed]. Owns the controller and closes the
/// feed.
class TwitchChatPanel extends StatefulWidget {
  const TwitchChatPanel({
    super.key,
    required this.feed,
    this.pauseAnimations,
    this.onPauseAnimationsChanged,
    this.settings,
    this.onSettingsChanged,
    this.checkRule,
    this.onSignIn,
    this.onAllowSending,
    this.showInput = true,
  });

  final ChatFeed feed;

  /// Explicit setting; `null` follows the system's reduce motion.
  final bool? pauseAnimations;
  final ValueChanged<bool>? onPauseAnimationsChanged;

  /// Viewer settings of the active profile (TWI-024); null: none yet.
  final ChatSettings? settings;

  /// Saves edited settings; null hides the settings button and "Block".
  final ValueChanged<ChatSettings>? onSettingsChanged;

  /// Checks a word rule in the settings (the core's regex engine).
  final ChatRuleCheck? checkRule;

  /// Signed out: sign in from the input.
  final VoidCallback? onSignIn;

  /// Signed in without the write permission: sign in again allowing it.
  final VoidCallback? onAllowSending;

  /// The input under the list (TV hides it: no keyboard).
  final bool showInput;

  @override
  State<TwitchChatPanel> createState() => _TwitchChatPanelState();
}

class _TwitchChatPanelState extends State<TwitchChatPanel> {
  ChatController? _controller;
  final _input = GlobalKey<TwitchChatInputState>();

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_controller == null) {
      final f = FormFactorScope.of(context);
      _controller = ChatController(
        widget.feed,
        capacity: chatCapacityFor(f),
        maxLinesPerSecond: chatRateLimitFor(f),
      );
      final s = widget.settings;
      if (s != null) _controller!.applySettings(s);
    }
  }

  @override
  void didUpdateWidget(TwitchChatPanel old) {
    super.didUpdateWidget(old);
    final s = widget.settings;
    if (s != null && s != old.settings && s != _controller?.settings) {
      _controller?.applySettings(s);
    }
  }

  void _save(ChatSettings s) {
    _controller?.applySettings(s);
    widget.onSettingsChanged?.call(s);
  }

  Future<void> _openSettings() async {
    final c = _controller!;
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => TwitchChatSettingsPage(
          settings: c.settings,
          onChanged: _save,
          checkRule: widget.checkRule,
          skippedRules: c.filterErrors.length,
        ),
      ),
    );
  }

  Future<void> _lineActions(ChatLine line) async {
    final l = context.l10n;
    final canBlock = widget.onSettingsChanged != null;
    final canMention = widget.showInput && widget.feed.canSend;
    if (!canBlock && !canMention) return;
    final name = line.displayName.isEmpty ? line.login : line.displayName;
    final picked = await showModalBottomSheet<String>(
      context: context,
      builder: (context) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (canMention)
              ListTile(
                leading: const Icon(Icons.alternate_email_rounded),
                title: Text(l.twitchChatMentionUser(name)),
                onTap: () => Navigator.of(context).pop('mention'),
              ),
            if (canBlock)
              ListTile(
                leading: const Icon(Icons.block_rounded),
                title: Text(l.twitchChatBlockUser(name)),
                onTap: () => Navigator.of(context).pop('block'),
              ),
          ],
        ),
      ),
    );
    if (!mounted) return;
    switch (picked) {
      case 'mention':
        _input.currentState?.mention(
          line.displayName.toLowerCase() == line.login
              ? line.displayName
              : line.login,
        );
      case 'block':
        _save(_controller!.settings.block(line.login));
    }
  }

  @override
  void dispose() {
    _controller?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = KDensity.of(context);
    final paused =
        widget.pauseAnimations ?? MediaQuery.disableAnimationsOf(context);
    final toggle = widget.onPauseAnimationsChanged;
    final header = Padding(
      padding: EdgeInsets.fromLTRB(d.gap / 2, d.gap / 4, d.gap / 4, d.gap / 4),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  l.twitchChatTitle(widget.feed.channel),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: d.type.headline,
                    fontWeight: FontWeight.w600,
                    color: KColors.textPrimary,
                  ),
                ),
                if (!widget.feed.canSend)
                  Text(
                    l.twitchChatReadOnly,
                    style: TextStyle(
                      fontSize: d.type.caption,
                      color: KColors.textSecondary,
                    ),
                  ),
              ],
            ),
          ),
          if (widget.onSettingsChanged != null)
            IconButton(
              key: const ValueKey('twitch-chat-settings'),
              tooltip: l.twitchChatSettings,
              icon: const Icon(Icons.tune_rounded),
              onPressed: _openSettings,
            ),
          if (toggle != null)
            IconButton(
              tooltip: paused
                  ? l.twitchChatPlayAnimations
                  : l.twitchChatPauseAnimations,
              isSelected: paused,
              icon: const Icon(Icons.motion_photos_on_outlined),
              selectedIcon: const Icon(Icons.motion_photos_paused_outlined),
              onPressed: () => toggle(!paused),
            ),
        ],
      ),
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        header,
        Expanded(
          child: TwitchChatView(
            controller: _controller!,
            pauseAnimations: paused,
            onLineAction: _lineActions,
          ),
        ),
        if (widget.showInput)
          TwitchChatInput(
            key: _input,
            controller: _controller!,
            onSignIn: widget.onSignIn,
            onAllowSending: widget.onAllowSending,
          ),
      ],
    );
  }
}

/// Full-screen read-only chat of [channel] over the core. "Pause animated
/// emotes" is the app setting (Settings → Twitch) or the system's reduce
/// motion; when it is on at start the core fetches still images only.
class TwitchChatPage extends StatefulWidget {
  const TwitchChatPage({super.key, required this.channel, this.demoRate});

  final String channel;

  /// Made-up chat at this rate instead of Twitch (demo mode).
  final int? demoRate;

  @override
  State<TwitchChatPage> createState() => _TwitchChatPageState();
}

class _TwitchChatPageState extends State<TwitchChatPage> {
  ChatFeed? _feed;
  Object? _error;
  var _started = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_started) return;
    _started = true;
    _start();
  }

  bool _paused(bool setting) =>
      setting || MediaQuery.disableAnimationsOf(context);

  Future<void> _start() async {
    final services = Services.read(context);
    final setting = services.twitchChatPauseAnimations.value;
    try {
      await RustLibrary.ensureInitialized();
      if (!mounted) return;
      final options = await chatOptions(context, animate: !_paused(setting));
      final rate = widget.demoRate;
      final feed = rate != null
          ? CoreChatFeed.demo(rate, options)
          : await openTwitchChatFeed(
              services,
              widget.channel,
              options,
            );
      if (!mounted) {
        feed.close();
        return;
      }
      setState(() => _feed = feed);
    } catch (e) {
      debugPrint('Twitch chat: $e');
      if (mounted) setState(() => _error = e);
    }
  }

  /// After a sign-in: the same channel, now signed in.
  /// The old panel's controller closes the old feed.
  void _reopen() {
    setState(() => _feed = null);
    _start();
  }

  @override
  Widget build(BuildContext context) {
    final feed = _feed;
    return Scaffold(
      backgroundColor: KColors.bgBase,
      appBar: AppBar(backgroundColor: KColors.bgBase),
      body: SafeArea(
        top: false,
        child: feed != null
            ? TwitchAppChatPanel(
                key: ObjectKey(feed),
                feed: feed,
                onReopen: widget.demoRate == null ? _reopen : null,
              )
            : Center(
                child: _error == null
                    ? const CircularProgressIndicator()
                    : Text(context.l10n.twitchChatDisconnected),
              ),
      ),
    );
  }
}
