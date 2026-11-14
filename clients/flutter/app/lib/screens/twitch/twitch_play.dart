// SPDX-License-Identifier: MPL-2.0
//
// Playing from the "Streams" section (TWI-013) by the TWI-004 decision:
// full builds with the signed config's permission play natively (player
// UI: TWI-014), store builds and a switched-off flag use the official
// embed (TWI-005), and without either player we offer "Open on Twitch".
// Sub-only VODs always go to Twitch (research 06 §2.4). Also the Device
// Code Flow sign-in dialog.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../../config.dart';
import '../../features/twitch/chat/chat_panel.dart';
import '../../l10n/l10n.dart';
import '../../services/app_services.dart';
import '../../twitch/twitch_backend.dart';
import '../../twitch/twitch_models.dart';
import '../../twitch/twitch_service.dart';
import '../../widgets/qr_code.dart';
import 'twitch_widgets.dart';

Future<void> playTwitch(BuildContext context, TwitchPlayTarget target) async {
  final twitch = Services.read(context).twitch;
  final l = context.l10n;
  switch (twitch.playRoute) {
    case TwitchPlayRoute.native:
      await twitch.nativePlayer!(context, target);
    case TwitchPlayRoute.embed:
      await twitch.embedPlayer!(context, target);
    case TwitchPlayRoute.openOnTwitch:
      await showOpenOnTwitch(context, target.url, message: l.twitchPlayerLater);
  }
}

/// The live chat of [login] (TWI-012): anonymous, read-only; the demo
/// build shows the made-up chat.
Future<void> openTwitchChat(BuildContext context, String login) =>
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => TwitchChatPage(
          channel: login,
          demoRate: AppConfig.demo ? 20 : null,
        ),
      ),
    );

Future<void> playTwitchVideo(BuildContext context, TwitchVideo video) {
  if (video.subOnly) {
    return showOpenOnTwitch(
      context,
      video.url,
      message: context.l10n.twitchSubOnlyMessage,
    );
  }
  return playTwitch(
    context,
    TwitchPlayTarget.vod(
      video.id,
      login: video.login,
      title: video.title,
      chapters: video.chapters,
      startAt: video.progress == null
          ? null
          : Duration(milliseconds: video.progress!.positionMs),
    ),
  );
}

/// Device Code Flow: the code, a QR code of the activation page, polling
/// until the user confirms. Returns the account or null.
Future<TwitchAccount?> showTwitchSignIn(BuildContext context) =>
    showDialog<TwitchAccount>(
      context: context,
      builder: (_) => const _SignInDialog(),
    );

class _SignInDialog extends StatefulWidget {
  const _SignInDialog();

  @override
  State<_SignInDialog> createState() => _SignInDialogState();
}

class _SignInDialogState extends State<_SignInDialog> {
  TwitchDeviceLogin? _login;
  bool _failed = false;
  Timer? _timer;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _start());
  }

  Future<void> _start() async {
    final twitch = Services.read(context).twitch;
    try {
      final login = await twitch.startLogin();
      if (!mounted) return;
      setState(() => _login = login);
      _timer = Timer.periodic(login.interval, (_) => _poll());
    } on TwitchUnavailable {
      if (mounted) setState(() => _failed = true);
    }
  }

  Future<void> _poll() async {
    final twitch = Services.read(context).twitch;
    try {
      final account = await twitch.pollLogin();
      if (account != null && mounted) {
        _timer?.cancel();
        Navigator.of(context).pop(account);
      }
    } on TwitchUnavailable {
      _timer?.cancel();
      if (mounted) setState(() => _failed = true);
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final login = _login;
    return AlertDialog(
      key: const ValueKey('twitch-sign-in'),
      title: Text(l.twitchSignIn),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (_failed)
              Text(l.twitchSignInFailed)
            else if (login == null)
              const Padding(
                padding: EdgeInsets.all(24),
                child: CircularProgressIndicator(),
              )
            else ...[
              Text(
                l.twitchSignInSteps('twitch.tv/activate'), // l10n-ignore
                style: theme.textTheme.bodyMedium,
              ),
              const SizedBox(height: 12),
              SelectableText(
                login.userCode,
                key: const ValueKey('twitch-user-code'),
                style: theme.textTheme.headlineMedium?.copyWith(
                  letterSpacing: 4,
                  fontWeight: FontWeight.w700,
                ),
              ),
              const SizedBox(height: 12),
              QrCodeView(data: login.verificationUri, size: 160),
              const SizedBox(height: 12),
              Text(l.twitchSignInWaiting, style: theme.textTheme.bodySmall),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          autofocus: true,
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.twitchClose),
        ),
      ],
    );
  }
}
