// SPDX-License-Identifier: MPL-2.0
//
// TWI-040: «Клип» on the live players (native and embed; store and full
// builds — it is the official Helix API). The first press asks Twitch for
// `clips:edit` with a new Device Code sign-in (minimal scopes, TWI-002);
// without a sign-in it goes to the sign-in. Twitch makes the clip from the
// last seconds of the stream (`POST /clips`), the core waits until it is
// there (`GET /clips?id=`), then the «Клип готов» card offers «Смотреть»,
// «Скопировать ссылку», «Поделиться» (phones: the system share menu) and
// «Открыть на Twitch» to trim and title it. A channel with clips off keeps
// the button off with the reason. On a TV the button is in the player's
// bar (the OSD menu), on a computer Alt+X makes a clip too (as on
// twitch.tv). VODs have no button: Helix clips only live streams.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../../l10n/l10n.dart';
import '../../platform/form_factor.dart';
import '../../services/app_services.dart';
import '../../services/external_links.dart';
import '../../twitch/twitch_models.dart';
import '../../widgets/qr_code.dart';
import '../../widgets/settings_tiles.dart' show copyLink;
import 'twitch_play.dart' show showTwitchSignIn;
import 'twitch_widgets.dart' show formatLength, showOpenOnTwitch;

/// Alt+X: a clip on a computer, as on twitch.tv.
bool isTwitchClipShortcut(KeyEvent event) =>
    event is KeyDownEvent &&
    event.logicalKey == LogicalKeyboardKey.keyX &&
    HardwareKeyboard.instance.isAltPressed;

/// The share channel of the Android shell (ShareBridge.kt).
const _shareChannel = MethodChannel('app.kinolune/share');

/// Hands [url] to the system share menu; false where there is none.
Future<bool> shareTwitchLink(String url) async {
  try {
    return await _shareChannel.invokeMethod<bool>('shareText', url) ?? false;
  } on MissingPluginException {
    return false;
  } on PlatformException {
    return false;
  }
}

/// Makes a clip of [login]'s live stream: sign-in with `clips:edit` when
/// needed, then the «Клип готов» card or a snack bar with the reason.
Future<void> makeTwitchClip(BuildContext context, String login) async {
  final twitch = Services.read(context).twitch;
  if (!twitch.canSignIn ||
      twitch.clipping(login) ||
      twitch.clipsOffReason(login) != null) {
    return;
  }
  final l = context.l10n;
  final messenger = ScaffoldMessenger.maybeOf(context);
  void snack(String text) {
    messenger
      ?..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(text)));
  }

  Future<bool> signIn() async {
    final account = await showTwitchSignIn(context, clips: true);
    return account != null && context.mounted && twitch.canMakeClips;
  }

  // First press: ask Twitch for clips:edit only now (TWI-002).
  if (!twitch.canMakeClips && !await signIn()) return;
  if (!context.mounted) return;
  snack(l.twitchClipMaking);
  var r = await twitch.makeClip(login);
  if (!context.mounted) return;
  if (r.outcome == TwitchClipOutcome.signInRequired ||
      r.outcome == TwitchClipOutcome.scopeRequired) {
    messenger?.hideCurrentSnackBar();
    if (!await signIn() || !context.mounted) return;
    snack(l.twitchClipMaking);
    r = await twitch.makeClip(login);
    if (!context.mounted) return;
  }
  messenger?.hideCurrentSnackBar();
  final clip = r.clip;
  switch (r.outcome) {
    case TwitchClipOutcome.ready when clip != null:
      await showTwitchClipReady(context, clip);
    case TwitchClipOutcome.disabled:
      snack(l.twitchClipsOff);
    case TwitchClipOutcome.offline:
      snack(l.twitchClipOffline);
    case TwitchClipOutcome.notReady:
      snack(l.twitchClipNotReady);
    case TwitchClipOutcome.rateLimited:
      snack(
        l.twitchClipRateLimited(
          r.retryAfter.inSeconds < 1 ? 30 : r.retryAfter.inSeconds,
        ),
      );
    case TwitchClipOutcome.signInRequired || TwitchClipOutcome.scopeRequired:
      break;
    case _:
      snack(l.twitchClipFailed);
  }
}

/// «Клип» on a live player. Nothing without a Twitch client id (no
/// sign-in possible, LEGAL-006).
class TwitchClipButton extends StatelessWidget {
  const TwitchClipButton({
    super.key,
    required this.login,
    this.compact = false,
  });

  final String login;

  /// An icon only (phones, the native player's narrow bar).
  final bool compact;

  @override
  Widget build(BuildContext context) {
    final twitch = Services.of(context).twitch;
    return ListenableBuilder(
      listenable: twitch,
      builder: (context, _) {
        if (!twitch.canSignIn) return const SizedBox.shrink();
        final l = context.l10n;
        if (twitch.clipping(login)) {
          return Padding(
            key: const ValueKey('twitch-clip-busy'),
            padding: const EdgeInsets.symmetric(horizontal: 12),
            child: Tooltip(
              message: l.twitchClipMaking,
              child: const SizedBox.square(
                dimension: 18,
                child: CircularProgressIndicator(
                  strokeWidth: 2,
                  color: Colors.white,
                ),
              ),
            ),
          );
        }
        final off = twitch.clipsOffReason(login) != null;
        final desktop = FormFactorScope.of(context) == FormFactor.desktop;
        final tooltip = off
            ? l.twitchClipsOff
            : desktop
            ? l.twitchClipShortcut
            : l.twitchClipTooltip;
        final onPressed = off
            ? null
            : () => unawaited(makeTwitchClip(context, login));
        const icon = Icon(Icons.movie_creation_outlined);
        if (compact) {
          return IconButton(
            key: const ValueKey('twitch-clip'),
            color: Colors.white,
            disabledColor: Colors.white38,
            tooltip: tooltip,
            onPressed: onPressed,
            icon: icon,
          );
        }
        return Tooltip(
          message: tooltip,
          child: TextButton.icon(
            key: const ValueKey('twitch-clip'),
            style: TextButton.styleFrom(
              foregroundColor: Colors.white,
              disabledForegroundColor: Colors.white38,
            ),
            onPressed: onPressed,
            icon: icon,
            label: Text(l.twitchClip),
          ),
        );
      },
    );
  }
}

/// The «Клип готов» card: phone, computer and TV.
Future<void> showTwitchClipReady(BuildContext context, TwitchNewClip clip) =>
    showDialog<void>(
      context: context,
      builder: (_) => TwitchClipReadyCard(clip: clip),
    );

class TwitchClipReadyCard extends StatelessWidget {
  const TwitchClipReadyCard({super.key, required this.clip});

  final TwitchNewClip clip;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final form = FormFactorScope.of(context);
    final tv = form == FormFactor.tv;
    final phone = form == FormFactor.phone;
    final details = [
      if (clip.broadcasterName.isNotEmpty) clip.broadcasterName,
      if (clip.duration > Duration.zero) formatLength(clip.duration),
    ].join(' · ');
    return AlertDialog(
      key: const ValueKey('twitch-clip-ready'),
      title: Text(l.twitchClipReady),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (clip.title.isNotEmpty)
              Text(clip.title, style: theme.textTheme.titleMedium),
            if (details.isNotEmpty)
              Text(details, style: theme.textTheme.bodySmall),
            const SizedBox(height: 12),
            // A TV has nothing to open or share with: the QR code.
            if (tv) ...[
              Center(child: QrCodeView(data: clip.url, size: 160)),
              const SizedBox(height: 8),
            ],
            SelectableText(
              clip.url,
              key: const ValueKey('twitch-clip-url'),
              style: theme.textTheme.bodySmall,
            ),
            const SizedBox(height: 8),
            Text(l.twitchClipEditHint, style: theme.textTheme.bodySmall),
          ],
        ),
      ),
      actionsOverflowButtonSpacing: 4,
      actions: [
        TextButton(
          key: const ValueKey('twitch-clip-watch'),
          autofocus: tv,
          onPressed: () => showOpenOnTwitch(context, clip.url),
          child: Text(l.twitchClipWatch),
        ),
        TextButton(
          key: const ValueKey('twitch-clip-copy'),
          onPressed: () => copyLink(context, clip.url),
          child: Text(l.twitchCopyLink),
        ),
        if (phone)
          TextButton(
            key: const ValueKey('twitch-clip-share'),
            onPressed: () async {
              if (!await shareTwitchLink(clip.url) && context.mounted) {
                await copyLink(context, clip.url);
              }
            },
            child: Text(l.twitchClipShare),
          ),
        TextButton(
          key: const ValueKey('twitch-clip-edit'),
          onPressed: () async {
            if (tv || !await openExternalLink(Uri.parse(clip.editUrl))) {
              if (context.mounted) {
                await showOpenOnTwitch(
                  context,
                  clip.editUrl,
                  message: l.twitchClipEditHint,
                );
              }
            }
          },
          child: Text(l.twitchOpenOnTwitch),
        ),
        TextButton(
          key: const ValueKey('twitch-clip-close'),
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.twitchClose),
        ),
      ],
    );
  }
}
