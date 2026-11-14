// SPDX-License-Identifier: MPL-2.0
//
// ChatFeed over the Rust core (`CoreTwitchChat` of pa_core, TWI-012): the
// core connects, renders badges and emote fragments, downloads images into
// its 24-hour chat cache and sends batches every ~150 ms. Signed in
// (TWI-024) it reads through EventSub, sends through Helix and applies the
// viewer's filters before rendering.

import 'dart:async';

import 'package:pa_core/pa_core.dart';

import 'chat_model.dart';
import 'chat_settings.dart';

class CoreChatFeed implements ChatFeed {
  CoreChatFeed._(this._chat) : channel = _chat.channel() {
    _sub = _chat.events().listen(
      (batch) {
        final out = <ChatUpdate>[];
        for (final e in batch) {
          final u = _convert(e);
          if (u != null) out.add(u);
        }
        if (out.isNotEmpty) _out.add(out);
      },
      onError: (Object e) =>
          _out.add(const [ConnectionChanged(ChatConnection.stopped)]),
      onDone: _out.close,
    );
  }

  /// Read-only chat of [channel] without sign-in (anonymous IRC).
  factory CoreChatFeed.anonymous(String channel, CoreChatOptions options) =>
      CoreChatFeed._(
        CoreTwitchChat.anonymous(channel: channel, options: options),
      );

  /// Made-up chat at [ratePerSecond] with local images (demo, benchmarks).
  factory CoreChatFeed.demo(int ratePerSecond, CoreChatOptions options) =>
      CoreChatFeed._(
        CoreTwitchChat.demo(ratePerSecond: ratePerSecond, options: options),
      );

  /// The signed-in chat of [channel] (TWI-024): EventSub, the channel's
  /// emotes and badges; sending when the sign-in allows it.
  factory CoreChatFeed.signedIn(
    CoreTwitch twitch, {
    required String channel,
    required String broadcasterId,
    required CoreChatOptions options,
  }) => CoreChatFeed._(
    CoreTwitchChat.signedIn(
      twitch: twitch,
      channel: channel,
      broadcasterId: broadcasterId,
      options: options,
    ),
  );

  final CoreTwitchChat _chat;
  final _out = StreamController<List<ChatUpdate>>();
  late final StreamSubscription<List<CoreChatEvent>> _sub;

  @override
  final String channel;

  @override
  bool get canSend => _chat.canSend();

  @override
  String? get ownLogin => _chat.ownLogin();

  @override
  List<ChatEmote> get emotes => [
    for (final e in _chat.emotes())
      ChatEmote(code: e.code, key: e.key, channel: e.channel),
  ];

  @override
  Future<ChatSendResult> send(String text, {String? replyTo}) async {
    try {
      final r = await _chat.send(text: text, replyTo: replyTo);
      if (r.sent) return ChatSent(r.messageId);
      return ChatDropped(
        code: r.dropCode ?? '',
        message: r.dropMessage ?? '',
      );
    } on CoreChatError catch (e) {
      return ChatSendFailed(switch (e.code) {
        'read_only' => ChatSendError.readOnly,
        'signed_out' => ChatSendError.signedOut,
        'invalid' => ChatSendError.invalid,
        'closed' => ChatSendError.closed,
        _ => ChatSendError.network,
      }, e.message);
    }
  }

  @override
  List<String> setFilters(ChatSettings settings) {
    final own = ownLogin;
    return _chat.setFilters(
      filters: CoreChatFilters(
        blocked: settings.blocked,
        words: [
          for (final w in settings.words)
            CoreChatWordRule(pattern: w.pattern, regex: w.regex, hide: w.hide),
        ],
        ownLogin: settings.highlightOwnName ? own : null,
        highlightKeywords: settings.highlightKeywords,
        hideBots: settings.hideBots,
        extraBots: settings.extraBots,
      ),
    );
  }

  @override
  Stream<List<ChatUpdate>> get updates => _out.stream;

  @override
  void close() {
    _chat.close();
    _sub.cancel();
    if (!_out.isClosed) _out.close();
  }
}

ChatUpdate? _convert(CoreChatEvent e) {
  switch (e.kind) {
    case CoreChatEventKind.message:
      final m = e.message;
      return m == null ? null : LineAdded(_line(m));
    case CoreChatEventKind.deleted:
      final id = e.targetId;
      return id == null ? null : LineDeleted(id);
    case CoreChatEventKind.userCleared:
      return UserCleared(
        userId: e.targetId ?? '',
        login: e.targetLogin ?? '',
        seconds: e.timeoutSecs,
        banned: e.banned,
      );
    case CoreChatEventKind.chatCleared:
      return const ChatCleared();
    case CoreChatEventKind.notice:
      final t = e.text;
      return t == null ? null : ServerNotice(t);
    case CoreChatEventKind.status:
      return ConnectionChanged(switch (e.status) {
        CoreChatStatus.connected ||
        CoreChatStatus.joined => ChatConnection.connected,
        CoreChatStatus.reconnecting => ChatConnection.reconnecting,
        CoreChatStatus.stopped => ChatConnection.stopped,
        null => ChatConnection.connecting,
      });
    case CoreChatEventKind.image:
      final i = e.image;
      return i == null
          ? null
          : ImageReady(
              ChatImage(key: i.key, path: i.path, animated: i.animated),
            );
    case CoreChatEventKind.imageFailed:
      // The view keeps the emote code as text.
      return null;
    case CoreChatEventKind.roomModes:
      final r = e.room;
      return r == null
          ? null
          : RoomModesChanged(
              ChatRoomModes(
                emoteOnly: r.emoteOnly,
                subscribersOnly: r.subscribersOnly,
                uniqueChat: r.uniqueChat,
                slowSeconds: r.slowSecs,
                followersOnlyMinutes: r.followersOnlyMins,
              ),
            );
  }
}

ChatLine _line(CoreChatMessage m) => ChatLine(
  id: m.id,
  userId: m.userId,
  login: m.login,
  displayName: m.displayName,
  color: m.color,
  badges: [
    for (final b in m.badges)
      ChatBadge(
        setId: b.setId,
        version: b.version,
        imageKey: b.key,
        title: b.title,
      ),
  ],
  fragments: [
    for (final f in m.fragments)
      f.emoteKey == null
          ? ChatFragment.text(f.text)
          : ChatFragment.emote(f.text, f.emoteKey!),
  ],
  isAction: m.isAction,
  replyTo: m.replyTo,
  noticeText: m.noticeText,
  highlighted: m.highlighted,
  firstMessage: m.firstMessage,
  mention: m.mention,
);
