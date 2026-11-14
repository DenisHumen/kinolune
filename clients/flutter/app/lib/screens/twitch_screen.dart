// SPDX-License-Identifier: MPL-2.0

import 'package:material_ui/material_ui.dart';

import '../config.dart';
import '../features/twitch/chat/chat_panel.dart';
import '../l10n/l10n.dart';
import '../widgets/cards.dart';

/// Twitch section (UI-002 placeholder). Until TWI-013 brings the real
/// section, it offers the read-only chat of a channel (TWI-012), which
/// works without signing in.
class TwitchScreen extends StatelessWidget {
  const TwitchScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        PageTitle(l.navTwitch),
        const Padding(
          padding: EdgeInsets.symmetric(horizontal: 24),
          child: OpenChatForm(),
        ),
        Expanded(
          child: EmptyState(
            icon: Icons.live_tv_outlined,
            title: l.twitchComingTitle,
            message: l.twitchComingMessage,
          ),
        ),
      ],
    );
  }
}

/// Twitch logins: 1–25 letters, digits and underscores.
final _login = RegExp(r'^[A-Za-z0-9_]{1,25}$');

/// Channel field + "Open chat".
class OpenChatForm extends StatefulWidget {
  const OpenChatForm({super.key});

  @override
  State<OpenChatForm> createState() => _OpenChatFormState();
}

class _OpenChatFormState extends State<OpenChatForm> {
  final _text = TextEditingController();
  var _invalid = false;

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  void _open() {
    final channel = _text.text.trim().replaceFirst(RegExp('^[@#]'), '');
    // Demo mode: the made-up chat, nothing goes to Twitch.
    final demo = AppConfig.demo;
    if (!demo && !_login.hasMatch(channel)) {
      setState(() => _invalid = true);
      return;
    }
    setState(() => _invalid = false);
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => TwitchChatPage(
          channel: channel.toLowerCase(),
          demoRate: demo ? 20 : null,
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Expanded(
          child: TextField(
            controller: _text,
            onSubmitted: (_) => _open(),
            textInputAction: TextInputAction.go,
            autocorrect: false,
            decoration: InputDecoration(
              labelText: l.twitchChatChannelLabel,
              helperText: l.twitchChatAnonymousHint,
              errorText: _invalid ? l.twitchChatInvalidChannel : null,
            ),
          ),
        ),
        const SizedBox(width: 12),
        Padding(
          padding: const EdgeInsets.only(top: 8),
          child: FilledButton(onPressed: _open, child: Text(l.twitchChatOpen)),
        ),
      ],
    );
  }
}
