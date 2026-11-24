// SPDX-License-Identifier: MPL-2.0
//
// The VOD chat replay of the native player over the core (TWI-016, full
// builds): online through GQL when the signed remote config turns
// `twitch.chat_replay` on; the player shows no replay otherwise.

import 'package:material_ui/material_ui.dart';
import 'package:pa_core/pa_core.dart' as core;

import '../features/twitch/chat/chat_panel.dart' show chatOptions;
import '../features/twitch/chat/core_replay_source.dart';
import '../features/twitch/chat/replay_feed.dart';
import '../screens/twitch/twitch_player_screen.dart' show TwitchReplayFactory;
import '../services/app_services.dart';
import 'core_twitch_backend.dart';

/// Opens the chat replay of a VOD target with [config]'s decision.
TwitchReplayFactory coreTwitchReplay(core.CoreRemoteConfig config) =>
    (context, target) async {
      final videoId = target.videoId;
      if (videoId == null) return null;
      final services = Services.read(context);
      var thirdParty = true;
      try {
        thirdParty = (await services.twitchChatSettings.ensureLoaded())
            .thirdPartyEmotes;
      } catch (e) {
        debugPrint('Twitch chat settings: $e');
      }
      if (!context.mounted) return null;
      final options = await chatOptions(
        context,
        animate:
            !services.twitchChatPauseAnimations.value &&
            !MediaQuery.disableAnimationsOf(context),
        thirdPartyEmotes: thirdParty,
      );
      final login = target.login ?? '';
      final backend = services.twitch.backend;
      final source = CoreReplaySource.online(
        config,
        videoId: videoId,
        channel: login,
        channelId: login.isEmpty
            ? null
            : services.twitch.channelOf(login).userId,
        options: options,
        twitch: backend is CoreTwitchBackend ? backend.coreTwitch : null,
      );
      return source == null ? null : ReplayChatFeed(source);
    };
