// SPDX-License-Identifier: MPL-2.0
//
// The chat as the app opens it (TWI-024): the signed-in chat when the user
// is signed in to Twitch (EventSub; sending when the sign-in allows it),
// else the anonymous read-only chat; the panel wired to the active
// profile's chat settings and to sign-in.

import 'dart:async';

import 'package:material_ui/material_ui.dart';
import 'package:pa_core/pa_core.dart' show CoreChatOptions;

import '../../../platform/form_factor.dart';
import '../../../screens/twitch/twitch_play.dart' show showTwitchSignIn;
import '../../../services/app_services.dart';
import '../../../twitch/core_twitch_backend.dart';
import 'chat_model.dart';
import 'chat_panel.dart';
import 'chat_settings.dart';
import 'core_chat_feed.dart';

/// The chat of [login]: made-up in the demo, signed-in when possible,
/// else anonymous.
Future<ChatFeed> openTwitchChatFeed(
  AppServices services,
  String login,
  CoreChatOptions options, {
  bool demo = false,
}) async {
  if (demo) return CoreChatFeed.demo(20, options);
  final backend = services.twitch.backend;
  if (backend is CoreTwitchBackend) {
    final signed = await backend.signedInChat(login, options);
    if (signed != null) return signed;
  }
  return CoreChatFeed.anonymous(login, options);
}

/// [TwitchChatPanel] with the app's settings: paused animations, the
/// active profile's chat settings, sign-in from the input. After a
/// sign-in [onReopen] asks the host to open the chat again (now signed
/// in, maybe allowed to send).
class TwitchAppChatPanel extends StatefulWidget {
  const TwitchAppChatPanel({super.key, required this.feed, this.onReopen});

  final ChatFeed feed;
  final VoidCallback? onReopen;

  @override
  State<TwitchAppChatPanel> createState() => _TwitchAppChatPanelState();
}

class _TwitchAppChatPanelState extends State<TwitchAppChatPanel> {
  ChatSettings? _settings;
  var _started = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_started) return;
    _started = true;
    unawaited(_load());
  }

  Future<void> _load() async {
    final store = Services.read(context).twitchChatSettings;
    try {
      final s = await store.ensureLoaded();
      if (mounted) setState(() => _settings = s);
    } catch (e) {
      debugPrint('Twitch chat settings: $e');
    }
  }

  Future<void> _signIn({required bool chat}) async {
    final account = await showTwitchSignIn(context, chat: chat);
    if (account != null && mounted) widget.onReopen?.call();
  }

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final tv = FormFactorScope.of(context) == FormFactor.tv;
    final canSignIn = services.twitch.backend.canSignIn;
    return ValueListenableBuilder<bool>(
      valueListenable: services.twitchChatPauseAnimations,
      builder: (context, paused, _) => TwitchChatPanel(
        feed: widget.feed,
        pauseAnimations: paused || MediaQuery.disableAnimationsOf(context),
        onPauseAnimationsChanged: services.setTwitchChatPauseAnimations,
        settings: _settings,
        onSettingsChanged: (s) {
          _settings = s;
          unawaited(services.twitchChatSettings.save(s));
        },
        checkRule: coreChatRuleCheck,
        onSignIn: canSignIn && widget.onReopen != null
            ? () => _signIn(chat: true)
            : null,
        onAllowSending: canSignIn && widget.onReopen != null
            ? () => _signIn(chat: true)
            : null,
        // A TV has no keyboard: reading only.
        showInput: !tv,
      ),
    );
  }
}
