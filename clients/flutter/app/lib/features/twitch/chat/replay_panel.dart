// SPDX-License-Identifier: MPL-2.0
//
// The chat panel of a Twitch recording (TWI-016): the live chat panel
// (TWI-012) over a [ReplayChatFeed], read-only, with the delay buttons
// («Показывать чат раньше / позже», ±1 s, up to ±30 s) in its header,
// a spinner until the first lines and «Чат записи недоступен» when Twitch
// gives no chat (the video plays on).

import 'package:material_ui/material_ui.dart';

import '../../../l10n/l10n.dart';
import '../../../theme/kinolune_theme.dart';
import 'chat_host.dart';
import 'replay_feed.dart';

class TwitchReplayChatPanel extends StatelessWidget {
  const TwitchReplayChatPanel({super.key, required this.feed});

  final ReplayChatFeed feed;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return Stack(
      fit: StackFit.expand,
      children: [
        ValueListenableBuilder<Duration>(
          valueListenable: feed.delay,
          builder: (context, delay, _) {
            final s = delay.inSeconds;
            return TwitchAppChatPanel(
              key: const ValueKey('twitch-replay-chat'),
              feed: feed,
              showInput: false,
              title: l.twitchReplayChatTitle,
              subtitle: s == 0
                  ? l.twitchReplayChatInSync
                  : s > 0
                  ? l.twitchReplayChatLater(s)
                  : l.twitchReplayChatSooner(-s),
              actions: [
                IconButton(
                  key: const ValueKey('twitch-replay-sooner'),
                  tooltip: l.twitchReplayChatShowSooner,
                  icon: const Icon(Icons.remove_rounded),
                  onPressed: () => feed.setDelay(delay - kChatReplayDelayStep),
                ),
                if (s != 0)
                  IconButton(
                    key: const ValueKey('twitch-replay-sync'),
                    tooltip: l.twitchReplayChatResetDelay,
                    icon: const Icon(Icons.sync_rounded),
                    onPressed: () => feed.setDelay(Duration.zero),
                  ),
                IconButton(
                  key: const ValueKey('twitch-replay-later'),
                  tooltip: l.twitchReplayChatShowLater,
                  icon: const Icon(Icons.add_rounded),
                  onPressed: () => feed.setDelay(delay + kChatReplayDelayStep),
                ),
              ],
            );
          },
        ),
        ValueListenableBuilder<ChatReplayStatus>(
          valueListenable: feed.status,
          builder: (context, status, _) => switch (status) {
            ChatReplayStatus.ready => const SizedBox.shrink(),
            ChatReplayStatus.loading => Center(
              key: const ValueKey('twitch-replay-loading'),
              child: Semantics(
                label: l.twitchReplayChatLoading,
                child: const CircularProgressIndicator(),
              ),
            ),
            ChatReplayStatus.unavailable => _Unavailable(),
          },
        ),
      ],
    );
  }
}

class _Unavailable extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = KDensity.of(context);
    return ColoredBox(
      key: const ValueKey('twitch-replay-unavailable'),
      color: KColors.bgElevated,
      child: Center(
        child: Padding(
          padding: EdgeInsets.all(d.gap),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                Icons.speaker_notes_off_outlined,
                color: KColors.textSecondary,
                size: d.type.headline * 2,
              ),
              SizedBox(height: d.gap / 2),
              Text(
                l.twitchReplayChatUnavailable,
                textAlign: TextAlign.center,
                style: TextStyle(
                  fontSize: d.type.headline,
                  fontWeight: FontWeight.w600,
                  color: KColors.textPrimary,
                ),
              ),
              SizedBox(height: d.gap / 4),
              Text(
                l.twitchReplayChatUnavailableHint,
                textAlign: TextAlign.center,
                style: TextStyle(
                  fontSize: d.type.caption,
                  color: KColors.textSecondary,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
