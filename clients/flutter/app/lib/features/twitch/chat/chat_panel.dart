// SPDX-License-Identifier: MPL-2.0
//
// A chat panel (TWI-012): header with the channel, the read-only mark of
// the anonymous chat and the "pause animations" switch, above the list.
// The player (TWI-014) puts the same panel beside or over the video.

import 'package:material_ui/material_ui.dart';
import 'package:pa_core/pa_core.dart';
import 'package:pa_library/pa_library.dart' show RustLibrary;
import 'package:path_provider/path_provider.dart';

import '../../../l10n/l10n.dart';
import '../../../platform/form_factor.dart';
import '../../../services/app_services.dart';
import '../../../theme/kinolune_theme.dart';
import 'chat_controller.dart';
import 'chat_model.dart';
import 'chat_view.dart';
import 'core_chat_feed.dart';

/// Preference key: 'true' pauses animated emotes; absent = follow the
/// system's reduce-motion setting.
const twitchChatPauseAnimationsKey = 'twitch_chat_pause_animations';

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

/// Header + list for [feed]. Owns the controller and closes the feed.
class TwitchChatPanel extends StatefulWidget {
  const TwitchChatPanel({
    super.key,
    required this.feed,
    this.pauseAnimations,
    this.onPauseAnimationsChanged,
  });

  final ChatFeed feed;

  /// Explicit setting; `null` follows the system's reduce motion.
  final bool? pauseAnimations;
  final ValueChanged<bool>? onPauseAnimationsChanged;

  @override
  State<TwitchChatPanel> createState() => _TwitchChatPanelState();
}

class _TwitchChatPanelState extends State<TwitchChatPanel> {
  ChatController? _controller;

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
          ),
        ),
      ],
    );
  }
}

/// Full-screen read-only chat of [channel] over the core, with the
/// "pause animations" setting kept in the app preferences.
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
  bool? _pause;
  var _started = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_started) return;
    _started = true;
    _start();
  }

  Future<void> _start() async {
    final prefs = Services.read(context).preferences;
    final saved = await prefs.read(twitchChatPauseAnimationsKey);
    if (!mounted) return;
    final pause = saved == null
        ? MediaQuery.disableAnimationsOf(context)
        : saved == 'true';
    try {
      await RustLibrary.ensureInitialized();
      if (!mounted) return;
      final options = await chatOptions(context, animate: !pause);
      final rate = widget.demoRate;
      final feed = rate != null
          ? CoreChatFeed.demo(rate, options)
          : CoreChatFeed.anonymous(widget.channel, options);
      if (!mounted) {
        feed.close();
        return;
      }
      setState(() {
        _pause = saved == null ? null : pause;
        _feed = feed;
      });
    } catch (e) {
      debugPrint('Twitch chat: $e');
      if (mounted) setState(() => _error = e);
    }
  }

  void _setPause(bool value) {
    setState(() => _pause = value);
    Services.read(
      context,
    ).preferences.write(twitchChatPauseAnimationsKey, '$value');
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
            ? TwitchChatPanel(
                feed: feed,
                pauseAnimations: _pause,
                onPauseAnimationsChanged: _setPause,
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
