// SPDX-License-Identifier: MPL-2.0
//
// The chat input (TWI-024, research 06 §3.3): enabled only when the feed
// can send (signed in with `user:write:chat`); a strip of room modes
// (slow, subscribers-, emote-, followers-only, unique) above it; @-mention
// autocomplete from recent chatters and emote autocomplete (`:code`, Tab)
// from the channel and global sets; a slow-mode countdown after a message;
// Twitch's reasons for a dropped message in the user's language.

import 'dart:async';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../../../l10n/l10n.dart';
import '../../../theme/kinolune_theme.dart';
import 'chat_autocomplete.dart';
import 'chat_controller.dart';
import 'chat_model.dart';
import 'chat_view.dart';

/// Twitch's limit.
const kChatMaxChars = 500;

/// Input and room-mode strip under the chat list.
class TwitchChatInput extends StatefulWidget {
  const TwitchChatInput({
    super.key,
    required this.controller,
    this.onSignIn,
    this.onAllowSending,
  });

  final ChatController controller;

  /// Signed out: the "Sign in" button (hidden when null).
  final VoidCallback? onSignIn;

  /// Signed in without the write permission: "Allow sending".
  final VoidCallback? onAllowSending;

  @override
  State<TwitchChatInput> createState() => TwitchChatInputState();
}

class TwitchChatInputState extends State<TwitchChatInput> {
  final _text = TextEditingController();
  late final _focus = FocusNode(onKeyEvent: _onKey);
  List<ChatSuggestion> _suggestions = const [];
  CompletionQuery? _query;
  var _selected = 0;
  var _sending = false;
  String? _error;
  var _wait = 0;
  Timer? _timer;

  ChatController get _c => widget.controller;
  ChatFeed get _feed => _c.feed;

  /// Seconds left before the next message (slow mode).
  int get waitSeconds => _wait;

  @override
  void initState() {
    super.initState();
    _c.addListener(_changed);
    _text.addListener(_typed);
  }

  @override
  void didUpdateWidget(TwitchChatInput old) {
    super.didUpdateWidget(old);
    if (old.controller != widget.controller) {
      old.controller.removeListener(_changed);
      _c.addListener(_changed);
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    _c.removeListener(_changed);
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  ChatRoomModes _modes = const ChatRoomModes();

  void _changed() {
    if (_c.roomModes != _modes) setState(() => _modes = _c.roomModes);
  }

  /// Puts `@name ` into the input (the "Mention" action on a line).
  void mention(String name) {
    final t = _text.text;
    final add = '@$name ';
    final next = t.isEmpty || t.endsWith(' ') ? '$t$add' : '$t $add';
    _text.value = TextEditingValue(
      text: next,
      selection: TextSelection.collapsed(offset: next.length),
    );
    _focus.requestFocus();
  }

  void _typed() {
    final sel = _text.selection;
    final q = sel.isCollapsed
        ? completionAt(_text.text, sel.baseOffset)
        : null;
    final s = q == null ? const <ChatSuggestion>[] : _suggest(q);
    if (_error != null || s.isNotEmpty || _suggestions.isNotEmpty) {
      setState(() {
        _error = null;
        _query = q;
        _suggestions = s;
        _selected = 0;
      });
    }
  }

  List<ChatSuggestion> _suggest(CompletionQuery q) => suggestionsFor(
    q,
    chatters: _c.recentChatters,
    emotes: _feed.emotes,
  );

  void _accept(ChatSuggestion s, [CompletionQuery? query]) {
    final q = query ?? _query;
    if (q == null) return;
    _text.value = applySuggestion(_text.value, q, s);
    setState(() {
      _suggestions = const [];
      _query = null;
    });
  }

  KeyEventResult _onKey(FocusNode node, KeyEvent e) {
    if (e is! KeyDownEvent && e is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    final k = e.logicalKey;
    if (k == LogicalKeyboardKey.tab) {
      if (_suggestions.isNotEmpty) {
        _accept(_suggestions[_selected]);
        return KeyEventResult.handled;
      }
      final sel = _text.selection;
      final q = sel.isCollapsed
          ? completionAt(_text.text, sel.baseOffset, tab: true)
          : null;
      final s = q == null ? const <ChatSuggestion>[] : _suggest(q);
      if (s.isNotEmpty) {
        _accept(s.first, q);
        return KeyEventResult.handled;
      }
      return KeyEventResult.ignored;
    }
    if (_suggestions.isEmpty) return KeyEventResult.ignored;
    if (k == LogicalKeyboardKey.arrowDown || k == LogicalKeyboardKey.arrowUp) {
      final d = k == LogicalKeyboardKey.arrowDown ? 1 : -1;
      setState(() {
        _selected = (_selected + d) % _suggestions.length;
      });
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.escape) {
      setState(() => _suggestions = const []);
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  bool get _ownChannel =>
      _feed.ownLogin?.toLowerCase() == _feed.channel.toLowerCase();

  Future<void> _send() async {
    if (_suggestions.isNotEmpty) {
      _accept(_suggestions[_selected]);
      return;
    }
    final text = _text.text.trim();
    if (text.isEmpty || _sending || _wait > 0 || !_feed.canSend) return;
    setState(() {
      _sending = true;
      _error = null;
    });
    final l = context.l10n;
    final result = await _feed.send(text);
    if (!mounted) return;
    setState(() {
      _sending = false;
      switch (result) {
        case ChatSent():
          _text.clear();
          final slow = _c.roomModes.slowSeconds;
          if (slow > 0 && !_ownChannel) _startWait(slow);
        case ChatDropped(:final code, :final message):
          _error = _dropText(l, code, message);
          if (code == 'msg_slowmode' && _c.roomModes.slowSeconds > 0) {
            _startWait(_c.roomModes.slowSeconds);
          }
        case ChatSendFailed(:final error):
          _error = switch (error) {
            ChatSendError.signedOut ||
            ChatSendError.readOnly => l.twitchChatSignInAgain,
            _ => l.twitchChatSendFailed,
          };
      }
    });
    _focus.requestFocus();
  }

  void _startWait(int seconds) {
    _timer?.cancel();
    _wait = seconds;
    _timer = Timer.periodic(const Duration(seconds: 1), (t) {
      if (!mounted) return t.cancel();
      setState(() => _wait--);
      if (_wait <= 0) t.cancel();
    });
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final style = ChatStyle.of(context);
    final small = style.body.copyWith(
      fontSize: style.fontSize * .85,
      color: style.secondary,
    );
    final modes = _modeTexts(l, _c.roomModes);
    final children = <Widget>[];
    if (modes.isNotEmpty) {
      children.add(
        Container(
          key: const ValueKey('twitch-chat-modes'),
          width: double.infinity,
          color: KColors.accent.withValues(alpha: .12),
          padding: style.padding.add(
            EdgeInsets.symmetric(vertical: style.lineGap),
          ),
          child: Text(modes.join(' · '), style: small),
        ),
      );
    }
    if (_suggestions.isNotEmpty) {
      children.add(_suggestionList(style));
    }
    children.add(
      Padding(
        padding: style.padding.add(
          EdgeInsets.symmetric(vertical: style.lineGap),
        ),
        child: _feed.canSend ? _field(l, style) : _disabled(l, small),
      ),
    );
    final notice = _error ?? (_wait > 0 ? l.twitchChatSlowWait(_wait) : null);
    if (notice != null) {
      children.add(
        Padding(
          padding: style.padding.add(
            EdgeInsets.only(bottom: style.lineGap),
          ),
          child: Text(
            notice,
            key: const ValueKey('twitch-chat-input-notice'),
            style: small.copyWith(
              color: _error != null
                  ? Theme.of(context).colorScheme.error
                  : style.secondary,
            ),
          ),
        ),
      );
    }
    return ColoredBox(
      color: style.background,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: children,
      ),
    );
  }

  Widget _disabled(AppLocalizations l, TextStyle small) {
    final signedIn = _feed.ownLogin != null;
    final action = signedIn ? widget.onAllowSending : widget.onSignIn;
    return Row(
      children: [
        Expanded(
          child: Text(
            signedIn ? l.twitchChatSendingOff : l.twitchChatSignInToChat,
            style: small,
          ),
        ),
        if (action != null)
          TextButton(
            onPressed: action,
            child: Text(signedIn ? l.twitchChatAllowSending : l.twitchChatSignIn),
          ),
      ],
    );
  }

  Widget _field(AppLocalizations l, ChatStyle style) {
    final blocked = _sending || _wait > 0;
    return Row(
      children: [
        Expanded(
          child: TextField(
            key: const ValueKey('twitch-chat-input'),
            controller: _text,
            focusNode: _focus,
            maxLength: kChatMaxChars,
            maxLengthEnforcement: MaxLengthEnforcement.enforced,
            buildCounter:
                (_, {required currentLength, required isFocused, maxLength}) =>
                    currentLength > kChatMaxChars - 100
                    ? Text('$currentLength/$maxLength')
                    : null,
            textInputAction: TextInputAction.send,
            onSubmitted: (_) => _send(),
            style: style.body,
            decoration: InputDecoration(
              hintText: l.twitchChatInputHint,
              isDense: true,
            ),
          ),
        ),
        IconButton(
          key: const ValueKey('twitch-chat-send'),
          tooltip: l.twitchChatSend,
          onPressed: blocked ? null : _send,
          icon: const Icon(Icons.send_rounded),
        ),
      ],
    );
  }

  Widget _suggestionList(ChatStyle style) => ConstrainedBox(
    constraints: BoxConstraints(maxHeight: style.fontSize * 2.4 * 5),
    child: Material(
      color: KColors.bgElevated2,
      child: ListView.builder(
        key: const ValueKey('twitch-chat-suggestions'),
        shrinkWrap: true,
        padding: EdgeInsets.zero,
        itemCount: _suggestions.length,
        itemBuilder: (context, i) {
          final s = _suggestions[i];
          final emote = s.emote;
          final img = emote == null ? null : _c.image(emote.key);
          return ListTile(
            dense: true,
            selected: i == _selected,
            leading: emote == null
                ? const Icon(Icons.alternate_email_rounded)
                : (img == null
                      ? const Icon(Icons.emoji_emotions_outlined)
                      : Image(
                          image: ResizeImage(
                            FileImage(File(img.path)),
                            height: (style.emoteHeight *
                                    MediaQuery.devicePixelRatioOf(context))
                                .round(),
                            policy: ResizeImagePolicy.fit,
                          ),
                          height: style.emoteHeight,
                        )),
            title: Text(s.label, style: style.body),
            onTap: () => _accept(s),
          );
        },
      ),
    ),
  );
}

/// Room modes as short texts, in a fixed order.
List<String> _modeTexts(AppLocalizations l, ChatRoomModes m) => [
  if (m.slowSeconds > 0) l.twitchChatModeSlow(m.slowSeconds),
  if (m.subscribersOnly) l.twitchChatModeSubsOnly,
  if (m.emoteOnly) l.twitchChatModeEmoteOnly,
  if (m.followersOnlyMinutes case final min?)
    min > 0 ? l.twitchChatModeFollowersFor(min) : l.twitchChatModeFollowersOnly,
  if (m.uniqueChat) l.twitchChatModeUnique,
];

String _dropText(AppLocalizations l, String code, String message) {
  if (code == 'msg_slowmode') return l.twitchChatDropSlow;
  if (code == 'msg_subsonly') return l.twitchChatDropSubsOnly;
  if (code == 'msg_emoteonly') return l.twitchChatDropEmoteOnly;
  if (code.startsWith('msg_followersonly')) return l.twitchChatDropFollowersOnly;
  if (code == 'msg_duplicate' || code == 'msg_r9k') {
    return l.twitchChatDropDuplicate;
  }
  return l.twitchChatDropOther(message.isEmpty ? code : message);
}
