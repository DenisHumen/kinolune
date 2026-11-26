// SPDX-License-Identifier: MPL-2.0
//
// «Фолловить / Вы фолловите на Twitch» (TWI-033, ADR-NNNN). Helix can only
// read follows (`channels/followed`); following and unfollowing happen on
// Twitch itself. A press puts the channel into the Kinolune favourites
// (live status, TWI-009 notifications) and opens `twitch.tv/<login>` in the
// Twitch app or the browser, where the user is usually signed in already;
// Kinolune sees neither a password nor cookies. Coming back to the app (or
// closing the QR window on a TV, TWI-038) asks Helix again. Without a
// Twitch sign-in the button is the favourite toggle only.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../../l10n/l10n.dart';
import '../../platform/form_factor.dart';
import '../../services/app_services.dart';
import '../../services/external_links.dart';
import '../../twitch/twitch_models.dart';
import '../../twitch/twitch_service.dart';
import '../../widgets/focusable.dart';
import 'twitch_widgets.dart';

/// The follow button of [channel]: on the channel page, in the live card
/// menu and in the players.
class TwitchFollowButton extends StatefulWidget {
  const TwitchFollowButton({
    super.key,
    required this.channel,
    this.autofocus = false,
    this.compact = false,
  });

  final TwitchChannel channel;
  final bool autofocus;

  /// An icon button with a tooltip (player bars).
  final bool compact;

  @override
  State<TwitchFollowButton> createState() => _TwitchFollowButtonState();
}

class _TwitchFollowButtonState extends State<TwitchFollowButton> {
  var _busy = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final twitch = Services.of(context).twitch;
    // The first time the status is unknown: ask Helix once.
    if (twitch.account != null &&
        twitch.followsOnTwitch(widget.channel.login) == null) {
      unawaited(twitch.refreshFollowed());
    }
  }

  Future<void> _press(TwitchService twitch, bool? follows) async {
    if (_busy) return;
    setState(() => _busy = true);
    try {
      if (twitch.account == null) {
        await twitch.toggleFavorite(widget.channel);
      } else {
        await followOnTwitch(
          context,
          widget.channel,
          unfollow: follows == true,
        );
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final twitch = Services.of(context).twitch;
    return ListenableBuilder(
      listenable: twitch,
      builder: (context, _) {
        final login = widget.channel.login;
        final signedIn = twitch.account != null;
        final follows = twitch.followsOnTwitch(login);
        final fav = twitch.isFavorite(login);
        final String label;
        final IconData icon;
        if (!signedIn) {
          label = fav ? l.twitchInKinoluneFavorites : l.twitchAddFavorite;
          icon = fav ? Icons.star_rounded : Icons.star_outline_rounded;
        } else if (follows == true) {
          label = l.twitchFollowingOnTwitch;
          icon = Icons.favorite_rounded;
        } else {
          label = l.twitchFollow;
          icon = Icons.favorite_outline_rounded;
        }
        final onPressed = _busy ? null : () => _press(twitch, follows);
        if (widget.compact) {
          return IconButton(
            key: const ValueKey('twitch-follow'),
            tooltip: label,
            autofocus: widget.autofocus,
            icon: Icon(icon),
            onPressed: onPressed,
          );
        }
        return KButton(
          key: const ValueKey('twitch-follow'),
          autofocus: widget.autofocus,
          primary: signedIn && follows != true,
          icon: icon,
          label: label,
          onPressed: onPressed,
        );
      },
    );
  }
}

/// Follows (or, with [unfollow], unfollows) [channel] the only way Twitch
/// allows apps (ADR-NNNN): the favourite changes here, the follow itself
/// on `twitch.tv/<login>`. Then the status is asked again from Helix.
Future<void> followOnTwitch(
  BuildContext context,
  TwitchChannel channel, {
  required bool unfollow,
}) async {
  final twitch = Services.of(context).twitch;
  final l = context.l10n;
  final name = channel.displayName;
  if (unfollow) {
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        key: const ValueKey('twitch-unfollow-dialog'),
        title: Text(l.twitchUnfollowTitle(name)),
        content: Text(l.twitchUnfollowMessage),
        actions: [
          TextButton(
            autofocus: true,
            onPressed: () => Navigator.of(context).pop(false),
            child: Text(l.commonCancel),
          ),
          FilledButton(
            key: const ValueKey('twitch-unfollow-confirm'),
            onPressed: () => Navigator.of(context).pop(true),
            child: Text(l.twitchUnfollow),
          ),
        ],
      ),
    );
    if (ok != true || !context.mounted) return;
    await twitch.removeFavorite(channel.login);
  } else {
    await twitch.addFavorite(channel);
  }
  if (!context.mounted) return;
  final url = channel.url;
  final tv = FormFactorScope.of(context) == FormFactor.tv;
  final messenger = ScaffoldMessenger.maybeOf(context);
  if (!tv && await openExternalLink(Uri.parse(url))) {
    TwitchFollowReturn.watch(twitch, channel.login);
    messenger?.showSnackBar(
      SnackBar(
        content: Text(
          unfollow ? l.twitchUnfollowOpened : l.twitchFollowOpened(name),
        ),
      ),
    );
    return;
  }
  if (!context.mounted) return;
  // A TV (or nothing could open the link): the QR window; scan it with the
  // phone and press Follow there. Closing it asks Helix again.
  await showOpenOnTwitch(
    context,
    url,
    message: unfollow ? l.twitchUnfollowQrHint : l.twitchFollowQrHint,
  );
  await twitch.refreshFollowed(channel.login);
}

/// Asks Helix again when the user comes back from Twitch (the app is
/// resumed or its window gets the focus back). One pending check at a
/// time; it gives up after [maxWait].
class TwitchFollowReturn {
  TwitchFollowReturn._(this._twitch) {
    _listener = AppLifecycleListener(onResume: _back);
    _timeout = Timer(maxWait, _stop);
  }

  static const maxWait = Duration(minutes: 30);

  static TwitchFollowReturn? _pending;

  /// Is a check waiting (tests).
  static bool get pending => _pending != null;

  /// Drops the waiting check (tests, sign-out).
  static void cancel() => _pending?._stop();

  static void watch(TwitchService twitch, String login) {
    _pending?._stop();
    _pending = TwitchFollowReturn._(twitch);
  }

  final TwitchService _twitch;
  late final AppLifecycleListener _listener;
  late final Timer _timeout;

  void _back() {
    _stop();
    unawaited(_twitch.refreshFollowed());
  }

  void _stop() {
    _timeout.cancel();
    _listener.dispose();
    if (identical(_pending, this)) _pending = null;
  }
}
