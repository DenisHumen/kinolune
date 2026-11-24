// SPDX-License-Identifier: MPL-2.0
//
// Twitch chat model of the UI (TWI-012), independent of the Rust bindings
// so widgets and the controller are tested with plain Dart feeds.

import 'dart:async';

import 'chat_settings.dart';

/// A badge in front of a name. [imageKey] is empty when there is no image
/// (anonymous chat without Helix): the view draws a mark for well-known
/// sets ([setId]).
class ChatBadge {
  const ChatBadge({
    required this.setId,
    this.version = '1',
    this.imageKey = '',
    this.title = '',
  });

  final String setId;
  final String version;
  final String imageKey;
  final String title;
}

/// Text, or an emote ([emoteKey] set, [text] is its code).
class ChatFragment {
  const ChatFragment.text(this.text) : emoteKey = null, overlay = false;
  const ChatFragment.emote(this.text, String key, {this.overlay = false})
    : emoteKey = key;

  final String text;
  final String? emoteKey;

  /// A zero-width emote (7TV, BTTV, TWI-023): drawn over the previous
  /// emote instead of after it.
  final bool overlay;

  bool get isEmote => emoteKey != null;
}

/// What moderation did to a line.
enum ChatLineFate { shown, deleted, timedOut, banned }

/// Kind of a line.
enum ChatLineKind {
  /// A user's message (possibly with a sub/raid [ChatLine.noticeText]).
  message,

  /// "X was timed out / banned", "chat was cleared", server notices.
  system,
}

/// One line of the chat list.
class ChatLine {
  ChatLine({
    required this.id,
    this.kind = ChatLineKind.message,
    this.userId = '',
    this.login = '',
    this.displayName = '',
    this.color,
    this.badges = const [],
    this.fragments = const [],
    this.isAction = false,
    this.replyTo,
    this.noticeText,
    this.highlighted = false,
    this.firstMessage = false,
    this.mention = false,
    this.system,
  });

  /// A system line built by the controller.
  ChatLine.system(this.id, ChatSystemText this.system)
    : kind = ChatLineKind.system,
      userId = '',
      login = '',
      displayName = '',
      color = null,
      badges = const [],
      fragments = const [],
      isAction = false,
      replyTo = null,
      noticeText = null,
      highlighted = false,
      firstMessage = false,
      mention = false;

  final String id;
  final ChatLineKind kind;
  final String userId;
  final String login;
  final String displayName;

  /// `#RRGGBB` chosen by the user, before contrast correction.
  final String? color;
  final List<ChatBadge> badges;
  final List<ChatFragment> fragments;
  final bool isAction;
  final String? replyTo;
  final String? noticeText;
  final bool highlighted;
  final bool firstMessage;

  /// Names the signed-in user or a highlight keyword (TWI-024).
  final bool mention;

  /// Text of a system line, localised by the view.
  final ChatSystemText? system;

  /// Moderation state; changes in place, the controller notifies.
  ChatLineFate fate = ChatLineFate.shown;

  /// Name as shown: display name, or login when the display name is in
  /// another script than the login (Twitch shows both then).
  String get shownName {
    if (displayName.isEmpty) return login;
    if (login.isEmpty || displayName.toLowerCase() == login) return displayName;
    return '$displayName ($login)';
  }
}

/// System texts the view localises.
sealed class ChatSystemText {
  const ChatSystemText();
}

class ChatClearedText extends ChatSystemText {
  const ChatClearedText();
}

class UserTimedOutText extends ChatSystemText {
  const UserTimedOutText(this.login, this.seconds);
  final String login;
  final int seconds;
}

class UserBannedText extends ChatSystemText {
  const UserBannedText(this.login);
  final String login;
}

/// A server notice, already in the user's language or Twitch's.
class ServerNoticeText extends ChatSystemText {
  const ServerNoticeText(this.text);
  final String text;
}

/// An image on disk.
class ChatImage {
  const ChatImage({
    required this.key,
    required this.path,
    required this.animated,
  });

  final String key;
  final String path;
  final bool animated;
}

/// Connection state of a feed.
enum ChatConnection { connecting, connected, reconnecting, stopped }

/// One change from a feed.
sealed class ChatUpdate {
  const ChatUpdate();
}

class LineAdded extends ChatUpdate {
  const LineAdded(this.line);
  final ChatLine line;
}

class LineDeleted extends ChatUpdate {
  const LineDeleted(this.messageId);
  final String messageId;
}

/// All lines of a user removed: [banned], or a timeout of [seconds], or a
/// plain clear (neither).
class UserCleared extends ChatUpdate {
  const UserCleared({
    required this.userId,
    required this.login,
    this.seconds,
    this.banned = false,
  });
  final String userId;
  final String login;
  final int? seconds;
  final bool banned;
}

class ChatCleared extends ChatUpdate {
  const ChatCleared();
}

class ServerNotice extends ChatUpdate {
  const ServerNotice(this.text);
  final String text;
}

class ConnectionChanged extends ChatUpdate {
  const ConnectionChanged(this.state);
  final ChatConnection state;
}

class ImageReady extends ChatUpdate {
  const ImageReady(this.image);
  final ChatImage image;
}

/// Room modes, full current state (TWI-024).
class RoomModesChanged extends ChatUpdate {
  const RoomModesChanged(this.modes);
  final ChatRoomModes modes;
}

/// Slow, subscribers-only, emote-only, followers-only and unique chat.
class ChatRoomModes {
  const ChatRoomModes({
    this.emoteOnly = false,
    this.subscribersOnly = false,
    this.uniqueChat = false,
    this.slowSeconds = 0,
    this.followersOnlyMinutes,
  });

  final bool emoteOnly;
  final bool subscribersOnly;
  final bool uniqueChat;

  /// Seconds between messages; 0 when slow mode is off.
  final int slowSeconds;

  /// Minimum follow age in minutes (0: any follower); null when off.
  final int? followersOnlyMinutes;

  bool get any =>
      emoteOnly ||
      subscribersOnly ||
      uniqueChat ||
      slowSeconds > 0 ||
      followersOnlyMinutes != null;

  @override
  bool operator ==(Object other) =>
      other is ChatRoomModes &&
      other.emoteOnly == emoteOnly &&
      other.subscribersOnly == subscribersOnly &&
      other.uniqueChat == uniqueChat &&
      other.slowSeconds == slowSeconds &&
      other.followersOnlyMinutes == followersOnlyMinutes;

  @override
  int get hashCode => Object.hash(
    emoteOnly,
    subscribersOnly,
    uniqueChat,
    slowSeconds,
    followersOnlyMinutes,
  );
}

/// An emote the user can type (autocomplete).
class ChatEmote {
  const ChatEmote({
    required this.code,
    required this.key,
    this.channel = false,
  });

  final String code;

  /// Image key, as in [ChatFragment.emoteKey].
  final String key;

  /// From the channel's own set.
  final bool channel;
}

/// Outcome of [ChatFeed.send].
sealed class ChatSendResult {
  const ChatSendResult();
}

class ChatSent extends ChatSendResult {
  const ChatSent(this.messageId);
  final String messageId;
}

/// Twitch received the message but did not post it (slow mode,
/// subscribers-only, duplicate, AutoMod…).
class ChatDropped extends ChatSendResult {
  const ChatDropped({required this.code, required this.message});

  /// Twitch's code, e.g. `msg_slowmode`, `msg_subsonly`, `msg_duplicate`.
  final String code;

  /// Twitch's text (English).
  final String message;
}

enum ChatSendError { readOnly, signedOut, invalid, network, closed }

class ChatSendFailed extends ChatSendResult {
  const ChatSendFailed(this.error, [this.detail = '']);
  final ChatSendError error;
  final String detail;
}

/// A live chat: batches of updates (the core sends one every ~150 ms).
abstract class ChatFeed {
  /// Channel login.
  String get channel;

  /// `false` for the anonymous read-only chat.
  bool get canSend;

  /// Login of the signed-in user; null for the anonymous chat.
  String? get ownLogin;

  /// Emotes of the channel and the global set (empty without Helix).
  List<ChatEmote> get emotes;

  /// Sends [text] (TWI-024); only when [canSend].
  Future<ChatSendResult> send(String text, {String? replyTo});

  /// Applies viewer filters to new messages; returns the problems of
  /// word rules that were skipped.
  List<String> setFilters(ChatSettings settings);

  /// Batches until [close].
  Stream<List<ChatUpdate>> get updates;

  void close();
}

/// A feed driven by code: tests and previews. Sending answers with
/// [sendResult] and records the texts in [sent]; [setFilters] records the
/// settings in [filters].
class ManualChatFeed implements ChatFeed {
  ManualChatFeed({
    this.channel = 'test_channel',
    this.canSend = false,
    this.ownLogin,
    this.emotes = const [],
    this.sendResult = const ChatSent('sent-1'),
  });

  final _controller = StreamController<List<ChatUpdate>>();

  @override
  final String channel;

  @override
  final bool canSend;

  @override
  final String? ownLogin;

  @override
  List<ChatEmote> emotes;

  ChatSendResult sendResult;
  final sent = <String>[];
  ChatSettings? filters;

  @override
  Future<ChatSendResult> send(String text, {String? replyTo}) async {
    sent.add(text);
    return sendResult;
  }

  @override
  List<String> setFilters(ChatSettings settings) {
    filters = settings;
    return const [];
  }

  @override
  Stream<List<ChatUpdate>> get updates => _controller.stream;

  void add(List<ChatUpdate> batch) => _controller.add(batch);

  @override
  void close() => _controller.close();
}
