// SPDX-License-Identifier: MPL-2.0
//
// [ChatReplaySource] over the Rust core (`CoreChatReplay` of pa_core,
// TWI-016, full builds): GQL pages of a VOD's chat behind the remote
// config's `twitch.chat_replay`, or a TwitchDownloader file next to a
// downloaded VOD. The core renders badges and emotes (TWI-023) and applies
// the viewer's filters (TWI-024) like in the live chat.

import 'dart:async';

import 'package:pa_core/pa_core.dart';

import 'chat_model.dart';
import 'chat_settings.dart';
import 'core_chat_feed.dart';
import 'replay_feed.dart';

class CoreReplaySource implements ChatReplaySource {
  CoreReplaySource(this._replay, {String? channel})
    : channel = channel ?? _replay.channel() {
    _sub = _replay.images().listen(
      (batch) {
        for (final i in batch) {
          _images.add(ChatImage(key: i.key, path: i.path, animated: i.animated));
        }
      },
      onError: (Object _) {},
      onDone: _images.close,
    );
  }

  /// The online replay of [videoId]; null when the config keeps it off.
  /// [twitch] (signed in) gives badge images and channel emotes.
  static CoreReplaySource? online(
    CoreRemoteConfig config, {
    required String videoId,
    required String channel,
    String? channelId,
    required CoreChatOptions options,
    CoreTwitch? twitch,
  }) {
    if (!chatReplayOnlineAllowed(config: config)) return null;
    try {
      final replay = CoreChatReplay.online(
        config: config,
        videoId: videoId,
        channel: channel,
        channelId: channelId,
        options: options,
      );
      if (twitch != null) unawaited(replay.loadChannelAssets(twitch: twitch));
      return CoreReplaySource(replay, channel: channel);
    } on CoreChatError {
      return null;
    }
  }

  /// The chat file next to the media file [mediaPath], if there is one.
  static Future<CoreReplaySource?> besideFile(
    String mediaPath,
    CoreChatOptions options,
  ) async {
    final path = findChatReplayFile(mediaPath: mediaPath);
    if (path == null) return null;
    try {
      return CoreReplaySource(
        await CoreChatReplay.openFile(path: path, options: options),
      );
    } on CoreChatError {
      return null;
    }
  }

  final CoreChatReplay _replay;
  final _images = StreamController<ChatImage>.broadcast();
  late final StreamSubscription<List<CoreChatImage>> _sub;

  @override
  final String channel;

  @override
  Future<ReplayChunk> chunkAt(int offsetMs, {required int lookbackMs}) async {
    try {
      final c = await _replay.chunkAt(
        offsetMs: offsetMs,
        lookbackMs: lookbackMs,
      );
      return ReplayChunk(
        fromMs: c.fromMs.toInt(),
        toMs: c.toMs?.toInt(),
        lines: [
          for (final l in c.lines)
            ReplayLine(l.offsetMs.toInt(), coreChatLine(l.message)),
        ],
      );
    } on CoreChatError catch (e) {
      throw ChatReplayUnavailable(e.code, e.message);
    }
  }

  @override
  Stream<ChatImage> get images => _images.stream;

  @override
  List<String> setFilters(ChatSettings settings) {
    _replay.setThirdPartyEmotes(enabled: settings.thirdPartyEmotes);
    return _replay.setFilters(filters: coreChatFilters(settings, null));
  }

  @override
  void close() {
    _replay.close();
    unawaited(_sub.cancel());
    if (!_images.isClosed) unawaited(_images.close());
    _replay.dispose();
  }
}
